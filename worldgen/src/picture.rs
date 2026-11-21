//! Pictures of a world at one level of detail, without the browser: for
//! a world's author to look at, and for checks to measure. Rays cross the
//! level's voxels exactly, one rhombic dodecahedron to the next; the tiles
//! they cross are generated as they reach them, as hosts make them
//! (generate_shown).

use std::collections::{BTreeSet, HashMap};
use std::panic::AssertUnwindSafe;
use std::sync::{Mutex, mpsc};
use std::time::{Duration, Instant};

use fcc::{FACE_KINDS, Look, VOXEL_FACES};

use crate::generator::{Generator, generate_shown, shown};
use crate::grid::{ALPHA, BRICK, Box3, COLOUR_ONE, GLOW_ONE, VoxelGrid};

/** Sites a tile is wide, at the picture's level. */
const TILE: i64 = 64;

/** A tile: where along each axis, in tiles. */
type Key = [i64; 3];

/** A picture's tiles: cubes, cut to its slab of heights (in the level's units). */
#[derive(Clone, Copy)]
struct Tiling {
    heights: (i64, i64),
}

impl Tiling {
    fn key(&self, p: [i64; 3]) -> Key {
        p.map(|v| v.div_euclid(TILE))
    }

    /** Tile `key`'s box: within the heights. */
    fn tile(&self, [i, j, k]: Key) -> Box3 {
        let (y0, y1) = ((j * TILE).clamp(self.heights.0, self.heights.1), ((j + 1) * TILE).clamp(self.heights.0, self.heights.1));
        Box3 { min: [i * TILE, y0, k * TILE], max: [(i + 1) * TILE, y1, (k + 1) * TILE] }
    }
}

/** The voxel's faces: towards its 12 neighbours, (p − site) · face ≤ 1 inside, in a level's units (fcc's VOXEL_FACES). */
pub const FACES: [[i64; 3]; FACE_KINDS] = {
    let mut faces = [[0; 3]; FACE_KINDS];
    let mut k = 0;
    while k < FACE_KINDS {
        faces[k] = [VOXEL_FACES[k][0] as i64, VOXEL_FACES[k][1] as i64, VOXEL_FACES[k][2] as i64];
        k += 1;
    }
    faces
};

/** Where the sun is, and how much light comes from the sky and from it. */
const SUN: [f64; 3] = [-0.48, 0.74, -0.47];
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
    /** Straight down from above the world, +x right and +z up, `width` lattice units across. */
    Top { width: f64 },
}

