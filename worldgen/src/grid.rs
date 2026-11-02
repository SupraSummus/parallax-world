//! Voxel storage as voxel-grid.ts lays it out: one material id per site of
//! a box, in bricks BRICK units wide, only bricks with voxels stored. The
//! same layout as the baker's Grid, which `into_grid` hands over.

use bake::Grid;

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
        }
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
    }

    pub fn into_grid(self) -> Grid {
        Grid { min: self.min.map(|v| v as i32), n: self.n.map(|v| v as i32), bricks: self.bricks, cells: self.cells }
    }
}

/** Material id of a site of the baker's grid, as VoxelGrid.get. */
pub fn site_of(grid: &Grid, x: i64, y: i64, z: i64) -> u8 {
    let [i, j, k] = [x - grid.min[0] as i64, y - grid.min[1] as i64, z - grid.min[2] as i64];
    let n = grid.n.map(|v| v as i64);
    if i < 0 || i >= n[0] || j < 0 || j >= n[1] || k < 0 || k >= n[2] || !is_site(x, y, z) {
        return 0;
    }
    let (bx, by) = ((n[0] + BRICK - 1) / BRICK, (n[1] + BRICK - 1) / BRICK);
    let at = grid.bricks[(((k >> 3) * by + (j >> 3)) * bx + (i >> 3)) as usize];
    if at < 0 { 0 } else { grid.cells[at as usize * BRICK_SITES + ((((k & 7) * BRICK + (j & 7)) * BRICK + (i & 7)) >> 1) as usize] }
}

impl VoxelGrid {
    /**
     * The grid of `b`, its bricks taken from `parts`: each a whole number of
     * bricks from the box's corner, no two with voxels in one brick of it
     * (VoxelGrid.compose in voxel-grid.ts). The rest of the box is empty.
     */
    pub fn compose(b: &Box3, parts: &[VoxelGrid]) -> VoxelGrid {
        let mut grid = VoxelGrid::new(b);
        let bz = (grid.n[2] + BRICK - 1) / BRICK;
        for part in parts {
            let offset: [i64; 3] = std::array::from_fn(|i| {
                assert_eq!((part.min[i] - grid.min[i]) % BRICK, 0, "a part is not a whole number of bricks from the box");
                (part.min[i] - grid.min[i]) / BRICK
            });
            let pz = (part.n[2] + BRICK - 1) / BRICK;
            for k in 0..pz {
                for j in 0..part.by {
                    for i in 0..part.bx {
                        let at = part.bricks[((k * part.by + j) * part.bx + i) as usize];
                        let [gi, gj, gk] = [i + offset[0], j + offset[1], k + offset[2]];
                        if at < 0 || gi < 0 || gj < 0 || gk < 0 || gi >= grid.bx || gj >= grid.by || gk >= bz {
                            continue;
                        }
                        grid.bricks[((gk * grid.by + gj) * grid.bx + gi) as usize] = (grid.cells.len() / BRICK_SITES) as i32;
                        let from = at as usize * BRICK_SITES;
                        grid.cells.extend_from_slice(&part.cells[from..from + BRICK_SITES]);
                    }
                }
            }
        }
        grid
    }
}
