//! The voxel grid (voxel-grid.ts) and the voxel's faces (fcc.ts), as the
//! TS side hands them over.

use std::sync::Arc;

/** Kinds of voxel faces. */
pub const FACE_KINDS: usize = 12;
/** f64 per kind in the geometry table: lattice offset (3), normal (3). */
pub const GEOMETRY: usize = 6;
/** i32 per face in the lattice table: R, U, F (3 each). */
pub const LATTICE: usize = 9;
/** Faces of the view (FACES in view.ts). */
pub const VIEW_FACES: usize = 12;

/**
 * The voxel's faces in a face's lattice coordinates, the same for every
 * face (VOXEL_GEOMETRY in bake.ts): kind k's neighbour offset (a, b, c)
 * and its normal in face coordinates.
 */
pub struct Geometry {
    pub offsets: [[i32; 3]; FACE_KINDS],
    pub normals: [[f64; 3]; FACE_KINDS],
}

impl Geometry {
    pub fn from_table(table: &[f64]) -> Geometry {
        let mut g = Geometry { offsets: [[0; 3]; FACE_KINDS], normals: [[0.0; 3]; FACE_KINDS] };
        for k in 0..FACE_KINDS {
            let t = &table[k * GEOMETRY..(k + 1) * GEOMETRY];
            for d in 0..3 {
                g.offsets[k][d] = t[d] as i32;
                g.normals[k][d] = t[3 + d];
            }
        }
        g
    }
}

/** A face's lattice vectors R, U, F (integer, see Face in view.ts). */
#[derive(Clone, Copy)]
pub struct Lattice {
    pub r: [i32; 3],
    pub u: [i32; 3],
    pub f: [i32; 3],
}

/** Bricks of the sites of a box (see VoxelGrid in voxel-grid.ts). */
pub const BRICK: i32 = 8;
const BRICK_SITES: usize = (BRICK * BRICK * BRICK / 2) as usize;

/** The sites of a box in bricks, BRICK units wide (see VoxelGrid). */
pub struct Grid {
    pub min: [i32; 3],
    pub n: [i32; 3],
    /** Per brick, z-major, then y, then x: where its sites start in `cells`, in BRICK_SITES; −1 if empty. */
    pub bricks: Vec<i32>,
    /** Each brick's sites, z-major, then y, then every other x. */
    pub cells: Vec<u8>,
}

impl Grid {
    /** A grid of the box with `used` bricks of sites; `bricks` and `cells` to be written. */
    pub fn new(min: [i32; 3], n: [i32; 3], used: usize) -> Grid {
        let count = n.map(|v| (v.max(0) + BRICK - 1) / BRICK);
        Grid { min, n, bricks: vec![-1; (count[0] * count[1] * count[2]) as usize], cells: vec![0; used * BRICK_SITES] }
    }

    /** Material id at site (x, y, z); 0 if empty or outside. */
    #[inline]
    fn at(&self, x: i32, y: i32, z: i32) -> u8 {
        let (x, y, z) = (x - self.min[0], y - self.min[1], z - self.min[2]);
        if x < 0 || x >= self.n[0] || y < 0 || y >= self.n[1] || z < 0 || z >= self.n[2] {
            return 0;
        }
        let bricks = [(self.n[0] + BRICK - 1) / BRICK, (self.n[1] + BRICK - 1) / BRICK];
        let at = self.bricks[(((z >> 3) * bricks[1] + (y >> 3)) * bricks[0] + (x >> 3)) as usize];
        if at < 0 {
            return 0;
        }
        self.cells[at as usize * BRICK_SITES + ((((z & 7) * BRICK + (y & 7)) * BRICK + (x & 7)) >> 1) as usize]
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

    /** Material id at site (x, y, z); 0 if empty, outside or in a tile with no grid. */
    #[inline]
    fn at(&self, x: i32, y: i32, z: i32) -> u8 {
        let (i, k) = ((x - self.origin[0]) >> self.shift, (z - self.origin[1]) >> self.shift);
        if i < 0 || i >= self.size[0] || k < 0 || k >= self.size[1] {
            return 0;
        }
        match &self.grids[(k * self.size[0] + i) as usize] {
            Some(grid) => grid.at(x, y, z),
            None => 0,
        }
    }
}

/** The sites read in a face's lattice coordinates (a, b, c) (see Face in view.ts). */
pub struct Reader<'a> {
    tiles: &'a Tiles,
    lattice: Lattice,
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
        Reader { tiles, lattice, min, max }
    }

    /** Material id at (a, b, c); 0 if empty, outside or not a site. */
    #[inline]
    pub fn at(&self, a: i32, b: i32, c: i32) -> u8 {
        if ((a ^ c) | (b ^ c)) & 1 != 0 {
            return 0;
        }
        let Lattice { r, u, f } = self.lattice;
        let x = (a * r[0] + c * f[0]) / 2 + b * u[0];
        let y = (a * r[1] + c * f[1]) / 2 + b * u[1];
        let z = (a * r[2] + c * f[2]) / 2 + b * u[2];
        self.tiles.at(x, y, z)
    }
}
