//! Walking on the ground, as in Minecraft: a body that gravity pulls down
//! and the world's voxels hold up and stop, that steps up onto (and down
//! from) what is low enough as it walks, and jumps.
//! Lattice units throughout, 0.125 m (worldgen's UNIT_SIZE).

use std::cell::RefCell;

use crate::VOXEL_FACES;
use crate::world::{ALPHA, Grid, Tiles};

/** Lattice units per metre. */
const M: f64 = 8.0;
const EYE_HEIGHT: f64 = 1.62 * M;
const HEIGHT: f64 = 1.8 * M;
const HALF_WIDTH: f64 = 0.3 * M;
/** What the body walks up onto, or down from, without jumping. */
const STEP: f64 = 0.55 * M;
const WALK_SPEED: f64 = 4.3 * M;
const SNEAK_SPEED: f64 = 1.3 * M;
const GRAVITY: f64 = 32.0 * M;
/** Lifts the feet about 1.1 m. */
const JUMP_SPEED: f64 = 8.4 * M;
const TERMINAL_SPEED: f64 = 78.0 * M;
/** How fast the eye catches up with a step, per second (see Walker::lag). */
const LAG_DECAY: f64 = 15.0;
/** No move is longer, so none passes through a voxel. */
const MAX_MOVE: f64 = 1.0;

/**
 * The level whose grid the body collides with: 0.25 m voxels, in which
 * pebbles, cones and blades of grass have coarsened away.
 */
pub const COLLISION_LEVEL: u32 = 1;

/** What the body collides with: voxels of the lattice `scale()` units apart, those at the sites `solid` says. */
pub trait Solid {
    fn solid(&self, site: [i32; 3]) -> bool;
    fn scale(&self) -> f64;
}

/**
 * The voxels of COLLISION_LEVEL's tiles that stop more than half the rays
 * through their sites, but those of materials `walk_through` marks
 * (plants, water: KindDef::walk_through), by id.
 * Sites no tile has a grid for are solid: the body stops there rather than fall.
 */
pub struct Ground<'a> {
    pub tiles: &'a Tiles,
    pub walk_through: &'a [bool],
}

impl Solid for Ground<'_> {
    fn solid(&self, [x, y, z]: [i32; 3]) -> bool {
        match self.tiles.at(x, y, z) {
            None => true,
            Some((id, alpha)) => alpha > ALPHA / 2 && !self.walk_through.get(id as usize).copied().unwrap_or(false),
        }
    }

    fn scale(&self) -> f64 {
        (1 << COLLISION_LEVEL) as f64
    }
}

/** Held keys: forward and right each −1, 0 or 1, along `yaw` (degrees, as the view's). */
#[derive(Clone, Copy, Default)]
pub struct WalkInput {
    pub forward: f64,
    pub right: f64,
    pub yaw: f64,
    pub jump: bool,
    pub sneak: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct Walker {
    /** The middle of the soles. */
    feet: [f64; 3],
    /** Upward speed, units a second. */
    vy: f64,
    /** Standing on something, or held where it is (see walk). */
    held: bool,
    /** How far the eye is above where the feet put it: it lags behind steps, so that small ones glide. */
    lag: f64,
}

impl Walker {
    /** A body with its eye at `eye`, about to fall. */
    pub fn at(eye: [f64; 3]) -> Walker {
        Walker { feet: [eye[0], eye[1] - EYE_HEIGHT, eye[2]], vy: 0.0, held: false, lag: 0.0 }
    }

    /** Where the camera goes. */
    pub fn eye(&self) -> [f64; 3] {
        [self.feet[0], self.feet[1] + EYE_HEIGHT + self.lag, self.feet[2]]
    }

    /** Whether it stays put while nothing is held. */
    pub fn settled(&self) -> bool {
        self.held && self.lag.abs() < 0.01
    }

