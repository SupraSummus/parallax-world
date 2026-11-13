//! Walking's exports (bake's walk.rs), for walk.ts: it starts a body
//! (walk_start), hands it its ground's materials and grid when they change
//! (walk_materials, walk_grid, walk_cells), and steps it (walk_step).

use std::cell::RefCell;

use bake::walk::{COLLISION_LEVEL, Ground, WalkInput, Walker};
use fcc::{Grid, Tiles};

struct Walking {
    body: Walker,
    tiles: Tiles,
    walk_through: Vec<bool>,
}

thread_local! {
    static WALKING: RefCell<Option<Walking>> = const { RefCell::new(None) };
    static RESULT: RefCell<[f64; 4]> = const { RefCell::new([0.0; 4]) };
}

/** Starts a body with its eye at (x, y, z), on no ground yet (held); returns the level of the grid it wants (COLLISION_LEVEL). */
#[unsafe(no_mangle)]
pub extern "C" fn walk_start(x: f64, y: f64, z: f64) -> u32 {
    WALKING.set(Some(Walking { body: Walker::at([x, y, z]), tiles: Tiles::new(0, []), walk_through: Vec::new() }));
    COLLISION_LEVEL
}

/** Where to write, for each of `count` materials, 1 if walked through, else 0. */
#[unsafe(no_mangle)]
pub extern "C" fn walk_materials(count: usize) -> *mut u8 {
    WALKING.with_borrow_mut(|w| {
        w.as_mut().map_or(std::ptr::null_mut(), |w| {
            w.walk_through = vec![false; count];
            // Bytes 0 and 1 are the bools false and true.
            w.walk_through.as_mut_ptr() as *mut u8
        })
    })
}

/** Starts the ground's grid, of the box given and `used` bricks of sites; returns where to write its bricks (i32), then its sites at walk_cells(). */
#[unsafe(no_mangle)]
pub extern "C" fn walk_grid(min_x: i32, min_y: i32, min_z: i32, nx: i32, ny: i32, nz: i32, used: usize) -> *mut u8 {
    WALKING.with_borrow_mut(|w| {
        w.as_mut().map_or(std::ptr::null_mut(), |w| {
            w.tiles = Tiles::one(Grid::new([min_x, min_y, min_z], [nx, ny, nz], used));
            w.tiles.only_mut().map_or(std::ptr::null_mut(), |g| g.bricks.as_mut_ptr() as *mut u8)
        })
    })
}

/** Where to write the sites of the grid walk_grid started. */
#[unsafe(no_mangle)]
pub extern "C" fn walk_cells() -> *mut u8 {
    WALKING.with_borrow_mut(|w| w.as_mut().and_then(|w| w.tiles.only_mut()).map_or(std::ptr::null_mut(), |g| g.cells.as_mut_ptr()))
}

/** Steps the body (Walker::walk); returns RESULT: its eye x, y, z, and 1 if settled, else 0. Null before walk_start. */
#[unsafe(no_mangle)]
pub extern "C" fn walk_step(forward: f64, right: f64, yaw: f64, jump: bool, sneak: bool, dt: f64) -> *const f64 {
    WALKING.with_borrow_mut(|w| {
        let Some(w) = w else { return std::ptr::null() };
        let ground = Ground { tiles: &w.tiles, walk_through: &w.walk_through };
        w.body = w.body.walk(WalkInput { forward, right, yaw, jump, sneak }, dt, &ground);
        let [x, y, z] = w.body.eye();
        RESULT.with_borrow_mut(|result| {
            *result = [x, y, z, f64::from(u8::from(w.body.settled()))];
            result.as_ptr()
        })
    })
}
