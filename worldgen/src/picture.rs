//! Pictures of a world at one level of detail, without the browser: for
//! a world's author to look at, and for checks to measure. Rays cross the
//! level's voxels exactly, one rhombic dodecahedron to the next; the tiles
//! they cross are generated as they reach them.

use std::collections::{BTreeSet, HashMap};
use std::panic::AssertUnwindSafe;
use std::sync::{Mutex, mpsc};
use std::time::{Duration, Instant};

use crate::generator::{Generator, generate, materials_of};
use crate::grid::{BRICK, Box3, VoxelGrid};

/** Sites per tile in x and z, at the picture's level; a tile spans the heights. */
const TILE: i64 = 64;

/** The voxel's faces: towards its 12 neighbours, (p − site) · face ≤ 1 inside, in a level's units. */
const FACES: [[i64; 3]; 12] = [
    [1, 1, 0],
    [1, -1, 0],
    [-1, 1, 0],
    [-1, -1, 0],
    [1, 0, 1],
    [1, 0, -1],
    [-1, 0, 1],
    [-1, 0, -1],
    [0, 1, 1],
    [0, 1, -1],
    [0, -1, 1],
    [0, -1, -1],
];

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
pub struct Picture {
    pub width: usize,
    pub height: usize,
    /** RGB, 3 bytes a pixel. */
    pub rgb: Vec<u8>,
    /** The material id hit (materials_of), 0 for none. */
    pub material: Vec<u8>,
    /** How far along its ray it was hit, lattice units (looking down, from just above the heights); infinite for none. */
    pub depth: Vec<f32>,
    /** How many tiles were generated, and the time it took, and that of the rays. */
    pub tiles: usize,
    pub generating: Duration,
    pub casting: Duration,
}

/** A ray through the level's voxels, in its units: where it is, the voxel it is in and the face it entered by (12: none). */
#[derive(Clone, Copy)]
struct Ray {
    pixel: usize,
    origin: [f64; 3],
    direction: [f64; 3],
    t: f64,
    end: f64,
    site: [i64; 3],
    face: usize,
}

#[derive(Clone, Copy)]
enum Step {
    Hit(u8, [f64; 3]),
    Miss,
    /** The tile (x, z) it reached is not generated yet. */
    Needs((i64, i64)),
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

/** The level's tiles generated so far, and the heights of sites in them. */
struct Tiles {
    tiles: HashMap<(i64, i64), Tile>,
    heights: (i64, i64),
}

fn tile_of(x: i64, z: i64) -> (i64, i64) {
    (x.div_euclid(TILE), z.div_euclid(TILE))
}

impl Tiles {
    /** Site (x, y, z)'s material; Err if its tile is not generated. */
    fn get(&self, [x, y, z]: [i64; 3]) -> Result<u8, (i64, i64)> {
        if y < self.heights.0 || y >= self.heights.1 {
            return Ok(0);
        }
        let key = tile_of(x, z);
        self.tiles.get(&key).map(|tile| tile.grid.get(x, y, z)).ok_or(key)
    }

