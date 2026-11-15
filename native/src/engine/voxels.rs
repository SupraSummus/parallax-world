//! The voxels engine: the view as rays through the world's own tiles
//! (voxels.wgsl), no chunks, nothing baked. A ray goes from voxel to voxel across their faces, each
//! level as far as its window holds (reach), then on in the next coarser
//! one. One ray is traced for every FACTOR × FACTOR pixels; each pixel then
//! takes the nearest of the voxels its four nearest samples met that its
//! own ray enters. See-through voxels add up along the samples' rays, in
//! the order met, each by the ray's path through it.
//!
//! The GPU holds, of each level, the bricks of a ball around the eye's
//! brick (Ball) in a window of WINDOW³ bricks, moved brick by brick: a
//! table of their slots in a pool, a torus (a brick's entry is at its
//! coordinates modulo WINDOW), so that the window moves without moving the
//! rest. The levels' balls share a radius, as large as the pool holds
//! (Windows). Beside it, which cells of coarser FCC lattices no voxel
//! meets (CELLS): rays go across those at once, as across a voxel. Tiles
//! never change once made, so a brick is uploaded once as it comes into a
//! ball, with its tile or as the ball moves, and its slot freed as it
//! leaves. A brick of no tile yet is unknown, and rays take the next
//! coarser level there: what is missing shows coarser, never as a hole.
//!
//! A voxel shows its look through the face a ray enters it by (a brick
//! whose voxels look unlike through some faces takes PARTS more slots for
//! them: Pool), times the light on it: shaded, by the way its surface
//! faces; lit, the light its tile found for it (world.rs, worldgen's
//! Light), its own glow counted in, uploaded with its brick to a pool of
//! its own.
//!
//! Lit, only what rays meet is lit: a brick with voxels and no light yet
//! is unknown, as one of no tile yet, so rays take the next coarser level
//! there, and ask for it (a bit per entry of the windows' tables, read
//! back: Asked). The world lights the bricks asked for, and the windows
//! place them once lit. In a world that is not flat, walls hide most of a
//! window's ball: what shows is far less.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use egui_wgpu::wgpu;
use fcc::GRID_LEVELS;
use fcc::light::{linear_to_srgb, rgb9e5, srgb_to_linear};
use worldgen::light::Light;

use super::{Engine, Frame, Look, Painter, Picture};
use crate::gpu::Easel;
use crate::view::{Vec3, focal_length, view_basis};
use crate::world::{Bricks, Level, Lit, TILE, Tile, TileId, Wants, World};

/** A level's window, in bricks a side (a power of two: the table is a torus). */
const WINDOW: i32 = 64;
/** Sites a brick is a side (bake's BRICK). */
const BRICK_SHIFT: u32 = 3;
const BRICK: i32 = 1 << BRICK_SHIFT;
/** Sites a brick holds: a word each, of its look, and lit, of its light. */
const BRICK_SITES: usize = (BRICK * BRICK * BRICK / 2) as usize;
/** Pixels each way per ray traced. */
const FACTOR: u32 = 4;
/** Bytes of bricks the pool holds at most. */
const POOL_BYTES: u64 = 256 << 20;

/** The largest radius of a window's ball (Ball): its bricks, at most RADIUS + 1 from the middle each way, within the window. */
const RADIUS: i32 = WINDOW / 2 - 2;
/** How far around the eye rays read, in a level's sites, at most: as far as a ball of RADIUS holds (reach_of). */
const READ: f64 = (RADIUS * BRICK) as f64;

const TABLE: usize = (WINDOW * WINDOW * WINDOW) as usize;
/**
 * Cells of S·FCC, at SCALES sizes S = BRICK · 4^j: rhombic dodecahedra S
 * times a voxel, about every other corner of blocks of 4^j bricks a side,
 * those whose coordinates (in blocks) add up to an even number. A ray goes
 * across one that no voxel meets from face to face at once, as it goes
 * across a voxel. A window holds, per cell, how many of the bricks around
 * it have voxels that meet it or are unknown, a torus as its table:
 * CELLS of the finer scale, then the coarser's (voxels.wgsl's fine and coarse).
 *
 * Every brick out of the ball is unknown, and counts in every cell around
 * it: counts start full, only bricks of tiles change them, and a ball
 * moving through unknown space costs them nothing. Rays read nothing past
 * the ball (reach); a cell at its edge counted full is only not crossed at
 * once. A cell's entry in the torus is also that of the cell a window
 * away, whose bricks only add to its count. So a cell counted hollow holds
 * no voxel the ball holds, as a ray stepping voxel by voxel would find.
 */
const SCALES: usize = 2;
const CELLS: usize = TABLE / 2;
const COUNTS: usize = CELLS + (CELLS >> 6);
/**
 * The cells of a scale about the corners of a brick's block and of its
 * neighbours, whose corner is e ∈ {−1, 0, 1}³ blocks from the block's: a
 * bit each, (e.z + 1) · 9 + (e.y + 1) · 3 + e.x + 1. Its voxels meet no others.
 */
const AROUND: u32 = (1 << 27) - 1;
/** A table's entries other than slots (voxels.wgsl's). */
const EMPTY: i32 = -1;
const UNKNOWN: i32 = -2;

const SKY_TOP: [f32; 3] = [0x3f as f32 / 255.0, 0x73 as f32 / 255.0, 0xb8 as f32 / 255.0];
const SKY_HORIZON: [f32; 3] = [0xb9 as f32 / 255.0, 0xd3 as f32 / 255.0, 0xe8 as f32 / 255.0];

/**
 * How far rays go through a level, in its sites, so that a window's ball
 * of `radius` holds all they read, wherever in its brick the eye is: past
 * where a ray is, the site of the voxel it is in, up to 1 farther each
 * way, and that site's neighbours, √2 farther again, whose faces shade
 * it; all within `radius` bricks of the eye's brick, box to box.
 */
fn reach_of(radius: i32) -> f64 {
    ((radius * BRICK) as f64 - 1.0 - std::f64::consts::SQRT_2).max(0.0)
}

/** How far rays read level `level`, in world units, its window's ball of `radius`, if they reach `far` in all. */
fn reach(level: usize, radius: i32, far: f64) -> f64 {
    (reach_of(radius) * (1u64 << level) as f64).min(far)
}

/** How many levels rays read to reach `far`: up to the first that does, or all. */
fn levels(far: f64) -> usize {
    (0..GRID_LEVELS).find(|&l| reach(l, RADIUS, far) >= far).map_or(GRID_LEVELS, |l| l + 1)
}

/** The corner, in blocks of scale `j`, of the cell bit `e` of AROUND names around brick `b`. */
fn around(b: [i32; 3], j: usize, e: u32) -> [i32; 3] {
    let e = e as i32;
    let q = b.map(|v| v >> (2 * j));
    [q[0] + e % 3 - 1, q[1] + e / 3 % 3 - 1, q[2] + e / 9 - 1]
}

/** Where cell `k` of scale `j` (its coordinates adding up to an even number) is in a window's counts. */
fn cell_entry(k: [i32; 3], j: usize) -> usize {
    let across = WINDOW >> (2 * j);
    let w = k.map(|v| v.rem_euclid(across));
    j * CELLS + (((w[2] * across + w[1]) * across + w[0]) / 2) as usize
}

/**
 * Each brick of `grid` with voxels: its place in the grid's bricks, where
 * it is in the level's bricks (whose sites start where the grid's bricks
 * do, modulo BRICK), and where its sites start in the grid's cells, in
 * bricks.
 */
fn bricks(grid: &fcc::Grid) -> impl Iterator<Item = (usize, [i32; 3], usize)> + '_ {
    let count = grid.n.map(|v| (v + BRICK - 1) / BRICK);
    grid.bricks.iter().enumerate().filter(|b| *b.1 >= 0).map(move |(place, &first)| {
        let b = place as i32;
        let at = [b % count[0], b / count[0] % count[1], b / count[0] / count[1]];
        (place, std::array::from_fn(|d| grid.min[d].div_euclid(BRICK) + at[d]), first as usize)
    })
}

/**
 * Per brick of `grid`, by its place in the grid's bricks, the cells of
 * each scale around it (AROUND's bits) that its voxels meet, SCALES words
 * a brick: those its sites meet (MEETING). By place, as buried bricks
 * share their sites but not where they are.
 */
pub fn meets(grid: &fcc::Grid) -> Vec<u32> {
    meets_of(grid, |i| grid.cells[i] != 0)
}

/** As `meets`, of the voxels `kept` keeps, by index in the grid's cells. */
fn meets_of(grid: &fcc::Grid, kept: impl Fn(usize) -> bool) -> Vec<u32> {
    let mut out = vec![0; grid.bricks.len() * SCALES];
    for (place, brick, first) in bricks(grid) {
        for r in 0..BRICK * BRICK * BRICK {
            let p = [r % BRICK, r / BRICK % BRICK, r / BRICK / BRICK];
            let site: [i32; 3] = std::array::from_fn(|d| grid.min[d].rem_euclid(BRICK) + brick[d] * BRICK + p[d]);
            if (site[0] + site[1] + site[2]) & 1 != 0 || !kept(first * BRICK_SITES + (r >> 1) as usize) {
                continue;
            }
            for (j, meeting) in MEETING.iter().enumerate() {
                let size = BRICK << (2 * j);
                let [x, y, z] = std::array::from_fn(|d| brick[d].rem_euclid(1 << (2 * j)) * BRICK + p[d]);
                out[place * SCALES + j] |= meeting[((z * size + y) * size + x) as usize];
            }
        }
    }
    out
}

/** S: how many sites a side the blocks of scale `j` are, whose corners its cells are about. */
pub fn cell_size(j: usize) -> i32 {
    BRICK << (2 * j)
}

/** How many blocks of scale `j` a tile is a side. */
fn blocks(j: usize) -> i32 {
    TILE as i32 / cell_size(j)
}

/**
 * Which cells of each scale a tile's voxels meet, from its bricks' `meets`:
 * a bit per corner of its blocks, and one more each way; and which its
 * see-through voxels meet, if it has any. For world.rs's Level::meets.
 */
#[derive(Clone)]
pub struct Met {
    all: [Vec<u64>; SCALES],
    see_through: Option<[Vec<u64>; SCALES]>,
}

