//! The voxel grid (voxel-grid.ts) as the TS side hands it over, tiles of
//! grids, and the voxel's faces (tables.rs).

use std::sync::Arc;

use crate::tables::VOXEL_FACES;

/** Kinds of voxel faces. */
pub const FACE_KINDS: usize = 12;
/** Faces of the view (FACES in view.ts). */
pub const VIEW_FACES: usize = 12;

/**
 * The voxel's faces in a face's lattice coordinates, the same for every
 * face (tables.rs): kind k's neighbour offset (a, b, c) and its normal in
 * face coordinates.
 */
pub struct Geometry {
    pub offsets: [[i32; 3]; FACE_KINDS],
    pub normals: [[f64; 3]; FACE_KINDS],
}

/** A face's lattice vectors R, U, F (integer, see Face in view.ts). */
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Lattice {
    pub r: [i32; 3],
    pub u: [i32; 3],
    pub f: [i32; 3],
}

impl Lattice {
    /** The kind (of VOXEL_FACES) of a voxel's face towards a camera looking along F: −F's. */
    pub fn front(&self) -> usize {
        VOXEL_FACES.iter().position(|&n| n == self.f.map(|v| -v)).expect("−F is a face")
    }
}

/**
 * The view faces' lattices, in the order of FACES in view.ts: one per
 * neighbour direction F, U the world axis across it, R = U × F.
 */
pub fn lattices() -> [Lattice; VIEW_FACES] {
    std::array::from_fn(|face| {
        let k = [1, 0, 2][face / 4];
        let (s, t) = [(1, 1), (-1, 1), (1, -1), (-1, -1)][face % 4];
        let mut f = [0; 3];
        f[(k + 1) % 3] = s;
        f[(k + 2) % 3] = t;
        let mut u = [0; 3];
        u[k] = 1;
        let r = [u[1] * f[2] - u[2] * f[1], u[2] * f[0] - u[0] * f[2], u[0] * f[1] - u[1] * f[0]];
        Lattice { r, u, f }
    })
}

/** Bricks of the sites of a box (see VoxelGrid in voxel-grid.ts). */
pub const BRICK: i32 = 8;
const BRICK_SITES: usize = (BRICK * BRICK * BRICK / 2) as usize;

/** A voxel's material on each of its faces, by kind (VOXEL_FACES). */
pub type Faces = [u8; FACE_KINDS];

/**
 * Words per site in a grid's `faces`: its index in `cells`, then its
 * faces' materials, four to a word, the first in the lowest byte.
 */
pub const FACE_WORDS: usize = 4;

/** A site's words in `faces`: its index in `cells` and its faces' materials. */
pub fn face_words(index: usize, faces: &Faces) -> [u32; FACE_WORDS] {
    let word = |w: usize| u32::from_le_bytes(std::array::from_fn(|i| faces[4 * w + i]));
    [index as u32, word(0), word(1), word(2)]
}

/**
 * The sites of a box in bricks, BRICK units wide (see VoxelGrid).
 * A voxel has a material on each of its 12 faces: what is seen of it
 * looking at that face. Mostly they are one, its site's in `cells`;
 * the sites whose faces differ list them in `faces`.
 */
pub struct Grid {
    pub min: [i32; 3],
    pub n: [i32; 3],
    /** Per brick, z-major, then y, then x: where its sites start in `cells`, in BRICK_SITES; −1 if empty. */
    pub bricks: Vec<i32>,
    /** Each brick's sites, z-major, then y, then every other x. */
    pub cells: Vec<u8>,
    /** The sites whose faces are not all of their material in `cells`, by index in `cells`: FACE_WORDS each (face_words). */
    pub faces: Vec<u32>,
}

impl Grid {
    /** A grid of the box with `used` bricks of sites; `bricks` and `cells` to be written. */
    pub fn new(min: [i32; 3], n: [i32; 3], used: usize) -> Grid {
        let count = n.map(|v| (v.max(0) + BRICK - 1) / BRICK);
        Grid { min, n, bricks: vec![-1; (count[0] * count[1] * count[2]) as usize], cells: vec![0; used * BRICK_SITES], faces: Vec::new() }
    }

    /** Site (x, y, z)'s index in `cells`; None if its brick is empty or it is outside. */
    #[inline]
    fn index(&self, x: i32, y: i32, z: i32) -> Option<usize> {
        let (x, y, z) = (x - self.min[0], y - self.min[1], z - self.min[2]);
        if x < 0 || x >= self.n[0] || y < 0 || y >= self.n[1] || z < 0 || z >= self.n[2] {
            return None;
        }
        let bricks = [(self.n[0] + BRICK - 1) / BRICK, (self.n[1] + BRICK - 1) / BRICK];
        let at = self.bricks[(((z >> 3) * bricks[1] + (y >> 3)) * bricks[0] + (x >> 3)) as usize];
        (at >= 0).then(|| at as usize * BRICK_SITES + ((((z & 7) * BRICK + (y & 7)) * BRICK + (x & 7)) >> 1) as usize)
    }

