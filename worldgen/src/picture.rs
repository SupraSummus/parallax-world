//! Pictures of a world at one level of detail, without the browser: for
//! a world's author to look at, and for checks to measure. Rays cross the
//! level's voxels exactly (lattice's ray.rs); the tiles they cross are
//! generated as they reach them, as hosts make them (generate_shown).

use std::collections::{BTreeSet, HashMap};
use std::panic::AssertUnwindSafe;
use std::sync::{Mutex, mpsc};
use std::time::{Duration, Instant};

use lattice::Look;
use lattice::ray::{FACES, Ray, over};

use crate::generator::{Generator, across, generate_shown, shown, turned};
use crate::grid::{ALPHA, BRICK, Box3, COLOUR_ONE, GLOW_ONE, VoxelGrid};
use crate::world::UP;

/** Sites a tile is wide, at the picture's level. */
const TILE: i64 = 64;

/** A tile: where along each axis, in tiles. */
type Key = [i64; 3];

/** A picture's tiles: cubes, cut to its slab of heights along UP (in the level's units). */
#[derive(Clone, Copy)]
struct Tiling {
    heights: (i64, i64),
}

impl Tiling {
    fn key(&self, p: [i64; 3]) -> Key {
        p.map(|v| v.div_euclid(TILE))
    }

    /** Tile `key`'s box: within the heights. */
    fn tile(&self, key: Key) -> Box3 {
        let cube = Box3 { min: key.map(|v| v * TILE), max: key.map(|v| (v + 1) * TILE) };
        let [from, to] = [cube.min[UP], cube.max[UP]].map(|v| v.clamp(self.heights.0, self.heights.1));
        cube.slab(UP, from, to)
    }
}

/** Where the sun is, over UP, and how much light comes from the sky and from it. */
const SUN: [f64; 3] = turned(UP, [-0.48, -0.47], 0.74);
const AMBIENT: f64 = 0.36;
const SKYLIGHT: f64 = 0.2;
const SUNLIGHT: f64 = 0.48;
/** Sky at the horizon and straight up; fog takes the horizon's. */
const HORIZON: [f64; 3] = [200.0, 218.0, 232.0];
const ZENITH: [f64; 3] = [118.0, 166.0, 222.0];
/** How much fog covers what is `far` away; nearer, as the square of the distance. */
const FOG: f64 = 0.5;

#[derive(Clone, Copy, Debug)]
pub enum Projection {
    /** Field of view across the picture's width, radians. */
    Perspective { fov: f64 },
    /** Straight down UP from above the world, +x right and +z up, `width` lattice units across. */
    Top { width: f64 },
}

#[derive(Clone, Copy, Debug)]
pub struct Camera {
    /** Lattice units: the eye; looking down, the middle of the area (its height is ignored). */
    pub eye: [f64; 3],
    /** Radians: yaw 0 looks along +z, a quarter turn along +x; pitch up from level (UP is +y; `turned` places the others). */
    pub yaw: f64,
    pub pitch: f64,
    pub projection: Projection,
    pub width: usize,
    pub height: usize,
    /** How far rays go, lattice units; fog thickens towards it. */
    pub far: f64,
}

impl Camera {
    /** A perspective camera at `eye` looking at `target`. */
    pub fn looking_at(eye: [f64; 3], target: [f64; 3], fov: f64, width: usize, height: usize, far: f64) -> Camera {
        let d = [0, 1, 2].map(|i| target[i] - eye[i]);
        let [a, b] = across(UP).map(|i| d[i]);
        let (yaw, pitch) = (a.atan2(b), d[UP].atan2(a.hypot(b)));
        Camera { eye, yaw, pitch, projection: Projection::Perspective { fov }, width, height, far }
    }

    /** Looking straight down at the area from (x0, z0) to (x1, z1), `width` pixels across. */
    pub fn top(min: [f64; 2], max: [f64; 2], width: usize) -> Camera {
        let across = max[0] - min[0];
        let height = ((max[1] - min[1]) / across * width as f64).round().max(1.0) as usize;
        let eye = turned(UP, [0, 1].map(|i| (min[i] + max[i]) / 2.0), 0.0);
        Camera { eye, yaw: 0.0, pitch: 0.0, projection: Projection::Top { width: across }, width, height, far: f64::INFINITY }
    }

