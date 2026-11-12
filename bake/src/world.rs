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

    /** Lattice coordinates (a, b, c) in the world: (a·R + c·F) / 2 + b·U. */
    pub fn world(&self, [a, b, c]: [i32; 3]) -> [i32; 3] {
        let Lattice { r, u, f } = self;
        std::array::from_fn(|d| (a * r[d] + c * f[d]) / 2 + b * u[d])
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

/** A whole voxel's alpha: it stops every ray through its site. */
pub const ALPHA: u8 = 255;

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
 * the sites whose faces differ list them in `faces`. Its alpha is how much
 * of the rays through its site it stops: mostly all (ALPHA), and `alpha`
 * lists them only for a grid with some see-through ones.
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
    /** By index in `cells`: each voxel's alpha, 0 where there is none; empty if all are whole. */
    pub alpha: Vec<u8>,
}

/** The words after site index `i` in `words`, `per` words a site, sorted by index; None if it has none. */
fn words_of(words: &[u32], per: usize, i: usize) -> Option<&[u32]> {
    let (mut lo, mut hi) = (0, words.len() / per);
    while lo < hi {
        let mid = (lo + hi) / 2;
        match (words[mid * per] as usize).cmp(&i) {
            std::cmp::Ordering::Less => lo = mid + 1,
            std::cmp::Ordering::Greater => hi = mid,
            std::cmp::Ordering::Equal => return Some(&words[mid * per + 1..(mid + 1) * per]),
        }
    }
    None
}

impl Grid {
    /** A grid of the box with `used` bricks of sites; `bricks` and `cells` to be written. */
    pub fn new(min: [i32; 3], n: [i32; 3], used: usize) -> Grid {
        let count = n.map(|v| (v.max(0) + BRICK - 1) / BRICK);
        Grid {
            min,
            n,
            bricks: vec![-1; (count[0] * count[1] * count[2]) as usize],
            cells: vec![0; used * BRICK_SITES],
            faces: Vec::new(),
            alpha: Vec::new(),
        }
    }

    /** The brick holding site (x, y, z): where its sites start in `cells`, in bricks; −1 if it is empty or outside. */
    #[inline]
    pub fn brick(&self, x: i32, y: i32, z: i32) -> i32 {
        let (x, y, z) = (x - self.min[0], y - self.min[1], z - self.min[2]);
        if x < 0 || x >= self.n[0] || y < 0 || y >= self.n[1] || z < 0 || z >= self.n[2] {
            return -1;
        }
        let bricks = [(self.n[0] + BRICK - 1) / BRICK, (self.n[1] + BRICK - 1) / BRICK];
        self.bricks[(((z >> 3) * bricks[1] + (y >> 3)) * bricks[0] + (x >> 3)) as usize]
    }

    /** Site (x, y, z)'s index in `cells`; None if its brick is empty or it is outside. */
    #[inline]
    fn index(&self, x: i32, y: i32, z: i32) -> Option<usize> {
        let at = self.brick(x, y, z);
        let (x, y, z) = (x - self.min[0], y - self.min[1], z - self.min[2]);
        (at >= 0).then(|| at as usize * BRICK_SITES + ((((z & 7) * BRICK + (y & 7)) * BRICK + (x & 7)) >> 1) as usize)
    }

    /** The material of the voxel at site (x, y, z), as stored in `cells`, and its alpha; (0, 0) if empty or outside. */
    #[inline]
    pub fn get(&self, x: i32, y: i32, z: i32) -> (u8, u8) {
        self.index(x, y, z).map_or((0, 0), |i| (self.cells[i], self.alpha_at(i)))
    }

    /** The material of face `kind` (of VOXEL_FACES) of the voxel at site (x, y, z), and its alpha; (0, 0) if empty or outside. */
    #[inline]
    pub fn face(&self, x: i32, y: i32, z: i32, kind: usize) -> (u8, u8) {
        let Some(i) = self.index(x, y, z) else { return (0, 0) };
        let material = match self.cells[i] {
            0 => 0,
            material if self.faces.is_empty() => material,
            material => self.faces_at(i).map_or(material, |faces| faces[kind]),
        };
        (material, self.alpha_at(i))
    }