#[derive(Clone, Copy, Debug)]
pub struct Camera {
    /** Lattice units: the eye; looking down, the middle of the area (its height is ignored). */
    pub eye: [f64; 3],
    /** Radians: yaw 0 looks along +z, a quarter turn along +x; pitch up from level. */
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
        let yaw = d[0].atan2(d[2]);
        let pitch = d[1].atan2(d[0].hypot(d[2]));
        Camera { eye, yaw, pitch, projection: Projection::Perspective { fov }, width, height, far }
    }

    /** Looking straight down at the area from (x0, z0) to (x1, z1), `width` pixels across. */
    pub fn top(min: [f64; 2], max: [f64; 2], width: usize) -> Camera {
        let across = max[0] - min[0];
        let height = ((max[1] - min[1]) / across * width as f64).round().max(1.0) as usize;
        let eye = [(min[0] + max[0]) / 2.0, 0.0, (min[1] + max[1]) / 2.0];
        Camera { eye, yaw: 0.0, pitch: 0.0, projection: Projection::Top { width: across }, width, height, far: f64::INFINITY }
    }

    /** Pixel (i, j)'s ray: from where, which way (unit), lattice units; from above `top` looking down. */
    fn ray(&self, i: usize, j: usize, top: f64) -> ([f64; 3], [f64; 3]) {
        let (u, v) = ((i as f64 + 0.5) / self.width as f64 - 0.5, 0.5 - (j as f64 + 0.5) / self.height as f64);
        let aspect = self.height as f64 / self.width as f64;
        match self.projection {
            Projection::Top { width } => ([self.eye[0] + u * width, top, self.eye[2] + v * width * aspect], [0.0, -1.0, 0.0]),
            Projection::Perspective { fov } => {
                let (sy, cy, sp, cp) = (self.yaw.sin(), self.yaw.cos(), self.pitch.sin(), self.pitch.cos());
                let forward = [sy * cp, sp, cy * cp];
                let right = [cy, 0.0, -sy];
                let up = [-sp * sy, cp, -sp * cy];
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
    /** What the voxel hit looks like through the face hit (fcc's Look), 0 for none. */
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
 * A ray through the level's voxels, in its units: where it is, the voxel
 * it is in and the face it entered by (12: none); composited, how much of
 * each channel of it gets through what it crossed: a medium stops them
 * unlike.
 */
#[derive(Clone, Copy)]
struct Ray {
    pixel: usize,
    origin: [f64; 3],
    direction: [f64; 3],
    t: f64,
    end: f64,
    site: [i64; 3],
    face: usize,
    through: [f64; 3],
}

impl Ray {
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
pub(crate) struct Seen {
    pub(crate) stopped: f64,
    pub(crate) shown: [f64; 3],
    pub(crate) glow: [f64; 3],
}

/** Composited, a ray stops where less than this of it gets through. */
const OPAQUE: f64 = 1e-4;

/**
 * How many crossings of a site one along a row of a lattice stands for.
 * A row runs through its sites' middles, along their longest chord, √2
 * spacings; a ray at any offset weaves through the teeth of the rows
 * beside, crossing 3/2 as many sites, each by a shorter chord (a voxel's
 * mean chord along a face's direction is its volume, 2, over its shadow,
 * a hexagon of 3/√2). A whole voxel's shadow falls on the rows beside
 * too: a row's ray does not see that.
 */
pub const CROSSING: f64 = 1.5;

/**
 * How many times what a site of `alpha` stops and glows in one crossing
 * it does in `crossings` (part of one, or CROSSING): what it stops,
 * 1 − (1 − alpha)^crossings, over alpha. A whole voxel, once.
 */
pub fn over(alpha: f64, crossings: f64) -> f64 {
    if alpha > 0.0 && crossings != 1.0 { (1.0 - (1.0 - alpha).powf(crossings)) / alpha } else { crossings }
}

/**
 * What the site `ray` is in shows it, as light sees it: its pieces (as
 * seen along the ray, fcc's `facing`) and its medium of the rays they let
 * through, channel by channel: how much of the ray stops (in the channel
 * most gets through), what shows (linear, premultiplied), and what the
 * site glows to it; the rest goes on (`through`). A site stops its alpha
 * of a ray crossing its voxel: here, of one crossing `part` of the
 * voxel's chord (starting or ending in it) that stands for `crossing`
 * crossings (CROSSING along a row, 1 at any offset), `over` both.
 */
fn composite(ray: &mut Ray, grid: &VoxelGrid, part: f64, crossing: f64) -> (f64, [f64; 3], [f64; 3]) {
    let [x, y, z] = ray.site;
    let (mut stops, mut shows, mut glow, mut faces) = ([0.0; 3], [0.0; 3], [0.0; 3], 0.0);
    for k in fcc::facing(ray.direction) {
        let (pieces, medium, glows) = grid.light(x, y, z, k);
        for c in 0..3 {
            stops[c] += (pieces.0 as f64 + medium.0[c] as f64) / ALPHA as f64;
            shows[c] += (pieces.1[c] as f64 + medium.1[c] as f64) / COLOUR_ONE as f64;
            glow[c] += glows[c] as f64 / GLOW_ONE as f64;
        }
        faces += 1.0;
    }
    let before = ray.left();
    let (mut shown, mut glows) = ([0.0; 3], [0.0; 3]);
    for c in 0..3 {
        let through = ray.through[c] * over(stops[c] / faces, part * crossing) / faces;
        (shown[c], glows[c]) = (through * shows[c], through * glow[c]);
        ray.through[c] -= through * stops[c];
    }
    (before - ray.left(), shown, glows)
}

#[derive(Clone, Copy)]
enum Step {
    Hit(Look, [f64; 3]),
    Miss,
    /** The tile it reached is not generated yet. */
    Needs(Key),
}

/** A tile's grid, and the height of its highest brick with voxels. */
struct Tile {
    grid: VoxelGrid,
    top: i64,
}

impl Tile {
    fn new(grid: VoxelGrid) -> Tile {
        let [bx, by] = [0, 1].map(|i| (grid.n[i] + BRICK - 1) / BRICK);
        let highest = grid.bricks.iter().enumerate().filter(|(_, at)| **at >= 0).map(|(i, _)| (i as i64 / bx) % by).max();
        let top = highest.map_or(grid.min[1] - 1, |j| grid.min[1] + (j + 1) * BRICK - 1);
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

    /** What site (x, y, z) looks like; Err if its tile is not generated. */
    fn get(&self, [x, y, z]: [i64; 3]) -> Result<Look, Key> {
        if y < self.tiling.heights.0 || y >= self.tiling.heights.1 {
            return Ok(0);
        }
        let key = self.tiling.key([x, y, z]);
        self.tiles.get(&key).map(|tile| tile.grid.get(x, y, z)).ok_or(key)
    }

    /** Which way site's surface faces: towards its empty neighbours, unit; zero if all are empty (or none). Its tile's grid is `grid`. */
    fn normal(&self, site: [i64; 3], grid: &VoxelGrid) -> Result<[f64; 3], Key> {
        let mut sum = [0.0; 3];
        let key = self.tiling.key(site);
        for face in FACES {
            let p = [0, 1, 2].map(|i| site[i] + face[i]);
            let id = if self.tiling.key(p) == key && p[1] >= self.tiling.heights.0 && p[1] < self.tiling.heights.1 {
                grid.get(p[0], p[1], p[2])
            } else {
                self.get(p)?
            };
            if id == 0 {
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
    fn march(&self, ray: &mut Ray) -> Step {
        let walk = Walk::new(ray.direction);
        let mut planes = walk.planes(ray);
        let mut current: Option<(Key, &Tile)> = None;
        loop {
            if ray.t > ray.end {
                return Step::Miss;
            }
            let [x, y, z] = ray.site;
            if y >= self.tiling.heights.0 && y < self.tiling.heights.1 {
                let key = self.tiling.key([x, y, z]);
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
                // Space whose voxels are all empty: a voxel is within 1 of its site on every axis.
                let empty = if y > tile.top {
                    let b = self.tiling.tile(key);
                    leaves(ray, [b.min[0], tile.top + 1, b.min[2]].map(|v| v as f64), b.max.map(|v| (v - 1) as f64))
                } else {
                    match tile.grid.get_or_empty_brick(x, y, z) {
                        Some(0) => None,
                        Some(_) => {
                            let look = tile.grid.along(x, y, z, ray.direction);
                            return match self.normal(ray.site, &tile.grid) {
                                Ok(normal) => Step::Hit(look, normal),
                                Err(key) => Step::Needs(key),
                            };
                        }
                        None => {
                            let corner = [0, 1, 2].map(|i| tile.grid.min[i] + (ray.site[i] - tile.grid.min[i]).div_euclid(BRICK) * BRICK);
                            leaves(ray, corner.map(|v| v as f64), corner.map(|v| (v + BRICK - 1) as f64))
                        }
                    }
                };
                if let Some(t) = empty {
                    ray.t = t;
                    ray.site = nearest_site([0, 1, 2].map(|i| ray.origin[i] + t * ray.direction[i]));
                    ray.face = FACES.len();
                    planes = walk.planes(ray);
                    continue;
                }
            }
            walk.cross(ray, &mut planes);
        }
    }
}

/**
 * What a ray from `origin` along `direction` (a level's units) sees of
 * `grid` before `end`, site by site, as `composite`: how far along it
 * enters each that stops some of it or glows, how much it stops, what
 * that shows and what it glows; till next to none of it gets through.
 * Each site counts as `crossing` crossings (CROSSING along a row).
 */
pub(crate) fn composited(grid: &VoxelGrid, origin: [f64; 3], direction: [f64; 3], end: f64, crossing: f64) -> Vec<(f64, Seen)> {
    let mut ray = Ray { pixel: 0, origin, direction, t: 0.0, end, site: nearest_site(origin), face: FACES.len(), through: [1.0; 3] };
    let walk = Walk::new(direction);
    let mut planes = walk.planes(&ray);
    let mut seen = Vec::new();
    while ray.t <= end && ray.left() >= OPAQUE {
        let t = ray.t;
        let part = walk.part(&ray, &planes);
        let (stopped, shown, glow) = composite(&mut ray, grid, part, crossing);
        if stopped > 0.0 || glow != [0.0; 3] {
            seen.push((t, Seen { stopped, shown, glow }));
        }
        walk.cross(&mut ray, &mut planes);
    }
    seen
}

/** What site `site` of `grid` (a level's units) shows a ray along `direction` crossing its voxel whole. */
pub(crate) fn seen_whole(grid: &VoxelGrid, site: [i64; 3], direction: [f64; 3]) -> Seen {
    let origin = site.map(|v| v as f64);
    let mut ray = Ray { pixel: 0, origin, direction, t: 0.0, end: 0.0, site, face: FACES.len(), through: [1.0; 3] };
    let (stopped, shown, glow) = composite(&mut ray, grid, 1.0, 1.0);
    Seen { stopped, shown, glow }
}

/**
 * The face a ray enters its voxel by: the one opposite the face it left
 * the voxel before by, or, if it came from elsewhere, the face whose plane
 * it crosses last on its way in ((p − site) · face ≤ 1 inside). Where it
 * lands, as light does (light/src/reference.rs); what it sees there is along
 * its direction, whichever face it enters by (fcc's `facing`).
 */
fn entered(ray: &Ray) -> usize {
    if ray.face < FACE_KINDS {
        return FACES.iter().position(|f| *f == FACES[ray.face].map(|v| -v)).expect("faces come in pairs");
    }
    let r = [0, 1, 2].map(|i| ray.origin[i] - ray.site[i] as f64);
    let enters = |f: &[i64; 3]| {
        let (f, along) = (f.map(|v| v as f64), dot(f.map(|v| v as f64), ray.direction));
        if along < 0.0 { (1.0 - dot(r, f)) / along } else { f64::NEG_INFINITY }
    };
    (0..FACE_KINDS).max_by(|&a, &b| enters(&FACES[a]).total_cmp(&enters(&FACES[b]))).expect("faces")
}

/** Where a ray from `origin` along `direction` (a level's units) first enters a voxel of `grid`, if before `end`: how far, the voxel's site and the face it enters by. */
pub fn first_hit(grid: &VoxelGrid, origin: [f64; 3], direction: [f64; 3], end: f64) -> Option<(f64, [i64; 3], usize)> {
    let mut ray = Ray { pixel: 0, origin, direction, t: 0.0, end, site: nearest_site(origin), face: FACES.len(), through: [1.0; 3] };
    let walk = Walk::new(direction);
    let mut planes = walk.planes(&ray);
    while ray.t <= end {
        let [x, y, z] = ray.site;
        if grid.get(x, y, z) != 0 {
            return Some((ray.t, ray.site, entered(&ray)));
        }
        walk.cross(&mut ray, &mut planes);
    }
    None
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

/**
 * How a ray crosses voxels: the faces it can leave one by (at most two
 * of each pair of axes; FACES.len() for none), and per face it leaves by,
 * how much later it reaches each face's plane in the next voxel.
 */
struct Walk {
    faces: [usize; 6],
    inverse: [f64; 6],
    after: [[f64; 6]; 6],
}

impl Walk {
    fn new(direction: [f64; 3]) -> Walk {
        let mut walk = Walk { faces: [FACES.len(); 6], inverse: [0.0; 6], after: [[0.0; 6]; 6] };
        let mut n = 0;
        for (i, face) in FACES.iter().enumerate() {
            let along = dot(direction, face.map(|v| v as f64));
            if along > 1e-12 {
                (walk.faces[n], walk.inverse[n]) = (i, 1.0 / along);
                n += 1;
            }
        }
        for k in 0..n {
            for j in 0..n {
                let (a, b) = (FACES[walk.faces[k]], FACES[walk.faces[j]]);
                walk.after[k][j] = (a[0] * b[0] + a[1] * b[1] + a[2] * b[2]) as f64 * walk.inverse[j];
            }
        }
        walk
    }

    /** Takes the ray into the next voxel: out by the face whose plane it reaches first. */
    fn cross(&self, ray: &mut Ray, planes: &mut [f64; 6]) {
        let mut k = 0;
        for j in 1..planes.len() {
            if planes[j] < planes[k] {
                k = j;
            }
        }
        ray.t = ray.t.max(planes[k]);
        ray.face = self.faces[k];
        ray.site = [0, 1, 2].map(|i| ray.site[i] + FACES[ray.face][i]);
        for (plane, after) in planes.iter_mut().zip(self.after[k]) {
            *plane += after;
        }
    }

    /**
     * The share of its voxel's chord the ray crosses, from where it is to
     * where it leaves the voxel or ends. The line enters the voxel at the
     * last of the planes opposite those it may leave by, (p − site) · face
     * = −1, and leaves at the first it leaves by.
     */
    fn part(&self, ray: &Ray, planes: &[f64; 6]) -> f64 {
        let leaves = planes.iter().fold(f64::INFINITY, |a, &p| a.min(p));
        let enters = planes.iter().zip(self.inverse).filter(|(p, _)| p.is_finite()).fold(f64::NEG_INFINITY, |a, (p, i)| a.max(p - 2.0 * i));
        let chord = leaves - enters;
        if chord > 0.0 { ((leaves.min(ray.end) - ray.t) / chord).clamp(0.0, 1.0) } else { 0.0 }
    }

    /** When the ray reaches each face's plane of its voxel's: (p − site) · face = 1. */
    fn planes(&self, ray: &Ray) -> [f64; 6] {
        let r = [0, 1, 2].map(|i| ray.origin[i] - ray.site[i] as f64);
        std::array::from_fn(|j| match FACES.get(self.faces[j]) {
            Some(f) => (1.0 - (r[0] * f[0] as f64 + r[1] * f[1] as f64 + r[2] * f[2] as f64)) * self.inverse[j],
            None => f64::INFINITY,
        })
    }
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn normalize(v: [f64; 3]) -> [f64; 3] {
    let length = dot(v, v).sqrt();
    if length > 0.0 { v.map(|c| c / length) } else { v }
}

/** The site whose voxel holds p, in a level's units. */
pub fn nearest_site(p: [f64; 3]) -> [i64; 3] {
    let mut r = p.map(|v| v.round());
    if (r[0] + r[1] + r[2]) as i64 & 1 != 0 {
        // Off by one in the coordinate rounded the furthest.
        let e = [0, 1, 2].map(|i| (p[i] - r[i]).abs());
        let k = if e[0] >= e[1] && e[0] >= e[2] {
            0
        } else if e[1] >= e[2] {
            1
        } else {
            2
        };
        r[k] += if p[k] > r[k] { 1.0 } else { -1.0 };
    }
    r.map(|v| v as i64)
}

fn sky(direction: [f64; 3]) -> [f64; 3] {
    let up = direction[1].max(0.0).sqrt();
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
    let make = &mut |keys: &[Key]| keys.iter().map(|key| generate_shown(generator, &tiling.tile(*key), level)).collect();
    picture_of_tiles(camera, &mut Tiles::new(level, heights), &tops, make)
}

/** Where rays enter the slab of heights from above, by the tile they are over (x, z). */
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
    let [i0, k0] =
        [camera.eye[0] - width / 2.0, camera.eye[2] - depth / 2.0].map(|v| (v / s).floor() as i64 - 1).map(|v| v.div_euclid(TILE));
    let [i1, k1] =
        [camera.eye[0] + width / 2.0, camera.eye[2] + depth / 2.0].map(|v| (v / s).ceil() as i64 + 1).map(|v| v.div_euclid(TILE));
    let tops = (i0..=i1).flat_map(|i| (k0..=k1).map(move |k| [i, k])).map(|[i, k]| {
        // With a site more round it: a ray reads its neighbours.
        let column = Box3 { min: [i * TILE - 1, heights.0, k * TILE - 1], max: [(i + 1) * TILE + 1, heights.1, (k + 1) * TILE + 1] };
        ([i, k], shown(generator, &column, level).max[1])
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
                    let grid = std::panic::catch_unwind(AssertUnwindSafe(|| generate_shown(generator, &tiling.tile(key), level)));
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
        let column = [origin[0], origin[2]].map(|v| (v.floor() as i64).div_euclid(TILE));
        let high = tops.get(&column).map_or(top, |&t| t.min(heights.1) as f64 + 1.0);
        let (mut t, mut end) = (0.0, camera.far / s);
        if direction[1] != 0.0 {
            let (a, b) = ((low - origin[1]) / direction[1], (high - origin[1]) / direction[1]);
            (t, end) = (a.min(b).max(t), a.max(b).min(end));
        } else if origin[1] < low || origin[1] > high {
            end = -1.0;
        }
        if t > end {
            picture.set(pixel, 0, f64::INFINITY, sky(direction));
            continue;
        }
        let site = nearest_site([0, 1, 2].map(|i| origin[i] + t * direction[i]));
        rays.push(Ray { pixel, origin, direction, t, end, site, face: FACES.len(), through: [1.0; 3] });
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
        for (ray, step) in rays.into_iter().zip(steps) {
            match step {
                Step::Needs(key) => {
                    needed.insert(key);
                    left.push(ray);
                }
                Step::Miss => picture.set(ray.pixel, 0, f64::INFINITY, sky(ray.direction)),
                Step::Hit(look, normal) => {
                    let entered = match FACES.get(ray.face) {
                        Some(face) => normalize(face.map(|v| -v as f64)),
                        None => ray.direction.map(|v| -v),
                    };
                    // Mostly the surface's way, a little the voxel face's, to see the voxels.
                    let n = if normal == [0.0; 3] { entered } else { normalize([0, 1, 2].map(|i| normal[i] + 0.3 * entered[i])) };
                    let light = AMBIENT + SKYLIGHT * (0.5 + 0.5 * n[1]) + SUNLIGHT * dot(n, normalize(SUN)).max(0.0);
                    let lit = [0, 8, 16].map(|shift| ((look >> shift) & 255) as f64 * light);
                    let distance = ray.t * s;
                    let f = (fog * (distance / camera.far).powi(2)).min(fog);
                    picture.set(ray.pixel, look, distance, [0, 1, 2].map(|i| lit[i] + (HORIZON[i] - lit[i]) * f));
                    picture.sites[ray.pixel] = ray.site;
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

    /** As a PNG file: 8-bit RGB, stored uncompressed. */
    pub fn png(&self) -> Vec<u8> {
        let mut raw = Vec::with_capacity((3 * self.width + 1) * self.height);
        for row in self.rgb.chunks(3 * self.width) {
            raw.push(0); // no filter
            raw.extend_from_slice(row);
        }
        // zlib: deflate's stored blocks, at most 65535 bytes each.
        let mut zlib = vec![0x78, 0x01];
        let blocks: Vec<&[u8]> = if raw.is_empty() { vec![&[]] } else { raw.chunks(65535).collect() };
        for (i, block) in blocks.iter().enumerate() {
            let length = block.len() as u16;
            zlib.push((i + 1 == blocks.len()) as u8);
            zlib.extend(length.to_le_bytes());
            zlib.extend((!length).to_le_bytes());
            zlib.extend_from_slice(block);
        }
        zlib.extend(adler32(&raw).to_be_bytes());
        let mut header = Vec::new();
        header.extend((self.width as u32).to_be_bytes());
        header.extend((self.height as u32).to_be_bytes());
        header.extend([8, 2, 0, 0, 0]); // 8 bits, RGB, deflate, no filter, not interlaced
        let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
        for (kind, data) in [(b"IHDR", header), (b"IDAT", zlib), (b"IEND", Vec::new())] {
            png.extend((data.len() as u32).to_be_bytes());
            let start = png.len();
            png.extend_from_slice(kind);
            png.extend(data);
            let crc = crc32(&png[start..]);
            png.extend(crc.to_be_bytes());
        }
        png
    }
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &b in bytes {
        crc ^= b as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 { 0xEDB8_8320 ^ (crc >> 1) } else { crc >> 1 };
        }
    }
    !crc
}

fn adler32(bytes: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    // 5552 bytes at most between reductions keep b from overflowing.
    for chunk in bytes.chunks(5552) {
        for &byte in chunk {
            a += byte as u32;
            b += a;
        }
        (a, b) = (a % 65521, b % 65521);
    }
    (b << 16) | a
}

#[cfg(test)]
mod tests;
