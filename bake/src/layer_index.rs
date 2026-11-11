//! Where a grid's voxels are along each face's layers: per layer c and
//! block of BLOCK columns a, the least and greatest b of its voxels, from
//! one pass over the grid, so a renderer can tell which chunk cells are
//! empty and how high their content reaches. Opposite faces share a table.
//! The web indexes grids through the exports at the bottom, and reads the
//! tables in voxel-grid.ts (LayerIndex there, tested there).

use std::cell::RefCell;

use crate::world::{BRICK, Grid, lattices};

/** Width of a column block, in lattice units along a. */
const BLOCK: i64 = 8;

/** The index for faces whose forward is `forward` or its opposite. */
#[derive(Clone)]
struct Table {
    forward: [i32; 3],
    c_min: i64,
    a_min: i64,
    layers: i64,
    blocks: i64,
    /** By (c − c_min, block of a − a_min): the least and greatest b, or least > greatest where there is no voxel. */
    low: Vec<i16>,
    high: Vec<i16>,
}

#[derive(Clone)]
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
    /** The index of `grid` for the view's faces (lattices). */
    pub fn new(grid: &Grid) -> LayerIndex {
        // Bounds of v · p over the corners of the bricks with voxels, not of the grid's box:
        // a tile's box spans all the world's heights, and tables over it grow with their square.
        let full: Vec<[i64; 3]> = bricks(grid).filter(|(_, sites)| sites.iter().any(|&s| s != 0)).map(|(p, _)| p).collect();
        let along = |v: [i32; 3]| {
            let corners = full.iter().flat_map(|p| (0..8).map(move |k| std::array::from_fn(|i| p[i] + (k >> i & 1) * (BRICK as i64 - 1))));
            let ends = corners.map(|p| dot(v, p));
            ends.clone().min().zip(ends.max()).unwrap_or((0, -1))
        };
        let mut shapes: Vec<[[i32; 3]; 3]> = Vec::new();
        for l in lattices().iter().filter(|l| !mirrored(l.f)) {
            if !shapes.iter().any(|s| s[2] == l.f) {
                shapes.push([l.r, l.u, l.f]);
            }
        }
        let mut tables: Vec<Table> = shapes
            .iter()
            .map(|&[r, _, f]| {
                let ((a_min, a_max), (c_min, c_max)) = (along(r), along(f));
                let (layers, blocks) = (c_max - c_min + 1, (a_max - a_min) / BLOCK + 1);
                let (low, high) = (vec![i16::MAX; (layers * blocks) as usize], vec![i16::MIN; (layers * blocks) as usize]);
                Table { forward: f, c_min, a_min, layers, blocks, low, high }
            })
            .collect();
        // One pass for all the faces: the voxels are read once.
        each_site(grid, |p| {
            for (t, &[r, u, f]) in tables.iter_mut().zip(&shapes) {
                let (a, b, c) = (dot(r, p) - t.a_min, dot(u, p) as i16, dot(f, p) - t.c_min);
                let at = (c * t.blocks + a / BLOCK) as usize;
                t.low[at] = t.low[at].min(b);
                t.high[at] = t.high[at].max(b);
            }
        });
        LayerIndex { tables }
    }

    /**
     * The least and greatest b of the voxels in the layers [c0, c1) with a
     * in [a0, a1) (or a little wider, by whole blocks) of the face looking
     * along `forward`, if any.
     */
    pub fn range(&self, forward: [i32; 3], c0: i64, c1: i64, a0: i64, a1: i64) -> Option<(i64, i64)> {
        let back = forward.map(|v| -v);
        let t = self.tables.iter().find(|t| t.forward == forward || t.forward == back)?;
        // The opposite face has the same up, and a and c negated.
        let (c0, c1, a0, a1) = if mirrored(forward) { (1 - c1, 1 - c0, 1 - a1, 1 - a0) } else { (c0, c1, a0, a1) };
        let k0 = ((a0 - t.a_min).div_euclid(BLOCK)).max(0);
        let k1 = ((a1 - 1 - t.a_min).div_euclid(BLOCK)).min(t.blocks - 1);
        let (mut lo, mut hi) = (i16::MAX, i16::MIN);
        for c in (c0 - t.c_min).max(0)..(c1 - t.c_min).min(t.layers) {
            for k in c * t.blocks + k0..=c * t.blocks + k1 {
                lo = lo.min(t.low[k as usize]);
                hi = hi.max(t.high[k as usize]);
            }
        }
        (lo <= hi).then_some((lo as i64, hi as i64))
    }
}