    /** The alpha of the voxel at index `i` of `cells`: 0 if none. */
    #[inline]
    pub fn alpha_at(&self, i: usize) -> u8 {
        match self.cells[i] {
            0 => 0,
            _ => self.alpha.get(i).copied().unwrap_or(ALPHA),
        }
    }

    /** The faces' materials of the site at index `i` of `cells`, if they differ. */
    fn faces_at(&self, i: usize) -> Option<Faces> {
        let words = words_of(&self.faces, FACE_WORDS, i)?;
        Some(std::array::from_fn(|k| words[k / 4].to_le_bytes()[k % 4]))
    }
}

/** A tile's (i, j, k): it holds x in [i · 2^shift, (i + 1) · 2^shift), y by j and z by k likewise. */
pub type TileId = [i32; 3];

/**
 * A level's sites as grids on a tiling of space by cubes, 2^shift sites a
 * side: a site no tile has a grid for is unknown; one a tile's grid does
 * not hold, empty. The grids are shared, so tiles are cheap to add.
 */
#[derive(Clone)]
pub struct Tiles {
    shift: u32,
    /** The table's first tile, the site of its corner, and its tiles along each axis. */
    from: TileId,
    origin: [i64; 3],
    size: [i32; 3],
    /** Tile by tile, z-major, then y. */
    grids: Vec<Option<Arc<Grid>>>,
    /** Bounds of the grids' sites, inclusive (min > max if none). */
    min: [i32; 3],
    max: [i32; 3],
}

impl Tiles {
    /** The tiles 2^shift sites a side that have a grid: `tiles` holds each one's id and grid, its box within the tile. */
    pub fn new(shift: u32, tiles: impl IntoIterator<Item = (TileId, Arc<Grid>)>) -> Tiles {
        let tiles: Vec<_> = tiles.into_iter().collect();
        let (mut from, mut to) = ([i32::MAX; 3], [i32::MIN; 3]);
        let (mut min, mut max) = ([i32::MAX; 3], [i32::MIN; 3]);
        for (id, grid) in &tiles {
            from = std::array::from_fn(|d| from[d].min(id[d]));
            to = std::array::from_fn(|d| to[d].max(id[d]));
            if grid.n.iter().all(|&n| n > 0) {
                min = std::array::from_fn(|d| min[d].min(grid.min[d]));
                max = std::array::from_fn(|d| max[d].max(grid.min[d] + grid.n[d] - 1));
            }
        }
        if tiles.is_empty() {
            (from, to) = ([0; 3], [-1; 3]);
        }
        let size = std::array::from_fn(|d| to[d] - from[d] + 1);
        let mut grids = vec![None; size.iter().map(|&n| n as usize).product()];
        for (id, grid) in tiles {
            let [i, j, k] = std::array::from_fn(|d| id[d] - from[d]);
            grids[((k * size[1] + j) * size[0] + i) as usize] = Some(grid);
        }
        Tiles { shift, from, origin: from.map(|v| (v as i64) << shift), size, grids, min, max }
    }

    /** A grid as the only tile: one holding all space, 2^32 sites a side from the least i32. */
    pub fn one(grid: Grid) -> Tiles {
        let (min, max) = (grid.min, std::array::from_fn(|d| grid.min[d] + grid.n[d] - 1));
        Tiles { shift: 32, from: [0; 3], origin: [i32::MIN as i64; 3], size: [1; 3], grids: vec![Some(Arc::new(grid))], min, max }
    }

    /** The material at site (x, y, z) and its alpha, (0, 0) if empty; None if no tile has a grid there. */
    pub fn at(&self, x: i32, y: i32, z: i32) -> Option<(u8, u8)> {
        Some(self.grid(self.tile(x, y, z))?.get(x, y, z))
    }

    /** Whether tile `id` has a grid. */
    pub fn has(&self, id: TileId) -> bool {
        self.grid(std::array::from_fn(|d| id[d] - self.from[d])).is_some()
    }

    /** Whether the tiles from `lo` to `hi`, inclusive, all have grids: those a chunk reads (chunk_reads). */
    pub fn has_all(&self, [lo, hi]: [TileId; 2]) -> bool {
        (lo[2]..=hi[2]).all(|k| (lo[1]..=hi[1]).all(|j| (lo[0]..=hi[0]).all(|i| self.has([i, j, k]))))
    }