impl Met {
    /** Of tile `id`, its grid and its bricks' `meets`. */
    pub fn new(id: TileId, grid: &fcc::Grid, meets: &[u32]) -> Met {
        let see_through = |i: usize| !matches!(fcc::alpha_of(grid.cells[i]), 0 | fcc::ALPHA);
        let see_through = (0..grid.cells.len()).any(see_through).then(|| Met::bits(id, grid, &meets_of(grid, see_through)));
        Met { all: Met::bits(id, grid, meets), see_through }
    }

    /** By scale, the bits of the cells `meets` says tile `id`'s bricks meet. */
    fn bits(id: TileId, grid: &fcc::Grid, meets: &[u32]) -> [Vec<u64>; SCALES] {
        let mut bits: [Vec<u64>; SCALES] = std::array::from_fn(|j| vec![0; ((blocks(j) + 2).pow(3) as usize).div_ceil(64)]);
        for (place, brick, _) in bricks(grid) {
            for (j, bits) in bits.iter_mut().enumerate() {
                for e in (0..27).filter(|e| meets[place * SCALES + j] >> e & 1 != 0) {
                    let k = around(brick, j, e);
                    if (k[0] + k[1] + k[2]) & 1 == 0 {
                        let i = Met::bit(id, j, k).expect("a cell about a corner of the tile's blocks");
                        bits[i / 64] |= 1 << (i % 64);
                    }
                }
            }
        }
        bits
    }

    /** Where cell `k` of scale `j` is in tile `id`'s bits, if about a corner of its blocks or one more each way. */
    fn bit(id: TileId, j: usize, k: [i32; 3]) -> Option<usize> {
        let (n, w) = (blocks(j), blocks(j) + 2);
        let at: [i32; 3] = std::array::from_fn(|d| k[d] - id[d] * n + 1);
        at.iter().all(|v| (0..w).contains(v)).then(|| ((at[2] * w + at[1]) * w + at[0]) as usize)
    }

    /** Whether tile `id`'s voxels meet cell `k` of scale `j`, or (`see_through`) its see-through ones. */
    pub fn has(&self, id: TileId, j: usize, k: [i32; 3], see_through: bool) -> bool {
        let bits = if see_through { self.see_through.as_ref() } else { Some(&self.all) };
        bits.is_some_and(|bits| Met::bit(id, j, k).is_some_and(|i| bits[j][i / 64] >> (i % 64) & 1 != 0))
    }

    /** The tiles whose voxels may meet cell `k` of scale `j`: those of the blocks around its centre. */
    pub fn tiles(j: usize, k: [i32; 3]) -> [TileId; 2] {
        [-1, 1].map(|d| k.map(|v| (v + d).div_euclid(blocks(j))))
    }
}

/**
 * Per scale, per site of a block (S a side) from its corner, the cells
 * around the block (AROUND's bits) that its voxel meets: a site's depend
 * on where in its block it is alone. Voxel and cell are both rhombic
 * dodecahedra, of 1 and of S, so they meet where the site is less than
 * 1 + S from the cell's centre across each pair of faces.
 */
static MEETING: std::sync::LazyLock<[Vec<u32>; SCALES]> = std::sync::LazyLock::new(|| {
    std::array::from_fn(|j| {
        let size = BRICK << (2 * j);
        (0..size * size * size)
            .map(|i| {
                let site = [i % size, i / size % size, i / size / size];
                (0..27)
                    .filter(|e| {
                        // The cell's centre, from the block's corner: around()'s, in sites.
                        let corner = [e % 3 - 1, e / 3 % 3 - 1, e / 9 - 1];
                        let [x, y, z]: [i32; 3] = std::array::from_fn(|d| (site[d] - corner[d] * size).abs());
                        x + y <= size && y + z <= size && x + z <= size
                    })
                    .fold(0, |bits, e| bits | 1 << e)
            })
            .collect()
    })
});

/** The sites of the brick whose sites start at `first` in `grid`'s cells (in bricks), by index in them. */
fn sites(first: usize) -> std::ops::Range<usize> {
    first * BRICK_SITES..(first + 1) * BRICK_SITES
}

/** Slots the brick of `grid` whose sites start at `first` takes: one, and PARTS more if its voxels look unlike through some faces. */
fn slots(grid: &fcc::Grid, first: usize) -> usize {
    1 + if grid.has_views(sites(first)) { PARTS } else { 0 }
}

/**
 * Part `part` of the brick whose sites start at `first` in `grid`'s cells
 * (in bricks) as the GPU takes it, a word a site: part 0, each site's
 * look; part 1 + k, its look through face k (of VOXEL_FACES), its site's
 * where they do not differ.
 */
fn words(grid: &fcc::Grid, first: usize, part: usize) -> impl Iterator<Item = u32> + '_ {
    sites(first).map(move |i| match grid.cells[i] {
        look if part == 0 || look == 0 => look,
        look => grid.views_at(i).map_or(look, |views| views[part - 1]),
    })
}

/**
 * The light on the voxels of the brick of `grid`, of `level`, whose sites
 * start at `corner`, by site in the order of its cells: Light::on_seen,
 * and its own glow as light on its colour, as RGB9E5; 0 where none.
 * So that, times its colour, it is what the voxel sends out.
 */
pub fn light(grid: &fcc::Grid, level: u32, corner: [i32; 3], light: &mut Light) -> Box<[u32]> {
    let mut out = vec![0; BRICK_SITES];
    for r in 0..BRICK * BRICK * BRICK {
        let site: [i32; 3] = std::array::from_fn(|d| corner[d] + [r % BRICK, r / BRICK % BRICK, r / BRICK / BRICK][d]);
        let look = grid.get(site[0], site[1], site[2]);
        if (site[0] + site[1] + site[2]) & 1 != 0 || look == 0 {
            continue;
        }
        let at = site.map(i64::from);
        out[(r >> 1) as usize] = light.on_seen(level, at).map_or(0, |on| {
            let (glow, colour) = (light.glow(level, at), look.to_le_bytes().map(|c| srgb_to_linear(c as f32 / 255.0)));
            rgb9e5(std::array::from_fn(|c| on[c] + if colour[c] > 0.0 { glow[c] / colour[c] } else { 0.0 }))
        });
    }
    out.into()
}

/** What the painter tells the engine, lit: the bricks rays asked for, as last read back, not yet taken; and whether frames to come may ask for more, or place what was lit. */
struct Asked {
    bricks: Option<Bricks>,
    busy: bool,
}

/** Till its painter has drawn (an engine made anew, as when the world is lit anew), frames to come may ask for anything: busy, so that they come. */
impl Default for Asked {
    fn default() -> Asked {
        Asked { bricks: None, busy: true }
    }
}

/** The voxels engine: each frame, the tiles of the levels its rays read, as they are; lit, the bricks they asked for, as last read back. */
#[derive(Default)]
pub struct Voxels {
    asked: Arc<Mutex<Asked>>,
    bricks: Arc<Bricks>,
}

impl Engine for Voxels {
    fn frame(&mut self, look: &Look, world: &World) -> Frame {
        let count = levels(look.far);
        let levels: Vec<_> = (0..count).map(|l| world.level(l).clone()).collect();
        let tiles: usize = levels.iter().map(|l| l.tiles.len()).sum();
        let lit = if look.light.is_some() { Lit::Bricks } else { Lit::Not };
        let reach = std::array::from_fn(|l| (l < count).then_some(READ));
        let busy = {
            let mut asked = self.asked.lock().expect("asked");
            if let Some(mut bricks) = asked.bricks.take() {
                // Nearest first in each level's own sites, the finer first as near: as rays read the levels.
                let far = |&(level, corner): &(usize, [i32; 3])| {
                    let at = look.camera.map(|v| v / (1u64 << level) as f64);
                    (0..3).map(|d| (f64::from(corner[d] + BRICK / 2) - at[d]).powi(2)).sum::<f64>()
                };
                bricks.sort_by(|a, b| far(a).total_cmp(&far(b)).then(a.0.cmp(&b.0)));
                self.bricks = Arc::new(bricks);
            }
            lit == Lit::Bricks && asked.busy
        };
        let asked = self.asked.clone();
        Frame {
            picture: Box::new(Scene { levels, look: *look, asked }),
            // Rays read all around the eye.
            wants: Wants { reach, lit, tiles: world.lacking(&reach, look.camera), bricks: self.bricks.clone() },
            busy,
            stats: format!("{tiles} tiles"),
        }
    }
}

/** What to draw: each level's tiles, as `look` sees them; where to tell what rays ask for. */
struct Scene {
    levels: Vec<Arc<Level>>,
    look: Look,
    asked: Arc<Mutex<Asked>>,
}

impl Picture for Scene {
    fn draw(&self, easel: &mut Easel, encoder: &mut wgpu::CommandEncoder, pixels: [u32; 2]) {
        let lit = self.look.light.is_some();
        // Lit or not, its pool is another.
        if easel.held::<Marcher>().is_some_and(|m| m.lit != lit) {
            easel.painter = None;
        }
        let (marcher, device, queue) = easel.painter(|device, format| Marcher::new(device, format, lit));
        marcher.draw(device, queue, encoder, self, pixels);
    }
}

/** voxels.wgsl's Globals. */
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Globals {
    resolution: [f32; 2],
    /** The picture's sides over the focal length. */
    extent: [f32; 2],
    fog: f32,
    sky_fog: f32,
    /** 1 if voxels are lit by their tiles' light, 0 shaded by the way their surface faces. */
    lit: u32,
    _pad: u32,
    sky_top: [f32; 4],
    horizon: [f32; 4],
    /** Right, up, forward, in the world: columns of a mat3x3f. */
    view: [[f32; 4]; 3],
}

impl Globals {
    fn of(look: &Look, pixels: [u32; 2]) -> Globals {
        let [width, height] = pixels.map(f64::from);
        let focal = focal_length(look.fov, width, height);
        let column = |v: Vec3| [v[0] as f32, v[1] as f32, v[2] as f32, 0.0];
        // Dimmed in linear terms, as light adds up.
        let sky = look.light.map_or(1.0, |b| b.sky);
        let rgba = |c: [f32; 3]| {
            let [r, g, b] = c.map(|c| linear_to_srgb(srgb_to_linear(c) * sky));
            [r, g, b, 1.0]
        };
        Globals {
            resolution: [width as f32, height as f32],
            extent: [(width / focal) as f32, (height / focal) as f32],
            fog: (look.far * look.fog) as f32,
            // What of the sky the fog hides, as of something at the far end.
            sky_fog: (1.0 - (-1.0 / look.fog).exp()) as f32,
            lit: u32::from(look.light.is_some()),
            _pad: 0,
            sky_top: rgba(SKY_TOP),
            horizon: rgba(SKY_HORIZON),
            view: view_basis(look.yaw, look.pitch).map(column),
        }
    }
}