    /** The material of face `kind` (of VOXEL_FACES) of the voxel at site (x, y, z); 0 if empty or outside. */
    #[inline]
    pub fn face(&self, x: i32, y: i32, z: i32, kind: usize) -> u8 {
        let Some(i) = self.index(x, y, z) else { return 0 };
        match self.cells[i] {
            0 => 0,
            material if self.faces.is_empty() => material,
            material => self.faces_at(i).map_or(material, |faces| faces[kind]),
        }
    }

    /** The faces' materials of the site at index `i` of `cells`, if they differ. */
    fn faces_at(&self, i: usize) -> Option<Faces> {
        let sites = self.faces.len() / FACE_WORDS;
        let (mut lo, mut hi) = (0, sites);
        while lo < hi {
            let mid = (lo + hi) / 2;
            match (self.faces[mid * FACE_WORDS] as usize).cmp(&i) {
                std::cmp::Ordering::Less => lo = mid + 1,
                std::cmp::Ordering::Greater => hi = mid,
                std::cmp::Ordering::Equal => {
                    let words = &self.faces[mid * FACE_WORDS + 1..(mid + 1) * FACE_WORDS];
                    return Some(std::array::from_fn(|k| words[k / 4].to_le_bytes()[k % 4]));
                }
            }
        }
        None
    }
}

/**
 * A level's sites as grids on a tiling of x and z: tile (i, k) holds x in
 * [i · 2^shift, (i + 1) · 2^shift), z likewise, and reads as empty until it
 * has a grid. The grids are shared, so tiles are cheap to add.
 */
#[derive(Clone)]
pub struct Tiles {
    shift: u32,
    /** The table's first tile, the site of its corner along x and z, and its tiles along each. */
    from: [i32; 2],
    origin: [i32; 2],
    size: [i32; 2],
    /** Tile by tile, z-major. */
    grids: Vec<Option<Arc<Grid>>>,
    /** Bounds of the grids' sites, inclusive (min > max if none). */
    min: [i32; 3],
    max: [i32; 3],
}

impl Tiles {
    /** The tiles 2^shift sites wide that have a grid: `tiles` holds each one's (i, k) and grid, its box within the tile. */
    pub fn new(shift: u32, tiles: impl IntoIterator<Item = ((i32, i32), Arc<Grid>)>) -> Tiles {
        let tiles: Vec<_> = tiles.into_iter().collect();
        let (mut from, mut to) = ([i32::MAX; 2], [i32::MIN; 2]);
        let (mut min, mut max) = ([i32::MAX; 3], [i32::MIN; 3]);
        for ((i, k), grid) in &tiles {
            from = [from[0].min(*i), from[1].min(*k)];
            to = [to[0].max(*i), to[1].max(*k)];
            for d in 0..3 {
                min[d] = min[d].min(grid.min[d]);
                max[d] = max[d].max(grid.min[d] + grid.n[d] - 1);
            }
        }
        if tiles.is_empty() {
            (from, to) = ([0, 0], [-1, -1]);
        }
        let size = [to[0] - from[0] + 1, to[1] - from[1] + 1];
        let mut grids = vec![None; (size[0] * size[1]) as usize];
        for ((i, k), grid) in tiles {
            grids[((k - from[1]) * size[0] + i - from[0]) as usize] = Some(grid);
        }
        Tiles { shift, from, origin: [from[0] << shift, from[1] << shift], size, grids, min, max }
    }

    /** A grid as the only tile: one as wide as any box. */
    pub fn one(grid: Grid) -> Tiles {
        let (min, max) = (grid.min, std::array::from_fn(|d| grid.min[d] + grid.n[d] - 1));
        Tiles {
            shift: i32::BITS - 1,
            from: [0, 0],
            origin: [grid.min[0], grid.min[2]],
            size: [1, 1],
            grids: vec![Some(Arc::new(grid))],
            min,
            max,
        }
    }

    /** Whether tile (i, k) has a grid. */
    pub fn has(&self, i: i32, k: i32) -> bool {
        let (i, k) = (i - self.from[0], k - self.from[1]);
        (0..self.size[0]).contains(&i) && (0..self.size[1]).contains(&k) && self.grids[(k * self.size[0] + i) as usize].is_some()
    }

    /** Whether tiles (i0, k0) to (i1, k1), inclusive, all have grids: those a chunk reads (chunk_reads). */
    pub fn has_all(&self, [i0, k0, i1, k1]: [i32; 4]) -> bool {
        (k0..=k1).all(|k| (i0..=i1).all(|i| self.has(i, k)))
    }

    /** The grid of the only tile, if no other bakers share it (see one). */
    pub fn only_mut(&mut self) -> Option<&mut Grid> {
        match self.grids.as_mut_slice() {
            [Some(grid)] => Arc::get_mut(grid),
            _ => None,
        }
    }