    /** The grid of the only tile, if no other bakers share it (see one). */
    pub fn only_mut(&mut self) -> Option<&mut Grid> {
        match self.grids.as_mut_slice() {
            [Some(grid)] => Arc::get_mut(grid),
            _ => None,
        }
    }

    /** The tile site (x, y, z) is in, relative to the table's first. */
    #[inline]
    fn tile(&self, x: i32, y: i32, z: i32) -> TileId {
        let o = self.origin;
        [((x as i64 - o[0]) >> self.shift) as i32, ((y as i64 - o[1]) >> self.shift) as i32, ((z as i64 - o[2]) >> self.shift) as i32]
    }

    /** The grid of tile `t`, relative to the table's first, if it has one. */
    #[inline]
    fn grid(&self, [i, j, k]: TileId) -> Option<&Grid> {
        let [nx, ny, nz] = self.size;
        if i < 0 || i >= nx || j < 0 || j >= ny || k < 0 || k >= nz {
            return None;
        }
        self.grids[((k * ny + j) * nx + i) as usize].as_deref()
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
     * faces towards the camera, and their alpha, (0, 0) where empty or
     * outside, to `visit` with each i, for a site (a, b, c): stepping a by
     * 2 steps (x, y, z) by R, and the grid only changes at a tile's edge.
     */
    #[inline]
    pub fn row(&self, a: i32, b: i32, c: i32, count: i32, mut visit: impl FnMut(i32, u8, u8)) {
        debug_assert!(((a ^ c) | (b ^ c)) & 1 == 0, "not a site");
        let r = self.lattice.r;
        let [mut x, mut y, mut z] = self.lattice.world([a, b, c]);
        let mut tile = self.tiles.tile(x, y, z);
        let mut grid = self.tiles.grid(tile);
        for i in 0..count {
            let here = self.tiles.tile(x, y, z);
            if here != tile {
                tile = here;
                grid = self.tiles.grid(tile);
            }
            let (material, alpha) = grid.map_or((0, 0), |grid| grid.face(x, y, z, self.front));
            visit(i, material, alpha);
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
    fn tiles_hold_their_cubes_alike_along_every_axis() {
        // A voxel in each of two tiles, one above the other, as shift 3 makes them: a brick each.
        let tile = |min: [i32; 3]| {
            let mut grid = Grid::new(min, [8, 8, 8], 1);
            grid.bricks[0] = 0;
            grid.cells[0] = 1 + min[1] as u8 / 8;
            Arc::new(grid)
        };
        let tiles = Tiles::new(
            3,
            [([0, 0, 0], tile([0, 0, 0])), ([0, 1, 0], tile([0, 8, 0])), ([1, 0, 0], Arc::new(Grid::new([8, 0, 0], [0; 3], 0)))],
        );
        assert_eq!(tiles.at(0, 0, 0), Some((1, ALPHA)));
        assert_eq!(tiles.at(0, 8, 0), Some((2, ALPHA)));
        assert_eq!(tiles.at(2, 0, 0), Some((0, 0)), "empty in its tile");
        assert_eq!(tiles.at(8, 0, 0), Some((0, 0)), "a tile whose grid holds nothing is empty");
        for unknown in [[0, 16, 0], [0, -1, 0], [0, 0, 8], [-1, 0, 0]] {
            assert_eq!(tiles.at(unknown[0], unknown[1], unknown[2]), None, "{unknown:?}: no tile there");
        }
        assert!(tiles.has_all([[0, 0, 0], [1, 0, 0]]) && tiles.has_all([[0, 0, 0], [0, 1, 0]]));
        assert!(!tiles.has_all([[0, 0, 0], [1, 1, 0]]) && !tiles.has([0, 0, 1]));
    }

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
                reader.row(a, b, c, 1, |_, m, _| material = Some(m));
                material.expect("read")
            };
            assert_eq!(read(plain), 1);
            assert_eq!(read(painted), 10 + lattice.front() as u8, "seen along {:?}", lattice.f);
            assert_eq!(read([6, 6, 2]), 0);
        }
    }
}