/** A grid level (Level in voxels.wgsl). */
#[repr(C)]
#[derive(Clone, Copy, Default, bytemuck::Pod, bytemuck::Zeroable)]
struct GpuLevel {
    min: [i32; 3],
    offset: u32,
    corner: [i32; 3],
    reach: f32,
    low: [f32; 3],
    filled: u32,
    high: [f32; 3],
    _pad: u32,
}

/** Where brick `b` (of a window holding it) is in a window's table. */
fn entry(b: [i32; 3]) -> usize {
    let w = b.map(|v| v.rem_euclid(WINDOW));
    ((w[2] * WINDOW + w[1]) * WINDOW + w[0]) as usize
}

/** The brick of a level, its bricks starting at `origin`, that `eye` is in. */
fn eye_brick(level: usize, eye: Vec3, origin: [i32; 3]) -> [i32; 3] {
    let scale = (1u64 << level) as f64;
    std::array::from_fn(|d| ((eye[d] / scale - origin[d] as f64) / BRICK as f64).floor() as i32)
}

/** How far apart, along an axis, two bricks `d` apart are, box to box, in bricks: squared. */
fn gap(d: i32) -> i32 {
    (d.abs() - 1).max(0).pow(2)
}

/**
 * The bricks a window holds: those no farther from its middle (the eye's
 * brick) than its radius, box to box. Up to RADIUS, inside the window
 * around the middle.
 */
#[derive(Clone, Copy, PartialEq, Debug)]
struct Ball {
    middle: [i32; 3],
    radius: i32,
}

impl Ball {
    fn contains(&self, b: [i32; 3]) -> bool {
        (0..3).map(|d| gap(b[d] - self.middle[d])).sum::<i32>() <= self.radius * self.radius
    }

    /** Its row at `y`, `z`: [from, to), empty if none. */
    fn row(&self, y: i32, z: i32) -> (i32, i32) {
        let left = self.radius * self.radius - gap(y - self.middle[1]) - gap(z - self.middle[2]);
        let w = if left < 0 { return (self.middle[0], self.middle[0]) } else { left.isqrt() + 1 };
        (self.middle[0] - w, self.middle[0] + w + 1)
    }

    /** How far around brick `b` it holds every brick: its radius less `b`'s distance from its middle, rounded up. */
    fn about(&self, b: [i32; 3]) -> i32 {
        let squared: i32 = (0..3).map(|d| (b[d] - self.middle[d]).pow(2)).sum();
        let root = squared.isqrt();
        self.radius - root - i32::from(root * root < squared)
    }

    /** The box [lo, hi) around it. */
    fn bounds(&self) -> ([i32; 3], [i32; 3]) {
        (self.middle.map(|v| v - self.radius - 1), self.middle.map(|v| v + self.radius + 2))
    }
}

/** The bricks of box [lo, hi) in `ball` and not in `except`, row by row: as many steps as bricks and rows. */
fn in_ball((lo, hi): ([i32; 3], [i32; 3]), ball: Ball, except: Option<Ball>) -> impl Iterator<Item = [i32; 3]> {
    let (around, past) = ball.bounds();
    let (lo, hi): ([i32; 3], [i32; 3]) = (std::array::from_fn(|d| lo[d].max(around[d])), std::array::from_fn(|d| hi[d].min(past[d])));
    (lo[2]..hi[2]).flat_map(move |z| {
        (lo[1]..hi[1]).flat_map(move |y| {
            let (from, to) = ball.row(y, z);
            let (from, to) = (from.max(lo[0]), to.min(hi[0]));
            // The row's run in `except`, skipped.
            let (a, b) = except.map_or((to, to), |e| e.row(y, z));
            (from..a.clamp(from, to.max(from))).chain(b.clamp(from, to.max(from))..to).map(move |x| [x, y, z])
        })
    })
}

/** Tile `id`'s box of bricks, [lo, hi), for a level whose bricks start at `origin`: its whole cube, its grid empty outside its box. */
fn tile_bricks(id: TileId, origin: [i32; 3]) -> ([i32; 3], [i32; 3]) {
    let lo = std::array::from_fn(|d| (id[d] * TILE as i32 - origin[d]).div_euclid(BRICK));
    (lo, lo.map(|v| v + TILE as i32 / BRICK))
}

/** A brick of a tile to upload: the slot it goes in, the tile, its place in the tile's grid's cells (in bricks), and which part of it (words). */
struct Added {
    slot: u32,
    tile: Arc<Tile>,
    first: usize,
    part: usize,
}

/** Slots a brick takes beyond its own if its voxels look unlike through some faces: its looks through each face, one a slot. */
const PARTS: usize = fcc::FACE_KINDS;

/**
 * The pool's slots: those free, and per slot, the next of its brick's
 * (EMPTY if none): a brick whose voxels look unlike through some faces
 * takes PARTS more, its looks through each face (words' parts), in a chain.
 */
struct Pool {
    free: Vec<u32>,
    next: Vec<i32>,
}

impl Pool {
    fn new(slots: usize) -> Pool {
        Pool { free: (0..slots as u32).rev().collect(), next: vec![EMPTY; slots] }
    }

    /** Takes slots for the brick at `first` of `tile`, `count` (slots), to upload in `added`; returns its own. */
    fn take(&mut self, tile: &Arc<Tile>, first: usize, count: usize, added: &mut Vec<Added>) -> u32 {
        let mut chain = Vec::with_capacity(count);
        for part in 0..count {
            let slot = self.free.pop().expect("slots for the bricks counted");
            added.push(Added { slot, tile: tile.clone(), first, part });
            chain.push(slot);
        }
        for pair in chain.windows(2) {
            self.next[pair[0] as usize] = pair[1] as i32;
        }
        chain[0]
    }

    /** Frees `slot` and those after it. */
    fn give(&mut self, slot: u32) -> usize {
        let (mut slot, mut count) = (slot as i32, 0);
        while slot >= 0 {
            self.free.push(slot as u32);
            slot = std::mem::replace(&mut self.next[slot as usize], EMPTY);
            count += 1;
        }
        count
    }
}

/** A tile a window holds bricks of; per brick of its grid (in its cells, its buried ones one: worldgen's VoxelGrid::bury), its slot (EMPTY if none) and how many entries hold it. */
struct Held {
    tile: Arc<Tile>,
    slots: Vec<(i32, u32)>,
}

/**
 * A level's window as the GPU holds it: from which tiles, the ball of
 * bricks it holds of them, its table (per brick a slot, EMPTY or UNKNOWN:
 * every brick outside the ball), and, per cell of each scale, how many
 * bricks around it have voxels that meet it or are unknown (SCALES's).
 * Bricks of a tile
 * that share their sites share a slot: the ground's rock, buried, a slot
 * a tile.
 */
struct Window {
    /** The tiles it holds, as last given; None while some are still to come or go. */
    level: Option<Arc<Level>>,
    tiles: HashMap<TileId, Held>,
    ball: Option<Ball>,
    /** Sites of the level's bricks start at origin + BRICK · b. */
    origin: [i32; 3],
    table: Vec<i32>,
    /** Per entry of the table, the cells of each scale around its brick it counts in (AROUND's bits): those its voxels meet, all if UNKNOWN. */
    meets: Vec<[u32; SCALES]>,
    counts: Vec<u32>,
    /** Per axis, per brick coordinate along it (modulo WINDOW, as the table), how many of the ball's bricks count in a cell. */
    rows: [[u32; WINDOW as usize]; 3],
    /** Slots it holds. */
    slots: usize,
    /** Lit, the bricks rays last asked for (asked's), to place once lit. */
    asked: Vec<[i32; 3]>,
}

impl Window {
    fn new() -> Window {
        Window {
            level: None,
            tiles: HashMap::new(),
            ball: None,
            origin: [0; 3],
            table: vec![UNKNOWN; TABLE],
            meets: vec![[AROUND; SCALES]; TABLE],
            // Every brick unknown: in a cell of scale j, the 27 blocks of 4^j bricks a side around it.
            counts: (0..COUNTS).map(|i| 27 << (6 * usize::from(i >= CELLS))).collect(),
            rows: [[0; WINDOW as usize]; 3],
            slots: 0,
            asked: Vec::new(),
        }
    }

    /** Where brick `b` starts, and the tile it is of. */
    fn site(&self, b: [i32; 3]) -> ([i32; 3], TileId) {
        let site: [i32; 3] = std::array::from_fn(|d| self.origin[d] + b[d] * BRICK);
        (site, site.map(|v| v.div_euclid(TILE as i32)))
    }

    /** Sets brick `b`'s entry, of the ball, and the cells around it that it counts in; returns the entry it had. */
    fn set(&mut self, b: [i32; 3], (entry, meets): (i32, [u32; SCALES])) -> i32 {
        let at = self::entry(b);
        let had = std::mem::replace(&mut self.meets[at], meets);
        if (had == [0; SCALES]) != (meets == [0; SCALES]) {
            for (rows, v) in self.rows.iter_mut().zip(b) {
                let count = &mut rows[v.rem_euclid(WINDOW) as usize];
                *count = if meets == [0; SCALES] { *count - 1 } else { *count + 1 };
            }
        }
        for j in 0..SCALES {
            for e in (0..27).filter(|e| (had[j] ^ meets[j]) >> e & 1 != 0) {
                let cell = around(b, j, e);
                if (cell[0] + cell[1] + cell[2]) & 1 == 0 {
                    let count = &mut self.counts[cell_entry(cell, j)];
                    *count = if meets[j] >> e & 1 != 0 { *count + 1 } else { *count - 1 };
                }
            }
        }
        std::mem::replace(&mut self.table[at], entry)
    }

    /** Counts brick `b`, coming into the ball (unknown, in every cell around it) or `by` −1 leaving it, in the rows of what the ball holds. */
    fn count(&mut self, b: [i32; 3], by: i32) {
        for (rows, v) in self.rows.iter_mut().zip(b) {
            let count = &mut rows[v.rem_euclid(WINDOW) as usize];
            *count = count.wrapping_add_signed(by);
        }
    }

