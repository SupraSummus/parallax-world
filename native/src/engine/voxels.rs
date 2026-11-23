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
//! never change once made, so a brick is uploaded once, and its slot
//! freed as it leaves its ball.
//!
//! The windows are kept on a thread of their own (Keeper), which a draw
//! hands a task and takes a patch from: what the GPU holds of them is a
//! draw or two late, and no draw waits for a step.
//!
//! Only what rays meet is made and held: a brick of no tile yet, or with
//! voxels and not placed yet, is unknown, so rays take the next coarser
//! level there (what is missing shows coarser, never as a hole) and ask
//! for it (a bit per entry of the windows' tables, read back: Asked). A
//! brick asked for of a tile made is placed; of none, the world is asked
//! for its tile. In a world that is not flat, walls hide most of a
//! window's ball: what shows is far less.
//!
//! A voxel shows how it looks along the ray, through the faces turned to
//! it (fcc's `facing`; a brick whose voxels look unlike through some faces
//! takes more slots, for what those see: `sights`, Pool), times the light on it: shaded,
//! by the way its surface faces; lit, the light its tile found for it
//! (world.rs, light's Light), and its own glow, uploaded with its brick
//! to a pool of its own; as the eye sees it (eye.rs), adapted to what the
//! samples' rays meet, metered and read back with what they asked for,
//! and with what the ray through the pointer's pixel meets (Probe,
//! inspect.rs).
//!
//! Lit, only what rays meet is lit: a brick asked for is unknown till it
//! has its light. The world lights the bricks asked for, and the windows
//! place them once lit.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};

use egui_wgpu::wgpu;
use fcc::GRID_LEVELS;
use fcc::light::{rgb9e5, srgb_to_linear};
use light::Light;

use super::{Engine, Frame, Look, Painter, Picture};
use crate::eye::{self, Adapted, BINS};
use crate::gpu::Easel;
use crate::threads::Post;
use crate::view::{Vec3, focal_length, view_basis};
use crate::world::{Level, Lit, Piece, Sight, TILE, Tile, TileId, Wants, World};

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

/** Tiles whose bricks are in its ball that a window lets go or takes in a step (Window::retarget): as many as are made with a few (worldgen's made_with). */
const STEP: usize = 64;

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
 * Only the ball's bricks count: an unknown one in every cell around it.
 * Rays read nothing past the ball (reach), so a cell across its edge is
 * crossed as far as they read, not stepped through to the level's end. A
 * cell's entry in the torus is also that of the cell a window away, whose
 * bricks only add to its count. So a cell counted hollow holds no voxel
 * the ball holds, as a ray stepping voxel by voxel would find.
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
/** In a slot's `next` (Pool): the slot after it holds what the brick's see-through voxels stop of each channel (`words`), not what its faces see. */
const STOPS: i32 = 1 << 30;
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

/** What rays see of `look`: as far off the way it looks as its picture's corners. */
fn sight(look: &Look) -> Sight {
    let [width, height] = look.size;
    Sight { forward: view_basis(look.yaw, look.pitch)[2], half: (width.hypot(height) / 2.0 / focal_length(look.fov, width, height)).atan() }
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
    // A power of two: a mask, not a division.
    let across = WINDOW >> (2 * j);
    let w = k.map(|v| v & (across - 1));
    j * CELLS + (((w[2] * across + w[1]) * across + w[0]) / 2) as usize
}

/**
 * Each brick of `grid` with voxels: its place in the grid's bricks, where
 * it is in the level's bricks (whose sites start where the grid's bricks
 * do, modulo BRICK), and where its sites start in the grid's cells, in
 * bricks.
 */
pub fn bricks(grid: &fcc::Grid) -> impl Iterator<Item = (usize, [i32; 3], usize)> + '_ {
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
        let see_through = |i: usize| fcc::medium(grid.cells[i]);
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

/** Sights a site's faces see beside its own look, at most, as the GPU holds them (`sights`). */
const SIGHTS: usize = 3;

/**
 * What the faces of a site of look `own` see, given its looks through
 * each (`views`), as worldgen's Sights hold it: the looks unlike its own,
 * each once; how many; and, two bits a face, which each sees: 0 its own,
 * else a sight's number from 1. Most sites' faces see two looks in all; a
 * face that sees one more than SIGHTS sees the site's own.
 */
fn sights(own: u32, views: &[u32]) -> ([u32; SIGHTS], usize, u32) {
    let (mut sights, mut count, mut which) = ([0; SIGHTS], 0, 0);
    for (k, &view) in views.iter().enumerate().filter(|(_, view)| **view != own) {
        let at = sights[..count].iter().position(|s| *s == view).unwrap_or(count);
        if at == SIGHTS {
            continue;
        }
        (sights[at], count) = (view, count.max(at + 1));
        which |= (at as u32 + 1) << (2 * k);
    }
    (sights, count, which)
}

/**
 * Slots the brick of `grid` whose sites start at `first` takes: one; one
 * more if some see-through voxel of it stops channels unlike; and, if its
 * voxels look unlike through some faces, one for which sight each face
 * sees and one per sight of the site with most (`sights`).
 */
fn slots(grid: &fcc::Grid, first: usize) -> usize {
    let most = grid.views_in(sites(first)).map(|(i, views)| sights(grid.cells[i], views).1).max();
    1 + usize::from(grid.has_stops(sites(first))) + most.map_or(0, |most| 1 + most)
}

/**
 * Part `part` of the brick whose sites start at `first` in `grid`'s cells
 * (in bricks) as the GPU takes it, a word a site: part 0, each site's
 * look; then, if the brick has any (`slots`), what each see-through site
 * stops of each channel (fcc's `stops_at`); then which sight each of its
 * faces sees, and its sights, one a part (`sights`).
 */
fn words(grid: &fcc::Grid, first: usize, part: usize) -> [u32; BRICK_SITES] {
    let (sites, stops) = (sites(first), usize::from(grid.has_stops(sites(first))));
    let mut out = [0; BRICK_SITES];
    if part == 0 {
        out.copy_from_slice(&grid.cells[sites]);
    } else if part == stops {
        for (word, i) in out.iter_mut().zip(sites) {
            if fcc::medium(grid.cells[i]) {
                *word = grid.stops_at(i);
            }
        }
    } else {
        for (i, views) in grid.views_in(sites.clone()) {
            let (sights, _, which) = sights(grid.cells[i], views);
            out[i - sites.start] = if part == stops + 1 { which } else { sights[part - stops - 2] };
        }
    }
    out
}

/** Words a site's light takes: the light on it, and what it glows. */
const LIGHT_WORDS: usize = 2;