    /**
     * The body `dt` seconds on, holding `input`. In something solid it
     * gets out on top if that is near; else (inside a hill, or where the
     * world isn't generated yet) it is held there, walking level, until out.
     */
    pub fn walk(mut self, input: WalkInput, dt: f64, ground: &impl Solid) -> Walker {
        let (sin, cos) = input.yaw.to_radians().sin_cos();
        let (mut dx, mut dz) = (input.right * cos + input.forward * sin, input.forward * cos - input.right * sin);
        let length = dx.hypot(dz).max(1.0);
        let speed = if input.sneak { SNEAK_SPEED } else { WALK_SPEED } * dt / length;
        (dx, dz) = (dx * speed, dz * speed);
        self.lag *= (-LAG_DECAY * dt).exp();

        if blocked(ground, self.feet) {
            self.vy = 0.0;
            self.held = true;
            match (1..=(HEIGHT / MAX_MOVE) as usize)
                .map(|n| n as f64 * MAX_MOVE)
                .find(|up| !blocked(ground, [self.feet[0], self.feet[1] + up, self.feet[2]]))
            {
                Some(up) => {
                    self.feet[1] += up;
                    self.lag -= up;
                }
                None => {
                    self.feet[0] += dx;
                    self.feet[2] += dz;
                }
            }
            return self;
        }

        let steps = (dx.abs().max(dz.abs()) / MAX_MOVE).ceil();
        for _ in 0..steps as usize {
            self.walk_along(ground, 0, dx / steps);
            self.walk_along(ground, 2, dz / steps);
        }

        if input.jump && self.held {
            self.vy = JUMP_SPEED;
            self.held = false;
        }
        if self.held {
            // Down onto what is at most STEP lower; off a higher edge, it falls.
            let before = self.feet;
            if shift(ground, &mut self.feet, 1, -STEP) {
                self.lag += before[1] - self.feet[1];
                return self;
            }
            self.feet = before;
            self.held = false;
        }
        self.vy = (self.vy - GRAVITY * dt).max(-TERMINAL_SPEED);
        let dy = self.vy * dt;
        let moves = (dy.abs() / MAX_MOVE).ceil();
        for _ in 0..moves as usize {
            if shift(ground, &mut self.feet, 1, dy / moves) {
                self.held = dy < 0.0;
                self.vy = 0.0;
                break;
            }
        }
        self
    }

    /** A move along `axis`: if blocked while standing, lifted STEP, moved and lowered onto what is there. */
    fn walk_along(&mut self, ground: &impl Solid, axis: usize, d: f64) {
        let mut ahead = self.feet;
        ahead[axis] += d;
        if !blocked(ground, ahead) {
            self.feet = ahead;
            return;
        }
        let mut lifted = [ahead[0], ahead[1] + STEP, ahead[2]];
        if self.held && !blocked(ground, [self.feet[0], self.feet[1] + STEP, self.feet[2]]) && !blocked(ground, lifted) {
            shift(ground, &mut lifted, 1, -STEP);
            self.lag += self.feet[1] - lifted[1];
            self.feet = lifted;
            return;
        }
        shift(ground, &mut self.feet, axis, d);
    }
}

/**
 * Whether the body with its feet at `feet` overlaps a solid voxel. Two
 * convex shapes are apart if and only if they are along one of their
 * faces' normals or their edges' cross products: for the body, a box, and
 * a voxel, the box's axes and the voxel's normals (VOXEL_FACES), along
 * each of which the voxel reaches 1 from its site.
 */
fn blocked(ground: &impl Solid, feet: [f64; 3]) -> bool {
    const AXES: [[i32; 3]; 3] = [[1, 0, 0], [0, 1, 0], [0, 0, 1]];
    let scale = ground.scale();
    let centre = [feet[0], feet[1] + HEIGHT / 2.0, feet[2]].map(|v| v / scale);
    let half = [HALF_WIDTH, HEIGHT / 2.0, HALF_WIDTH].map(|v| v / scale);
    let range = |i: usize| (centre[i] - half[i] - 1.0).floor() as i32..=(centre[i] + half[i] + 1.0).ceil() as i32;
    let sites = range(0).flat_map(|x| range(1).flat_map(move |y| range(2).map(move |z| [x, y, z])));
    sites.filter(|[x, y, z]| (x + y + z) & 1 == 0).any(|site| {
        let apart = AXES.iter().chain(&VOXEL_FACES).any(|n| {
            let along: f64 = (0..3).map(|i| f64::from(n[i]) * (f64::from(site[i]) - centre[i])).sum();
            let reach: f64 = (0..3).map(|i| f64::from(n[i].abs()) * half[i]).sum();
            along.abs() >= 1.0 + reach
        });
        !apart && ground.solid(site)
    })
}

/** Moves `feet` by `d` along `axis` as far as nothing blocks it, to within a 64th; returns whether something did. */
fn shift(ground: &impl Solid, feet: &mut [f64; 3], axis: usize, d: f64) -> bool {
    let at = |t: f64| {
        let mut p = *feet;
        p[axis] += t * d;
        blocked(ground, p)
    };
    if !at(1.0) {
        feet[axis] += d;
        return false;
    }
    let (mut free, mut hit) = (0.0, 1.0);
    for _ in 0..6 {
        let mid = (free + hit) / 2.0;
        if at(mid) { hit = mid } else { free = mid }
    }
    feet[axis] += free * d;
    true
}

// The WebAssembly interface: walk.ts starts a body (walk_start), hands it
// its ground's materials and grid when they change (walk_materials,
// walk_grid, walk_cells), and steps it (walk_step).

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

#[cfg(test)]
mod tests {
    use super::*;

