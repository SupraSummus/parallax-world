//! The baker's exports (bake/), for chunk-baker.ts: it starts a world,
//! writes its grids through the pointers these return, then bakes chunk
//! after chunk, reading each result from RESULT and the splat and block
//! buffers it points to.

use std::cell::RefCell;

use bake::{Baker, Slice};
use fcc::{Grid, VIEW_WORDS};

/** Baking a world, once started. */
enum State {
    None,
    Ready(Box<Baker>),
}

thread_local! {
    static STATE: RefCell<State> = const { RefCell::new(State::None) };
    static RESULT: RefCell<[f64; 11]> = const { RefCell::new([0.0; 11]) };
    /** What sites() returns. */
    static SITES: RefCell<Vec<i32>> = const { RefCell::new(Vec::new()) };
}

/** Starts a world, with no grids yet. */
#[unsafe(no_mangle)]
pub extern "C" fn world_start() {
    STATE.set(State::Ready(Box::default()));
}

/**
 * Sets the ready world's grid of `level`, of the box given and `used`
 * bricks of sites; returns where to write its bricks (i32), then its sites
 * at world_cells(). Chunks of a level are baked only once it has a grid.
 */
#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub extern "C" fn world_grid(level: usize, min_x: i32, min_y: i32, min_z: i32, nx: i32, ny: i32, nz: i32, used: usize) -> *mut u8 {
    STATE.with_borrow_mut(|state| match state {
        State::Ready(baker) => baker.set_grid(level, Grid::new([min_x, min_y, min_z], [nx, ny, nz], used)).bricks.as_mut_ptr() as *mut u8,
        _ => std::ptr::null_mut(),
    })
}

/** Where to write the sites of the grid world_grid set last. */
#[unsafe(no_mangle)]
pub extern "C" fn world_cells() -> *mut u32 {
    STATE.with_borrow_mut(|state| match state {
        State::Ready(baker) => baker.last_grid().map_or(std::ptr::null_mut(), |g| g.cells.as_mut_ptr()),
        _ => std::ptr::null_mut(),
    })
}

/**
 * Where to write the views (Grid::views) of the grid world_grid set
 * last: VIEW_WORDS words (u32) for each of `sites` sites, by index.
 */
#[unsafe(no_mangle)]
pub extern "C" fn world_views(sites: usize) -> *mut u32 {
    STATE.with_borrow_mut(|state| match state {
        State::Ready(baker) => baker.last_grid().map_or(std::ptr::null_mut(), |g| {
            g.views = vec![0; sites * VIEW_WORDS];
            g.views.as_mut_ptr()
        }),
        _ => std::ptr::null_mut(),
    })
}

/**
 * Bakes a chunk (see Baker::bake); returns RESULT: mask width, height,
 * origin x, origin y, extent width, height, mask scale, splats, splats
 * pointer (f32, 5 each), blocks pointer, blocks length.
 */
#[unsafe(no_mangle)]
pub extern "C" fn bake(face: u32, level: i32, atoms: i32, layer: i32, cutoff: f64, i: i32, j: i32) -> *const f64 {
    STATE.with_borrow_mut(|state| {
        let State::Ready(baker) = state else { return std::ptr::null() };
        let baked = baker.bake(face as usize, &Slice { level, atoms, layer, cutoff }, i, j);
        RESULT.with_borrow_mut(|result| {
            *result = [
                baked.width as f64,
                baked.height as f64,
                baked.origin_x,
                baked.origin_y,
                baked.extent_width,
                baked.extent_height,
                baked.scale,
                baked.count as f64,
                baker.numbers.as_ptr() as usize as f64,
                baker.blocks.as_ptr() as usize as f64,
                baker.blocks.len() as f64,
            ];
            result.as_ptr()
        })
    })
}

/**
 * The sites of the last baked chunk's splats, for
 * tests: a, b, c (i32) each, in world units.
 */
#[unsafe(no_mangle)]
pub extern "C" fn sites() -> *const i32 {
    STATE.with_borrow_mut(|state| match state {
        State::Ready(baker) => SITES.with_borrow_mut(|sites| {
            sites.clear();
            sites.extend(baker.splat_sites().flatten());
            sites.as_ptr()
        }),
        _ => std::ptr::null(),
    })
}