/**
 * The light on the voxels of the brick of `grid`, of `level`, whose sites
 * start at `corner`, by site in the order of its cells, LIGHT_WORDS each:
 * Light::on_seen, and its own glow, as RGB9E5; 0 where none. What a voxel
 * sends out is its colour times the one, and the other. Both are of the
 * site's place alone, `grid` saying only which sites have them: so where
 * `was`, another grid of the tile and the light of its brick here, has a
 * voxel too, its light is taken, not found again.
 */
pub fn light(grid: &fcc::Grid, level: u32, corner: [i32; 3], light: &mut Light, was: Option<(&fcc::Grid, &[u32])>) -> Box<[u32]> {
    let mut out = vec![0; BRICK_SITES * LIGHT_WORDS];
    for r in 0..BRICK * BRICK * BRICK {
        let site: [i32; 3] = std::array::from_fn(|d| corner[d] + [r % BRICK, r / BRICK % BRICK, r / BRICK / BRICK][d]);
        let look = grid.get(site[0], site[1], site[2]);
        if (site[0] + site[1] + site[2]) & 1 != 0 || look == 0 {
            continue;
        }
        let i = (r >> 1) as usize * LIGHT_WORDS;
        let words = &mut out[i..i + LIGHT_WORDS];
        match was {
            Some((had, found)) if had.get(site[0], site[1], site[2]) != 0 => words.copy_from_slice(&found[i..i + LIGHT_WORDS]),
            _ => {
                let at = site.map(i64::from);
                if let Some(on) = light.on_seen(level, at) {
                    words.copy_from_slice(&[rgb9e5(on), rgb9e5(light.glow(level, at))]);
                }
            }
        }
    }
    out.into()
}

/**
 * The bricks rays asked for, as read back, by level and where their sites
 * start, each with whether its tile is made (so it waits for light).
 */
type Asked = Vec<(usize, [i32; 3], bool)>;

/** The voxels engine: each frame, the tiles of the levels its rays read, as they are; and what they asked for, as last read back. */
pub struct Voxels {
    /** What rays asked for, as the windows' thread tells (Windows::hear): the last told is what they ask for. */
    asked: (Sender<Asked>, Receiver<Asked>),
    /** Whether frames to come may ask for more, or place what was made or lit, as its painter last drew. */
    busy: Arc<AtomicBool>,
    pieces: Arc<Vec<(usize, Piece)>>,
    /** What its last frame showed, from where: one of other tiles or from elsewhere has yet to ask for what it lacks. */
    shown: (Vec<Arc<Level>>, Vec3),
}

/** Till its painter has drawn (an engine made anew, as when the world is lit anew), frames to come may ask for anything: busy, so that they come. */
impl Default for Voxels {
    fn default() -> Voxels {
        Voxels { asked: channel(), busy: Arc::new(AtomicBool::new(true)), pieces: Arc::default(), shown: Default::default() }
    }
}

impl Engine for Voxels {
    fn frame(&mut self, look: &Look, world: &World) -> Frame {
        let count = levels(look.far);
        let levels: Vec<_> = (0..count).map(|l| world.level(l).clone()).collect();
        let tiles: usize = levels.iter().map(|l| l.tiles.len()).sum();
        let lit = if look.light.is_some() { Lit::Bricks } else { Lit::Not };
        let reach = std::array::from_fn(|l| (l < count).then_some(READ));
        if let Some(mut bricks) = self.asked.1.try_iter().last() {
            // Nearest first in each level's own sites, the finer first as near: as rays read the levels. Those of no
            // tile before those to light: a tile is little work next to its light, which no ray asks for till it is made.
            let far = |&(level, corner, _): &(usize, [i32; 3], bool)| {
                let at = look.camera.map(|v| v / (1u64 << level) as f64);
                (0..3).map(|d| (f64::from(corner[d] + BRICK / 2) - at[d]).powi(2)).sum::<f64>()
            };
            bricks.sort_by(|a, b| a.2.cmp(&b.2).then(far(a).total_cmp(&far(b))).then(a.0.cmp(&b.0)));
            // A brick of a tile made, to light; of none, its tile to make, as wanted as the nearest of its bricks asked for.
            let piece = |(level, corner, made): (usize, [i32; 3], bool)| {
                (level, if made { Piece::Brick(corner) } else { Piece::Tile(corner.map(|v| v.div_euclid(TILE as i32))) })
            };
            let mut tiles = HashSet::new();
            let once = |&(level, piece): &(usize, Piece)| !matches!(piece, Piece::Tile(_)) || tiles.insert((level, piece));
            self.pieces = Arc::new(bricks.into_iter().map(piece).filter(once).collect());
        }
        let busy = self.busy.load(Ordering::Relaxed);
        let same =
            self.shown.1 == look.camera && self.shown.0.len() == count && self.shown.0.iter().zip(&levels).all(|(a, b)| Arc::ptr_eq(a, b));
        self.shown = (levels.clone(), look.camera);
        let (asked, painted) = (self.asked.0.clone(), self.busy.clone());
        Frame {
            picture: Box::new(Scene { levels, look: *look, asked, busy: painted }),
            wants: Wants { reach, lit, asked: self.pieces.clone(), sight: Some(sight(look)) },
            busy: busy || !same,
            stats: format!("{tiles} tiles"),
        }
    }
}

/** What to draw: each level's tiles, as `look` sees them; where to tell what rays ask for, and whether frames to come are busy. */
struct Scene {
    levels: Vec<Arc<Level>>,
    look: Look,
    asked: Sender<Asked>,
    busy: Arc<AtomicBool>,
}