    /** Makes brick `b` of the ball UNKNOWN, its slots freed once no entry holds it. */
    fn release(&mut self, b: [i32; 3], pool: &mut Pool) {
        if self.set(b, (UNKNOWN, [AROUND; SCALES])) < 0 {
            return;
        }
        let ([x, y, z], id) = self.site(b);
        let held = self.tiles.get_mut(&id).expect("a slot's tile");
        let (slot, uses) = &mut held.slots[held.tile.grid.brick(x, y, z) as usize];
        *uses -= 1;
        if *uses == 0 {
            self.slots -= pool.give(*slot as u32);
            *slot = EMPTY;
        }
    }

    /**
     * Sets brick `b` of the ball, unless set already, from its tile: EMPTY,
     * or the slot its sites are in, from `pool` if none is yet (to upload,
     * in `added`: as many as `needs` says); UNKNOWN still if no tile is held
     * there, or, `lit`, its light is not found yet.
     */
    fn place(&mut self, b: [i32; 3], lit: bool, pool: &mut Pool, added: &mut Vec<Added>) {
        if self.table[entry(b)] != UNKNOWN {
            return;
        }
        let ([x, y, z], id) = self.site(b);
        let Some(held) = self.tiles.get_mut(&id) else { return };
        let grid = &held.tile.grid;
        let Some(place) = grid.place(x, y, z).filter(|&at| grid.bricks[at] >= 0) else {
            self.set(b, (EMPTY, [0; SCALES]));
            return;
        };
        let first = grid.bricks[place] as usize;
        if lit && held.tile.light(first).is_none() {
            return;
        }
        let (slot, uses) = &mut held.slots[first];
        if *slot < 0 {
            let count = slots(&held.tile.grid, first);
            *slot = pool.take(&held.tile, first, count, added) as i32;
            self.slots += count;
        }
        *uses += 1;
        let entry = (*slot, std::array::from_fn(|j| held.tile.meets[place * SCALES + j]));
        self.set(b, entry);
    }

    /**
     * Where its bricks start, and the brick its ball heads for, given
     * `level`'s `tiles` about `eye`: the eye's, or where it is if there
     * are none; None if it holds nothing and there are none to hold.
     */
    fn aim(&self, level: usize, tiles: &Level, eye: Vec3) -> Option<([i32; 3], [i32; 3])> {
        match (tiles.tiles.values().next(), self.ball) {
            (Some(tile), _) => {
                let origin = tile.grid.min.map(|v| v.rem_euclid(BRICK));
                Some((origin, eye_brick(level, eye, origin)))
            }
            (None, Some(ball)) => Some((self.origin, ball.middle)),
            (None, None) => None,
        }
    }

    /** Whether it is short of holding `tiles` of `level` as a ball about `eye` (retarget's). */
    fn behind(&self, level: usize, tiles: &Arc<Level>, eye: Vec3) -> bool {
        self.aim(level, tiles, eye).is_some_and(|(_, target)| {
            !self.level.as_ref().is_some_and(|l| Arc::ptr_eq(l, tiles)) || self.ball.is_none_or(|b| b.middle != target)
        })
    }

    /**
     * Moves the window a step toward the ball of `radius` about `eye`'s
     * brick of `level` (aim's), holding `tiles`: what the ball leaves, or a tile gone (or made anew), becomes
     * UNKNOWN. Returns the bricks to place: those of tiles it holds that
     * the ball gains, and those of a tile new.
     *
     * A step is a brick along one axis, and one tile whose bricks are in
     * the ball, gone or (once none is) the nearest new: what it places
     * is a shell of the ball and a tile, however fast the eye or many the
     * tiles. Rays take the next level for what is still UNKNOWN; a tile
     * gone is held till its step. A ball that holds nothing around the eye
     * starts anew there, the window as made (clear).
     */
    fn retarget(&mut self, level: usize, tiles: &Arc<Level>, eye: Vec3, radius: i32, pool: &mut Pool) -> Vec<[i32; 3]> {
        // Until it has tiles, it holds nothing and rays do not read it.
        let Some((origin, target)) = self.aim(level, tiles, eye) else { return Vec::new() };
        assert!(self.ball.is_none() || self.origin == origin, "a level's bricks moved");
        self.origin = origin;
        if self.ball.is_some_and(|old| old.about(target) < 0) {
            self.clear(pool);
        }
        let middle = self.ball.map_or(target, |old| {
            let d = (0..3).max_by_key(|&d| (target[d] - old.middle[d]).abs()).expect("an axis");
            std::array::from_fn(|k| old.middle[k] + if k == d { (target[d] - old.middle[d]).signum() } else { 0 })
        });
        let (old, ball) = (self.ball, Ball { middle, radius });
        if let Some(old) = old {
            for b in in_ball(old.bounds(), old, Some(ball)) {
                self.release(b, pool);
                self.count(b, -1);
            }
        }
        for b in in_ball(ball.bounds(), ball, old) {
            self.count(b, 1);
        }
        self.ball = Some(ball);
        let mut pending: Vec<[i32; 3]> = self.tiles.keys().flat_map(|&id| in_ball(tile_bricks(id, origin), ball, old)).collect();
        let meets = |id: TileId| in_ball(tile_bricks(id, origin), ball, None).next().is_some();
        let (mut step, mut left) = (true, 0);
        let gone: Vec<TileId> = (self.tiles.iter())
            .filter(|&(id, h)| tiles.tiles.get(id).is_none_or(|t| !Arc::ptr_eq(t, &h.tile)))
            .map(|(id, _)| *id)
            .collect();
        for id in gone {
            if !meets(id) {
                self.tiles.remove(&id);
            } else if std::mem::take(&mut step) {
                for b in in_ball(tile_bricks(id, origin), ball, None) {
                    self.release(b, pool);
                }
                self.tiles.remove(&id);
            } else {
                left += 1;
            }
        }
        let mut new: Vec<_> = tiles.tiles.iter().filter(|&(id, _)| !self.tiles.contains_key(id)).collect();
        new.sort_by_key(|(id, _)| {
            let (lo, hi) = tile_bricks(**id, origin);
            (0..3).map(|d| (lo[d] + hi[d] - 2 * target[d]).pow(2)).sum::<i32>()
        });
        for (&id, tile) in new {
            assert!((0..3).all(|d| tile.grid.min[d].rem_euclid(BRICK) == origin[d]), "a tile off its level's bricks");
            if meets(id) && !std::mem::take(&mut step) {
                left += 1;
                continue;
            }
            self.tiles.insert(id, Held { tile: tile.clone(), slots: vec![(EMPTY, 0); tile.grid.cells.len() / BRICK_SITES] });
            pending.extend(in_ball(tile_bricks(id, origin), ball, None));
        }
        // Tiles still to change: it is behind, and steps again.
        self.level = (left == 0).then(|| tiles.clone());
        pending
    }

    /** Holds nothing, as made: its slots given back to `pool`, its tiles let go, all unknown at once. */
    fn clear(&mut self, pool: &mut Pool) {
        for held in self.tiles.values() {
            for &(slot, _) in held.slots.iter().filter(|(slot, _)| *slot >= 0) {
                self.slots -= pool.give(slot as u32);
            }
        }
        assert_eq!(self.slots, 0, "slots held of no tile");
        *self = Window::new();
    }

    /** Its ball of `radius` instead, what it leaves UNKNOWN, what it gains unknown (to place: shell's); and of `pending`, those still in it. */
    fn resize(&mut self, radius: i32, pending: &mut Vec<[i32; 3]>, pool: &mut Pool) {
        let Some(old) = self.ball else { return };
        let ball = Ball { radius, ..old };
        for b in in_ball(old.bounds(), old, Some(ball)) {
            self.release(b, pool);
            self.count(b, -1);
        }
        for b in in_ball(ball.bounds(), ball, Some(old)) {
            self.count(b, 1);
        }
        self.ball = Some(ball);
        pending.retain(|&b| ball.contains(b));
    }

    /** The bricks a ball of `radius` would hold that its own does not. */
    fn shell(&self, radius: i32) -> Vec<[i32; 3]> {
        let Some(ball) = self.ball else { return Vec::new() };
        let grown = Ball { radius, ..ball };
        in_ball(grown.bounds(), grown, Some(ball)).collect()
    }

    /**
     * The brick whose entry in its table is `e`, if rays may ask for it:
     * unknown in its ball, of a tile it holds, with voxels (so waiting for
     * light, or lit and to place).
     */
    fn asked(&self, e: usize) -> Option<[i32; 3]> {
        let ball = self.ball?;
        let w = [e % WINDOW as usize, e / WINDOW as usize % WINDOW as usize, e / (WINDOW * WINDOW) as usize].map(|v| v as i32);
        // The window's corner (gpu's): its bricks, from there.
        let low = ball.middle.map(|v| v - WINDOW / 2);
        let b = std::array::from_fn(|d| low[d] + (w[d] - low[d]).rem_euclid(WINDOW));
        let ([x, y, z], id) = self.site(b);
        let voxels = self.tiles.get(&id).is_some_and(|h| h.tile.grid.brick(x, y, z) >= 0);
        (ball.contains(b) && self.table[entry(b)] == UNKNOWN && voxels).then_some(b)
    }

    /** Of the bricks rays last asked for, those lit since and not placed yet. */
    fn ripe(&self) -> Vec<[i32; 3]> {
        let lit = |b: [i32; 3]| {
            let ([x, y, z], id) = self.site(b);
            let held = self.tiles.get(&id);
            held.is_some_and(|h| usize::try_from(h.tile.grid.brick(x, y, z)).is_ok_and(|first| h.tile.light(first).is_some()))
        };
        let ball = self.ball;
        self.asked
            .iter()
            .copied()
            .filter(|&b| ball.is_some_and(|ball| ball.contains(b)) && self.table[entry(b)] == UNKNOWN && lit(b))
            .collect()
    }

    /** The slots placing `bricks` would take from the pool: none for those that wait for light, `lit`. */
    fn needs(&self, bricks: &[[i32; 3]], lit: bool) -> usize {
        let mut new = HashMap::new();
        for &b in bricks {
            let ([x, y, z], id) = self.site(b);
            let Some(held) = self.tiles.get(&id) else { continue };
            if let Ok(first) = usize::try_from(held.tile.grid.brick(x, y, z))
                && held.slots[first].0 < 0
                && (!lit || held.tile.light(first).is_some())
            {
                new.entry((id, first)).or_insert_with(|| slots(&held.tile.grid, first));
            }
        }
        new.values().sum()
    }

