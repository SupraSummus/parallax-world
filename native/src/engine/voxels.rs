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
//! Lit, the voxels show the light their tiles found for them (world.rs,
//! worldgen's Light, as the splats' bakers find it), uploaded with their
//! bricks to a pool of its own. Till its tile is lit, a brick is unknown,
//! as one of no tile yet: what is not lit yet shows coarser, lit. A tile
//! lit is a new tile in the place of the one made, so its bricks are
//! uploaded then.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use egui_wgpu::wgpu;
use fcc::GRID_LEVELS;
use fcc::light::{colour, linear_to_srgb, srgb_to_linear};
use worldgen::light::Light;

use super::{Engine, Frame, Look, Painter, Picture};
use crate::gpu::Easel;
use crate::view::{Vec3, focal_length, view_basis};
use crate::world::{Level, Lit, TILE, Tile, TileId, Wants, World};

/** A level's window, in bricks a side (a power of two: the table is a torus). */
const WINDOW: i32 = 64;
/** Sites a brick is a side (bake's BRICK), and the words its sites take, two a word. */
const BRICK_SHIFT: u32 = 3;
const BRICK: i32 = 1 << BRICK_SHIFT;
const BRICK_WORDS: usize = (BRICK * BRICK * BRICK / 4) as usize;
/** Sites a brick holds: a word of light each, lit. */
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
 * A cell at the ball's edge reaches past it, where the window's entries
 * count as their bricks last did, and its entry in the torus is also that
 * of the cell a window away, whose bricks only add to its count. So a
 * cell counted hollow holds no voxel the ball holds, as a ray stepping
 * voxel by voxel would find: and the ball holds all the rays read (reach).
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
 * Per brick of `grid` with voxels, in its order in the grid's cells, the
 * cells of each scale around it (AROUND's bits) that its voxels meet,
 * SCALES words a brick: those its sites meet (MEETING).
 */
pub fn meets(grid: &fcc::Grid) -> Vec<u32> {
    let mut out = vec![0; grid.cells.len() / BRICK_SITES * SCALES];
    let count = grid.n.map(|v| (v + BRICK - 1) / BRICK);
    for (b, &first) in grid.bricks.iter().enumerate() {
        if first < 0 {
            continue;
        }
        let b = b as i32;
        // The brick, in the level's bricks (whose sites start where the grid's bricks do, modulo BRICK).
        let brick: [i32; 3] =
            std::array::from_fn(|d| grid.min[d].div_euclid(BRICK) + [b % count[0], b / count[0] % count[1], b / count[0] / count[1]][d]);
        for r in 0..BRICK * BRICK * BRICK {
            let p = [r % BRICK, r / BRICK % BRICK, r / BRICK / BRICK];
            let site: [i32; 3] = std::array::from_fn(|d| grid.min[d].rem_euclid(BRICK) + brick[d] * BRICK + p[d]);
            if (site[0] + site[1] + site[2]) & 1 != 0 || grid.cells[first as usize * BRICK_SITES + (r >> 1) as usize] == 0 {
                continue;
            }
            for (j, meeting) in MEETING.iter().enumerate() {
                let size = BRICK << (2 * j);
                let [x, y, z] = std::array::from_fn(|d| brick[d].rem_euclid(1 << (2 * j)) * BRICK + p[d]);
                out[first as usize * SCALES + j] |= meeting[((z * size + y) * size + x) as usize];
            }
        }
    }
    out
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

/** The brick whose sites start at `first` in `grid`'s cells (in bricks) as the GPU takes it, BRICK_WORDS words: each site material | alpha << 8, two a word. */
fn words(grid: &fcc::Grid, first: usize) -> impl Iterator<Item = u32> + '_ {
    let site = |i: usize| match grid.cells[i] {
        0 => 0,
        m => u32::from(m) | u32::from(grid.alpha.get(i).copied().unwrap_or(fcc::ALPHA)) << 8,
    };
    (first * BRICK_SITES..(first + 1) * BRICK_SITES).step_by(2).map(move |i| site(i) | site(i + 1) << 16)
}

/**
 * Per site of `grid`, of `level`, in the order of its cells, the light
 * from its voxel that rays see (Light::seen) as an sRGB colour (fcc's
 * light::colour); 0 where it has none.
 */
pub fn lights(grid: &fcc::Grid, level: u32, light: &mut Light) -> Vec<u32> {
    let mut out = vec![0; grid.cells.len()];
    let count = grid.n.map(|v| (v + BRICK - 1) / BRICK);
    for (b, &first) in grid.bricks.iter().enumerate() {
        if first < 0 {
            continue;
        }
        let b = b as i32;
        let corner = [b % count[0], b / count[0] % count[1], b / count[0] / count[1]].map(|v| v * BRICK);
        for r in 0..BRICK * BRICK * BRICK {
            let [x, y, z] = [r % BRICK, r / BRICK % BRICK, r / BRICK / BRICK];
            let site: [i32; 3] = std::array::from_fn(|d| grid.min[d] + corner[d] + [x, y, z][d]);
            let i = first as usize * BRICK_SITES + (r >> 1) as usize;
            if (site[0] + site[1] + site[2]) & 1 == 0 && grid.cells[i] != 0 {
                out[i] = light.seen(level, site.map(i64::from)).map_or(0, colour);
            }
        }
    }
    out
}

/** The voxels engine: each frame, the tiles of the levels its rays read, as they are. */
pub struct Voxels;

impl Engine for Voxels {
    fn frame(&mut self, look: &Look, world: &World) -> Frame {
        let count = levels(look.far);
        let levels: Vec<_> = (0..count).map(|l| world.level(l).clone()).collect();
        let tiles: usize = levels.iter().map(|l| l.tiles.len()).sum();
        let lit = if look.sky.is_some() { Lit::Tiles } else { Lit::Not };
        let reach = std::array::from_fn(|l| (l < count).then_some(READ));
        Frame {
            picture: Box::new(Scene { levels, palette: world.palette.clone(), look: *look }),
            // Rays read all around the eye.
            wants: Wants { reach, lit, tiles: world.lacking(&reach, look.camera, lit) },
            busy: false,
            stats: format!("{tiles} tiles"),
        }
    }
}

/** What to draw: each level's tiles, the materials' colours or, lit, the voxels' light, as `look` sees them. */
struct Scene {
    levels: Vec<Arc<Level>>,
    palette: Arc<Vec<u32>>,
    look: Look,
}

impl Picture for Scene {
    fn draw(&self, easel: &mut Easel, encoder: &mut wgpu::CommandEncoder, pixels: [u32; 2]) {
        let lit = self.look.sky.is_some();
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
    /** 1 if voxels are coloured by their light, 0 by their material and the way their faces face. */
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
        let sky = look.sky.unwrap_or(1.0);
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
            lit: u32::from(look.sky.is_some()),
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
    filled: u32,
    _pad: [u32; 3],
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

/** A brick of a tile to upload: the slot it goes in, the tile, and its place in the tile's grid's cells. */
type Added = (u32, Arc<Tile>, usize);

/** A tile a window holds bricks of; per brick of its grid (in its cells, its buried ones one: worldgen's VoxelGrid::bury), its slot (EMPTY if none) and how many entries hold it. */
struct Held {
    tile: Arc<Tile>,
    slots: Vec<(i32, u32)>,
}

/**
 * A level's window as the GPU holds it: from which tiles, the ball of
 * bricks it holds of them, its table (per brick a slot, EMPTY or UNKNOWN:
 * every brick outside the ball), and, per cell of each scale, how many
 * bricks around it have voxels that meet it or are unknown. Rays read the
 * ball only (reach_of), so the counts are right for its bricks: one that
 * leaves it keeps counting as it did, till another takes its entry (and
 * empty air coming in where empty air was changes none). Bricks of a tile
 * that share their sites share a slot: the ground's rock, buried, a slot
 * a tile.
 */
struct Window {
    level: Option<Arc<Level>>,
    tiles: HashMap<TileId, Held>,
    ball: Option<Ball>,
    /** Sites of the level's bricks start at origin + BRICK · b. */
    origin: [i32; 3],
    table: Vec<i32>,
    /** Per entry of the table, the cells of each scale around its brick it counts in (AROUND's bits): those its voxels meet, all if UNKNOWN. */
    meets: Vec<[u32; SCALES]>,
    counts: Vec<u16>,
    /** Slots it holds. */
    bricks: usize,
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
            // Every brick around each cell unknown: 27 blocks of 64^j bricks.
            counts: (0..COUNTS).map(|i| (AROUND.count_ones() as u16) << (6 * (i / CELLS))).collect(),
            bricks: 0,
        }
    }

    /** Where brick `b` starts, and the tile it is of. */
    fn site(&self, b: [i32; 3]) -> ([i32; 3], TileId) {
        let site: [i32; 3] = std::array::from_fn(|d| self.origin[d] + b[d] * BRICK);
        (site, site.map(|v| v.div_euclid(TILE as i32)))
    }

    /** Sets brick `b`'s entry, and the cells around it that it counts in; returns the entry it had. */
    fn set(&mut self, b: [i32; 3], (entry, meets): (i32, [u32; SCALES])) -> i32 {
        let at = self::entry(b);
        let had = std::mem::replace(&mut self.meets[at], meets);
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

    /** Makes brick `b` UNKNOWN to rays, its slot freed once no entry holds it; the cells around it are as they were (to place it again, or out of the ball). */
    fn release(&mut self, b: [i32; 3], free: &mut Vec<u32>) {
        if std::mem::replace(&mut self.table[entry(b)], UNKNOWN) < 0 {
            return;
        }
        let ([x, y, z], id) = self.site(b);
        let held = self.tiles.get_mut(&id).expect("a slot's tile");
        let (slot, uses) = &mut held.slots[held.tile.grid.brick(x, y, z) as usize];
        *uses -= 1;
        if *uses == 0 {
            free.push(*slot as u32);
            *slot = EMPTY;
            self.bricks -= 1;
        }
    }

    /**
     * Sets brick `b` of the ball, unless set already, from its tile: EMPTY,
     * or the slot its sites are in, one from `free` if none is yet (to
     * upload, in `added`: as many as `needs` says); UNKNOWN if no tile is
     * held there.
     */
    fn place(&mut self, b: [i32; 3], free: &mut Vec<u32>, added: &mut Vec<Added>) {
        if self.table[entry(b)] != UNKNOWN {
            return;
        }
        let ([x, y, z], id) = self.site(b);
        let Some(held) = self.tiles.get_mut(&id) else {
            self.set(b, (UNKNOWN, [AROUND; SCALES]));
            return;
        };
        let Ok(first) = usize::try_from(held.tile.grid.brick(x, y, z)) else {
            self.set(b, (EMPTY, [0; SCALES]));
            return;
        };
        let (slot, uses) = &mut held.slots[first];
        if *slot < 0 {
            let new = free.pop().expect("slots for the bricks counted");
            *slot = new as i32;
            added.push((new, held.tile.clone(), first));
            self.bricks += 1;
        }
        *uses += 1;
        let entry = (*slot, std::array::from_fn(|j| held.tile.meets[first * SCALES + j]));
        self.set(b, entry);
    }

    /**
     * Moves the window to the ball of `radius` about `eye`'s brick of
     * `level`, and to `tiles` (if `lit`, those lit only): what tiles gone
     * (new ones in place of others too) or the ball left becomes UNKNOWN.
     * Returns the bricks to place: those of tiles gone or new, and those
     * the ball gained; None if neither the tiles nor the ball moved.
     */
    fn retarget(
        &mut self,
        level: usize,
        tiles: &Arc<Level>,
        lit: bool,
        eye: Vec3,
        radius: i32,
        free: &mut Vec<u32>,
    ) -> Option<Vec<[i32; 3]>> {
        let held = |tile: &Tile| !lit || tile.lights.is_some();
        // Where its bricks start, its tiles say; until it has any, it holds nothing and rays do not read it.
        let origin = match (tiles.tiles.values().next(), self.ball) {
            (Some(tile), _) => tile.grid.min.map(|v| v.rem_euclid(BRICK)),
            (None, Some(_)) => self.origin,
            (None, None) => return None,
        };
        let ball = Ball { middle: eye_brick(level, eye, origin), radius };
        if self.level.as_ref().is_some_and(|l| Arc::ptr_eq(l, tiles)) && self.ball == Some(ball) {
            return None;
        }
        assert!(self.ball.is_none() || self.origin == origin, "a level's bricks moved");
        self.origin = origin;
        let mut pending: Vec<[i32; 3]> = match self.ball {
            Some(old) => {
                let gone: Vec<TileId> = (self.tiles.iter())
                    .filter(|&(id, h)| tiles.tiles.get(id).is_none_or(|t| !held(t) || !Arc::ptr_eq(t, &h.tile)))
                    .map(|(id, _)| *id)
                    .collect();
                for b in in_ball(old.bounds(), old, Some(ball)) {
                    self.release(b, free);
                }
                let mut pending: Vec<_> = in_ball(ball.bounds(), ball, Some(old)).collect();
                for id in gone {
                    for b in in_ball(tile_bricks(id, origin), ball, None) {
                        self.release(b, free);
                        pending.push(b);
                    }
                    self.tiles.remove(&id);
                }
                pending
            }
            None => in_ball(ball.bounds(), ball, None).collect(),
        };
        for (id, tile) in tiles.tiles.iter().filter(|(_, t)| held(t)) {
            assert!((0..3).all(|d| tile.grid.min[d].rem_euclid(BRICK) == origin[d]), "a tile off its level's bricks");
            if !self.tiles.contains_key(id) {
                self.tiles.insert(*id, Held { tile: tile.clone(), slots: vec![(EMPTY, 0); tile.grid.cells.len() / BRICK_SITES] });
                pending.extend(in_ball(tile_bricks(*id, origin), ball, None));
            }
        }
        (self.level, self.ball) = (Some(tiles.clone()), Some(ball));
        Some(pending)
    }

    /** Its ball of `radius` instead, what it leaves UNKNOWN; and of `pending`, those still in it. */
    fn resize(&mut self, radius: i32, pending: &mut Vec<[i32; 3]>, free: &mut Vec<u32>) {
        let Some(old) = self.ball else { return };
        let ball = Ball { radius, ..old };
        for b in in_ball(old.bounds(), old, Some(ball)) {
            self.release(b, free);
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

    /** The slots placing `bricks` would take from the pool. */
    fn needs(&self, bricks: &[[i32; 3]]) -> usize {
        let mut new = HashSet::new();
        for &b in bricks {
            let ([x, y, z], id) = self.site(b);
            let Some(held) = self.tiles.get(&id) else { continue };
            if let Ok(first) = usize::try_from(held.tile.grid.brick(x, y, z))
                && held.slots[first].0 < 0
            {
                new.insert((id, first));
            }
        }
        new.len()
    }

    /** As voxels.wgsl reads it, the level drawn as far as `far` (world units) or its ball holds, unless it holds nothing yet: rays read none past 0. */
    fn gpu(&self, level: usize, far: f64) -> GpuLevel {
        let corner = self.ball.map_or([0; 3], |b| b.middle.map(|v| v - WINDOW / 2));
        GpuLevel {
            min: std::array::from_fn(|d| self.origin[d] + corner[d] * BRICK),
            offset: (level * TABLE) as u32,
            corner,
            reach: self.ball.map_or(0.0, |b| reach(level, b.radius, far) as f32),
            filled: (level * COUNTS) as u32,
            _pad: [0; 3],
        }
    }
}

/**
 * The levels' windows and the pool of slots they share. The windows are
 * balls of one radius, each in its own level's bricks, as large as the
 * pool holds, up to RADIUS: short of slots, all shrink a brick, each level
 * read as much less far, its rays taking the next coarser one sooner. With
 * room to spare (an eighth of the pool), they grow a brick a sync, if
 * the bricks that gains fit.
 */
struct Windows {
    windows: Vec<Window>,
    free: Vec<u32>,
    radius: i32,
    /** Slots free that growing takes, as last found (0: to find). */
    growing: usize,
    margin: usize,
    /** The tiles of a level not read. */
    none: Arc<Level>,
}

impl Windows {
    fn new(pool: usize) -> Windows {
        // Balls of radius 0, of 27 bricks each, always fit.
        assert!(pool >= GRID_LEVELS * 27, "a pool of {pool} slots");
        Windows {
            windows: (0..GRID_LEVELS).map(|_| Window::new()).collect(),
            free: (0..pool as u32).rev().collect(),
            radius: RADIUS,
            growing: 0,
            margin: pool / 8,
            none: Arc::new(Level::new(HashMap::new())),
        }
    }

    /** The slots placing each window's `pending` bricks would take. */
    fn needs(&self, pending: &[Vec<[i32; 3]>]) -> usize {
        self.windows.iter().zip(pending).map(|(w, bricks)| w.needs(bricks)).sum()
    }

    /** Places each window's `pending` bricks, which fit. */
    fn place(&mut self, pending: Vec<Vec<[i32; 3]>>, added: &mut Vec<Added>) {
        for (window, bricks) in self.windows.iter_mut().zip(pending) {
            for b in bricks {
                window.place(b, &mut self.free, added);
            }
        }
    }

    /**
     * Brings each level's window to its tiles in `levels` (if `lit`, those
     * lit only; none past them) around `eye`, first deciding the radius,
     * then placing what that holds. Returns the bricks to upload (every
     * slot freed before any is taken: each once) and which windows changed.
     */
    fn sync(&mut self, levels: &[Arc<Level>], lit: bool, eye: Vec3) -> (Vec<Added>, [bool; GRID_LEVELS]) {
        let free = &mut self.free;
        let pending: Vec<Option<Vec<[i32; 3]>>> = (self.windows.iter_mut().enumerate())
            .map(|(level, w)| w.retarget(level, levels.get(level).unwrap_or(&self.none), lit, eye, self.radius, free))
            .collect();
        let mut changed = std::array::from_fn(|level| pending[level].is_some());
        let mut pending: Vec<Vec<[i32; 3]>> = pending.into_iter().map(Option::unwrap_or_default).collect();
        // Short of slots: every window a brick less, till what is to place fits.
        while self.needs(&pending) > self.free.len() {
            self.radius -= 1;
            for (window, bricks) in self.windows.iter_mut().zip(&mut pending) {
                window.resize(self.radius, bricks, &mut self.free);
            }
            (self.growing, changed) = (0, [true; GRID_LEVELS]);
        }
        let mut added = Vec::new();
        self.place(pending, &mut added);
        // Room to spare: every window a brick more, if what that gains fits with room still.
        if self.radius < RADIUS && self.free.len() >= self.growing.max(self.margin) {
            let radius = self.radius + 1;
            let shells: Vec<_> = self.windows.iter().map(|w| w.shell(radius)).collect();
            let needs = self.needs(&shells);
            if needs + self.margin <= self.free.len() {
                for window in &mut self.windows {
                    window.resize(radius, &mut Vec::new(), &mut self.free);
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

/** The voxels on a device, drawn into a picture that the window or a shot shows. */
pub struct Marcher {
    /** Globals, levels, tables, the pool, the palette, the eye, cells' counts, and the pool's bricks' light (none unlit). */
    buffers: [wgpu::Buffer; 8],
    layouts: [wgpu::BindGroupLayout; 3],
    pipelines: [wgpu::RenderPipeline; 3],
    windows: Windows,
    palette: Option<Arc<Vec<u32>>>,
    targets: Option<Targets>,
    /** Whether it draws lit scenes (Look's `sky`). */
    pub lit: bool,
}

/** voxels.wgsl, with the constants it shares with this file. */
fn shader() -> String {
    format!(
        "const WINDOW = {WINDOW};\nconst CELLS = {CELLS}u;\nconst BRICK = {BRICK};\nconst BRICK_SHIFT = {BRICK_SHIFT}u;\nconst BRICK_WORDS = {BRICK_WORDS}u;\nconst BRICK_SITES = {BRICK_SITES}u;\nconst FACTOR = {FACTOR}u;\nconst EMPTY = {EMPTY};\nconst UNKNOWN = {UNKNOWN};\n{}",
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
        let largest = device.limits().max_storage_buffer_binding_size / (BRICK_WORDS.max(light_words) as u64 * 4);
        let pool = (POOL_BYTES / ((BRICK_WORDS + light_words) as u64 * 4)).min(largest);
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
            buffer("pool", pool as usize * BRICK_WORDS * 4, storage),
            buffer("palette", 256 * 4, storage),
            buffer("eye", 16, uniform),
            buffer("filled", GRID_LEVELS * COUNTS * 2, storage),
            buffer("lights", (pool as usize * light_words * 4).max(4), storage),
        ];
        let entry = |binding: u32, ty| wgpu::BindGroupLayoutEntry { binding, visibility: wgpu::ShaderStages::FRAGMENT, ty, count: None };
        let buffer_type = |ty| wgpu::BindingType::Buffer { ty, has_dynamic_offset: false, min_binding_size: None };
        let read = buffer_type(wgpu::BufferBindingType::Storage { read_only: true });
        let uniform_type = buffer_type(wgpu::BufferBindingType::Uniform);
        let texture =
            |sample_type| wgpu::BindingType::Texture { sample_type, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false };
        let uint = texture(wgpu::TextureSampleType::Uint);
        let all = [
            entry(0, uniform_type),
            entry(1, read),
            entry(2, read),
            entry(3, read),
            entry(4, read),
            entry(5, uniform_type),
            entry(6, read),
            entry(7, read),
            entry(8, uint),
            entry(9, uint),
        ];
        let shown = [entry(10, texture(wgpu::TextureSampleType::Float { filterable: false }))];
        // The samples' pass draws into the textures the resolving one reads, which draws the picture shown.
        let layouts = [&all[..8], &all[..], &shown[..]]
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
        Marcher { buffers, layouts, pipelines, windows: Windows::new(pool as usize), palette: None, targets: None, lit }
    }

    /** Brings the voxels on the GPU up to date with `scene`, then draws it into a picture of `pixels`: the samples, then each pixel resolving its own. */
    fn draw(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, encoder: &mut wgpu::CommandEncoder, scene: &Scene, pixels: [u32; 2]) {
        let look = &scene.look;
        if !self.palette.as_ref().is_some_and(|p| Arc::ptr_eq(p, &scene.palette)) {
            let mut palette = scene.palette.to_vec();
            palette.resize(256, 0);
            queue.write_buffer(&self.buffers[4], 0, bytemuck::cast_slice(&palette));
            self.palette = Some(scene.palette.clone());
        }
        let mut levels = [GpuLevel::default(); GRID_LEVELS];
        let (added, changed) = self.windows.sync(&scene.levels, self.lit, look.camera);
        upload(queue, &self.buffers[3], &added, BRICK_WORDS, |tile, first| words(&tile.grid, first));
        if self.lit {
            upload(queue, &self.buffers[7], &added, BRICK_SITES, |tile, first| {
                tile.lights.as_ref().expect("a tile held lit")[first * BRICK_SITES..][..BRICK_SITES].iter().copied()
            });
        }
        for (level, gpu) in levels.iter_mut().enumerate() {
            let window = &self.windows.windows[level];
            if changed[level] {
                queue.write_buffer(&self.buffers[2], (level * TABLE * 4) as u64, bytemuck::cast_slice(&window.table));
                queue.write_buffer(&self.buffers[6], (level * COUNTS * 2) as u64, bytemuck::cast_slice(&window.counts));
            }
            if level < scene.levels.len() {
                *gpu = window.gpu(level, look.far);
            }
        }
        queue.write_buffer(&self.buffers[1], 0, bytemuck::cast_slice(&levels));
        queue.write_buffer(&self.buffers[0], 0, bytemuck::bytes_of(&Globals::of(look, pixels)));
        let eye = look.camera.map(|v| v as f32);
        queue.write_buffer(&self.buffers[5], 0, bytemuck::cast_slice(&[eye[0], eye[1], eye[2], 0.0]));
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
        let mut entries: Vec<_> = self
            .buffers
            .iter()
            .enumerate()
            .map(|(k, b)| wgpu::BindGroupEntry { binding: k as u32, resource: b.as_entire_binding() })
            .collect();
        for (k, view) in views.iter().enumerate() {
            entries.push(wgpu::BindGroupEntry { binding: 8 + k as u32, resource: wgpu::BindingResource::TextureView(view) });
        }
        let shown = [wgpu::BindGroupEntry { binding: 10, resource: wgpu::BindingResource::TextureView(&picture_view) }];
        let group = |layout, entries: &[wgpu::BindGroupEntry]| {
            device.create_bind_group(&wgpu::BindGroupDescriptor { label: None, layout, entries })
        };
        let groups = [group(&self.layouts[0], &entries[..8]), group(&self.layouts[1], &entries), group(&self.layouts[2], &shown)];
        Targets { pixels, samples, picture, groups }
    }
}

/** Copies `added` bricks into `pool`, `per` words a brick as `words` gives them, in runs of slots one after another. */
fn upload<'a, W: IntoIterator<Item = u32>>(
    queue: &wgpu::Queue,
    pool: &wgpu::Buffer,
    added: &'a [Added],
    per: usize,
    words: impl Fn(&'a Tile, usize) -> W,
) {
    let mut added: Vec<_> = added.iter().map(|(slot, tile, first)| (*slot, tile.as_ref(), *first)).collect();
    added.sort_unstable_by_key(|a| a.0);
    let mut run: Vec<u32> = Vec::new();
    let write = |run: &[u32], end: u32| {
        let start = end as usize - run.len() / per;
        queue.write_buffer(pool, (start * per * 4) as u64, bytemuck::cast_slice(run));
    };
    for (k, &(slot, tile, first)) in added.iter().enumerate() {
        if k > 0 && slot != added[k - 1].0 + 1 {
            write(&run, added[k - 1].0 + 1);
            run.clear();
        }
        run.extend(words(tile, first));
    }
    if let Some(last) = added.last() {
        write(&run, last.0 + 1);
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

    /** Bricks held, of the pool's slots, and those left out for want of room. */
    fn describe(&self, _: &Easel) -> String {
        let Windows { windows, free, radius, .. } = &self.windows;
        let held: usize = windows.iter().map(|w| w.bricks).sum();
        format!(": {held} bricks of {} on the GPU, windows {radius} bricks of {RADIUS} round", held + free.len())
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, HashSet};

    use super::*;

    /** The tiles of level 0 of the forest of seed 1 around `eye`. */
    fn forest_tiles(eye: Vec3) -> Arc<Level> {
        let mut world = World::new(worlds::by_name("forest").expect("forest").make, 1, None, 4);
        let wants = Wants { reach: std::array::from_fn(|l| (l == 0).then_some(fcc::GRID_REACH)), lit: Lit::Not, tiles: Vec::new() };
        loop {
            world.update(eye, &wants);
            if !world.busy() {
                return world.level(0).clone();
            }
            world.wait();
        }
    }

    /** Syncs `windows` to level 0's `tiles` around `eye`, keeping what they upload in `slots`; whether any window changed. */
    fn sync(windows: &mut Windows, tiles: &Arc<Level>, eye: Vec3, slots: &mut HashMap<u32, Vec<u32>>) -> bool {
        let (added, changed) = windows.sync(std::slice::from_ref(tiles), false, eye);
        for (slot, tile, first) in added {
            slots.insert(slot, words(&tile.grid, first).collect());
        }
        changed.contains(&true)
    }

    /**
     * What level 0's window holds, per entry of its ball: EMPTY, UNKNOWN or
     * a slot's words, as `slots` say they were uploaded, and the cells it
     * counts in; and the slots it takes. Its counts are as its entries say.
     */
    type Holds = (Vec<Option<(Result<Vec<u32>, i32>, [u32; SCALES])>>, usize);

    fn holds(windows: &Windows, slots: &HashMap<u32, Vec<u32>>) -> Holds {
        let w = &windows.windows[0];
        let mut counts = vec![0u16; COUNTS];
        for b in window_bricks([WINDOW / 2; 3]) {
            for j in 0..SCALES {
                for e in (0..27).filter(|e| w.meets[entry(b)][j] >> e & 1 != 0) {
                    let cell = around(b, j, e);
                    if (cell[0] + cell[1] + cell[2]) & 1 == 0 {
                        counts[cell_entry(cell, j)] += 1;
                    }
                }
            }
        }
        assert!(w.counts == counts, "counts as the entries say");
        let mut held = vec![None; TABLE];
        for b in w.ball.map(|ball| in_ball(ball.bounds(), ball, None)).into_iter().flatten() {
            let e = w.table[entry(b)];
            held[entry(b)] = Some((if e >= 0 { Ok(slots[&(e as u32)].clone()) } else { Err(e) }, w.meets[entry(b)]));
        }
        (held, w.bricks)
    }

    /** What windows made anew, of `radius` and room for all, hold of `tiles` around `eye`. */
    fn anew(radius: i32, tiles: &Arc<Level>, eye: Vec3) -> Holds {
        let (mut windows, mut slots) = (Windows::new(1 << 20), HashMap::new());
        (windows.radius, windows.growing) = (radius, usize::MAX);
        sync(&mut windows, tiles, eye, &mut slots);
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
        let (mut windows, mut slots) = (Windows::new(1 << 20), HashMap::new());
        // No tiles yet: nothing held, nothing read.
        assert!(!sync(&mut windows, &Arc::new(Level::new(HashMap::new())), eye, &mut slots));
        assert_eq!(windows.windows[0].gpu(0, f64::MAX).reach, 0.0);
        for (tiles, eye) in &steps {
            assert!(sync(&mut windows, tiles, *eye, &mut slots), "a change");
            assert_eq!(windows.radius, RADIUS, "room for all");
            assert!(holds(&windows, &slots) == anew(RADIUS, tiles, *eye), "at {eye:?}");
            assert!(!sync(&mut windows, tiles, *eye, &mut slots), "nothing moved");
        }
        // Lit, tiles not lit yet are not held: rays take the next level there.
        let mut lit = Windows::new(1 << 20);
        lit.sync(std::slice::from_ref(&all), true, eye);
        assert!(lit.windows[0].table.iter().all(|&e| e == UNKNOWN));
    }

    #[test]
    fn windows_short_of_slots_hold_as_large_a_ball_as_fits() {
        let eye = [10.0, 30.0, -20.0];
        let tiles = forest_tiles(eye);
        let (all, wanted) = anew(RADIUS, &tiles, eye);
        // Buried bricks of a tile share a slot: fewer slots than bricks held.
        let entries = all.iter().flatten().filter(|(e, _)| e.is_ok()).count();
        assert!(wanted < entries, "{wanted} slots for {entries} bricks");
        let pool = wanted / 3;
        let (mut windows, mut slots) = (Windows::new(pool), HashMap::new());
        for eye in [eye, [13.0, 31.0, -9.0], [21.0, 33.0, -3.0], eye] {
            sync(&mut windows, &tiles, eye, &mut slots);
            let w = &windows.windows[0];
            let ball = w.ball.expect("a ball");
            assert!(ball.radius == windows.radius && windows.radius < RADIUS && w.bricks <= pool, "at {eye:?}");
            // Every brick of a tile made in the ball held, every other unknown.
            for b in window_bricks(ball.middle) {
                let inside = ball.contains(b) && tiles.tiles.contains_key(&w.site(b).1);
                assert_eq!(w.table[entry(b)] != UNKNOWN, inside, "brick {b:?}, at {eye:?}");
            }
            assert!(holds(&windows, &slots) == anew(windows.radius, &tiles, eye), "at {eye:?}");
        }
        // More room: they grow, a brick at a sync, to all.
        windows.free.extend(pool as u32..2 * wanted as u32);
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
        let mut windows = Windows::new(1 << 20);
        sync(&mut windows, &tiles, eye, &mut HashMap::new());
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
            let tiles: HashSet<_> = world.lacking(&reach, eye, Lit::Not).into_iter().map(|(_, id)| id).collect();
            for radius in [0, 1, 13, RADIUS] {
                let ball = Ball { middle: eye_brick(0, eye, [0; 3]), radius };
                for way in ways.clone() {
                    let length = way.iter().map(|v| v * v).sum::<f64>().sqrt();
                    let site = worldgen::picture::nearest_site(std::array::from_fn(|d| eye[d] + way[d] / length * reach_of(radius)));
                    for s in std::iter::once([0; 3]).chain(fcc::VOXEL_FACES).map(|n| std::array::from_fn(|d| site[d] as i32 + n[d])) {
                        let b = s.map(|v: i32| v.div_euclid(BRICK));
                        assert!(ball.contains(b), "eye {eye:?}: site {s:?} out of the ball of {radius}");
                        assert!(tiles.contains(&s.map(|v| v.div_euclid(TILE as i32))), "eye {eye:?}: site {s:?} in no tile wanted");
                    }
                }
            }
        }
    }

    #[test]
    fn no_voxel_the_window_holds_meets_a_cell_counted_hollow() {
        let eye = [10.0, 30.0, -20.0];
        let tiles = forest_tiles(eye);
        // Moved, so that bricks of both sides of the torus have been in it.
        let mut windows = Windows::new(1 << 20);
        sync(&mut windows, &tiles, [eye[0] - 300.0, eye[1], eye[2] + 200.0], &mut HashMap::new());
        sync(&mut windows, &tiles, eye, &mut HashMap::new());
        let window = &windows.windows[0];
        let (ball, origin) = (window.ball.expect("a ball"), window.origin);
        let (lo, hi) = ball.bounds();
        let (lo, hi) = (lo.map(|v| v * BRICK), hi.map(|v| v * BRICK));
        // Every voxel of the ball, and every cell of each scale it meets, at the ball's edge too.
        let mut voxels = 0;
        for z in lo[2]..hi[2] {
            for y in lo[1]..hi[1] {
                for x in (lo[0] + (lo[0] + y + z + origin.iter().sum::<i32>()).rem_euclid(2)..hi[0]).step_by(2) {
                    let p = [x + origin[0], y + origin[1], z + origin[2]];
                    if !ball.contains([x, y, z].map(|v| v.div_euclid(BRICK))) || tiles.grids.at(p[0], p[1], p[2]).is_none_or(|v| v.0 == 0) {
                        continue;
                    }
                    voxels += 1;
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
}
