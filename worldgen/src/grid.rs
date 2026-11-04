//! Voxel storage as voxel-grid.ts lays it out: one material id per site of
//! a box, in bricks BRICK units wide, only bricks with voxels stored. The
//! same layout as the baker's Grid, which `into_grid` hands over.

use std::collections::HashMap;
use std::sync::OnceLock;

use bake::Grid;

/**
 * Study (not for merging): view-dependent coarse voxels. WORLDGEN_ANISO=1
 * makes measured far forms record, per coarse site, the material seen
 * from each of the 6 axis directions (`VoxelGrid::aniso`), and rays pick
 * one by their direction. WORLDGEN_ANISO_GROUPS: 6 (each direction its
 * own), 3 (from above, from below, from the sides), up (from above, and
 * the rest); WORLDGEN_ANISO_PICK: dom (the ray's dominant axis) or mix
 * (an axis with the chance |d_i| / Σ|d_j|, its faces' projected areas).
 */
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Aniso {
    pub on: bool,
    pub groups: u8,
    pub mix: bool,
    /**
     * WORLDGEN_ANISO_OPACITY=1: measured sites are kept per axis (the share of the cell's columns holding a
     * fine site), see-through (id 0) along the others; 2: per direction, by the columns' first fine sites
     * from that end that land in the cell, of those reaching it (a coarsened depth map per direction).
     */
    pub opacity: u8,
}

pub fn aniso() -> Aniso {
    static MODE: OnceLock<Aniso> = OnceLock::new();
    *MODE.get_or_init(|| {
        let var = |name: &str| std::env::var(name).unwrap_or_default();
        Aniso {
            on: var("WORLDGEN_ANISO") == "1",
            groups: match var("WORLDGEN_ANISO_GROUPS").as_str() {
                "3" => 3,
                "up" => 1,
                "ts" => 2,
                "axes" => 4,
                _ => 6,
            },
            mix: var("WORLDGEN_ANISO_PICK") == "mix",
            opacity: var("WORLDGEN_ANISO_OPACITY").parse().unwrap_or(0),
        }
    })
}

/**
 * Which of the 6 directions a ray along `d` sees a voxel from: 2·axis +
 * 1 if it comes from the axis's high side (d_axis < 0), else 2·axis.
 */
pub fn direction_seen(d: [f64; 3]) -> usize {
    let mode = aniso();
    let m = d.map(f64::abs);
    let axis = if mode.mix {
        // A hash of the direction: the same ray picks the same.
        let h = d.iter().fold(0x9e37_79b9_7f4a_7c15u64, |h, v| (h ^ v.to_bits()).wrapping_mul(0x1000_0000_01b3).rotate_left(29));
        let h = h ^ (h >> 31);
        let u = (h.wrapping_mul(0xbf58_476d_1ce4_e5b9) >> 11) as f64 / (1u64 << 53) as f64 * (m[0] + m[1] + m[2]);
        if u < m[0] {
            0
        } else if u < m[0] + m[1] {
            1
        } else {
            2
        }
    } else if m[0] >= m[1] && m[0] >= m[2] {
        0
    } else if m[1] >= m[2] {
        1
    } else {
        2
    };
    2 * axis + (d[axis] < 0.0) as usize
}

pub const BRICK: i64 = 8;
const BRICK_SITES: usize = (BRICK * BRICK * BRICK / 2) as usize;

/** Half-open box of integer points: min ≤ p < max on each axis; lattice units, or a level's. */
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Box3 {
    pub min: [i64; 3],
    pub max: [i64; 3],
}

impl Box3 {
    pub fn holds(&self, p: [i64; 3]) -> bool {
        (0..3).all(|i| p[i] >= self.min[i] && p[i] < self.max[i])
    }

    pub fn overlaps(&self, b: &Box3) -> bool {
        (0..3).all(|i| self.min[i] < b.max[i] && b.min[i] < self.max[i])
    }

    pub fn inside(&self, b: &Box3) -> bool {
        (0..3).all(|i| self.min[i] >= b.min[i] && self.max[i] <= b.max[i])
    }

    pub fn intersect(&self, b: &Box3) -> Box3 {
        Box3 { min: [0, 1, 2].map(|i| self.min[i].max(b.min[i])), max: [0, 1, 2].map(|i| self.max[i].min(b.max[i])) }
    }

    pub fn widen(&self, m: i64) -> Box3 {
        Box3 { min: self.min.map(|v| v - m), max: self.max.map(|v| v + m) }
    }
}

pub fn is_site(x: i64, y: i64, z: i64) -> bool {
    (x + y + z) & 1 == 0
}

/** The sites of a box: see VoxelGrid in voxel-grid.ts. */
pub struct VoxelGrid {
    pub min: [i64; 3],
    pub n: [i64; 3],
    bx: i64,
    by: i64,
    /** Per brick, z-major, then y, then x: where its sites start in `cells`, in BRICK_SITES; −1 if empty. */
    pub bricks: Vec<i32>,
    pub cells: Vec<u8>,
    /** Study: per site with a view-dependent material, its id seen from each of the 6 directions (see `aniso`). */
    pub aniso: HashMap<[i64; 3], [u8; 6]>,
}