    /** Pixel (i, j)'s ray: from where, which way (unit), lattice units; from above `top` looking down. */
    fn ray(&self, i: usize, j: usize, top: f64) -> ([f64; 3], [f64; 3]) {
        let (u, v) = ((i as f64 + 0.5) / self.width as f64 - 0.5, 0.5 - (j as f64 + 0.5) / self.height as f64);
        let aspect = self.height as f64 / self.width as f64;
        match self.projection {
            Projection::Top { width } => {
                let [a, b] = across(UP).map(|i| self.eye[i]);
                (turned(UP, [a + u * width, b + v * width * aspect], top), turned(UP, [0.0; 2], -1.0))
            }
            Projection::Perspective { fov } => {
                let (sy, cy, sp, cp) = (self.yaw.sin(), self.yaw.cos(), self.pitch.sin(), self.pitch.cos());
                let forward = turned(UP, [sy * cp, cy * cp], sp);
                let right = turned(UP, [cy, -sy], 0.0);
                let up = turned(UP, [-sp * sy, -sp * cy], cp);
                let span = 2.0 * (fov / 2.0).tan();
                let d: [f64; 3] = std::array::from_fn(|k| forward[k] + u * span * right[k] + v * span * aspect * up[k]);
                (self.eye, normalize(d))
            }
        }
    }
}

/** A picture, row by row from the top: colours, and what each pixel's ray hit and how far away. */
#[derive(Default)]
pub struct Picture {
    pub width: usize,
    pub height: usize,
    /** RGB, 3 bytes a pixel. */
    pub rgb: Vec<u8>,
    /** What the voxel hit looks like through the face hit (lattice's Look), 0 for none. */
    pub looks: Vec<Look>,
    /** How far along its ray it was hit, lattice units (looking down, from just above the heights); infinite for none. */
    pub depth: Vec<f32>,
    /** The site of the voxel hit, in the level's units; any where none was. */
    pub sites: Vec<[i64; 3]>,
    /** How many tiles were generated, and the time it took, and that of the rays. */
    pub tiles: usize,
    pub generating: Duration,
    pub casting: Duration,
}

/**
 * A pixel's ray through the level's voxels, in its units, till `end`;
 * composited, how much of each channel of it gets through what it
 * crossed: a medium stops them unlike.
 */
#[derive(Clone, Copy)]
struct Cast {
    pixel: usize,
    ray: Ray,
    end: f64,
    through: [f64; 3],
}

impl Cast {
    fn new(pixel: usize, ray: Ray, end: f64) -> Cast {
        Cast { pixel, ray, end, through: [1.0; 3] }
    }

    /** How much of it gets through, in the channel most does. */
    fn left(&self) -> f64 {
        self.through.into_iter().fold(0.0, f64::max)
    }
}

/**
 * What rays see composited, each site by its alpha through what is in
 * front of it, as light does: how much of them it stops, 0 what gets
 * through everything, and what that shows (linear, premultiplied); and
 * what glowing sites send them (linear, of a site all of a glowing white).
 */
#[derive(Clone, Debug, Default)]
pub struct Seen {
    pub stopped: f64,
    pub shown: [f64; 3],
    pub glow: [f64; 3],
}

/** Composited, a ray stops where less than this of it gets through. */
const OPAQUE: f64 = 1e-4;

/**
 * What the site `cast` is in shows it, as light sees it: its pieces (as
 * seen along the ray, lattice's `facing`) and its medium of the rays they let
 * through, channel by channel: how much of the ray stops (in the channel
 * most gets through), what shows (linear, premultiplied), and what the
 * site glows to it; the rest goes on (`through`): of a ray going `path`
 * in its voxel, so many crossings of it (lattice's `over`).
 */
fn composite(cast: &mut Cast, grid: &VoxelGrid, path: f64) -> (f64, [f64; 3], [f64; 3]) {
    let [x, y, z] = cast.ray.site;
    let (mut stops, mut shows, mut glow, mut faces) = ([0.0; 3], [0.0; 3], [0.0; 3], 0.0);
    for k in lattice::facing(cast.ray.direction) {
        let (pieces, medium, glows) = grid.light(x, y, z, k);
        for c in 0..3 {
            stops[c] += (pieces.0 as f64 + medium.0[c] as f64) / ALPHA as f64;
            shows[c] += (pieces.1[c] as f64 + medium.1[c] as f64) / COLOUR_ONE as f64;
            glow[c] += glows[c] as f64 / GLOW_ONE as f64;
        }
        faces += 1.0;
    }
    let before = cast.left();
    let (mut shown, mut glows) = ([0.0; 3], [0.0; 3]);
    for c in 0..3 {
        let through = cast.through[c] * over(stops[c] / faces, path) / faces;
        (shown[c], glows[c]) = (through * shows[c], through * glow[c]);
        cast.through[c] -= through * stops[c];
    }
    (before - cast.left(), shown, glows)
}