/** The grid's bricks that hold sites: each one's least corner, in world coordinates, and its sites. */
fn bricks(grid: &Grid) -> impl Iterator<Item = ([i64; 3], &[u8])> {
    let [bx, by] = [0, 1].map(|i| (grid.n[i] + BRICK - 1) / BRICK);
    let count = (BRICK * BRICK * BRICK / 2) as usize;
    grid.bricks.iter().enumerate().filter(|(_, at)| **at >= 0).map(move |(b, &at)| {
        let b = b as i32;
        let corner = [b % bx, (b / bx) % by, b / (bx * by)];
        (std::array::from_fn(|i| (grid.min[i] + corner[i] * BRICK) as i64), &grid.cells[at as usize * count..][..count])
    })
}

/** Every site of the grid with a voxel, in world coordinates. */
fn each_site(grid: &Grid, mut visit: impl FnMut([i64; 3])) {
    for ([x0, y0, z0], sites) in bricks(grid) {
        let mut site = 0;
        for z in z0..z0 + BRICK as i64 {
            for y in y0..y0 + BRICK as i64 {
                let x1 = x0 + ((x0 + y + z) & 1);
                for dx in (0..BRICK as i64).step_by(2) {
                    if sites[site] != 0 {
                        visit([x1 + dx, y, z]);
                    }
                    site += 1;
                }
            }
        }
    }
}

// The WebAssembly interface: voxel-grid.ts writes a grid through the
// pointers index_grid and index_cells return, then reads its index's tables
// from what index_build returns, until the next grid.

/** Numbers per table in index_build's result. */
const TABLE: usize = 9;
/** Tables of an index: a face and its opposite share one. */
const TABLES: usize = 6;

thread_local! {
    static GRID: RefCell<Option<Grid>> = const { RefCell::new(None) };
    static INDEX: RefCell<Option<LayerIndex>> = const { RefCell::new(None) };
    static RESULT: RefCell<[f64; TABLES * TABLE]> = const { RefCell::new([0.0; TABLES * TABLE]) };
}

/** Starts a grid of the box given and `used` bricks of sites to index; returns where to write its bricks (i32), then its sites at index_cells(). */
#[unsafe(no_mangle)]
pub extern "C" fn index_grid(min_x: i32, min_y: i32, min_z: i32, nx: i32, ny: i32, nz: i32, used: usize) -> *mut u8 {
    INDEX.set(None);
    GRID.with_borrow_mut(|grid| grid.insert(Grid::new([min_x, min_y, min_z], [nx, ny, nz], used)).bricks.as_mut_ptr() as *mut u8)
}

/** Where to write the sites of the grid index_grid started. */
#[unsafe(no_mangle)]
pub extern "C" fn index_cells() -> *mut u8 {
    GRID.with_borrow_mut(|grid| grid.as_mut().map_or(std::ptr::null_mut(), |g| g.cells.as_mut_ptr()))
}

/**
 * Indexes the grid index_grid started; returns RESULT: per table, its
 * forward (x, y, z), c min, a min, layers, blocks, and pointers to its
 * low and high (i16, layers × blocks each). Null without a grid.
 */
#[unsafe(no_mangle)]
pub extern "C" fn index_build() -> *const f64 {
    let Some(grid) = GRID.take() else { return std::ptr::null() };
    INDEX.with_borrow_mut(|index| {
        let index = index.insert(LayerIndex::new(&grid));
        RESULT.with_borrow_mut(|result| {
            for (t, out) in index.tables.iter().zip(result.chunks_mut(TABLE)) {
                let [x, y, z] = t.forward.map(f64::from);
                let numbers = [t.c_min as f64, t.a_min as f64, t.layers as f64, t.blocks as f64];
                let pointers = [t.low.as_ptr() as usize as f64, t.high.as_ptr() as usize as f64];
                out.copy_from_slice(&[[x, y, z].as_slice(), &numbers, &pointers].concat());
            }
            result.as_ptr()
        })
    })
}
