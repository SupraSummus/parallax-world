//! The WebAssembly interface: worldgen.ts starts a forest of a seed, then
//! generates box after box of it, reading each result from RESULT and the
//! buffers it points to, which the next call reuses. The module is the
//! web's only one: it holds the baker's interface too (bake/src/lib.rs,
//! bake/src/coverage.rs), as a dependency's exports are the module's.

use std::cell::RefCell;

use crate::forest::{REACH, forest, heights};
use crate::generator::{Generator, generate, materials_of};
use crate::grid::{Box3, VoxelGrid};

/** The forest started last, its materials as [rgb, type] pairs, and the grid generated last. */
struct State {
    generator: Generator,
    materials: Vec<u32>,
    grid: Option<VoxelGrid>,
}

thread_local! {
    static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
    static RESULT: RefCell<[f64; 10]> = const { RefCell::new([0.0; 10]) };
}

fn result(values: &[f64]) -> *const f64 {
    RESULT.with_borrow_mut(|result| {
        result[..values.len()].copy_from_slice(values);
        result.as_ptr()
    })
}

/** '#rrggbb' as a number; 0 for none (material 0, empty). */
fn rgb(color: &str) -> u32 {
    color.strip_prefix('#').and_then(|hex| u32::from_str_radix(hex, 16).ok()).unwrap_or(0)
}

/**
 * Starts the forest of `seed` (an integer; JS numbers are f64); returns
 * RESULT: spawn x, y, z, REACH, materials, and a pointer to their
 * [rgb, type] pairs (u32).
 */
#[unsafe(no_mangle)]
pub extern "C" fn forest_start(seed: f64) -> *const f64 {
    let (generator, spawn) = forest(seed as i64 as u64);
    let materials = materials_of(&generator).iter().flat_map(|m| [rgb(m.color), m.material_type as u32]).collect();
    STATE.with_borrow_mut(|state| {
        let State { materials, .. } = state.insert(State { generator, materials, grid: None });
        result(&[
            spawn[0] as f64,
            spawn[1] as f64,
            spawn[2] as f64,
            REACH,
            (materials.len() / 2) as f64,
            materials.as_ptr() as usize as f64,
        ])
    })
}

/** The heights the forest has anything at in units of `level`; returns RESULT: min y, max y (exclusive). */
#[unsafe(no_mangle)]
pub extern "C" fn forest_heights(level: u32) -> *const f64 {
    let (min, max) = heights(level);
    result(&[min as f64, max as f64])
}

/**
 * Generates the box from (min_x, min_y, min_z) to (max_x, max_y, max_z)
 * (exclusive) at `level` (see generate); returns RESULT: the grid's min x,
 * y, z, nx, ny, nz, a pointer to its bricks (i32), their count, a pointer
 * to its sites (u8) and their count. Null before forest_start.
 */
#[unsafe(no_mangle)]
pub extern "C" fn forest_generate(level: u32, min_x: f64, min_y: f64, min_z: f64, max_x: f64, max_y: f64, max_z: f64) -> *const f64 {
    STATE.with_borrow_mut(|state| {
        let Some(state) = state else { return std::ptr::null() };
        let b = Box3 { min: [min_x as i64, min_y as i64, min_z as i64], max: [max_x as i64, max_y as i64, max_z as i64] };
        let grid = state.grid.insert(generate(&state.generator, &b, level));
        result(&[
            grid.min[0] as f64,
            grid.min[1] as f64,
            grid.min[2] as f64,
            grid.n[0] as f64,
            grid.n[1] as f64,
            grid.n[2] as f64,
            grid.bricks.as_ptr() as usize as f64,
            grid.bricks.len() as f64,
            grid.cells.as_ptr() as usize as f64,
            grid.cells.len() as f64,
        ])
    })
}