    /** As voxels.wgsl reads it, the level drawn as far as `far` (world units) or its ball holds around `eye`, unless it holds nothing yet: rays read none past 0. */
    fn gpu(&self, level: usize, eye: Vec3, far: f64) -> GpuLevel {
        let corner = self.ball.map_or([0; 3], |b| b.middle.map(|v| v - WINDOW / 2));
        let (low, high) = self.bounds();
        GpuLevel {
            min: std::array::from_fn(|d| self.origin[d] + corner[d] * BRICK),
            offset: (level * TABLE) as u32,
            corner,
            reach: self.ball.map_or(0.0, |b| reach(level, b.about(eye_brick(level, eye, self.origin)), far) as f32),
            low,
            filled: (level * COUNTS) as u32,
            high,
            _pad: 0,
        }
    }

    /**
     * The box, in the level's sites, that the voxels of its ball's bricks
     * and its unknown bricks are in: past it, rays meet nothing of the
     * level. Far off if none.
     */
    fn bounds(&self) -> ([f32; 3], [f32; 3]) {
        let Some(ball) = self.ball else { return ([1e30; 3], [1e30; 3]) };
        let (lo, hi) = ball.bounds();
        let mut out = ([1e30; 3], [1e30; 3]);
        for d in 0..3 {
            let mut held = (lo[d]..hi[d]).filter(|v| self.rows[d][v.rem_euclid(WINDOW) as usize] != 0);
            if let (Some(first), last) = (held.next(), held.next_back()) {
                // A brick's voxels: its sites, and 1 farther each way.
                let site = |b: i32| (self.origin[d] + b * BRICK) as f32;
                (out.0[d], out.1[d]) = (site(first) - 1.0, site(last.unwrap_or(first)) + BRICK as f32);
            }
        }
        out
    }
}

/**
 * The levels' windows and the pool of slots they share. The windows are
 * balls of one radius, each in its own level's bricks, as large as the
 * pool holds, up to RADIUS: short of slots, all shrink a brick, each level
 * read as much less far, its rays taking the next coarser one sooner. With
 * room to spare (an eighth of the pool), they grow a brick a sync, if
 * the bricks that gains fit. A sync steps one window, each in turn: what
 * it costs is bounded, however fast the eye or many the tiles; a window
 * behind, rays read it as far as it holds around the eye (Ball::about).
 */
struct Windows {
    windows: Vec<Window>,
    pool: Pool,
    radius: i32,
    /** Slots free that growing takes, as last found (0: to find). */
    growing: usize,
    margin: usize,
    /** The tiles of a level not read. */
    none: Arc<Level>,
    /** The window stepped last. */
    last: usize,
    /** Whether bricks with voxels wait for their light (Window::place). */
    lit: bool,
}

impl Windows {
    fn new(pool: usize, lit: bool) -> Windows {
        // Balls of radius 0, of 27 bricks each, always fit.
        assert!(pool >= GRID_LEVELS * 27, "a pool of {pool} slots");
        Windows {
            windows: (0..GRID_LEVELS).map(|_| Window::new()).collect(),
            pool: Pool::new(pool),
            radius: RADIUS,
            growing: 0,
            margin: pool / 8,
            none: Arc::new(Level::new(HashMap::new())),
            last: 0,
            lit,
        }
    }

    /**
     * The window to step (retarget) given `levels` about `eye`: the first
     * behind after the one stepped last, so that each has its turn.
     */
    fn next(&self, levels: &[Arc<Level>], eye: Vec3) -> Option<usize> {
        let tiles = |level: usize| levels.get(level).unwrap_or(&self.none);
        (1..=GRID_LEVELS).map(|k| (self.last + k) % GRID_LEVELS).find(|&l| self.windows[l].behind(l, tiles(l), eye))
    }

    /** The slots placing each window's `pending` bricks would take. */
    fn needs(&self, pending: &[Vec<[i32; 3]>]) -> usize {
        self.windows.iter().zip(pending).map(|(w, bricks)| w.needs(bricks, self.lit)).sum()
    }

    /** Places each window's `pending` bricks, which fit. */
    fn place(&mut self, pending: Vec<Vec<[i32; 3]>>, added: &mut Vec<Added>) {
        for (window, bricks) in self.windows.iter_mut().zip(pending) {
            for b in bricks {
                window.place(b, self.lit, &mut self.pool, added);
            }
        }
    }

    /**
     * Steps a level's window toward its tiles in `levels` (none past them)
     * around `eye` (next's, retarget's), and has every window place what
     * rays asked for that is lit now (ripe's); then decides
     * the radius and places what that holds. Returns the bricks to upload
     * (every slot freed before any is taken: each once) and which windows
     * changed.
     */
    fn sync(&mut self, levels: &[Arc<Level>], eye: Vec3) -> (Vec<Added>, [bool; GRID_LEVELS]) {
        let (mut pending, mut changed) = (vec![Vec::new(); GRID_LEVELS], [false; GRID_LEVELS]);
        if let Some(level) = self.next(levels, eye) {
            let tiles = levels.get(level).unwrap_or(&self.none);
            pending[level] = self.windows[level].retarget(level, tiles, eye, self.radius, &mut self.pool);
            (changed[level], self.last) = (true, level);
        }
        for (level, window) in self.windows.iter().enumerate() {
            let ripe = window.ripe();
            changed[level] |= !ripe.is_empty();
            pending[level].extend(ripe);
        }
        // Short of slots: every window a brick less, till what is to place fits.
        while self.needs(&pending) > self.pool.free.len() {
            self.radius -= 1;
            for (window, bricks) in self.windows.iter_mut().zip(&mut pending) {
                window.resize(self.radius, bricks, &mut self.pool);
            }
            (self.growing, changed) = (0, [true; GRID_LEVELS]);
        }
        let mut added = Vec::new();
        self.place(pending, &mut added);
        // Room to spare: every window a brick more, if what that gains fits with room still.
        if self.radius < RADIUS && self.pool.free.len() >= self.growing.max(self.margin) {
            let radius = self.radius + 1;
            let shells: Vec<_> = self.windows.iter().map(|w| w.shell(radius)).collect();
            let needs = self.needs(&shells);
            if needs + self.margin <= self.pool.free.len() {
                for window in &mut self.windows {
                    window.resize(radius, &mut Vec::new(), &mut self.pool);
                }
                self.place(shells, &mut added);
                (self.radius, self.growing, changed) = (radius, 0, [true; GRID_LEVELS]);
            } else {
                self.growing = needs + self.margin;
            }
        }
        (added, changed)
    }
}

const PICTURE: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
const SAMPLE_HITS: wgpu::TextureFormat = wgpu::TextureFormat::Rgba32Uint;
const SAMPLE_COLOURS: wgpu::TextureFormat = wgpu::TextureFormat::Rg32Uint;

/** Textures as big as the picture: the samples', the picture, and the groups binding them. */
struct Targets {
    pixels: [u32; 2],
    samples: [wgpu::Texture; 2],
    picture: wgpu::Texture,
    groups: [wgpu::BindGroup; 3],
}

/** Words of the bits rays ask for bricks by, one per entry of the windows' tables (voxels.wgsl's `asked`). */
const ASKED_WORDS: usize = GRID_LEVELS * TABLE / 32;

/** Where reading back what rays asked for is: not, copied from a frame (the windows' changes then), or mapping what was copied. */
enum Reading {
    Idle,
    Copied(u64),
    Mapping(Arc<AtomicBool>, u64),
}

/** The voxels on a device, drawn into a picture that the window or a shot shows. */
pub struct Marcher {
    /**
     * Globals, levels, tables, the pool, the eye, cells'
     * counts, the pool's bricks' light (none unlit), its slots' next
     * (Pool's), and, lit, the bricks rays asked for (ASKED_WORDS).
     */
    buffers: [wgpu::Buffer; 9],
    /** What the asked were copied to, to read. */
    readback: wgpu::Buffer,
    reading: Reading,
    layouts: [wgpu::BindGroupLayout; 3],
    pipelines: [wgpu::RenderPipeline; 3],
    windows: Windows,
    targets: Option<Targets>,
    /** Whether it draws lit scenes (Look's `sky`). */
    pub lit: bool,
    /** Whether the windows held all they were given at the last draw (Windows::sync's syncs being bounded); lit, and what rays asked for of them as they are was read back. */
    settled: bool,
    /** How many draws changed the windows; lit, as of the frame rays last asked in, read back, and whether they asked for any brick. */
    changes: u64,
    read: (u64, bool),
}

/** voxels.wgsl, with the constants it shares with this file. */
fn shader() -> String {
    format!(
        "const WINDOW = {WINDOW};\nconst CELLS = {CELLS}u;\nconst BRICK = {BRICK};\nconst BRICK_SHIFT = {BRICK_SHIFT}u;\nconst BRICK_SITES = {BRICK_SITES}u;\nconst FACTOR = {FACTOR}u;\nconst EMPTY = {EMPTY};\nconst UNKNOWN = {UNKNOWN};\nconst FACES = array<vec3f, {}>({});\n{}",
        fcc::FACE_KINDS,
        fcc::VOXEL_FACES.map(|[x, y, z]| format!("vec3f({x}.0, {y}.0, {z}.0)")).join(", "),
        include_str!("voxels.wgsl")
    )
}