impl VoxelGrid {
    /** A grid of the sites in `b`, widened by one in x if needed to start and end on an even x. */
    pub fn new(b: &Box3) -> VoxelGrid {
        let min_x = b.min[0].div_euclid(2) * 2;
        let nx = 0.max((b.max[0] + 1).div_euclid(2) * 2 - min_x);
        let ny = 0.max(b.max[1] - b.min[1]);
        let nz = 0.max(b.max[2] - b.min[2]);
        let (bx, by, bz) = ((nx + BRICK - 1) / BRICK, (ny + BRICK - 1) / BRICK, (nz + BRICK - 1) / BRICK);
        VoxelGrid {
            min: [min_x, b.min[1], b.min[2]],
            n: [nx, ny, nz],
            bx,
            by,
            bricks: vec![-1; (bx * by * bz) as usize],
            cells: Vec::new(),
            aniso: HashMap::new(),
        }
    }

    /** Study: the material at (x, y, z) as a ray along `d` sees it. */
    pub fn get_seen(&self, x: i64, y: i64, z: i64, d: [f64; 3]) -> u8 {
        let id = self.get(x, y, z);
        if id == 0 || self.aniso.is_empty() {
            return id;
        }
        self.aniso.get(&[x, y, z]).map_or(id, |ids| ids[direction_seen(d)])
    }

    /** The box of its sites. */
    pub fn clip(&self) -> Box3 {
        Box3 { min: self.min, max: [0, 1, 2].map(|i| self.min[i] + self.n[i]) }
    }

    /** Where site (x, y, z) is in `cells`: Err(brick) if that brick is empty, None outside the box or off the sites. */
    fn slot(&self, x: i64, y: i64, z: i64) -> Option<Result<usize, usize>> {
        let (i, j, k) = (x - self.min[0], y - self.min[1], z - self.min[2]);
        if i < 0 || i >= self.n[0] || j < 0 || j >= self.n[1] || k < 0 || k >= self.n[2] || !is_site(x, y, z) {
            return None;
        }
        let brick = (((k >> 3) * self.by + (j >> 3)) * self.bx + (i >> 3)) as usize;
        let at = self.bricks[brick];
        Some(if at < 0 {
            Err(brick)
        } else {
            Ok(at as usize * BRICK_SITES + ((((k & 7) * BRICK + (j & 7)) * BRICK + (i & 7)) >> 1) as usize)
        })
    }

    /** Material id at (x, y, z); 0 if empty, outside the box or not a site. */
    pub fn get(&self, x: i64, y: i64, z: i64) -> u8 {
        match self.slot(x, y, z) {
            Some(Ok(i)) => self.cells[i],
            _ => 0,
        }
    }

    /** As `get`, but None if the brick of (x, y, z) holds nothing: rays skip it. */
    pub fn get_or_empty_brick(&self, x: i64, y: i64, z: i64) -> Option<u8> {
        match self.slot(x, y, z) {
            Some(Ok(i)) => Some(self.cells[i]),
            Some(Err(_)) => None,
            None => Some(0),
        }
    }

    /** Sets site (x, y, z) to material `id` (0 clears); outside the box or off the sites, nothing. */
    pub fn set(&mut self, x: i64, y: i64, z: i64, id: u8) {
        let i = match self.slot(x, y, z) {
            None => return,
            Some(Ok(i)) => i,
            Some(Err(_)) if id == 0 => return,
            Some(Err(brick)) => {
                self.bricks[brick] = (self.cells.len() / BRICK_SITES) as i32;
                self.cells.resize(self.cells.len() + BRICK_SITES, 0);
                self.slot(x, y, z).and_then(Result::ok).expect("brick just added")
            }
        };
        self.cells[i] = id;
        if !self.aniso.is_empty() {
            self.aniso.remove(&[x, y, z]);
        }
    }

    pub fn into_grid(self) -> Grid {
        Grid { min: self.min.map(|v| v as i32), n: self.n.map(|v| v as i32), bricks: self.bricks, cells: self.cells }
    }
}

/**
 * The grid of `b`, its bricks taken from `parts`: each a whole number of
 * bricks from the box's corner, no two with voxels in one brick of it
 * (VoxelGrid.compose in voxel-grid.ts). The rest of the box is empty.
 */
pub fn compose(b: &Box3, parts: &[&Grid]) -> Grid {
    let VoxelGrid { min, n, bx, by, mut bricks, mut cells, .. } = VoxelGrid::new(b);
    let bz = (n[2] + BRICK - 1) / BRICK;
    for part in parts {
        let offset: [i64; 3] = std::array::from_fn(|i| {
            assert_eq!((part.min[i] as i64 - min[i]) % BRICK, 0, "a part is not a whole number of bricks from the box");
            (part.min[i] as i64 - min[i]) / BRICK
        });
        let [pbx, pby, pbz] = part.n.map(|v| (v as i64 + BRICK - 1) / BRICK);
        for k in 0..pbz {
            for j in 0..pby {
                for i in 0..pbx {
                    let at = part.bricks[((k * pby + j) * pbx + i) as usize];
                    let [gi, gj, gk] = [i + offset[0], j + offset[1], k + offset[2]];
                    if at < 0 || gi < 0 || gj < 0 || gk < 0 || gi >= bx || gj >= by || gk >= bz {
                        continue;
                    }
                    bricks[((gk * by + gj) * bx + gi) as usize] = (cells.len() / BRICK_SITES) as i32;
                    let from = at as usize * BRICK_SITES;
                    cells.extend_from_slice(&part.cells[from..from + BRICK_SITES]);
                }
            }
        }
    }
    Grid { min: min.map(|v| v as i32), n: n.map(|v| v as i32), bricks, cells }
}
