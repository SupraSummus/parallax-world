//! The WebAssembly interface: worldgen.ts lists the worlds, starts one of
//! a seed, then generates box after box of it, reading each result from
//! RESULT and the buffers it points to, which the next call reuses. The
//! module is the web's only one: it holds the baker's interface too
//! (bake/src/lib.rs, bake/src/coverage.rs), as a dependency's exports are
//! the module's.

use std::cell::RefCell;

// Linked for its exports, which nothing here calls.
use bake as _;

use worldgen::World;
use worldgen::generator::{generate_shown, materials_of};
use worldgen::grid::{Box3, FACE_WORDS, Grid};

use crate::WORLDS;

/** The world started last, its materials' colours (as worldgen_start gives them, by id), and the grid generated last. */
struct State {
    world: World,
    materials: Vec<u32>,
    grid: Option<Grid>,
}

thread_local! {
    static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
    static RESULT: RefCell<[f64; 12]> = const { RefCell::new([0.0; 12]) };
    /** The names of WORLDS, in UTF-8, each ended by a NUL. */
    static NAMES: Vec<u8> = WORLDS.iter().flat_map(|e| e.name.bytes().chain([0])).collect();
}

fn result(values: &[f64]) -> *const f64 {
    RESULT.with_borrow_mut(|result| {
        result[..values.len()].copy_from_slice(values);
        result.as_ptr()
    })
}

/** The worlds; returns RESULT: their count, and a pointer to NAMES and its length in bytes. */
#[unsafe(no_mangle)]
pub extern "C" fn worldgen_worlds() -> *const f64 {
    NAMES.with(|names| result(&[WORLDS.len() as f64, names.as_ptr() as usize as f64, names.len() as f64]))
}

/**
 * Starts world `world` (its index in WORLDS; the first if past them) of
 * `seed` (an integer; JS numbers are f64); returns RESULT: spawn x, y, z,
 * materials, and a pointer to their colours (u32, 0xrrggbb), bit 24 set
 * if a walking body goes through it (KindDef::walk_through).
 */
#[unsafe(no_mangle)]
pub extern "C" fn worldgen_start(world: u32, seed: f64) -> *const f64 {
    let make = WORLDS.get(world as usize).unwrap_or(&WORLDS[0]).make;
    let world = make(seed as i64 as u64);
    let materials = materials_of(&world.generator).iter().map(|m| m.rgb() | u32::from(m.walk_through) << 24).collect();
    STATE.with_borrow_mut(|state| {
        let State { world, materials, .. } = state.insert(State { world, materials, grid: None });
        let [x, y, z] = world.spawn.map(|v| v as f64);
        result(&[x, y, z, materials.len() as f64, materials.as_ptr() as usize as f64])
    })
}

/**
 * The heights the world started last has anything at in units of
 * `level`; returns RESULT: min y, max y (exclusive). Null before
 * worldgen_start.
 */
#[unsafe(no_mangle)]
pub extern "C" fn worldgen_heights(level: u32) -> *const f64 {
    STATE.with_borrow(|state| {
        let Some(state) = state else { return std::ptr::null() };
        let (min, max) = state.world.heights_at(level);
        result(&[min as f64, max as f64])
    })
}

/**
 * Generates the box from (min_x, min_y, min_z) to (max_x, max_y, max_z)
 * (exclusive) at `level` as hosts make it (see generate_shown); returns RESULT: the grid's min x, y, z, nx, ny, nz, a pointer to its
 * bricks (i32), their count, a pointer to its sites (u8) and their count,
 * a pointer to the faces of the sites whose faces differ (u32, FACE_WORDS
 * each) and their count. Null before worldgen_start.
 */
#[unsafe(no_mangle)]
pub extern "C" fn worldgen_generate(level: u32, min_x: f64, min_y: f64, min_z: f64, max_x: f64, max_y: f64, max_z: f64) -> *const f64 {
    STATE.with_borrow_mut(|state| {
        let Some(state) = state else { return std::ptr::null() };
        let b = Box3 { min: [min_x as i64, min_y as i64, min_z as i64], max: [max_x as i64, max_y as i64, max_z as i64] };
        let grid = state.grid.insert(generate_shown(&state.world.generator, &b, level).into_grid(false));
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
            grid.faces.as_ptr() as usize as f64,
            (grid.faces.len() / FACE_WORDS) as f64,
        ])
    })
}