impl Marcher {
    /** Draws onto targets of `format` when shown; voxels coloured by their light if `lit` (Look's `sky`). */
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat, lit: bool) -> Marcher {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("voxels.wgsl"),
            source: wgpu::ShaderSource::Wgsl(shader().into()),
        });
        // Lit, a brick's light takes a word a site, beside its own words.
        let light_words = if lit { BRICK_SITES } else { 0 };
        let largest = device.limits().max_storage_buffer_binding_size / (BRICK_SITES as u64 * 4);
        let pool = (POOL_BYTES / ((BRICK_SITES + light_words) as u64 * 4)).min(largest);
        let buffer = |label, size: usize, usage| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: size as u64,
                usage: usage | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        };
        let (uniform, storage) = (wgpu::BufferUsages::UNIFORM, wgpu::BufferUsages::STORAGE);
        let buffers = [
            buffer("globals", size_of::<Globals>(), uniform),
            buffer("levels", GRID_LEVELS * size_of::<GpuLevel>(), storage),
            buffer("tables", GRID_LEVELS * TABLE * 4, storage),
            buffer("pool", pool as usize * BRICK_SITES * 4, storage),
            buffer("eye", 16, uniform),
            buffer("filled", GRID_LEVELS * COUNTS * 4, storage),
            buffer("lights", (pool as usize * light_words * 4).max(4), storage),
            buffer("next", pool as usize * 4, storage),
            buffer("asked", if lit { ASKED_WORDS * 4 } else { 4 }, storage | wgpu::BufferUsages::COPY_SRC),
        ];
        let readback = buffer("asked read", if lit { ASKED_WORDS * 4 } else { 4 }, wgpu::BufferUsages::MAP_READ);
        let entry = |binding: u32, ty| wgpu::BindGroupLayoutEntry { binding, visibility: wgpu::ShaderStages::FRAGMENT, ty, count: None };
        let buffer_type = |ty| wgpu::BindingType::Buffer { ty, has_dynamic_offset: false, min_binding_size: None };
        let read = buffer_type(wgpu::BufferBindingType::Storage { read_only: true });
        let write = buffer_type(wgpu::BufferBindingType::Storage { read_only: false });
        let uniform_type = buffer_type(wgpu::BufferBindingType::Uniform);
        let texture =
            |sample_type| wgpu::BindingType::Texture { sample_type, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false };
        let uint = texture(wgpu::TextureSampleType::Uint);
        let all = [
            entry(0, uniform_type),
            entry(1, read),
            entry(2, read),
            entry(3, read),
            entry(4, uniform_type),
            entry(5, read),
            entry(6, read),
            entry(7, read),
            entry(8, uint),
            entry(9, uint),
            entry(11, write),
        ];
        let samples = [&all[..8], &all[10..]].concat();
        let shown = [entry(10, texture(wgpu::TextureSampleType::Float { filterable: false }))];
        // The samples' pass draws into the textures the resolving one reads, which draws the picture shown; the
        // samples' rays ask for bricks.
        let layouts = [&samples[..], &all[..10], &shown[..]]
            .map(|entries| device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: None, entries }));
        let pipeline = |entry: &str, layout: &wgpu::BindGroupLayout, formats: &[wgpu::TextureFormat]| {
            let targets: Vec<_> = formats.iter().map(|&f| Some(f.into())).collect();
            let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: None,
                bind_group_layouts: &[Some(layout)],
                immediate_size: 0,
            });
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(entry),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("screen_vertex"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(entry),
                    compilation_options: Default::default(),
                    targets: &targets,
                }),
                primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleStrip, ..Default::default() },
                depth_stencil: None,
                multisample: Default::default(),
                multiview_mask: None,
                cache: None,
            })
        };
        let pipelines = [
            pipeline("sample_fragment", &layouts[0], &[SAMPLE_HITS, SAMPLE_COLOURS]),
            pipeline("resolve_fragment", &layouts[1], &[PICTURE]),
            pipeline("show_fragment", &layouts[2], &[format]),
        ];
        let (windows, reading) = (Windows::new(pool as usize, lit), Reading::Idle);
        let (targets, read) = (None, (u64::MAX, false));
        Marcher { buffers, readback, reading, layouts, pipelines, windows, targets, lit, settled: true, changes: 0, read }
    }

    /**
     * Brings the voxels on the GPU up to date with `scene`, then draws it
     * into a picture of `pixels`: the samples, then each pixel resolving
     * its own. Lit, it reads back what rays asked for as far as it can
     * without waiting (`ask`), and copies what they ask for now to read.
     */
    fn draw(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, encoder: &mut wgpu::CommandEncoder, scene: &Scene, pixels: [u32; 2]) {
        let look = &scene.look;
        if self.lit {
            self.ask(device, &scene.asked);
        }
        let mut levels = [GpuLevel::default(); GRID_LEVELS];
        let (added, changed) = self.windows.sync(&scene.levels, look.camera);
        self.changes += u64::from(changed.contains(&true));
        // Lit, settled once a frame of the windows as they are has asked for nothing.
        self.settled = self.windows.next(&scene.levels, look.camera).is_none() && (!self.lit || self.read.0 == self.changes);
        upload(device, encoder, &self.buffers[3], &added, BRICK_SITES, |a| words(&a.tile.grid, a.first, a.part));
        let next = &self.windows.pool.next;
        upload(device, encoder, &self.buffers[7], &added, 1, |a| [next[a.slot as usize] as u32]);
        if self.lit {
            // A brick's own slot only: those of its faces have no light.
            upload(device, encoder, &self.buffers[6], added.iter().filter(|a| a.part == 0), BRICK_SITES, |a| {
                a.tile.light(a.first).expect("a brick placed lit").iter().copied()
            });
        }
        for (level, gpu) in levels.iter_mut().enumerate() {
            let window = &self.windows.windows[level];
            if changed[level] {
                queue.write_buffer(&self.buffers[2], (level * TABLE * 4) as u64, bytemuck::cast_slice(&window.table));
                queue.write_buffer(&self.buffers[5], (level * COUNTS * 4) as u64, bytemuck::cast_slice(&window.counts));
            }
            if level < scene.levels.len() {
                *gpu = window.gpu(level, look.camera, look.far);
            }
        }
        queue.write_buffer(&self.buffers[1], 0, bytemuck::cast_slice(&levels));
        queue.write_buffer(&self.buffers[0], 0, bytemuck::bytes_of(&Globals::of(look, pixels)));
        let eye = look.camera.map(|v| v as f32);
        queue.write_buffer(&self.buffers[4], 0, bytemuck::cast_slice(&[eye[0], eye[1], eye[2], 0.0]));
        if self.targets.as_ref().is_none_or(|t| t.pixels != pixels) {
            self.targets = Some(self.targets(device, pixels));
        }
        let targets = self.targets.as_ref().expect("targets");
        let passes = [(&targets.samples[..], &targets.groups[0]), (std::slice::from_ref(&targets.picture), &targets.groups[1])];
        for ((textures, group), pipeline) in passes.into_iter().zip(&self.pipelines) {
            let views: Vec<_> = textures.iter().map(|t| t.create_view(&Default::default())).collect();
            let attachments: Vec<_> = views
                .iter()
                .map(|view| {
                    Some(wgpu::RenderPassColorAttachment {
                        view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store },
                    })
                })
                .collect();
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &attachments,
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, group, &[]);
            pass.draw(0..4, 0..1);
        }
        if self.lit && matches!(self.reading, Reading::Idle) {
            encoder.copy_buffer_to_buffer(&self.buffers[8], 0, &self.readback, 0, self.readback.size());
            encoder.clear_buffer(&self.buffers[8], 0, None);
            self.reading = Reading::Copied(self.changes);
        }
        let busy = !self.settled || self.read.1;
        scene.asked.lock().expect("asked").busy = busy;
    }

    /**
     * Reads back what rays asked for: maps what the last frame copied, or,
     * once mapped, tells `asked` the bricks the windows wait to light that
     * rays asked for (Window::asked), which the windows place once lit.
     */
    fn ask(&mut self, device: &wgpu::Device, asked: &Mutex<Asked>) {
        // What it waits for, done: the map's callback.
        _ = device.poll(wgpu::PollType::Poll);
        match std::mem::replace(&mut self.reading, Reading::Idle) {
            Reading::Copied(changes) => {
                let done = Arc::new(AtomicBool::new(false));
                let mapped = done.clone();
                self.readback.slice(..).map_async(wgpu::MapMode::Read, move |r| mapped.store(r.is_ok(), Ordering::Release));
                self.reading = Reading::Mapping(done, changes);
            }
            Reading::Mapping(done, changes) if done.load(Ordering::Acquire) => {
                let mut bricks = Vec::new();
                for window in &mut self.windows.windows {
                    window.asked.clear();
                }
                {
                    let bytes = self.readback.slice(..).get_mapped_range().expect("mapped");
                    let words: &[u32] = bytemuck::cast_slice(&bytes);
                    for (k, &word) in words.iter().enumerate().filter(|(_, w)| **w != 0) {
                        for bit in (0..32).filter(|bit| word >> bit & 1 != 0) {
                            let (level, e) = ((k * 32 + bit) / TABLE, (k * 32 + bit) % TABLE);
                            let window = &mut self.windows.windows[level];
                            if let Some(b) = window.asked(e) {
                                window.asked.push(b);
                                bricks.push((level, window.site(b).0));
                            }
                        }
                    }
                }
                self.readback.unmap();
                self.read = (changes, !bricks.is_empty());
                asked.lock().expect("asked").bricks = Some(bricks);
            }
            waiting => self.reading = waiting,
        }
    }

    fn targets(&self, device: &wgpu::Device, pixels: [u32; 2]) -> Targets {
        let target = |label, [width, height]: [u32; 2], format, usage| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d { width: width.max(1), height: height.max(1), depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING | usage,
                view_formats: &[],
            })
        };
        let picture = target("picture", pixels, PICTURE, wgpu::TextureUsages::COPY_SRC);
        let samples =
            [SAMPLE_HITS, SAMPLE_COLOURS].map(|f| target("samples", pixels.map(|v| v.div_ceil(FACTOR)), f, wgpu::TextureUsages::empty()));
        let views = samples.each_ref().map(|t| t.create_view(&Default::default()));
        let picture_view = picture.create_view(&Default::default());
        // Bound as numbered, the asked at 11.
        let mut entries: Vec<_> = self.buffers[..8]
            .iter()
            .enumerate()
            .map(|(k, b)| wgpu::BindGroupEntry { binding: k as u32, resource: b.as_entire_binding() })
            .collect();
        for (k, view) in views.iter().enumerate() {
            entries.push(wgpu::BindGroupEntry { binding: 8 + k as u32, resource: wgpu::BindingResource::TextureView(view) });
        }
        entries.push(wgpu::BindGroupEntry { binding: 11, resource: self.buffers[8].as_entire_binding() });
        let shown = [wgpu::BindGroupEntry { binding: 10, resource: wgpu::BindingResource::TextureView(&picture_view) }];
        let group = |layout, entries: &[wgpu::BindGroupEntry]| {
            device.create_bind_group(&wgpu::BindGroupDescriptor { label: None, layout, entries })
        };
        let sampling = [&entries[..8], &entries[10..]].concat();
        let groups = [group(&self.layouts[0], &sampling), group(&self.layouts[1], &entries[..10]), group(&self.layouts[2], &shown)];
        Targets { pixels, samples, picture, groups }
    }
}

/**
 * Copies `added` slots into `pool`, `per` words a slot as `words` gives
 * them: one buffer, copied from per run of consecutive slots
 * (runs are many, and a write_buffer costs as much as a large copy).
 */