#[derive(Clone, Copy)]
enum Step {
    Hit(Look, [f64; 3]),
    Miss,
    /** The tile it reached is not generated yet. */
    Needs(Key),
}

/** A tile's grid, and the last height along UP its bricks with voxels reach. */
struct Tile {
    grid: VoxelGrid,
    top: i64,
}

impl Tile {
    fn new(grid: VoxelGrid) -> Tile {
        let top = grid.end_of_bricks(UP).unwrap_or(grid.min[UP]) - 1;
        Tile { grid, top }
    }
}

/** A level's tiles generated so far, and the heights of sites in them. */
struct Tiles {
    tiles: HashMap<Key, Tile>,
    level: u32,
    tiling: Tiling,
}

impl Tiles {
    /** None yet, of `level` between `heights` (in the level's units). */
    fn new(level: u32, heights: (i64, i64)) -> Tiles {
        Tiles { tiles: HashMap::new(), level, tiling: Tiling { heights } }
    }

    /** Whether site `p` is within the heights. */
    fn within(&self, p: [i64; 3]) -> bool {
        (self.tiling.heights.0..self.tiling.heights.1).contains(&p[UP])
    }

    /** Whether site `p` shows whole (VoxelGrid's `shows_whole`); Err if its tile is not generated. */
    fn whole(&self, p: [i64; 3]) -> Result<bool, Key> {
        if !self.within(p) {
            return Ok(false);
        }
        let key = self.tiling.key(p);
        self.tiles.get(&key).map(|tile| tile.grid.shows_whole(p[0], p[1], p[2])).ok_or(key)
    }

    /** Which way site's surface faces: towards its neighbours not whole (`whole`), unit; zero if none is whole (or all). Its tile's grid is `grid`. */
    fn normal(&self, site: [i64; 3], grid: &VoxelGrid) -> Result<[f64; 3], Key> {
        let mut sum = [0.0; 3];
        let key = self.tiling.key(site);
        for face in FACES {
            let p = [0, 1, 2].map(|i| site[i] + face[i]);
            let whole = if self.tiling.key(p) == key && self.within(p) { grid.shows_whole(p[0], p[1], p[2]) } else { self.whole(p)? };
            if !whole {
                (0..3).for_each(|i| sum[i] += face[i] as f64);
            }
        }
        Ok(normalize(sum))
    }

    /**
     * How far the ray goes: to a voxel, out of the slab of heights or past
     * its end, or to a tile not generated. It skips what holds nothing:
     * the space over a tile's highest brick, and empty bricks.
     */
    fn march(&self, cast: &mut Cast) -> Step {
        let ray = &mut cast.ray;
        let mut current: Option<(Key, &Tile)> = None;
        loop {
            if ray.t > cast.end {
                return Step::Miss;
            }
            let [x, y, z] = ray.site;
            if self.within(ray.site) {
                let key = self.tiling.key(ray.site);
                let tile = match current {
                    Some((k, tile)) if k == key => tile,
                    _ => match self.tiles.get(&key) {
                        Some(tile) => {
                            current = Some((key, tile));
                            tile
                        }
                        None => return Step::Needs(key),
                    },
                };
                // Space whose voxels are all empty: a voxel is the cube from its site to the next.
                let empty = if ray.site[UP] > tile.top {
                    let b = self.tiling.tile(key);
                    let mut min = b.min;
                    min[UP] = tile.top + 1;
                    leaves(ray, min.map(|v| v as f64), b.max.map(|v| v as f64))
                } else {
                    match tile.grid.get_or_empty_brick(x, y, z) {
                        Some(_) if !tile.grid.shows_whole(x, y, z) => None,
                        Some(_) => {
                            let look = tile.grid.along(x, y, z, ray.direction);
                            return match self.normal(ray.site, &tile.grid) {
                                Ok(normal) => Step::Hit(look, normal),
                                Err(key) => Step::Needs(key),
                            };
                        }
                        None => {
                            let corner = [0, 1, 2].map(|i| tile.grid.min[i] + (ray.site[i] - tile.grid.min[i]).div_euclid(BRICK) * BRICK);
                            leaves(ray, corner.map(|v| v as f64), corner.map(|v| (v + BRICK) as f64))
                        }
                    }
                };
                if let Some(t) = empty {
                    *ray = Ray::new(ray.origin, ray.direction, t);
                    continue;
                }
            }
            ray.cross();
        }
    }
}

