//! The layer index's exports (bake's LayerIndex), for voxel-grid.ts: it
//! writes a grid through the pointers index_grid and index_cells return,
//! then reads its index's tables from what index_build returns, until the
//! next grid.

use std::cell::RefCell;

use bake::LayerIndex;
use fcc::Grid;

/** Numbers per table in index_build's result. */
const TABLE: usize = 10;
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
 * forward (x, y, z), c min, a min, b min, layers, blocks, and pointers
 * to its low and high (i16, layers × blocks each). Null without a grid.
 */
#[unsafe(no_mangle)]
pub extern "C" fn index_build() -> *const f64 {
    let Some(grid) = GRID.take() else { return std::ptr::null() };
    INDEX.with_borrow_mut(|index| {
        let index = index.insert(LayerIndex::new(&grid));
        RESULT.with_borrow_mut(|result| {
            for (t, out) in index.tables.iter().zip(result.chunks_mut(TABLE)) {
                let [x, y, z] = t.forward.map(f64::from);
                let numbers = [t.c_min as f64, t.a_min as f64, t.b_min as f64, t.layers as f64, t.blocks as f64];
                let pointers = [t.low.as_ptr() as usize as f64, t.high.as_ptr() as usize as f64];
                out.copy_from_slice(&[[x, y, z].as_slice(), &numbers, &pointers].concat());
            }
            result.as_ptr()
        })
    })
}