fn upload<'a, W: IntoIterator<Item = u32>>(
    device: &wgpu::Device,
    encoder: &mut wgpu::CommandEncoder,
    pool: &wgpu::Buffer,
    added: impl IntoIterator<Item = &'a Added>,
    per: usize,
    words: impl Fn(&'a Added) -> W,
) {
    use wgpu::util::DeviceExt;
    let mut added: Vec<_> = added.into_iter().collect();
    if added.is_empty() {
        return;
    }
    added.sort_unstable_by_key(|a| a.slot);
    let all: Vec<u32> = added.iter().flat_map(|&a| words(a)).collect();
    assert_eq!(all.len(), added.len() * per, "{per} words a slot");
    let usage = wgpu::BufferUsages::COPY_SRC;
    let from =
        device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("upload"), contents: bytemuck::cast_slice(&all), usage });
    let bytes = (per * 4) as u64;
    let mut start = 0;
    for k in 1..=added.len() {
        if k == added.len() || added[k].slot != added[k - 1].slot + 1 {
            let size = (k - start) as u64 * bytes;
            encoder.copy_buffer_to_buffer(&from, start as u64 * bytes, pool, u64::from(added[start].slot) * bytes, size);
            start = k;
        }
    }
}

impl Painter for Marcher {
    fn picture(&self) -> &wgpu::Texture {
        &self.targets.as_ref().expect("a picture").picture
    }

    fn show(&self, pass: &mut wgpu::RenderPass<'_>) {
        if let Some(targets) = &self.targets {
            pass.set_pipeline(&self.pipelines[2]);
            pass.set_bind_group(0, &targets.groups[2], &[]);
            pass.draw(0..4, 0..1);
        }
    }

    fn settled(&self) -> bool {
        self.settled
    }

    /** Lit, whether rays asked for bricks to light, last it read back. */
    fn asked(&self) -> bool {
        self.read.1
    }

    /** Slots held, of the pool's, and the windows' radius. */
    fn describe(&self, _: &Easel) -> String {
        let Windows { windows, pool, radius, .. } = &self.windows;
        let held: usize = windows.iter().map(|w| w.slots).sum();
        format!(": {held} slots of {} on the GPU, windows {radius} bricks of {RADIUS} round", held + pool.free.len())
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, HashSet};

    use super::*;

    /**
     * The tiles of level 0 of the forest of seed 1 around `eye`; in every
     * 64th brick of each, its first voxel made to look unlike through each
     * face, so that windows hold their views too.
     */
    fn forest_tiles(eye: Vec3) -> Arc<Level> {
        let mut world = World::new(worlds::by_name("forest").expect("forest").make, 1, None, 4);
        let reach = std::array::from_fn(|l| (l == 0).then_some(fcc::GRID_REACH));
        let wants = Wants { reach, lit: Lit::Not, tiles: Vec::new(), bricks: Arc::default() };
        loop {
            world.update(eye, &wants);
            if world.making() == 0 {
                break;
            }
            world.wait();
        }
        let faced = |id: TileId, tile: &Tile| {
            let g = &tile.grid;
            let firsts = (0..g.cells.len() / BRICK_SITES).step_by(64);
            let sites = firsts.filter_map(|first| sites(first).find(|&i| g.cells[i] != 0));
            let views =
                sites.flat_map(|i| std::iter::once(i as u32).chain((0..fcc::FACE_KINDS as u32).map(move |k| g.cells[i] + k))).collect();
            Tile::new(id, Arc::new(fcc::Grid { views, ..(**g).clone() }))
        };
        Arc::new(Level::new(world.level(0).tiles.iter().map(|(&id, tile)| (id, Arc::new(faced(id, tile)))).collect()))
    }

    /** Syncs `windows` to level 0's `tiles` around `eye`, keeping what they upload in `slots`; whether any window changed. */
    fn sync(windows: &mut Windows, tiles: &Arc<Level>, eye: Vec3, slots: &mut HashMap<u32, Vec<u32>>) -> bool {
        let (added, changed) = windows.sync(std::slice::from_ref(tiles), eye);
        for a in added {
            slots.insert(a.slot, words(&a.tile.grid, a.first, a.part).collect());
        }
        changed.contains(&true)
    }

    /**
     * Syncs `windows` as `sync` does till nothing changes; whether any
     * did. Each sync moves level 0's ball a brick along an axis at most
     * (or anew about the eye, the old one holding nothing around it), and
     * places one new tile's bricks in it at most.
     */
    fn settle(windows: &mut Windows, tiles: &Arc<Level>, eye: Vec3, slots: &mut HashMap<u32, Vec<u32>>) -> bool {
        for k in 0..1000 {
            let w = &windows.windows[0];
            let (was, had): (_, HashSet<_>) = (w.ball, w.tiles.keys().copied().collect());
            if !sync(windows, tiles, eye, slots) {
                return k > 0;
            }
            let w = &windows.windows[0];
            let ball = w.ball.expect("a ball");
            if let Some(was) = was {
                let stepped = (0..3).map(|d| (ball.middle[d] - was.middle[d]).abs()).sum::<i32>() <= 1;
                assert!(stepped || was.about(ball.middle) < 0, "{was:?} to {ball:?}, at {eye:?}");
            }
            let new = w.tiles.keys().filter(|&id| !had.contains(id) && in_ball(tile_bricks(*id, w.origin), ball, None).next().is_some());
            assert!(new.count() <= 1, "new tiles in the ball, at {eye:?}");
        }
        panic!("unsettled at {eye:?}");
    }

    /**
     * What level 0's window holds, per entry of its ball: EMPTY, UNKNOWN or
     * its slots' words, as `slots` say they were uploaded, and the cells it
     * counts in; the slots it takes; and its counts, as its entries say.
     */
    type Holds = (Vec<Option<(Result<Vec<u32>, i32>, [u32; SCALES])>>, usize, Vec<u32>);

    fn holds(windows: &Windows, slots: &HashMap<u32, Vec<u32>>) -> Holds {
        let w = &windows.windows[0];
        let (mut counts, mut rows) = (vec![0u32; COUNTS], [[0; WINDOW as usize]; 3]);
        for b in window_bricks(w.ball.map_or([WINDOW / 2; 3], |ball| ball.middle)) {
            if w.ball.is_some_and(|ball| ball.contains(b)) && w.meets[entry(b)] != [0; SCALES] {
                (0..3).for_each(|d| rows[d][b[d].rem_euclid(WINDOW) as usize] += 1);
            }
            for j in 0..SCALES {
                for e in (0..27).filter(|e| w.meets[entry(b)][j] >> e & 1 != 0) {
                    let cell = around(b, j, e);
                    if (cell[0] + cell[1] + cell[2]) & 1 == 0 {
                        counts[cell_entry(cell, j)] += 1;
                    }
                }
            }
        }
        assert!(w.counts == counts && w.rows == rows, "counts and rows as the entries say");
        // A slot's words, and those of the slots after it (its faces').
        let words = |mut slot: i32| {
            let mut words = Vec::new();
            while slot >= 0 {
                words.extend(&slots[&(slot as u32)]);
                slot = windows.pool.next[slot as usize];
            }
            words
        };
        let mut held = vec![None; TABLE];
        for b in w.ball.map(|ball| in_ball(ball.bounds(), ball, None)).into_iter().flatten() {
            let e = w.table[entry(b)];
            held[entry(b)] = Some((if e >= 0 { Ok(words(e)) } else { Err(e) }, w.meets[entry(b)]));
        }
        (held, w.slots, counts)
    }

    /** What windows made anew, of `radius` and room for all, hold of `tiles` around `eye`. */
    fn anew(radius: i32, tiles: &Arc<Level>, eye: Vec3) -> Holds {
        let (mut windows, mut slots) = (Windows::new(1 << 20, false), HashMap::new());
        (windows.radius, windows.growing) = (radius, usize::MAX);
        settle(&mut windows, tiles, eye, &mut slots);
        holds(&windows, &slots)
    }

    /** The bricks of the window about `middle`. */
    fn window_bricks(middle: [i32; 3]) -> impl Iterator<Item = [i32; 3]> {
        let (lo, hi) = (middle.map(|v| v - WINDOW / 2), middle.map(|v| v + WINDOW / 2));
        (lo[2]..hi[2]).flat_map(move |z| (lo[1]..hi[1]).flat_map(move |y| (lo[0]..hi[0]).map(move |x| [x, y, z])))
    }

    #[test]
    fn a_window_moved_and_its_tiles_changed_holds_what_a_new_one_would() {
        let eye = [10.0, 30.0, -20.0];
        let all = forest_tiles(eye);
        let mut ids: Vec<_> = all.tiles.keys().copied().collect();
        ids.sort();
        assert!(ids.len() > 4, "{} tiles", ids.len());
        // Some tiles of all, then others, as the eye moves through bricks and across the torus's seams.
        let some = |skip: usize| Arc::new(Level::new(ids.iter().skip(skip).step_by(2).map(|id| (*id, all.tiles[id].clone())).collect()));
        let steps =
            [(some(0), eye), (all.clone(), [13.0, 31.0, -9.0]), (some(1), [-140.0, 30.0, 70.0]), (all.clone(), [-130.0, 22.0, 75.0])];
        let (mut windows, mut slots) = (Windows::new(1 << 20, false), HashMap::new());
        // No tiles yet: nothing held, nothing read.
        assert!(!sync(&mut windows, &Arc::new(Level::new(HashMap::new())), eye, &mut slots));
        assert_eq!(windows.windows[0].gpu(0, eye, f64::MAX).reach, 0.0);
        for (tiles, eye) in &steps {
            assert!(settle(&mut windows, tiles, *eye, &mut slots), "a change");
            assert_eq!(windows.radius, RADIUS, "room for all");
            assert!(holds(&windows, &slots) == anew(RADIUS, tiles, *eye), "at {eye:?}");
            assert!(!sync(&mut windows, tiles, *eye, &mut slots), "nothing moved");
        }
    }