/**
 * What a ray from `origin` along `direction` (a level's units) sees of
 * `grid` before `end`, site by site, as `composite`: how far along it
 * enters each that stops some of it or glows, how much it stops, what
 * that shows and what it glows; till next to none of it gets through.
 */
pub fn composited(grid: &VoxelGrid, origin: [f64; 3], direction: [f64; 3], end: f64) -> Vec<(f64, Seen)> {
    let mut cast = Cast::new(0, Ray::new(origin, direction, 0.0), end);
    let mut seen = Vec::new();
    while cast.ray.t <= end && cast.left() >= OPAQUE {
        let t = cast.ray.t;
        let path = cast.ray.path(end);
        let (stopped, shown, glow) = composite(&mut cast, grid, path);
        if stopped > 0.0 || glow != [0.0; 3] {
            seen.push((t, Seen { stopped, shown, glow }));
        }
        cast.ray.cross();
    }
    seen
}

/** What site `site` of `grid` (a level's units) shows a ray along `direction` crossing its voxel whole. */
pub(crate) fn seen_whole(grid: &VoxelGrid, site: [i64; 3], direction: [f64; 3]) -> Seen {
    // From its middle, half its chord.
    let mut cast = Cast::new(0, Ray::new(site.map(|v| v as f64 + 0.5), direction, 0.0), 0.0);
    let path = 2.0 * cast.ray.path(f64::INFINITY);
    let (stopped, shown, glow) = composite(&mut cast, grid, path);
    Seen { stopped, shown, glow }
}