    /** The tile (i, k) column (x, z) is in, relative to the table's first. */
    #[inline]
    fn tile(&self, x: i32, z: i32) -> [i32; 2] {
        [(x - self.origin[0]) >> self.shift, (z - self.origin[1]) >> self.shift]
    }

    /** Tile (i, k)'s grid, if it has one. */
    #[inline]
    fn grid(&self, [i, k]: [i32; 2]) -> Option<&Grid> {
        if i < 0 || i >= self.size[0] || k < 0 || k >= self.size[1] {
            return None;
        }
        self.grids[(k * self.size[0] + i) as usize].as_deref()
    }
}

/**
 * The sites read in a face's lattice coordinates (a, b, c) (see Face in
 * view.ts), each voxel's material the one of its face towards the
 * camera, −F: a slice is drawn as seen along F, so that is the face its
 * voxels show (docs/architecture.md, section 3).
 */
pub struct Reader<'a> {
    tiles: &'a Tiles,
    lattice: Lattice,
    /** The kind of the faces towards the camera. */
    front: usize,
    /** Bounds of a, b, c over the sites, inclusive. */
    pub min: [i32; 3],
    pub max: [i32; 3],
}

impl<'a> Reader<'a> {
    pub fn new(tiles: &'a Tiles, lattice: Lattice) -> Reader<'a> {
        let mut min = [i32::MAX; 3];
        let mut max = [i32::MIN; 3];
        for x in [tiles.min[0], tiles.max[0]] {
            for y in [tiles.min[1], tiles.max[1]] {
                for z in [tiles.min[2], tiles.max[2]] {
                    for (d, v) in [lattice.r, lattice.u, lattice.f].iter().enumerate() {
                        let along = x * v[0] + y * v[1] + z * v[2];
                        min[d] = min[d].min(along);
                        max[d] = max[d].max(along);
                    }
                }
            }
        }
        if (0..3).any(|d| tiles.min[d] > tiles.max[d]) {
            (min, max) = ([0; 3], [-1; 3]);
        }
        Reader { tiles, lattice, front: lattice.front(), min, max }
    }

    /**
     * Material ids at (a + 2i, b, c) for i in [0, count), those of the
     * faces towards the camera, 0 where empty or
     * outside, to `visit` with each i, for a site (a, b, c): stepping a by
     * 2 steps (x, y, z) by R, and the grid only changes at a tile's edge.
     */
    #[inline]
    pub fn row(&self, a: i32, b: i32, c: i32, count: i32, mut visit: impl FnMut(i32, u8)) {
        debug_assert!(((a ^ c) | (b ^ c)) & 1 == 0, "not a site");
        let Lattice { r, u, f } = self.lattice;
        let mut x = (a * r[0] + c * f[0]) / 2 + b * u[0];
        let mut y = (a * r[1] + c * f[1]) / 2 + b * u[1];
        let mut z = (a * r[2] + c * f[2]) / 2 + b * u[2];
        let mut tile = self.tiles.tile(x, z);
        let mut grid = self.tiles.grid(tile);
        for i in 0..count {
            let here = self.tiles.tile(x, z);
            if here != tile {
                tile = here;
                grid = self.tiles.grid(tile);
            }
            visit(i, grid.map_or(0, |grid| grid.face(x, y, z, self.front)));
            x += r[0];
            y += r[1];
            z += r[2];
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_face_reads_the_material_of_the_voxels_faces_towards_it() {
        // Two voxels, one of material 1 all round, one whose faces are each of a material of their own.
        let mut grid = Grid::new([0, 0, 0], [8, 8, 8], 1);
        grid.bricks[0] = 0;
        let index = |[x, y, z]: [i32; 3]| (((z * BRICK + y) * BRICK + x) >> 1) as usize;
        let (plain, painted) = ([2, 2, 2], [4, 4, 2]);
        grid.cells[index(plain)] = 1;
        grid.cells[index(painted)] = 1;
        let faces: Faces = std::array::from_fn(|k| 10 + k as u8);
        grid.faces = face_words(index(painted), &faces).to_vec();
        let tiles = Tiles::one(grid);
        for lattice in lattices() {
            let reader = Reader::new(&tiles, lattice);
            let read = |[x, y, z]: [i32; 3]| {
                let [a, b, c] = [lattice.r, lattice.u, lattice.f].map(|v| v[0] * x + v[1] * y + v[2] * z);
                let mut material = None;
                reader.row(a, b, c, 1, |_, m| material = Some(m));
                material.expect("read")
            };
            assert_eq!(read(plain), 1);
            assert_eq!(read(painted), 10 + lattice.front() as u8, "seen along {:?}", lattice.f);
            assert_eq!(read([6, 6, 2]), 0);
        }
    }
}