    /** Voxels 2 units apart: a floor, its top's tips at y = 0, and those at sites in boxes [min, max) on it. */
    struct Scene(&'static [([f64; 3], [f64; 3])]);

    impl Solid for Scene {
        fn solid(&self, site: [i32; 3]) -> bool {
            let p = site.map(|v| v as f64 * self.scale());
            p[1] < 0.0 || self.0.iter().any(|(lo, hi)| (0..3).all(|d| (lo[d]..hi[d]).contains(&p[d])))
        }
        fn scale(&self) -> f64 {
            2.0
        }
    }

    const STILL: WalkInput = WalkInput { forward: 0.0, right: 0.0, yaw: 0.0, jump: false, sneak: false };
    const FORWARD: WalkInput = WalkInput { forward: 1.0, ..STILL };

    fn run(ground: &impl Solid, mut body: Walker, input: WalkInput, seconds: f64) -> Walker {
        for _ in 0..(seconds * 60.0) as usize {
            body = body.walk(input, 1.0 / 60.0, ground);
        }
        body
    }

    /** Dropped from 3 m at (0, z), once standing. */
    fn standing(ground: &impl Solid, z: f64) -> Walker {
        run(ground, Walker::at([0.0, 24.0 + EYE_HEIGHT, z]), STILL, 2.0)
    }

    #[test]
    fn stands_on_the_ground_and_walks_at_walking_speed() {
        let ground = Scene(&[]);
        let body = standing(&ground, 0.0);
        assert!(body.settled() && body.feet[1].abs() < 0.1, "{body:?}");
        let walked = run(&ground, body, FORWARD, 2.0);
        assert!((walked.feet[2] - 2.0 * WALK_SPEED).abs() < 1.0, "{walked:?}");
    }

    #[test]
    fn keeps_its_feet_on_the_tips_of_the_floor_wherever_it_stands_and_walks() {
        // The floor's top is its voxels' tips, the body always over some: the feet are at their height.
        let ground = Scene(&[]);
        for (x, yaw) in [(0.0, 0.0), (0.5, 30.0), (1.0, 45.0), (1.5, 90.0)] {
            let mut body = run(&ground, Walker::at([x, 24.0 + EYE_HEIGHT, 0.7]), STILL, 2.0);
            for _ in 0..60 {
                assert!(body.held && body.feet[1].abs() < 0.1, "at x = {x}, walking {yaw}°: {body:?}");
                body = body.walk(WalkInput { yaw, ..FORWARD }, 1.0 / 60.0, &ground);
            }
        }
    }

    #[test]
    fn steps_up_and_down_a_low_step_but_stops_at_a_wall() {
        let ground = Scene(&[([-48.0, 0.0, 0.0], [48.0, 2.0, 24.0]), ([-48.0, 0.0, 48.0], [48.0, 16.0, 56.0])]);
        let body = run(&ground, standing(&ground, -16.0), FORWARD, 4.0);
        assert!(body.feet[2] > 40.0 && body.feet[2] < 48.0 && body.feet[1].abs() < 0.1 && body.held, "{body:?}");
    }

    #[test]
    fn jumps_onto_a_ledge_it_cannot_step_onto() {
        let ground = Scene(&[([-48.0, 0.0, 0.0], [48.0, 8.0, 96.0])]);
        let walked = run(&ground, standing(&ground, -16.0), FORWARD, 2.0);
        assert!(walked.feet[2] < 0.0 && walked.feet[1] < 1.0, "{walked:?}");
        let jumped = run(&ground, walked, WalkInput { jump: true, ..FORWARD }, 1.0);
        assert!(jumped.feet[2] > 0.0 && jumped.feet[1] > 7.0, "{jumped:?}");
    }

    #[test]
    fn is_held_where_no_tile_is_yet_and_walks_level() {
        let none = Tiles::new(0, []);
        let ground = Ground { tiles: &none, walk_through: &[] };
        let body = run(&ground, Walker::at([0.0, 50.0, 0.0]), FORWARD, 1.0);
        assert!(body.settled() && body.eye()[1] == 50.0 && body.feet[2] > 30.0, "{body:?}");
    }
}