    #[test]
    fn lit_a_window_places_the_bricks_rays_asked_for_once_lit() {
        let eye = [10.0, 30.0, -20.0];
        let tiles = forest_tiles(eye);
        let mut windows = Windows::new(1 << 20, true);
        let sync = |windows: &mut Windows| windows.sync(std::slice::from_ref(&tiles), eye).1.contains(&true);
        while sync(&mut windows) {}
        // No light found: the bricks with voxels unknown, rays may ask for them, the empty ones empty.
        let w = &windows.windows[0];
        let ball = w.ball.expect("a ball");
        let held: Vec<[i32; 3]> =
            window_bricks(ball.middle).filter(|&b| ball.contains(b) && tiles.tiles.contains_key(&w.site(b).1)).collect();
        let unknown: Vec<[i32; 3]> = held.iter().copied().filter(|&b| w.table[entry(b)] == UNKNOWN).collect();
        assert!(held.iter().all(|&b| matches!(w.table[entry(b)], UNKNOWN | EMPTY)));
        assert!(unknown.len() > 1000 && unknown.len() < held.len(), "{} of {} bricks unknown", unknown.len(), held.len());
        assert!(unknown.iter().all(|&b| w.asked(entry(b)) == Some(b)), "every unknown brick may be asked for");
        // Rays ask for half, and some of all are lit (with them the buried bricks of a tile, which share their sites):
        // those placed that were asked for and are lit, no other.
        let world = (worlds::by_name("forest").expect("forest").make)(1);
        let mut light = Light::new(world, worldgen::light::Shared::new(worldgen::light::Brightness { sky: 1.0, sun: 1.0, glow: 0.0 }));
        for &b in unknown.iter().step_by(3) {
            let (corner, id) = w.site(b);
            tiles.tiles[&id].lit(0, corner, &mut light);
        }
        let lit = |b: [i32; 3]| {
            let ([x, y, z], id) = w.site(b);
            let tile = &tiles.tiles[&id];
            tile.light(tile.grid.brick(x, y, z) as usize).is_some()
        };
        let lit: Vec<bool> = unknown.iter().map(|&b| lit(b)).collect();
        windows.windows[0].asked = unknown.iter().copied().step_by(2).collect();
        assert!(sync(&mut windows), "the lit placed");
        let w = &windows.windows[0];
        for (k, &b) in unknown.iter().enumerate() {
            let placed = w.table[entry(b)] >= 0;
            assert_eq!(placed, k % 2 == 0 && lit[k], "brick {b:?}");
            assert_eq!(w.asked(entry(b)).is_some(), !placed, "brick {b:?}");
        }
        assert!(!sync(&mut windows), "nothing more to place");
    }

    #[test]
    fn windows_short_of_slots_hold_as_large_a_ball_as_fits() {
        let eye = [10.0, 30.0, -20.0];
        let tiles = forest_tiles(eye);
        let (all, wanted, _) = anew(RADIUS, &tiles, eye);
        // Buried bricks of a tile share a slot: fewer slots than bricks held.
        let entries = all.iter().flatten().filter(|(e, _)| e.is_ok()).count();
        assert!(wanted < entries, "{wanted} slots for {entries} bricks");
        let pool = wanted / 3;
        let (mut windows, mut slots) = (Windows::new(pool, false), HashMap::new());
        for eye in [eye, [13.0, 31.0, -9.0], [21.0, 33.0, -3.0], eye] {
            settle(&mut windows, &tiles, eye, &mut slots);
            let w = &windows.windows[0];
            let ball = w.ball.expect("a ball");
            assert!(ball.radius == windows.radius && windows.radius < RADIUS && w.slots <= pool, "at {eye:?}");
            // Every brick of a tile made in the ball held, every other unknown.
            for b in window_bricks(ball.middle) {
                let inside = ball.contains(b) && tiles.tiles.contains_key(&w.site(b).1);
                assert_eq!(w.table[entry(b)] != UNKNOWN, inside, "brick {b:?}, at {eye:?}");
            }
            assert!(holds(&windows, &slots) == anew(windows.radius, &tiles, eye), "at {eye:?}");
        }
        // More room: they grow, a brick at a sync, to all.
        windows.pool.free.extend(pool as u32..2 * wanted as u32);
        windows.pool.next.resize(2 * wanted, EMPTY);
        for _ in 0..RADIUS {
            sync(&mut windows, &tiles, eye, &mut slots);
        }
        assert_eq!(windows.radius, RADIUS);
        assert!(holds(&windows, &slots) == anew(RADIUS, &tiles, eye));
    }

    #[test]
    fn a_window_holds_the_bricks_of_every_tile_made_past_the_heights_too() {
        // Over the forest's heights, where its tiles are made empty (world.rs): each brick of a tile made is held,
        // as empty or with voxels, not unknown, so rays read on through it, not the next level. Those over
        // the heights are empty.
        let tall = World::new(worlds::by_name("forest").expect("forest").make, 1, None, 1).heights[0].1 as f64;
        let eye = [10.0, tall + 100.0, -20.0];
        let tiles = forest_tiles(eye);
        let mut windows = Windows::new(1 << 20, false);
        settle(&mut windows, &tiles, eye, &mut HashMap::new());
        let w = &windows.windows[0];
        let ball = w.ball.expect("a ball");
        let (mut made, mut over) = (0, 0);
        for b in window_bricks(ball.middle).filter(|&b| ball.contains(b)) {
            let (site, id) = w.site(b);
            if !tiles.tiles.contains_key(&id) {
                continue;
            }
            made += 1;
            assert_ne!(w.table[entry(b)], UNKNOWN, "brick {b:?} of a tile made");
            if f64::from(site[1]) >= tall {
                over += 1;
                assert_eq!(w.table[entry(b)], EMPTY, "brick {b:?}, over the heights");
            }
        }
        assert!(made > 10_000 && over > 1000, "{made} bricks of tiles made, {over} over the heights");
    }

    #[test]
    fn the_window_and_the_tiles_wanted_hold_all_a_ray_reads_of_what_it_meets() {
        // The voxel a ray is in, wherever it is up to its level's reach, and that voxel's neighbours, whose faces
        // shade it: eyes along a line across a tile, rays every way, balls of any radius.
        let world = World::new(worlds::by_name("forest").expect("forest").make, 1, None, 1);
        let reach = std::array::from_fn(|l| (l == 0).then_some(READ));
        let ways = (0..125).map(|i| [i % 5 - 2, i / 5 % 5 - 2, i / 25 - 2].map(f64::from)).filter(|w| w != &[0.0; 3]);
        for step in 0..TILE * 4 / 3 {
            let t = step as f64 * 0.75;
            let eye = [t, 0.6 * t + 31.3, 0.3 * t - 17.9];
            let tiles: HashSet<_> = world.lacking(&reach, eye).into_iter().map(|(_, id)| id).collect();
            // Balls about the eye's brick, or behind it, rays reading as far as they hold around it.
            let balls = [0, 1, 13, RADIUS].into_iter().flat_map(|radius| {
                [[0, 0, 0], [1, -1, 1], [3, 0, -2], [-7, 5, 4]]
                    .map(|behind| Ball { middle: std::array::from_fn(|d| eye_brick(0, eye, [0; 3])[d] + behind[d]), radius })
            });
            for ball in balls.filter(|b| b.about(eye_brick(0, eye, [0; 3])) >= 0) {
                let read = reach_of(ball.about(eye_brick(0, eye, [0; 3])));
                for way in ways.clone() {
                    let length = way.iter().map(|v| v * v).sum::<f64>().sqrt();
                    let site = worldgen::picture::nearest_site(std::array::from_fn(|d| eye[d] + way[d] / length * read));
                    for s in std::iter::once([0; 3]).chain(fcc::VOXEL_FACES).map(|n| std::array::from_fn(|d| site[d] as i32 + n[d])) {
                        let b = s.map(|v: i32| v.div_euclid(BRICK));
                        assert!(ball.contains(b), "eye {eye:?}: site {s:?} out of {ball:?}");
                        assert!(tiles.contains(&s.map(|v| v.div_euclid(TILE as i32))), "eye {eye:?}: site {s:?} in no tile wanted");
                    }
                }
            }
        }
    }

    #[test]
    fn no_voxel_the_window_holds_is_out_of_its_box_or_meets_a_cell_counted_hollow() {
        let eye = [10.0, 30.0, -20.0];
        let tiles = forest_tiles(eye);
        // Moved, so that bricks of both sides of the torus have been in it.
        let mut windows = Windows::new(1 << 20, false);
        settle(&mut windows, &tiles, [eye[0] - 300.0, eye[1], eye[2] + 200.0], &mut HashMap::new());
        settle(&mut windows, &tiles, eye, &mut HashMap::new());
        let window = &windows.windows[0];
        let (ball, origin, (low, high)) = (window.ball.expect("a ball"), window.origin, window.bounds());
        let (lo, hi) = ball.bounds();
        let (lo, hi) = (lo.map(|v| v * BRICK), hi.map(|v| v * BRICK));
        // Every voxel of the ball, and every cell of each scale it meets, at the ball's edge too.
        let mut voxels = 0;
        for z in lo[2]..hi[2] {
            for y in lo[1]..hi[1] {
                for x in (lo[0] + (lo[0] + y + z + origin.iter().sum::<i32>()).rem_euclid(2)..hi[0]).step_by(2) {
                    let p = [x + origin[0], y + origin[1], z + origin[2]];
                    if !ball.contains([x, y, z].map(|v| v.div_euclid(BRICK))) || tiles.grids.at(p[0], p[1], p[2]).is_none_or(|v| v == 0) {
                        continue;
                    }
                    voxels += 1;
                    assert!((0..3).all(|d| low[d] < p[d] as f32 && (p[d] as f32) < high[d]), "a voxel at {p:?} out of {low:?}..{high:?}");
                    for j in 0..SCALES {
                        let size = BRICK << (2 * j);
                        // The cells' centres origin + size · k, k's coordinates adding up to an even number, within size + 1 of p.
                        let near = |d: usize| ([x, y, z][d] - size).div_euclid(size)..=([x, y, z][d] + size).div_euclid(size);
                        for k in near(2).flat_map(|kz| near(1).flat_map(move |ky| near(0).map(move |kx| [kx, ky, kz]))) {
                            let [ax, ay, az] = std::array::from_fn(|d| ([x, y, z][d] - k[d] * size).abs());
                            if (k[0] + k[1] + k[2]) & 1 == 0 && ax + ay <= size && ay + az <= size && ax + az <= size {
                                assert_ne!(window.counts[cell_entry(k, j)], 0, "a voxel at {p:?} meets the cell of scale {j} about {k:?}");
                            }
                        }
                    }
                }
            }
        }
        assert!(voxels > 100_000, "{voxels} voxels");
    }

    #[test]
    fn the_shader_is_valid_and_lays_out_what_this_file_writes() {
        use crate::gpu::wgsl;
        let module = wgsl::shader(&shader());
        wgsl::assert_layout!(&module, "Globals", Globals { resolution, extent, fog, sky_fog, lit, sky_top, horizon, view });
        wgsl::assert_layout!(&module, "Level", GpuLevel { min, offset, corner, reach, low, filled, high });
    }
}