/** When the ray leaves the box from `min` to `max` (a level's units), if it is in it now and goes further in it. */
fn leaves(ray: &Ray, min: [f64; 3], max: [f64; 3]) -> Option<f64> {
    let (mut enter, mut leave) = (f64::NEG_INFINITY, f64::INFINITY);
    for i in 0..3 {
        let d = ray.direction[i];
        if d == 0.0 {
            if ray.origin[i] < min[i] || ray.origin[i] > max[i] {
                return None;
            }
            continue;
        }
        let (a, b) = ((min[i] - ray.origin[i]) / d, (max[i] - ray.origin[i]) / d);
        (enter, leave) = (enter.max(a.min(b)), leave.min(a.max(b)));
    }
    (enter <= ray.t + 1e-9 && leave > ray.t + 1e-9).then_some(leave)
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn normalize(v: [f64; 3]) -> [f64; 3] {
    let length = dot(v, v).sqrt();
    if length > 0.0 { v.map(|c| c / length) } else { v }
}

fn sky(direction: [f64; 3]) -> [f64; 3] {
    let up = direction[UP].max(0.0).sqrt();
    [0, 1, 2].map(|i| HORIZON[i] + (ZENITH[i] - HORIZON[i]) * up)
}

/**
 * What `camera` sees of the world at `level` (sites 2^level lattice units
 * apart) between `heights` (in the level's units). The tiles its rays
 * reach are generated on this thread; the rays are cast on all.
 */
pub fn picture(generator: &Generator, camera: &Camera, level: u32, heights: (i64, i64)) -> Picture {
    let tops = tops_of(generator, camera, level, heights);
    let tiling = Tiling { heights };
    let make = &mut |keys: &[Key]| keys.iter().map(|key| generate_shown(generator, &tiling.tile(*key), level, UP)).collect();
    picture_of_tiles(camera, &mut Tiles::new(level, heights), &tops, make)
}

/** Where rays enter the slab of heights from above, by the tile they are over (across UP). */
type Tops = HashMap<[i64; 2], i64>;

/**
 * Looking down, the top of what may show (`shown`) of each column of
 * tiles under the camera, in the level's units: over it all is whole,
 * and empty, as nothing is over a world's heights. So rays from above
 * don't cross an empty sky tile by tile, each made alone. None looking
 * any other way.
 */
fn tops_of(generator: &Generator, camera: &Camera, level: u32, heights: (i64, i64)) -> Tops {
    let Projection::Top { width } = camera.projection else { return Tops::new() };
    let (s, depth) = ((1i64 << level) as f64, width * camera.height as f64 / camera.width as f64);
    let ([a, b], half) = (across(UP).map(|i| camera.eye[i]), [width / 2.0, depth / 2.0]);
    let [i0, k0] = [a - half[0], b - half[1]].map(|v| (v / s).floor() as i64 - 1).map(|v| v.div_euclid(TILE));
    let [i1, k1] = [a + half[0], b + half[1]].map(|v| (v / s).ceil() as i64 + 1).map(|v| v.div_euclid(TILE));
    let tops = (i0..=i1).flat_map(|i| (k0..=k1).map(move |k| [i, k])).map(|column| {
        // With a site more round it: a ray reads its neighbours.
        let (mut min, mut max) = ([heights.0; 3], [heights.1; 3]);
        for (a, c) in across(UP).into_iter().zip(column) {
            (min[a], max[a]) = (c * TILE - 1, (c + 1) * TILE + 1);
        }
        (column, shown(generator, &Box3 { min, max }, level, UP).max[UP])
    });
    tops.collect()
}

/** As `picture`, generating tiles on all threads. */
pub fn picture_on_threads(generator: &Generator, camera: &Camera, level: u32, heights: (i64, i64)) -> Picture {
    let tops = tops_of(generator, camera, level, heights);
    let tiles = Tiles::new(level, heights);
    let tiling = tiles.tiling;
    let (jobs, queue) = mpsc::channel::<(usize, Key)>();
    let (done, results) = mpsc::channel();
    let queue = Mutex::new(queue);
    std::thread::scope(|scope| {
        for _ in 0..threads() {
            let (queue, done) = (&queue, done.clone());
            scope.spawn(move || {
                loop {
                    let Ok((i, key)) = queue.lock().expect("no thread panicked").recv() else { break };
                    // A generator's panic, passed on to the picture's thread rather than leaving it waiting.
                    let grid = std::panic::catch_unwind(AssertUnwindSafe(|| generate_shown(generator, &tiling.tile(key), level, UP)));
                    done.send((i, grid)).expect("the picture waits for it");
                }
            });
        }
        drop(done);
        let picture = picture_of_tiles(camera, &mut { tiles }, &tops, &mut |keys: &[Key]| {
            for job in keys.iter().copied().enumerate() {
                jobs.send(job).expect("a thread takes it");
            }
            let mut grids: Vec<Option<VoxelGrid>> = keys.iter().map(|_| None).collect();
            for _ in keys {
                let (i, grid) = results.recv().expect("a thread sends it");
                grids[i] = Some(grid.unwrap_or_else(|panic| std::panic::resume_unwind(panic)));
            }
            grids.into_iter().map(|grid| grid.expect("each sent")).collect()
        });
        // The threads end with the jobs.
        drop(jobs);
        picture
    })
}

fn threads() -> usize {
    std::thread::available_parallelism().map_or(1, |n| n.get())
}

/** Generates the tiles of the keys it is given, in their order. */
type MakeTiles<'a> = dyn FnMut(&[Key]) -> Vec<VoxelGrid> + 'a;

/** The picture `camera` sees in `tiles`, from above under `tops`, generating those it lacks with `make`. */
fn picture_of_tiles(camera: &Camera, tiles: &mut Tiles, tops: &Tops, make: &mut MakeTiles) -> Picture {
    let (level, heights) = (tiles.level, tiles.tiling.heights);
    let s = (1i64 << level) as f64;
    let pixels = camera.width * camera.height;
    let mut picture = Picture {
        width: camera.width,
        height: camera.height,
        rgb: vec![0; 3 * pixels],
        looks: vec![0; pixels],
        depth: vec![f32::INFINITY; pixels],
        sites: vec![[0; 3]; pixels],
        tiles: 0,
        generating: Duration::ZERO,
        casting: Duration::ZERO,
    };
    // Each ray from where it enters the slab of heights (and a voxel's reach past them) to where it leaves it, or `far`.
    let (low, top) = ((heights.0 - 1) as f64, (heights.1 + 1) as f64);
    let mut rays = Vec::new();
    for pixel in 0..pixels {
        let (from, direction) = camera.ray(pixel % camera.width, pixel / camera.width, top * s);
        let origin = from.map(|v| v / s);
        let column = across(UP).map(|i| (origin[i].floor() as i64).div_euclid(TILE));
        let high = tops.get(&column).map_or(top, |&t| t.min(heights.1) as f64 + 1.0);
        let (mut t, mut end) = (0.0, camera.far / s);
        if direction[UP] != 0.0 {
            let (a, b) = ((low - origin[UP]) / direction[UP], (high - origin[UP]) / direction[UP]);
            (t, end) = (a.min(b).max(t), a.max(b).min(end));
        } else if origin[UP] < low || origin[UP] > high {
            end = -1.0;
        }
        if t > end {
            picture.set(pixel, 0, f64::INFINITY, sky(direction));
            continue;
        }
        rays.push(Cast::new(pixel, Ray::new(origin, direction, t), end));
    }
    let fog = match camera.projection {
        Projection::Perspective { .. } => FOG,
        Projection::Top { .. } => 0.0,
    };
    let before = tiles.tiles.len();
    // In rounds: cast every ray till it stops or needs a tile, then generate the tiles needed.
    while !rays.is_empty() {
        let start = Instant::now();
        let mut steps = vec![Step::Miss; rays.len()];
        {
            let work = Mutex::new(rays.chunks_mut(1024).zip(steps.chunks_mut(1024)));
            let tiles = &*tiles;
            std::thread::scope(|scope| {
                for _ in 0..threads() {
                    scope.spawn(|| {
                        loop {
                            let Some((rays, steps)) = work.lock().expect("no thread panicked").next() else { break };
                            for (ray, step) in rays.iter_mut().zip(steps) {
                                *step = tiles.march(ray);
                            }
                        }
                    });
                }
            });
        }
        let mut needed = BTreeSet::new();
        let mut left = Vec::new();
        for (cast, step) in rays.into_iter().zip(steps) {
            let (pixel, ray) = (cast.pixel, cast.ray);
            match step {
                Step::Needs(key) => {
                    needed.insert(key);
                    left.push(cast);
                }
                Step::Miss => picture.set(pixel, 0, f64::INFINITY, sky(ray.direction)),
                Step::Hit(look, normal) => {
                    let entered = normalize(FACES[ray.entered()].map(|v| v as f64));
                    // Mostly the surface's way, a little the voxel face's, to see the voxels.
                    let n = if normal == [0.0; 3] { entered } else { normalize([0, 1, 2].map(|i| normal[i] + 0.3 * entered[i])) };
                    let light = AMBIENT + SKYLIGHT * (0.5 + 0.5 * n[UP]) + SUNLIGHT * dot(n, normalize(SUN)).max(0.0);
                    let lit = [0, 8, 16].map(|shift| ((look >> shift) & 255) as f64 * light);
                    let distance = ray.t * s;
                    let f = (fog * (distance / camera.far).powi(2)).min(fog);
                    picture.set(pixel, look, distance, [0, 1, 2].map(|i| lit[i] + (HORIZON[i] - lit[i]) * f));
                    picture.sites[pixel] = ray.site;
                }
            }
        }
        rays = left;
        picture.casting += start.elapsed();
        let start = Instant::now();
        let needed: Vec<Key> = needed.into_iter().collect();
        tiles.tiles.extend(needed.iter().copied().zip(make(&needed).into_iter().map(Tile::new)));
        picture.generating += start.elapsed();
    }
    picture.tiles = tiles.tiles.len() - before;
    picture
}

impl Picture {
    fn set(&mut self, pixel: usize, look: Look, depth: f64, rgb: [f64; 3]) {
        self.looks[pixel] = look;
        self.depth[pixel] = depth as f32;
        for (i, c) in rgb.into_iter().enumerate() {
            self.rgb[3 * pixel + i] = c.round().clamp(0.0, 255.0) as u8;
        }
    }

    /** As a PNG file. */
    pub fn png(&self) -> Vec<u8> {
        crate::png::encode(self.width, self.height, 3, &self.rgb)
    }
}

#[cfg(test)]
mod tests;
