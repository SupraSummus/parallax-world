//! Where a grid's voxels are along each face's layers (LayerIndex in
//! voxel-grid.ts): per layer c and block of BLOCK columns a, the least and
//! greatest b of its voxels, from one pass over the grid, so the renderer can
//! tell which chunk cells are empty and how high their content reaches.
//! A world's tiles are indexed each on its own, as it is generated.

use bake::Grid;

/** Width of a column block, in lattice units along a. */
const BLOCK: i64 = 8;
const BRICK: i32 = 8;

type Lattice = [[i32; 3]; 3];

/** The index for faces whose forward is `forward` or its opposite. */
struct Table {
    forward: [i32; 3],
    c_min: i64,
    a_min: i64,
    layers: i64,
    blocks: i64,
    /** By (c − c_min, block of a − a_min): the least and greatest b, or least > greatest where there is no voxel. */
    low: Vec<i32>,
    high: Vec<i32>,
}

pub struct LayerIndex {
    tables: Vec<Table>,
}

/** Whether a face's forward is the opposite of one with its first non-zero component positive. */
fn mirrored(f: [i32; 3]) -> bool {
    f.into_iter().find(|&v| v != 0).is_some_and(|v| v < 0)
}

fn dot(v: [i32; 3], p: [i64; 3]) -> i64 {
    v[0] as i64 * p[0] + v[1] as i64 * p[1] + v[2] as i64 * p[2]
}

impl LayerIndex {
    pub fn new(grid: &Grid, lattices: &[Lattice]) -> LayerIndex {
        let lo = grid.min.map(i64::from);
        let hi: [i64; 3] = std::array::from_fn(|i| lo[i] + (grid.n[i] as i64 - 1).max(0));
        // Bounds of v · p over the grid's box.
        let along = |v: [i32; 3]| {
            let ends = (0..8).map(|k| dot(v, std::array::from_fn(|i| if k >> i & 1 == 0 { lo[i] } else { hi[i] }))).collect::<Vec<_>>();
            (*ends.iter().min().expect("corners"), *ends.iter().max().expect("corners"))
        };
        let mut shapes: Vec<Lattice> = Vec::new();
        for l in lattices.iter().filter(|l| !mirrored(l[2])) {
            if !shapes.iter().any(|s| s[2] == l[2]) {
                shapes.push(*l);
            }
        }
        let tables = shapes
            .iter()
            .map(|&[r, u, f]| {
                let ((a_min, a_max), (c_min, c_max)) = (along(r), along(f));
                let (layers, blocks) = (c_max - c_min + 1, (a_max - a_min) / BLOCK + 1);
                let mut low = vec![i32::MAX; (layers * blocks) as usize];
                let mut high = vec![i32::MIN; (layers * blocks) as usize];
                each_site(grid, |p| {
                    let (a, b, c) = (dot(r, p) - a_min, dot(u, p) as i32, dot(f, p) - c_min);
                    let at = (c * blocks + a / BLOCK) as usize;
                    low[at] = low[at].min(b);
                    high[at] = high[at].max(b);
                });
                Table { forward: f, c_min, a_min, layers, blocks, low, high }
            })
            .collect();
        LayerIndex { tables }
    }

    /**
     * The least and greatest b of the voxels in a face's layers [c0, c1)
     * with a in [a0, a1) (or a little wider, by whole blocks), if any.
     */
    pub fn range(&self, lattice: &Lattice, c0: i64, c1: i64, a0: i64, a1: i64) -> Option<(i64, i64)> {
        let f = lattice[2];
        let back = f.map(|v| -v);
        let t = self.tables.iter().find(|t| t.forward == f || t.forward == back)?;
        // The opposite face has the same up, and a and c negated.
        let (c0, c1, a0, a1) = if mirrored(f) { (1 - c1, 1 - c0, 1 - a1, 1 - a0) } else { (c0, c1, a0, a1) };
        let k0 = ((a0 - t.a_min).div_euclid(BLOCK)).max(0);
        let k1 = ((a1 - 1 - t.a_min).div_euclid(BLOCK)).min(t.blocks - 1);
        let (mut lo, mut hi) = (i32::MAX, i32::MIN);
        for c in (c0 - t.c_min).max(0)..(c1 - t.c_min).min(t.layers) {
            for k in c * t.blocks + k0..=c * t.blocks + k1 {
                lo = lo.min(t.low[k as usize]);
                hi = hi.max(t.high[k as usize]);
            }
        }
        (lo <= hi).then_some((lo as i64, hi as i64))
    }
}

/** Every site of the grid with a voxel, in world coordinates. */
fn each_site(grid: &Grid, mut visit: impl FnMut([i64; 3])) {
    let [bx, by] = [(grid.n[0] + BRICK - 1) / BRICK, (grid.n[1] + BRICK - 1) / BRICK];
    for (b, &at) in grid.bricks.iter().enumerate() {
        if at < 0 {
            continue;
        }
        let b = b as i32;
        let [x0, y0, z0] = [b % bx, (b / bx) % by, b / (bx * by)].map(|v| (v * BRICK) as i64);
        let [x0, y0, z0] = [x0 + grid.min[0] as i64, y0 + grid.min[1] as i64, z0 + grid.min[2] as i64];
        let mut site = at as usize * (BRICK * BRICK * BRICK / 2) as usize;
        for z in z0..z0 + BRICK as i64 {
            for y in y0..y0 + BRICK as i64 {
                let x1 = x0 + ((x0 + y + z) & 1);
                for dx in (0..BRICK as i64).step_by(2) {
                    if grid.cells[site] != 0 {
                        visit([x1 + dx, y, z]);
                    }
                    site += 1;
                }
            }
        }
    }
}