    /** Which way site's surface faces: towards its empty neighbours, unit; zero if all are empty (or none). Its tile's grid is `grid`. */
    fn normal(&self, site: [i64; 3], grid: &VoxelGrid) -> Result<[f64; 3], (i64, i64)> {
        let mut sum = [0.0; 3];
        let key = tile_of(site[0], site[2]);
        for face in FACES {
            let [x, y, z] = [0, 1, 2].map(|i| site[i] + face[i]);
            let id =
                if tile_of(x, z) == key && y >= self.heights.0 && y < self.heights.1 { grid.get(x, y, z) } else { self.get([x, y, z])? };
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
        let mut current: Option<((i64, i64), &Tile)> = None;
        loop {
            if ray.t > ray.end {
                return Step::Miss;
            }
            let [x, y, z] = ray.site;
            if y >= self.heights.0 && y < self.heights.1 {
                let key = tile_of(x, z);
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
                    let [x0, z0] = [key.0 * TILE, key.1 * TILE];
                    leaves(ray, [x0, tile.top + 1, z0].map(|v| v as f64), [(x0 + TILE - 1) as f64, f64::INFINITY, (z0 + TILE - 1) as f64])
                } else {
                    match tile.grid.get_or_empty_brick(x, y, z) {
                        Some(0) => None,
                        Some(id) => {
                            return match self.normal(ray.site, &tile.grid) {
                                Ok(normal) => Step::Hit(id, normal),
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
            // Out by the face whose plane it reaches first.
            let mut k = 0;
            for j in 1..planes.len() {
                if planes[j] < planes[k] {
                    k = j;
                }
            }
            ray.t = ray.t.max(planes[k]);
            ray.face = walk.faces[k];
            ray.site = [0, 1, 2].map(|i| ray.site[i] + FACES[ray.face][i]);
            for (plane, after) in planes.iter_mut().zip(walk.after[k]) {
                *plane += after;
            }
        }
    }
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
fn nearest_site(p: [f64; 3]) -> [i64; 3] {
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
    let colours: Vec<u32> = materials_of(generator).iter().map(|m| m.rgb()).collect();
    picture_of_tiles(camera, level, heights, &colours, &mut |keys| {
        keys.iter().map(|key| generate(generator, &tile(*key, heights), level)).collect()
    })
}

/**
 * As `picture`, generating tiles on all threads, each with its own of the
 * generators `make` makes (the same world: a generator is not Send).
 */
pub fn picture_on_threads(make: &(dyn Fn() -> Generator + Sync), camera: &Camera, level: u32, heights: (i64, i64)) -> Picture {
    let colours: Vec<u32> = materials_of(&make()).iter().map(|m| m.rgb()).collect();
    let (jobs, queue) = mpsc::channel::<(usize, (i64, i64))>();
    let (done, results) = mpsc::channel();
    let queue = Mutex::new(queue);
    std::thread::scope(|scope| {
        for _ in 0..threads() {
            let (queue, done) = (&queue, done.clone());
            scope.spawn(move || {
                let generator = make();
                loop {
                    let Ok((i, key)) = queue.lock().expect("no thread panicked").recv() else { break };
                    // A generator's panic, passed on to the picture's thread rather than leaving it waiting.
                    let grid = std::panic::catch_unwind(AssertUnwindSafe(|| generate(&generator, &tile(key, heights), level)));
                    done.send((i, grid)).expect("the picture waits for it");
                }
            });
        }
        drop(done);
        let picture = picture_of_tiles(camera, level, heights, &colours, &mut |keys| {
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

/** Tile (x, z)'s box, in a level's units. */
fn tile((x, z): (i64, i64), heights: (i64, i64)) -> Box3 {
    Box3 { min: [x * TILE, heights.0, z * TILE], max: [(x + 1) * TILE, heights.1, (z + 1) * TILE] }
}

fn threads() -> usize {
    std::thread::available_parallelism().map_or(1, |n| n.get())
}

/** Generates the tiles of the keys it is given, in their order. */
type MakeTiles<'a> = dyn FnMut(&[(i64, i64)]) -> Vec<VoxelGrid> + 'a;

fn picture_of_tiles(camera: &Camera, level: u32, heights: (i64, i64), colours: &[u32], make: &mut MakeTiles) -> Picture {
    let s = (1i64 << level) as f64;
    let pixels = camera.width * camera.height;
    let mut picture = Picture {
        width: camera.width,
        height: camera.height,
        rgb: vec![0; 3 * pixels],
        material: vec![0; pixels],
        depth: vec![f32::INFINITY; pixels],
        tiles: 0,
        generating: Duration::ZERO,
        casting: Duration::ZERO,
    };
    // Each ray from where it enters the slab of heights (and a voxel's reach past them) to where it leaves it, or `far`.
    let (low, high) = ((heights.0 - 1) as f64, (heights.1 + 1) as f64);
    let top = high * s;
    let mut rays = Vec::new();
    for pixel in 0..pixels {
        let (from, direction) = camera.ray(pixel % camera.width, pixel / camera.width, top);
        let origin = from.map(|v| v / s);
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
        rays.push(Ray { pixel, origin, direction, t, end, site, face: FACES.len() });
    }
    let fog = match camera.projection {
        Projection::Perspective { .. } => FOG,
        Projection::Top { .. } => 0.0,
    };
    let mut tiles = Tiles { tiles: HashMap::new(), heights };
    // In rounds: cast every ray till it stops or needs a tile, then generate the tiles needed.
    while !rays.is_empty() {
        let start = Instant::now();
        let mut steps = vec![Step::Miss; rays.len()];
        {
            let work = Mutex::new(rays.chunks_mut(1024).zip(steps.chunks_mut(1024)));
            let tiles = &tiles;
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
                Step::Hit(id, normal) => {
                    let entered = match FACES.get(ray.face) {
                        Some(face) => normalize(face.map(|v| -v as f64)),
                        None => ray.direction.map(|v| -v),
                    };
                    // Mostly the surface's way, a little the voxel face's, to see the voxels.
                    let n = if normal == [0.0; 3] { entered } else { normalize([0, 1, 2].map(|i| normal[i] + 0.3 * entered[i])) };
                    let light = AMBIENT + SKYLIGHT * (0.5 + 0.5 * n[1]) + SUNLIGHT * dot(n, normalize(SUN)).max(0.0);
                    let c = colours[id as usize];
                    let lit = [16, 8, 0].map(|shift| ((c >> shift) & 255) as f64 * light);
                    let distance = ray.t * s;
                    let f = (fog * (distance / camera.far).powi(2)).min(fog);
                    picture.set(ray.pixel, id, distance, [0, 1, 2].map(|i| lit[i] + (HORIZON[i] - lit[i]) * f));
                }
            }
        }
        rays = left;
        picture.casting += start.elapsed();
        let start = Instant::now();
        let needed: Vec<(i64, i64)> = needed.into_iter().collect();
        tiles.tiles.extend(needed.iter().copied().zip(make(&needed).into_iter().map(Tile::new)));
        picture.generating += start.elapsed();
    }
    picture.tiles = tiles.tiles.len();
    picture
}

impl Picture {
    fn set(&mut self, pixel: usize, id: u8, depth: f64, rgb: [f64; 3]) {
        self.material[pixel] = id;
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
mod tests {
    use super::*;
    use crate::generator::{ANY_SIZE, KindDef, Layer, Node, Op, Population, SOLID, ball};

    #[test]
    fn checksums_are_zlib_and_png_ones() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(adler32(b"Wikipedia"), 0x11E6_0398);
        assert_eq!(adler32(&vec![255; 100_000]), {
            let (mut a, mut b) = (1u64, 0u64);
            for _ in 0..100_000 {
                a = (a + 255) % 65521;
                b = (b + a) % 65521;
            }
            ((b << 16) | a) as u32
        });
    }

    #[test]
    fn a_png_holds_its_pixels() {
        let (width, height) = (300, 250); // more than one stored block
        let rgb: Vec<u8> = (0..3 * width * height).map(|i| (i * 7 % 251) as u8).collect();
        let picture = Picture {
            width,
            height,
            rgb: rgb.clone(),
            material: vec![0; width * height],
            depth: vec![0.0; width * height],
            tiles: 0,
            generating: Duration::ZERO,
            casting: Duration::ZERO,
        };
        let png = picture.png();
        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
        assert_eq!(&png[png.len() - 12..], [0, 0, 0, 0, b'I', b'E', b'N', b'D', 0xAE, 0x42, 0x60, 0x82]);
        // Its chunks, each with its CRC; the image data inflated from its stored blocks.
        let (mut at, mut idat) = (8, Vec::new());
        while at < png.len() {
            let length = u32::from_be_bytes(png[at..at + 4].try_into().unwrap()) as usize;
            let body = &png[at + 4..at + 8 + length];
            assert_eq!(crc32(body), u32::from_be_bytes(png[at + 8 + length..at + 12 + length].try_into().unwrap()));
            match &body[..4] {
                b"IHDR" => assert_eq!(body[4..], [0, 0, 1, 44, 0, 0, 0, 250, 8, 2, 0, 0, 0]),
                b"IDAT" => idat.extend_from_slice(&body[4..]),
                _ => {}
            }
            at += 12 + length;
        }
        assert_eq!((u16::from_be_bytes([idat[0], idat[1]]) % 31, idat[0] & 15), (0, 8));
        let (mut raw, mut at) = (Vec::new(), 2);
        loop {
            let length = u16::from_le_bytes([idat[at + 1], idat[at + 2]]) as usize;
            assert_eq!(!length as u16, u16::from_le_bytes([idat[at + 3], idat[at + 4]]));
            raw.extend_from_slice(&idat[at + 5..at + 5 + length]);
            let last = idat[at] & 1 == 1;
            at += 5 + length;
            if last {
                break;
            }
        }
        assert_eq!(u32::from_be_bytes(idat[at..at + 4].try_into().unwrap()), adler32(&raw));
        let rows: Vec<u8> = raw
            .chunks(3 * width + 1)
            .flat_map(|row| {
                assert_eq!(row[0], 0);
                row[1..].to_vec()
            })
            .collect();
        assert_eq!(rows, rgb);
    }

    /** A ball of radius 40 at the origin, on the corner of four tiles. */
    fn a_ball() -> Generator {
        Generator {
            kinds: vec![KindDef { kind_type: 1, colors: &["#c04030"] }],
            seed: 1,
            layers: vec![Layer::Population(Population {
                name: "ball",
                order: 1.0,
                cell: 1024,
                reach: [64; 3],
                size: ANY_SIZE,
                anchor: Box::new(|cell| if cell.i == 0 && cell.j == 0 && cell.k == 0 { Some([0.0; 3]) } else { None }),
                make: Box::new(|at, _| Some(Node::of(vec![ball(at, 40.0, 0, SOLID, Op::Fill)]))),
                far: None,
            })],
        }
    }

    #[test]
    fn a_picture_of_a_ball_shows_it_in_the_middle() {
        let generator = a_ball();
        for level in [0, 2] {
            let heights = (-64 >> level, 64 >> level);
            // From 200 units away, 60° across: the ball spans about a fifth of the picture.
            let camera = Camera::looking_at([0.0, 0.0, -200.0], [0.0; 3], 60f64.to_radians(), 64, 48, 1000.0);
            let seen = picture(&generator, &camera, level, heights);
            let middle = 24 * 64 + 32;
            assert_eq!(seen.material[middle], 1, "level {level}: the ball is in the middle");
            assert!((155.0..=162.0).contains(&seen.depth[middle]), "level {level}: {} away", seen.depth[middle]);
            assert_eq!((seen.material[0], seen.depth[0]), (0, f32::INFINITY), "level {level}: the corner misses");
            let threaded = picture_on_threads(&a_ball, &camera, level, heights);
            assert!(threaded.rgb == seen.rgb && threaded.depth == seen.depth, "level {level}: the same on threads");
            let from_above = picture(&generator, &Camera::top([-100.0, -100.0], [100.0, 100.0], 32), level, heights);
            assert_eq!((from_above.material[16 * 32 + 16], from_above.material[0]), (1, 0), "level {level}: from above");
        }
    }
}