impl Picture for Scene {
    fn draw(&self, easel: &mut Easel, encoder: &mut wgpu::CommandEncoder, pixels: [u32; 2]) {
        let lit = self.look.light.is_some();
        // Lit or not, its pool is another.
        if easel.held::<Marcher>().is_some_and(|m| m.lit != lit) {
            easel.painter = None;
        }
        let still = easel.still;
        let (marcher, device, queue) = easel.painter(|device, format| Marcher::new(device, format, lit));
        marcher.draw(device, queue, encoder, self, pixels, still);
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
    /** The multiple of light the eye shows (eye.rs): lit, as it adapted; else 1. */
    exposure: f32,
    /** The sky's light, linear. */
    sky_top: [f32; 4],
    horizon: [f32; 4],
    /** Right, up, forward, in the world: columns of a mat3x3f. */
    view: [[f32; 4]; 3],
    /** The light the eye shows white: unlit, 1, the light as it is. */
    white: f32,
    _pad: u32,
    /** The pixel whose ray to probe (Probe), negative if none. */
    pointer: [f32; 2],
}

impl Globals {
    fn of(look: &Look, pixels: [u32; 2], marks: [f32; 2]) -> Globals {
        let [width, height] = pixels.map(f64::from);
        let focal = focal_length(look.fov, width, height);
        let column = |v: Vec3| [v[0] as f32, v[1] as f32, v[2] as f32, 0.0];
        let sky = look.light.map_or(1.0, |b| b.sky);
        let rgba = |c: [f32; 3]| {
            let [r, g, b] = c.map(|c| srgb_to_linear(c) * sky);
            [r, g, b, 1.0]
        };
        let [exposure, white] = if look.light.is_some() { look.eye.shows(marks) } else { [1.0; 2] };
        Globals {
            resolution: [width as f32, height as f32],
            extent: [(width / focal) as f32, (height / focal) as f32],
            fog: (look.far * look.fog) as f32,
            // What of the sky the fog hides, as of something at the far end.
            sky_fog: (1.0 - (-1.0 / look.fog).exp()) as f32,
            lit: u32::from(look.light.is_some()),
            exposure,
            sky_top: rgba(SKY_TOP),
            horizon: rgba(SKY_HORIZON),
            view: view_basis(look.yaw, look.pitch).map(column),
            white,
            _pad: 0,
            // Within the picture, so that a frame with a pointer always probes.
            pointer: look
                .pointer
                .map_or([-1.0; 2], |p| std::array::from_fn(|d| (p[d] * f64::from(pixels[d])).floor().min(f64::from(pixels[d] - 1)) as f32)),
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
    let w = b.map(|v| v & (WINDOW - 1));
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

/**
 * The pool's slots: those free, and per slot, the next of its brick's
 * (EMPTY if none), words' parts in a chain: a brick whose voxels look
 * unlike through some faces takes more, which sight each face sees, then
 * its sights; before them, marked STOPS, one for what its see-through
 * voxels stop of each channel, if they differ.
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
        if tile.grid.has_stops(sites(first)) {
            self.next[chain[0] as usize] |= STOPS;
        }
        chain[0]
    }

    /** Frees `slot` and those after it. */
    fn give(&mut self, slot: u32) -> usize {
        let (mut slot, mut count) = (slot as i32, 0);
        while slot >= 0 {
            self.free.push(slot as u32);
            slot = after(std::mem::replace(&mut self.next[slot as usize], EMPTY));
            count += 1;
        }
        count
    }
}

/** The slot a slot's `next` names, EMPTY if none: without its mark. */
fn after(next: i32) -> i32 {
    if next < 0 { next } else { next & !STOPS }
}

/** A tile a window holds bricks of; per brick of its grid (in its cells, its buried ones one: worldgen's VoxelGrid::bury), its slot (EMPTY if none) and how many entries hold it. */
struct Held {
    tile: Arc<Tile>,
    slots: Vec<(i32, u32)>,
}

impl Held {
    /** `tile`, none of its bricks in a slot yet. */
    fn new(tile: &Arc<Tile>) -> Held {
        Held { tile: tile.clone(), slots: vec![(EMPTY, 0); tile.grid.cells.len() / BRICK_SITES] }
    }
}

/**
 * A level's window as the GPU holds it: from which tiles, the ball of
 * bricks it holds of them, its table (per brick a slot, EMPTY or UNKNOWN:
 * every brick outside the ball), and, per cell of each scale, how many
 * of the ball's bricks around it have voxels that meet it or are unknown
 * (SCALES's). Bricks of a tile that share their sites share a slot: the
 * ground's rock, buried, a slot a tile.
 */
struct Window {
    /** The tiles it holds, as last given; None while some are still to come or go. */
    level: Option<Arc<Level>>,
    tiles: HashMap<TileId, Held>,
    ball: Option<Ball>,
    /** Sites of the level's bricks start at origin + BRICK · b. */
    origin: [i32; 3],
    table: Vec<i32>,
    /** Per entry of the table, the cells of each scale around its brick it counts in (AROUND's bits): those its voxels meet, all if UNKNOWN in the ball, none out of it. */
    meets: Vec<[u32; SCALES]>,
    counts: Vec<u32>,
    /** Per axis, per brick coordinate along it (modulo WINDOW, as the table), how many of the ball's bricks count in a cell. */
    rows: [[u32; WINDOW as usize]; 3],
    /** Slots it holds. */
    slots: usize,
    /** The bricks of its tiles rays last asked for (asked's), to place once lit. */
    asked: Vec<[i32; 3]>,
    /** Whether an entry or a count changed since last asked (Windows::sync). */
    changed: bool,
}

impl Window {
    fn new() -> Window {
        Window {
            level: None,
            tiles: HashMap::new(),
            ball: None,
            origin: [0; 3],
            table: vec![UNKNOWN; TABLE],
            meets: vec![[0; SCALES]; TABLE],
            counts: vec![0; COUNTS],
            rows: [[0; WINDOW as usize]; 3],
            slots: 0,
            asked: Vec::new(),
            // All of it to write: the GPU's is another's, or none's.
            changed: true,
        }
    }

    /** Where brick `b` starts, and the tile it is of. */
    fn site(&self, b: [i32; 3]) -> ([i32; 3], TileId) {
        let site: [i32; 3] = std::array::from_fn(|d| self.origin[d] + b[d] * BRICK);
        (site, site.map(|v| v.div_euclid(TILE as i32)))
    }

    /** Sets brick `b`'s entry and the cells around it that it counts in; returns the entry it had. */
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
        let was = std::mem::replace(&mut self.table[at], entry);
        self.changed |= was != entry || had != meets;
        was
    }

    /** Makes brick `b` UNKNOWN, counting in the cells `around` it: all (AROUND) in the ball, none as it leaves; its slots freed once no entry holds it. */
    fn release(&mut self, b: [i32; 3], around: u32, pool: &mut Pool) {
        if self.set(b, (UNKNOWN, [around; SCALES])) < 0 {
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
     * or, `asked` for by a ray, the slot its sites are in, from `pool` if
     * none is yet (to upload, in `added`: as many as `needs` says); UNKNOWN
     * still if no tile is held there, no ray asked for it, or, `lit`, its
     * light is not found yet.
     */
    fn place(&mut self, b: [i32; 3], lit: bool, asked: bool, pool: &mut Pool, added: &mut Vec<Added>) {
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
        if !asked || lit && held.tile.light(first).is_none() {
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
     * `level`'s `tiles` about `eye`, if rays read it (`read`): the eye's,
     * with tiles or none yet, all unknown then, for rays to ask for. Not
     * read, where it is; None if it holds nothing then.
     */
    fn aim(&self, level: usize, tiles: &Level, eye: Vec3, read: bool) -> Option<([i32; 3], [i32; 3])> {
        match (tiles.tiles.iter().next(), self.ball) {
            (Some((_, tile)), _) => {
                let origin = tile.grid.min.map(|v| v.rem_euclid(BRICK));
                Some((origin, eye_brick(level, eye, origin)))
            }
            (None, _) if read => Some((self.origin, eye_brick(level, eye, self.origin))),
            (None, Some(ball)) => Some((self.origin, ball.middle)),
            (None, None) => None,
        }
    }

    /** Whether it is short of holding `tiles` of `level` as a ball about `eye` (retarget's). */
    fn behind(&self, level: usize, tiles: &Arc<Level>, eye: Vec3, read: bool) -> bool {
        self.aim(level, tiles, eye, read).is_some_and(|(_, target)| {
            !self.level.as_ref().is_some_and(|l| Arc::ptr_eq(l, tiles)) || self.ball.is_none_or(|b| b.middle != target)
        })
    }

    /**
     * Moves the window a step toward the ball of `radius` about `eye`'s
     * brick of `level` (aim's), holding `tiles`: what the ball leaves, or a tile gone (or made anew), becomes
     * UNKNOWN. Returns the bricks to place: those of tiles it holds that
     * the ball gains, and those of a tile new.
     *
     * A step is a brick along one axis, and STEP tiles whose bricks are in
     * the ball, gone or (once none is) the nearest new: what it places
     * is a shell of the ball and those tiles', however fast the eye or
     * many the tiles. Rays take the next level for what is still UNKNOWN; a tile
     * gone is held till its step, and one made anew placed in it. A ball that holds nothing around the eye
     * starts anew there, the window as made (clear).
     */
    fn retarget(&mut self, level: usize, tiles: &Arc<Level>, eye: Vec3, read: bool, radius: i32, pool: &mut Pool) -> Vec<[i32; 3]> {
        let Some((origin, target)) = self.aim(level, tiles, eye, read) else { return Vec::new() };
        assert!(self.ball.is_none() || self.origin == origin, "a level's bricks moved");
        if self.ball.is_some_and(|old| old.about(target) < 0) {
            self.clear(pool);
        }
        self.origin = origin;
        let middle = self.ball.map_or(target, |old| {
            let d = (0..3).max_by_key(|&d| (target[d] - old.middle[d]).abs()).expect("an axis");
            std::array::from_fn(|k| old.middle[k] + if k == d { (target[d] - old.middle[d]).signum() } else { 0 })
        });
        let (old, ball) = (self.ball, Ball { middle, radius });
        if let Some(old) = old {
            for b in in_ball(old.bounds(), old, Some(ball)) {
                self.release(b, 0, pool);
            }
        }
        // What the ball gains: unknown, till placed from the tiles held (none, begun anew).
        let mut pending: Vec<[i32; 3]> = Vec::new();
        if old.is_some() {
            pending.extend(in_ball(ball.bounds(), ball, old));
            for &b in &pending {
                self.set(b, (UNKNOWN, [AROUND; SCALES]));
            }
        } else {
            self.fill(ball);
        }
        self.ball = Some(ball);
        let meets = |id: TileId| in_ball(tile_bricks(id, origin), ball, None).next().is_some();
        let (mut steps, mut left) = (STEP, 0);
        let gone: Vec<TileId> = (self.tiles.iter())
            .filter(|&(id, h)| tiles.tiles.get(*id).is_none_or(|t| !Arc::ptr_eq(t, &h.tile)))
            .map(|(id, _)| *id)
            .collect();
        for id in gone {
            if !meets(id) {
                self.tiles.remove(&id);
            } else if steps > 0 {
                steps -= 1;
                // One made anew (World::now) takes its place in the same step, what showed of the old placed of it as
                // if rays asked for it (ripe's, once lit): else rays would read the next level there till they ask.
                let anew = tiles.tiles.get(id).is_some();
                for b in in_ball(tile_bricks(id, origin), ball, None) {
                    if anew && self.table[entry(b)] >= 0 {
                        self.asked.push(b);
                    }
                    self.release(b, AROUND, pool);
                }
                self.tiles.remove(&id);
                if let Some(tile) = tiles.tiles.get(id) {
                    self.tiles.insert(id, Held::new(tile));
                    pending.extend(in_ball(tile_bricks(id, origin), ball, None));
                }
            } else {
                left += 1;
            }
        }
        let mut new: Vec<_> = tiles.tiles.iter().filter(|(id, _)| !self.tiles.contains_key(id)).collect();
        new.sort_by_key(|(id, _)| {
            let (lo, hi) = tile_bricks(*id, origin);
            (0..3).map(|d| (lo[d] + hi[d] - 2 * target[d]).pow(2)).sum::<i32>()
        });
        for (id, tile) in new {
            assert!((0..3).all(|d| tile.grid.min[d].rem_euclid(BRICK) == origin[d]), "a tile off its level's bricks");
            if meets(id) {
                if steps == 0 {
                    left += 1;
                    continue;
                }
                steps -= 1;
            }
            self.tiles.insert(id, Held::new(tile));
            pending.extend(in_ball(tile_bricks(id, origin), ball, None));
        }
        // Tiles still to change: it is behind, and steps again.
        self.level = (left == 0).then(|| tiles.clone());
        pending
    }

    /**
     * Makes every brick of `ball` unknown in a window as made, as `set`
     * would brick by brick, but at once: a whole ball begun anew is what a
     * step costs at most, however fast the eye. A cell's count is that of
     * the ball's bricks in the blocks around its corner: of each row of
     * bricks there, as many as are in the ball's row.
     */
    fn fill(&mut self, ball: Ball) {
        let (lo, hi) = ball.bounds();
        let wide = (hi[1] - lo[1]) as usize;
        let rows: Vec<(i32, i32)> = (lo[2]..hi[2]).flat_map(|z| (lo[1]..hi[1]).map(move |y| ball.row(y, z))).collect();
        for (i, &(from, to)) in rows.iter().enumerate() {
            for b in (from..to).map(|x| [x, lo[1] + (i % wide) as i32, lo[2] + (i / wide) as i32]) {
                self.meets[entry(b)] = [AROUND; SCALES];
                for (rows, v) in self.rows.iter_mut().zip(b) {
                    rows[(v & (WINDOW - 1)) as usize] += 1;
                }
            }
        }
        let mut near = Vec::new();
        for j in 0..SCALES {
            // The cells with a block of the ball's box around them, and the bricks of the blocks around cell `k`, along an axis.
            let cells = |d: usize| (lo[d] >> (2 * j)) - 1..=((hi[d] - 1) >> (2 * j)) + 1;
            let bricks = |k: i32| ((k - 1) << (2 * j), (k + 2) << (2 * j));
            for (kz, ky) in cells(2).flat_map(|kz| cells(1).map(move |ky| (kz, ky))) {
                let ((y0, y1), (z0, z1)) = (bricks(ky), bricks(kz));
                near.clear();
                for z in z0.max(lo[2])..z1.min(hi[2]) {
                    near.extend((y0.max(lo[1])..y1.min(hi[1])).map(|y| rows[(z - lo[2]) as usize * wide + (y - lo[1]) as usize]));
                }
                for kx in cells(0).filter(|kx| (kx + ky + kz) & 1 == 0) {
                    let (x0, x1) = bricks(kx);
                    let count: i32 = near.iter().map(|&(from, to)| (to.min(x1) - from.max(x0)).max(0)).sum();
                    // A cell a window away shares the entry: both add to it.
                    self.counts[cell_entry([kx, ky, kz], j)] += count as u32;
                }
            }
        }
        self.changed = true;
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
            self.release(b, 0, pool);
        }
        for b in in_ball(ball.bounds(), ball, Some(old)) {
            self.set(b, (UNKNOWN, [AROUND; SCALES]));
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
     * The brick whose entry in its table was `e` as rays read it, its
     * ball `seen` then, if they may ask for it,
     * unknown in its ball, and whether it holds its tile: of a tile it
     * holds, one with voxels (to place, once lit if it is to be); of
     * none, any: its tile is to make.
     */
    fn asked(&self, e: usize, seen: Option<Ball>) -> Option<([i32; 3], bool)> {
        let (seen, ball) = (seen?, self.ball?);
        let w = [e % WINDOW as usize, e / WINDOW as usize % WINDOW as usize, e / (WINDOW * WINDOW) as usize].map(|v| v as i32);
        // The window's corner as rays read it (Shape::gpu): its bricks, from there.
        let low = seen.middle.map(|v| v - WINDOW / 2);
        let b = std::array::from_fn(|d| low[d] + (w[d] - low[d]).rem_euclid(WINDOW));
        let ([x, y, z], id) = self.site(b);
        let held = self.tiles.get(&id);
        let asked = held.is_none_or(|h| h.tile.grid.brick(x, y, z) >= 0);
        (ball.contains(b) && self.table[entry(b)] == UNKNOWN && asked).then_some((b, held.is_some()))
    }

    /** Of the bricks rays last asked for, those not placed yet, and, `lit`, lit since. */
    fn ripe(&self, lit: bool) -> Vec<[i32; 3]> {
        let lit = |b: [i32; 3]| {
            let ([x, y, z], id) = self.site(b);
            let held = self.tiles.get(&id);
            !lit || held.is_some_and(|h| usize::try_from(h.tile.grid.brick(x, y, z)).is_ok_and(|first| h.tile.light(first).is_some()))
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

    /** As rays read it (Shape). */
    fn shape(&self) -> Shape {
        Shape { origin: self.origin, ball: self.ball, bounds: self.bounds() }
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

/** A window as rays read it: where its bricks start, its ball, and the box what it holds is in (Window::bounds). */
#[derive(Clone, Copy)]
struct Shape {
    origin: [i32; 3],
    ball: Option<Ball>,
    bounds: ([f32; 3], [f32; 3]),
}

impl Shape {
    /** As voxels.wgsl reads it, the level drawn as far as `far` (world units) or its ball holds around `eye`, unless it holds nothing yet: rays read none past 0. */
    fn gpu(&self, level: usize, eye: Vec3, far: f64) -> GpuLevel {
        let corner = self.ball.map_or([0; 3], |b| b.middle.map(|v| v - WINDOW / 2));
        let (low, high) = self.bounds;
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
}

/**
 * What rays asked for, read back: the windows' changes as the frame they
 * asked in was drawn, the balls it read (an entry is a brick by its
 * window's ball: since, that may have begun anew elsewhere), and the
 * words that are not 0, each with its place.
 */
type Read = (u64, [Option<Ball>; GRID_LEVELS], Vec<(usize, u32)>);

/**
 * What a draw has the windows do (Windows::step): hold the `levels`'
 * tiles around `eye`; and what rays asked for, if read back since the
 * last: at which of the windows' changes, and the words that are not 0,
 * each with its place; `asked` is told the bricks.
 */
struct Task {
    levels: Vec<Arc<Level>>,
    eye: Vec3,
    read: Option<Read>,
    asked: Sender<Asked>,
}

/** Slots' words to copy into a pool, `per` a slot: the slots, in order, and their words. */
struct Words {
    slots: Vec<u32>,
    words: Vec<u32>,
    per: usize,
}

impl Words {
    fn of<'a, W: AsRef<[u32]>>(added: impl IntoIterator<Item = &'a Added>, per: usize, words: impl Fn(&'a Added) -> W) -> Words {
        let mut added: Vec<_> = added.into_iter().collect();
        added.sort_unstable_by_key(|a| a.slot);
        let mut all = Vec::with_capacity(added.len() * per);
        for &a in &added {
            all.extend_from_slice(words(a).as_ref());
        }
        assert_eq!(all.len(), added.len() * per, "{per} words a slot");
        Words { slots: added.iter().map(|a| a.slot).collect(), words: all, per }
    }
}

/**
 * What a step changed, for the GPU to hold the windows as they are after
 * it, all of it or none: the bricks placed (their words, their slots'
 * next, and, lit, their light), the tables and counts of the windows
 * that changed, every window's shape. And of the task: its tiles and
 * eye, whether a window is still behind them (Windows::next), and what
 * was read back (at which changes, and whether rays asked for any brick).
 */
struct Patch {
    of: (Vec<Arc<Level>>, Vec3),
    behind: bool,
    read: Option<(u64, bool)>,
    bricks: [Words; 2],
    lights: Option<Words>,
    tables: Vec<(usize, Vec<i32>, Vec<u32>)>,
    shapes: [Shape; GRID_LEVELS],
    /** Slots held, slots free, and the windows' radius. */
    held: [usize; 2],
    radius: i32,
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
        (1..=GRID_LEVELS).map(|k| (self.last + k) % GRID_LEVELS).find(|&l| self.windows[l].behind(l, tiles(l), eye, l < levels.len()))
    }

    /** The slots placing each window's `pending` bricks would take. */
    fn needs(&self, pending: &[Vec<[i32; 3]>]) -> usize {
        self.windows.iter().zip(pending).map(|(w, bricks)| w.needs(bricks, self.lit)).sum()
    }

    /** Places each window's `pending` bricks, which fit: `asked` for by rays, or not, so only known empty if they are. */
    fn place(&mut self, pending: Vec<Vec<[i32; 3]>>, asked: bool, added: &mut Vec<Added>) {
        for (window, bricks) in self.windows.iter_mut().zip(pending) {
            for b in bricks {
                window.place(b, self.lit, asked, &mut self.pool, added);
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
        let (mut found, mut added) = (vec![Vec::new(); GRID_LEVELS], Vec::new());
        if let Some(level) = self.next(levels, eye) {
            let tiles = levels.get(level).unwrap_or(&self.none);
            found[level] = self.windows[level].retarget(level, tiles, eye, level < levels.len(), self.radius, &mut self.pool);
            self.last = level;
        }
        self.place(found, false, &mut added);
        let mut pending: Vec<_> = self.windows.iter().map(|w| w.ripe(self.lit)).collect();
        // Short of slots: every window a brick less, till what is to place fits.
        while self.needs(&pending) > self.pool.free.len() {
            self.radius -= 1;
            for (window, bricks) in self.windows.iter_mut().zip(&mut pending) {
                window.resize(self.radius, bricks, &mut self.pool);
            }
            self.growing = 0;
        }
        self.place(pending, true, &mut added);
        // Room to spare: every window a brick more, if what that gains would fit with room still, were it all asked for.
        if self.radius < RADIUS && self.pool.free.len() >= self.growing.max(self.margin) {
            let radius = self.radius + 1;
            let shells: Vec<_> = self.windows.iter().map(|w| w.shell(radius)).collect();
            let needs = self.needs(&shells);
            if needs + self.margin <= self.pool.free.len() {
                for window in &mut self.windows {
                    window.resize(radius, &mut Vec::new(), &mut self.pool);
                }
                self.place(shells, false, &mut added);
                (self.radius, self.growing) = (radius, 0);
            } else {
                self.growing = needs + self.margin;
            }
        }
        (added, std::array::from_fn(|level| std::mem::take(&mut self.windows[level].changed)))
    }

    /**
     * Tells `asked` the bricks of the words read back that the windows
     * wait for, a tile or light (Window::asked), which they place once
     * made and lit; whether there is any.
     */
    fn hear(&mut self, seen: &[Option<Ball>; GRID_LEVELS], words: &[(usize, u32)], asked: &Sender<Asked>) -> bool {
        let mut bricks = Vec::new();
        for window in &mut self.windows {
            window.asked.clear();
        }
        for &(k, word) in words {
            for bit in (0..32).filter(|bit| word >> bit & 1 != 0) {
                let (level, e) = ((k * 32 + bit) / TABLE, (k * 32 + bit) % TABLE);
                let window = &mut self.windows[level];
                if let Some((b, held)) = window.asked(e, seen[level]) {
                    if held {
                        window.asked.push(b);
                    }
                    bricks.push((level, window.site(b).0, held));
                }
            }
        }
        let any = !bricks.is_empty();
        // None to tell, once the engine is made anew.
        _ = asked.send(bricks);
        any
    }

    /** Does `task`: hears what was read back, syncs, and packs what that changed. */
    fn step(&mut self, task: Task) -> Patch {
        let read = task.read.map(|(changes, seen, words)| (changes, self.hear(&seen, &words, &task.asked)));
        let (added, changed) = self.sync(&task.levels, task.eye);
        let next = &self.pool.next;
        let bricks = [
            Words::of(&added, BRICK_SITES, |a| words(&a.tile.grid, a.first, a.part)),
            Words::of(&added, 1, |a| [next[a.slot as usize] as u32]),
        ];
        // A brick's own slot only: the rest of its slots have no light.
        let lights = self.lit.then(|| {
            Words::of(added.iter().filter(|a| a.part == 0), BRICK_SITES * LIGHT_WORDS, |a| {
                a.tile.light(a.first).expect("a brick placed lit")
            })
        });
        let tables =
            (0..GRID_LEVELS).filter(|&l| changed[l]).map(|l| (l, self.windows[l].table.clone(), self.windows[l].counts.clone())).collect();
        Patch {
            behind: self.next(&task.levels, task.eye).is_some(),
            of: (task.levels, task.eye),
            read,
            bricks,
            lights,
            tables,
            shapes: std::array::from_fn(|l| self.windows[l].shape()),
            held: [self.windows.iter().map(|w| w.slots).sum(), self.pool.free.len()],
            radius: self.radius,
        }
    }
}

/**
 * The windows, kept on a thread of their own: what a step costs (a ball
 * begun anew, its bricks placed) is not the frame's. A draw hands over a
 * task and takes the patch ready, of an earlier task; the thread does a
 * task once its last patch was taken, so there is one a draw at most.
 * Both go by channels, so a draw waits for the thread only when told to
 * (threads.rs): with each task it says whether it took a patch since the last.
 */
struct Keeper {
    tasks: Post<(Task, bool)>,
    patches: Receiver<Patch>,
    /** Whether a patch was taken that the thread has not been told of. */
    taken: bool,
}

impl Keeper {
    fn new(windows: Windows) -> Keeper {
        let (send, patches) = channel();
        let tasks = Post::spawn(|tasks| vec![move || keep(windows, &tasks, &send)]);
        Keeper { tasks, patches, taken: false }
    }

    /** Hands over `task` in place of the last, if not begun (what that read back kept, unless `task` read more); the patch ready, if any, or, told to `wait`, the next. */
    fn swap(&mut self, task: Task, wait: bool) -> Option<Patch> {
        let mut patch = match self.patches.try_recv() {
            Ok(patch) => Some(patch),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => panic!("the windows' thread stopped"),
        };
        self.taken |= patch.is_some();
        self.tasks.send((task, std::mem::take(&mut self.taken)));
        if wait && patch.is_none() {
            patch = Some(self.patches.recv().expect("the windows' thread stopped"));
            self.taken = true;
        }
        patch
    }
}

/** The windows' thread: the last task given at a time, once the last one's patch was taken, till either end is gone. */
fn keep(mut windows: Windows, tasks: &Receiver<(Task, bool)>, patches: &Sender<Patch>) {
    let (mut task, mut waiting) = (None::<Task>, false);
    loop {
        loop {
            match tasks.try_recv() {
                Ok((mut next, taken)) => {
                    if let Some(last) = task.take() {
                        next.read = next.read.or(last.read);
                    }
                    task = Some(next);
                    waiting &= !taken;
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => return,
            }
        }
        match task.take_if(|_| !waiting) {
            Some(task) => {
                if patches.send(windows.step(task)).is_err() {
                    return;
                }
                waiting = true;
            }
            None => std::thread::park(),
        }
    }
}

const PICTURE: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
const SAMPLE_HITS: wgpu::TextureFormat = wgpu::TextureFormat::Rgba32Uint;
const SAMPLE_COLOURS: wgpu::TextureFormat = wgpu::TextureFormat::Rgba32Uint;

/** Textures as big as the picture: the samples', the picture, and the groups binding them. */
struct Targets {
    pixels: [u32; 2],
    samples: [wgpu::Texture; 2],
    picture: wgpu::Texture,
    groups: [wgpu::BindGroup; 3],
}

/** Words of the bits rays ask for bricks by, one per entry of the windows' tables (voxels.wgsl's `asked`). */
const ASKED_WORDS: usize = GRID_LEVELS * TABLE / 32;

/** See-through voxels the pointer's ray crosses that it writes down, at most. */
const MEDIA: usize = 8;

/** voxels.wgsl's Told. */
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Told {
    metered: [u32; BINS],
    probe: Probe,
}

/** voxels.wgsl's Probe. */
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Probe {
    pub site: [i32; 3],
    pub level: u32,
    pub on: [f32; 3],
    pub distance: f32,
    pub glow: [f32; 3],
    pub crossed: u32,
    pub through: [f32; 3],
    pub path: f32,
    media: [Crossed; MEDIA],
}

/** voxels.wgsl's Crossed. */
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Crossed {
    pub site: [i32; 3],
    pub level: u32,
    pub through: [f32; 3],
    pub path: f32,
}

impl Probe {
    /** Whether a voxel stopped the ray (NONE and LOST are no level). */
    pub fn stopped(&self) -> bool {
        (self.level as usize) < GRID_LEVELS
    }

    /** The see-through voxels it wrote down. */
    pub fn media(&self) -> &[Crossed] {
        &self.media[..(self.crossed as usize).min(MEDIA)]
    }
}

/** Where reading back what rays asked for is: not, copied from a frame (the windows' changes and balls then: Read's; its pointer), or mapping what was copied. */
enum Reading {
    Idle,
    Copied(u64, [Option<Ball>; GRID_LEVELS], Option<[f64; 2]>),
    Mapping(Arc<AtomicBool>, u64, [Option<Ball>; GRID_LEVELS], Option<[f64; 2]>),
}

/** The voxels on a device, drawn into a picture that the window or a shot shows. */
pub struct Marcher {
    /**
     * Globals, levels, tables, the pool, the eye, cells'
     * counts, the pool's bricks' light (none unlit), its slots' next
     * (Pool's), and, lit, the bricks rays asked for (ASKED_WORDS) and
     * the meter's bins and what the pointer's ray met (Told).
     */
    buffers: [wgpu::Buffer; 10],
    /** What the last two were copied to, to read, one after the other. */
    readback: wgpu::Buffer,
    /** What the pointer's ray met, as last read back, and the pointer, as that frame's Look had it. */
    pub probed: Option<([f64; 2], Probe)>,
    /** What the eye adapted to, as the meter last read back says. */
    adapted: Adapted,
    reading: Reading,
    layouts: [wgpu::BindGroupLayout; 3],
    pipelines: [wgpu::RenderPipeline; 3],
    keeper: Keeper,
    /** The windows as the GPU holds them: their shapes, the slots held and free, their radius (the last Patch's). */
    shapes: [Shape; GRID_LEVELS],
    held: [usize; 2],
    radius: i32,
    /** What was read back, for the next task. */
    heard: Option<Read>,
    targets: Option<Targets>,
    /** Whether it draws lit scenes (Look's `sky`). */
    pub lit: bool,
    /** Whether the GPU held, at the last draw, the windows with all that draw gave them (a patch comes a draw late at best, and steps are bounded), and what rays asked for of them as they are was read back. */
    settled: bool,
    /** How many patches changed the windows; as of the frame rays last asked in, read back, and whether they asked for any brick. */
    changes: u64,
    read: (u64, bool),
}

/** voxels.wgsl, with the constants it shares with this file. */
fn shader() -> String {
    format!(
        "const BINS = {BINS}u;\nconst LOWEST = {:?};\nconst SPAN = {:?};\nconst WINDOW = {WINDOW};\nconst CELLS = {CELLS}u;\nconst BRICK = {BRICK};\nconst BRICK_SHIFT = {BRICK_SHIFT}u;\nconst BRICK_SITES = {BRICK_SITES}u;\nconst LIGHT_WORDS = {LIGHT_WORDS}u;\nconst FACTOR = {FACTOR}u;\nconst EMPTY = {EMPTY};\nconst STOPS = {STOPS};\nconst UNKNOWN = {UNKNOWN};\nconst MEDIA = {MEDIA}u;\nconst FACES = array<vec3f, {}>({});\n{}",
        eye::LOWEST,
        eye::SPAN,
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
        // Lit, a brick's light takes LIGHT_WORDS a site, beside its own words.
        let light_words = if lit { BRICK_SITES * LIGHT_WORDS } else { 0 };
        let largest = device.limits().max_storage_buffer_binding_size / (BRICK_SITES.max(light_words) as u64 * 4);
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
            buffer("asked", ASKED_WORDS * 4, storage | wgpu::BufferUsages::COPY_SRC),
            buffer("told", size_of::<Told>(), storage | wgpu::BufferUsages::COPY_SRC),
        ];
        let readback = buffer("read", ASKED_WORDS * 4 + size_of::<Told>(), wgpu::BufferUsages::MAP_READ);
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
            entry(12, write),
        ];
        let samples = [&all[..8], &all[10..]].concat();
        let shown = [entry(10, texture(wgpu::TextureSampleType::Float { filterable: false }))];
        // The samples' pass draws into the textures the resolving one reads, which draws the picture shown; rays
        // ask for bricks, the samples' meter the light they meet, and the pointer's writes down what it meets.
        let layouts = [&samples[..], &all[..], &shown[..]]
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
        let windows = Windows::new(pool as usize, lit);
        let (shapes, held, radius) = (std::array::from_fn(|l| windows.windows[l].shape()), [0, pool as usize], windows.radius);
        let (keeper, reading, heard) = (Keeper::new(windows), Reading::Idle, None);
        let (targets, read) = (None, (u64::MAX, false));
        Marcher {
            buffers,
            readback,
            probed: None,
            adapted: Adapted::default(),
            reading,
            layouts,
            pipelines,
            keeper,
            shapes,
            held,
            radius,
            heard,
            targets,
            lit,
            settled: true,
            changes: 0,
            read,
        }
    }

    /**
     * Has the windows brought up to date with `scene` (a Task), and the
     * GPU with the windows as far as they are (a Patch), then draws the
     * scene into a picture of `pixels`: the samples, then each pixel
     * resolving its own. It reads back what rays asked for as far as it
     * can without waiting (`ask`), and copies what they ask for now to read.
     * `still` (Easel's), it waits for a patch: each draw a step nearer.
     */
    fn draw(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        scene: &Scene,
        pixels: [u32; 2],
        still: bool,
    ) {
        let look = &scene.look;
        self.ask(device);
        let task = Task { levels: scene.levels.clone(), eye: look.camera, read: self.heard.take(), asked: scene.asked.clone() };
        let patch = self.keeper.swap(task, still);
        // Settled once the GPU holds what this draw's tiles and eye come to, and what rays asked for of that was read back.
        let mut current = false;
        if let Some(patch) = patch {
            for (words, buffer) in patch.bricks.iter().zip([3, 7]).chain(patch.lights.iter().zip([6])) {
                upload(device, encoder, &self.buffers[buffer], words);
            }
            for (level, table, counts) in &patch.tables {
                queue.write_buffer(&self.buffers[2], (level * TABLE * 4) as u64, bytemuck::cast_slice(table));
                queue.write_buffer(&self.buffers[5], (level * COUNTS * 4) as u64, bytemuck::cast_slice(counts));
            }
            self.changes += u64::from(!patch.tables.is_empty());
            self.read = patch.read.unwrap_or(self.read);
            (self.shapes, self.held, self.radius) = (patch.shapes, patch.held, patch.radius);
            let same = patch.of.0.len() == scene.levels.len() && patch.of.0.iter().zip(&scene.levels).all(|(a, b)| Arc::ptr_eq(a, b));
            current = !patch.behind && same && patch.of.1 == look.camera;
        }
        self.settled = current && self.read.0 == self.changes;
        let mut levels = [GpuLevel::default(); GRID_LEVELS];
        for (level, gpu) in levels.iter_mut().enumerate().take(scene.levels.len()) {
            *gpu = self.shapes[level].gpu(level, look.camera, look.far);
        }
        queue.write_buffer(&self.buffers[1], 0, bytemuck::cast_slice(&levels));
        self.adapted.adapt(still);
        queue.write_buffer(&self.buffers[0], 0, bytemuck::bytes_of(&Globals::of(look, pixels, self.adapted.marks())));
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
        if matches!(self.reading, Reading::Idle) {
            let mut offset = 0;
            for buffer in &self.buffers[8..] {
                encoder.copy_buffer_to_buffer(buffer, 0, &self.readback, offset, buffer.size());
                encoder.clear_buffer(buffer, 0, None);
                offset += buffer.size();
            }
            self.reading = Reading::Copied(self.changes, self.shapes.map(|s| s.ball), look.pointer);
        }
        let busy = !self.settled || self.read.1 || self.adapted.adapting();
        scene.busy.store(busy, Ordering::Relaxed);
    }

    /**
     * Reads back what rays asked for: maps what the last frame copied, or,
     * once mapped, keeps its words that are not 0 for the next task
     * (`heard`), and has the eye adapt towards what was metered.
     */
    fn ask(&mut self, device: &wgpu::Device) {
        // What it waits for, done: the map's callback.
        _ = device.poll(wgpu::PollType::Poll);
        match std::mem::replace(&mut self.reading, Reading::Idle) {
            Reading::Copied(changes, seen, pointer) => {
                let done = Arc::new(AtomicBool::new(false));
                let mapped = done.clone();
                self.readback.slice(..).map_async(wgpu::MapMode::Read, move |r| mapped.store(r.is_ok(), Ordering::Release));
                self.reading = Reading::Mapping(done, changes, seen, pointer);
            }
            Reading::Mapping(done, changes, seen, pointer) if done.load(Ordering::Acquire) => {
                {
                    let bytes = self.readback.slice(..).get_mapped_range().expect("mapped");
                    let (words, told) = bytemuck::cast_slice::<_, u32>(&bytes).split_at(ASKED_WORDS);
                    let told: Told = bytemuck::pod_read_unaligned(bytemuck::cast_slice(told));
                    self.heard = Some((changes, seen, words.iter().copied().enumerate().filter(|(_, w)| *w != 0).collect()));
                    self.adapted.metered(&told.metered);
                    self.probed = pointer.map(|pointer| (pointer, told.probe));
                }
                self.readback.unmap();
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
        // Bound as numbered, the asked at 11, the told at 12.
        let mut entries: Vec<_> = self.buffers[..8]
            .iter()
            .enumerate()
            .map(|(k, b)| wgpu::BindGroupEntry { binding: k as u32, resource: b.as_entire_binding() })
            .collect();
        for (k, view) in views.iter().enumerate() {
            entries.push(wgpu::BindGroupEntry { binding: 8 + k as u32, resource: wgpu::BindingResource::TextureView(view) });
        }
        for (k, buffer) in self.buffers[8..].iter().enumerate() {
            entries.push(wgpu::BindGroupEntry { binding: 11 + k as u32, resource: buffer.as_entire_binding() });
        }
        let shown = [wgpu::BindGroupEntry { binding: 10, resource: wgpu::BindingResource::TextureView(&picture_view) }];
        let group = |layout, entries: &[wgpu::BindGroupEntry]| {
            device.create_bind_group(&wgpu::BindGroupDescriptor { label: None, layout, entries })
        };
        let sampling = [&entries[..8], &entries[10..]].concat();
        let groups = [group(&self.layouts[0], &sampling), group(&self.layouts[1], &entries), group(&self.layouts[2], &shown)];
        Targets { pixels, samples, picture, groups }
    }
}

/**
 * Copies `words` into `pool`: one buffer, copied from per run of
 * consecutive slots (runs are many, and a write_buffer costs as much as a
 * large copy).
 */
fn upload(device: &wgpu::Device, encoder: &mut wgpu::CommandEncoder, pool: &wgpu::Buffer, Words { slots, words, per }: &Words) {
    use wgpu::util::DeviceExt;
    if slots.is_empty() {
        return;
    }
    let usage = wgpu::BufferUsages::COPY_SRC;
    let from = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("upload"),
        contents: bytemuck::cast_slice(words),
        usage,
    });
    let bytes = (per * 4) as u64;
    let mut start = 0;
    for k in 1..=slots.len() {
        if k == slots.len() || slots[k] != slots[k - 1] + 1 {
            let size = (k - start) as u64 * bytes;
            encoder.copy_buffer_to_buffer(&from, start as u64 * bytes, pool, u64::from(slots[start]) * bytes, size);
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
        let ([held, free], radius) = (self.held, self.radius);
        format!(": {held} slots of {} on the GPU, windows {radius} bricks of {RADIUS} round", held + free)
    }
}

#[cfg(test)]
mod tests;
