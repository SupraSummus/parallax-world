//! Light on the lattices 2^k·FCC: a prototype, for look's pictures and
//! native's `--light`, whose bakers ask for the light from the faces of
//! the voxels they show (fcc's light.rs).
//!
//! Light is kept at the sites it gets through, not in whole voxels. It
//! crosses a site from face to face: a line through two faces' middles
//! goes on through the middles of the next sites' faces, by two steps in
//! turn, towards the neighbours across them (`line`). Of a voxel's 12
//! faces each sees the other 11 about alike (a rhombic dodecahedron's
//! form factors are all within a tenth of 1/11), so the 132 lines through
//! a site weigh alike, and the light on a face of a voxel is the mean of
//! the 11 lines that leave the site it faces through it (`lands`): no
//! cosines. So a site keeps its light by the face of the next it lands
//! on, 11 lines a face (`Leaving`), each found only where asked: a face
//! lit, a reflection, a finer level's line read past its steps, each want
//! one face's lines.
//!
//! Along a line, light comes from the sites of its level for a few of
//! its steps, two a period (Shared's, STEPS unless set), then from the
//! next coarser level's same line, where that level has room for light
//! (`past`); or from more steps, where the coarser fills what the finer
//! leaves empty. Each site met sends what it glows, and of what it stops
//! (its alpha, a medium's by channel) its colour times the light that
//! lands on its face turned back; the rest of what comes from past it
//! gets through. So what comes from far comes from coarser levels, the
//! coarsest's from the sky, once a line has gone as far as the world is
//! high (`top_steps`). Sites that reflect less than the tolerance reflect
//! together, as the next one met that brings them over it, and nothing
//! comes from past where less than the tolerance gets through (`along`):
//! the light on few sites is asked for, where many stop little.
//!
//! The light on a site is what the sites near it reflect of the light on
//! them, so it settles round by round: round 0 has what glows, the sky and
//! the coarser level's light, each next round what the sites met reflect
//! of theirs in the round before, so round n has light bounced n times at
//! most; found once what it lands on a face with changes by less than a
//! tolerance (Shared's, TOLERANCE unless set), in a round or two in the
//! open, or at ROUNDS, where light bounces back and forth. Of each round a
//! site keeps what each face's lines land with (`round`): what the sites
//! met past it reflect, and what is to settle.
//!
//! What is found at a site is a function of the world alone, so the Lights
//! of all threads keep it together (Shared), each site's found once: no
//! level's sites apart from another's, and each value of a site kept once
//! found, by whichever finds it first, so that no Light waits for another
//! to read it, and two finding it at once do no harm. They read the
//! world's tiles (worldgen's tiles.rs), as hosts draw them: lit, a tile's sites are
//! made once, for what draws it and what lights it (Shared::make_tiles).
//!
//! Light sees each site as it is (VoxelGrid::light), as hosts take it: what
//! its pieces and its medium stop of its rays, a share. So the coarse
//! sites around a small lamp glow as bright as the lamp, on average over
//! their faces, and those small things fill in part stop as much as the
//! things do. What fills a site in part, a line through its middle meets
//! more often than its share says (fcc's CROSSING, `crossed`).

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use fcc::light::{FaceLight, linear_to_srgb, srgb_to_linear};
use fcc::ray::{CROSSING, FACES, nearest_site, over};
use fcc::{GRID_LEVELS, Look, opposite};

use worldgen::World;
use worldgen::grid::{ALPHA, BRICK, COLOUR_ONE, GLOW_ONE, Grid, VoxelGrid};
use worldgen::picture::Picture;
use worldgen::tiles::{TILE, made_with, make_sites, parted};

type Rgb = [f32; 3];

/** A voxel's faces, and the lines through a site: one from each face to each but the one opposite (`line`). */
const F: usize = FACES.len();
const N: usize = F * G;
/** The lines that leave a site by one face: all but the one that would turn back. */
const G: usize = F - 1;

/**
 * Line `l`'s two steps, as faces: light comes along it from the neighbour
 * across the first, where it came from the one across the second, and so
 * on in turn, and goes on towards the neighbour opposite the second.
 * Lines are in the order of that second face, 11 each: the face of the
 * site past that the light lands on (`lands`).
 */
const fn line(l: usize) -> [usize; 2] {
    let (b, a) = (l / G, l % G);
    [if a < opposite(b) { a } else { a + 1 }, b]
}

/** Periods of a line's two steps before the coarser level's light may take over, unless set (Shared::stepping), and at most, where none does: in a closed place, or a world with no heights. */
const STEPS: usize = 1;
const MOST_STEPS: usize = 64;
/**
 * How little of the light on a site is not followed, unless set
 * (Shared::settling): its change from a round to the next, for it to be
 * found; what a site met reflects, and what gets through from past it
 * (`along`).
 */
const TOLERANCE: f32 = 1e-2;
/**
 * Rounds at most, so bounces: where most of the light reflects, spread
 * wide, it settles in tens of rounds, each reading further round
 * (docs/light.md).
 */
const ROUNDS: usize = 8;
/**
 * Sites begun between one forgetting and the next (Shared::begun): fewer
 * hold less, and under what a view reads at once, lighting slows by half
 * (docs/light.md).
 */
const KEPT_SITES: usize = 1 << 17;

/**
 * The tiles of a level made as one box (tiles' made_with): their sites,
 * generated once by whichever reads one first, a Light or a host (those
 * that want them meanwhile wait), and what the Lights found at them so
 * far, by tile, bottom up: nothing till light is found in it.
 */
struct Made {
    tiles: Vec<[i64; 3]>,
    sites: Arc<OnceLock<VoxelGrid>>,
    lit: Box<[OnceLock<Arc<Lit>>]>,
}

/**
 * What the Lights found in a tile: by brick (z-major, then y, then x) and
 * site in it, nothing till begun; and
 * whether a Light read it since the tiles not read were last forgotten
 * (Shared::begun).
 */
struct Lit {
    read: AtomicBool,
    bricks: [OnceLock<Box<Sites>>; TILE_BRICKS],
}

type Sites = [OnceLock<Box<Site>>; BRICK_SITES];

/** Bricks a tile is a side, and in all. */
const SIDE: i64 = TILE / BRICK;
const TILE_BRICKS: usize = (SIDE * SIDE * SIDE) as usize;
/** Sites a brick holds: every other one of its BRICK³ points. */
const BRICK_SITES: usize = (BRICK * BRICK * BRICK / 2) as usize;

impl Made {
    /** The same tiles and sites, of what was found only the tiles' `keep` says to. */
    fn keeping(&self, keep: impl Fn(&Lit) -> bool) -> Made {
        let kept = |lit: &OnceLock<Arc<Lit>>| lit.get().filter(|lit| keep(lit)).cloned().map_or_else(OnceLock::new, OnceLock::from);
        Made { tiles: self.tiles.clone(), sites: self.sites.clone(), lit: self.lit.iter().map(kept).collect() }
    }

    /** What is kept of `site`, one of its tiles': its tile marked read. */
    fn site(&self, site: [i64; 3]) -> &OnceLock<Box<Site>> {
        let tile = (tile_of(site[1]) - self.tiles[0][1]) as usize;
        let lit =
            self.lit[tile].get_or_init(|| Arc::new(Lit { read: AtomicBool::new(false), bricks: std::array::from_fn(|_| OnceLock::new()) }));
        // Written only when it changes: Lights reading the same tile do not take the mark from each other.
        if !lit.read.load(Ordering::Relaxed) {
            lit.read.store(true, Ordering::Relaxed);
        }
        let [x, y, z] = site.map(|v| (v & (TILE - 1)) / BRICK);
        let brick = lit.bricks[((z * SIDE + y) * SIDE + x) as usize].get_or_init(|| Box::new(std::array::from_fn(|_| OnceLock::new())));
        let [x, y, z] = site.map(|v| v & (BRICK - 1));
        &brick[(((z * BRICK + y) * BRICK + x) >> 1) as usize]
    }
}

/**
 * A light in a word, as GPUs' RGB9E5 (fcc's `rgb9e5`): 9 bits a channel
 * and an exponent they share, within a 512th of its brightest channel,
 * from 2⁻²⁴ (less is none) to 65408 (more is that). Most of what a site
 * keeps is this.
 */
type Packed = u32;

/**
 * `light` in three channels of `M` bits and an exponent of `E` bits they
 * share, each `m · 2^(e − B)`, rounded to the nearest: within 2^−M of its
 * brightest channel. What is too bright is the most there is; too dark
 * for the least exponent, it loses bits.
 */
fn shared<const M: u32, const E: u32, const B: i32>(light: Rgb) -> u32 {
    let most = light.iter().fold(0.0f32, |m, &c| m.max(c));
    if most <= 0.0 {
        return 0;
    }
    let top = (1 << E) - 1;
    // The exponent that puts the brightest channel in the top bit of its M: from its float's, floor(log2).
    let floor = ((most.to_bits() >> 23) & 255) as i32 - 127;
    let mut e = (floor + 1 - M as i32 + B).clamp(0, top);
    let scale = |e: i32| f32::from_bits(((B - e + 127) as u32) << 23);
    if e < top && (most * scale(e)).round() >= (1 << M) as f32 {
        e += 1;
    }
    let mantissa = |c: f32| ((c.max(0.0) * scale(e)).round() as u32).min((1 << M) - 1);
    let [r, g, b] = light.map(mantissa);
    (e as u32) << (3 * M) | b << (2 * M) | g << M | r
}

/** The light in a word of `shared`'s. */
fn unshared<const M: u32, const E: u32, const B: i32>(word: u32) -> Rgb {
    let mask = (1 << M) - 1;
    let step = f32::from_bits((((word >> (3 * M)) as i32 - B + 127) as u32) << 23);
    [0, M, 2 * M].map(|shift| ((word >> shift) & mask) as f32 * step)
}

/** `light` packed (Packed). */
fn pack(light: Rgb) -> Packed {
    shared::<9, 5, 24>(light)
}

fn unpack(packed: Packed) -> Rgb {
    unshared::<9, 5, 24>(packed)
}

/** What is kept of a site: by the face of the site past that light lands on, what is kept of the lines that do (Leaving), once begun. */
#[derive(Default)]
struct Site {
    faces: [OnceLock<Box<Leaving>>; F],
}

/**
 * What is kept of the lines that leave a site by a face, each once found:
 * the same whichever Light finds it. Packed, a third of what floats take.
 */
#[derive(Default)]
struct Leaving {
    /** What comes along them. */
    paths: OnceLock<Paths>,
    /** What they land with on the face past, in each round (`round`); 0 till found (`round_of`). */
    rounds: [AtomicU32; ROUNDS + 1],
    /** The light along each, settled. */
    found: OnceLock<[Packed; G]>,
}

/** Round `n` of `leaving`, if found: a light that packs to none is kept as the least there is (`keep`). */
fn round_of(leaving: &Leaving, n: usize) -> Option<Packed> {
    Some(leaving.rounds[n].load(Ordering::Relaxed)).filter(|&w| w != 0)
}

/**
 * A site met that reflects (Paths), in a word: the line, of a face's, in
 * 4 bits, its steps along it less one in 7, and the share of the light on
 * its face turned back that comes, of 6 bits a channel and an exponent of
 * 3 they share (`shared`): from 1 to 2⁻⁸, as what reflects less than the
 * tolerance is owed to the next (`along`).
 */
type Reflects = u32;

fn reflects(k: usize, step: usize, share: Rgb) -> Reflects {
    debug_assert!(k < 16 && (1..=128).contains(&step));
    (k as u32) << 28 | ((step - 1) as u32) << 21 | shared::<6, 3, 13>(share)
}

/** The line, the steps and the share of a Reflects. */
fn reflected_by(r: Reflects) -> (usize, usize, Rgb) {
    ((r >> 28) as usize, ((r >> 21) & 127) as usize + 1, unshared::<6, 3, 13>(r & 0x1f_ffff))
}

/** The tile that holds coordinate `v` of a site, by shifting: tiles and bricks are powers of 2 a side. */
fn tile_of(v: i64) -> i64 {
    const { assert!(TILE.count_ones() == 1 && BRICK.count_ones() == 1) };
    v >> TILE.trailing_zeros()
}

/** What is made, by level and the key of its first tile. */
type Boxes = Map<(usize, [i64; 3]), Arc<Made>>;

/**
 * What a Shared's Lights keep of what was made: lit anew (Shared::unlit),
 * a copy of what it was lit from, taken when first read, by a thread that
 * lights: a host's thread that draws takes no lock the Lights take.
 */
#[derive(Default)]
struct Kept {
    boxes: OnceLock<Mutex<Boxes>>,
    from: Mutex<Option<Arc<Kept>>>,
}

impl Kept {
    fn boxes(&self) -> &Mutex<Boxes> {
        self.boxes.get_or_init(|| {
            let from = self.from.lock().expect("from").take();
            let made = |from: Arc<Kept>| {
                let from = from.boxes().lock().expect("made");
                from.iter().map(|(key, made)| (*key, Arc::new(made.keeping(|_| false)))).collect()
            };
            Mutex::new(from.map(made).unwrap_or_default())
        })
    }
}

/**
 * What the Lights of one world under one brightness, and taking as many
 * steps along each level, share (a thread's each), and the host with
 * them: the world's tiles of every level, with all the Lights find at
 * their sites, so each tile is made once, and each site's light found
 * once, whichever thread lights beside it. Tiles are kept till the host
 * forgets them (`forget`); the light of a tile, while Lights read it (`begun`).
 */
#[derive(Clone)]
pub struct Shared {
    brightness: Brightness,
    steps: usize,
    tolerance: f32,
    made: Arc<Kept>,
    /** Sites whose light was begun, so far (`begun`). */
    begun: Arc<AtomicUsize>,
    /** Times something made was forgotten or replaced: a Light then lets go of what it read (Level::made). */
    forgotten: Arc<AtomicUsize>,
}

impl Shared {
    /** Nothing made yet, for a world under `brightness`. */
    pub fn new(brightness: Brightness) -> Shared {
        Shared { brightness, steps: STEPS, tolerance: TOLERANCE, made: Arc::default(), begun: Arc::default(), forgotten: Arc::default() }
    }

    pub fn brightness(&self) -> Brightness {
        self.brightness
    }

    /** The same world, none of it lit: its tiles' sites kept (Kept). */
    fn unlit(&self) -> Shared {
        let made = Kept { boxes: OnceLock::new(), from: Mutex::new(Some(self.made.clone())) };
        Shared { made: Arc::new(made), begun: Arc::default(), forgotten: Arc::default(), ..self.clone() }
    }

    /** The same world under `brightness`: its tiles' sites kept. */
    pub fn under(&self, brightness: Brightness) -> Shared {
        Shared { brightness, ..self.unlit() }
    }

    /** Periods of a line's two steps before the coarser level's light may take over. */
    pub fn steps(&self) -> usize {
        self.steps
    }

    /** The same world, a line taking `steps` periods of its two steps along each level before the coarser's: its tiles' sites kept. */
    pub fn stepping(&self, steps: usize) -> Shared {
        assert!((1..=MOST_STEPS).contains(&steps), "{steps} steps");
        Shared { steps, ..self.unlit() }
    }

    /** How little of the light on a site is not followed (TOLERANCE). */
    pub fn tolerance(&self) -> f32 {
        self.tolerance
    }

    /** The same world, less than `tolerance` of the light on a site not followed (TOLERANCE): its tiles' sites kept. */
    pub fn settling(&self, tolerance: f32) -> Shared {
        assert!(tolerance > 0.0, "tolerance {tolerance}");
        Shared { tolerance, ..self.unlit() }
    }

    /** What tile `key` of `level` of `world` is made in, begun if not yet. */
    fn made(&self, world: &World, level: u32, key: [i64; 3]) -> Arc<Made> {
        let tiles: Vec<[i64; 3]> = made_with(world, level, key).collect();
        let key = (level as usize, tiles[0]);
        self.made
            .boxes()
            .lock()
            .expect("made")
            .entry(key)
            .or_insert_with(|| Arc::new(Made { lit: tiles.iter().map(|_| OnceLock::new()).collect(), tiles, sites: Arc::default() }))
            .clone()
    }

    /**
     * tiles' make_tiles, their sites kept for the Lights, or theirs if a
     * Light made them: so what is drawn and lit is made once.
     */
    pub fn make_tiles(&self, world: &World, level: u32, key: [i64; 3]) -> Vec<([i64; 3], Grid)> {
        let made = self.made(world, level, key);
        parted(made.sites.get_or_init(|| make_sites(world, level, key)), world, level, key)
    }

    /**
     * Counts a site whose light is begun. Every KEPT_SITES, the light of
     * the tiles no Light read since the last time is forgotten, to find
     * again if asked: what is being lit, and what it reads, is read all
     * the while.
     */
    fn begun(&self) {
        if (self.begun.fetch_add(1, Ordering::Relaxed) + 1).is_multiple_of(KEPT_SITES) {
            // Aside, none waiting for all of them meanwhile: what was forgotten or made anew since is left as it is.
            let made: Vec<_> = self.made.boxes().lock().expect("made").iter().map(|(key, made)| (*key, made.clone())).collect();
            let kept: Vec<_> = made.iter().map(|(_, made)| Arc::new(made.keeping(|lit| lit.read.swap(false, Ordering::Relaxed)))).collect();
            // What they were is let go once none waits for the lock: after `now`, as `made` goes.
            let mut now = self.made.boxes().lock().expect("made");
            for ((key, was), kept) in made.iter().zip(kept) {
                if let Some(made) = now.get_mut(key).filter(|made| Arc::ptr_eq(made, was)) {
                    *made = kept;
                }
            }
            self.forgotten.fetch_add(1, Ordering::Relaxed);
        }
    }

    /**
     * Forgets what was made whose every tile `far` says is far, by level
     * and key, and its light with it, to make again if asked. Returns
     * what it forgot, if anything, for its caller to let go where it has
     * the time: freeing it is long.
     */
    pub fn forget(&self, far: impl Fn(usize, [i64; 3]) -> bool) -> Option<Box<dyn Send>> {
        let mut made = self.made.boxes().lock().expect("made");
        let gone: Vec<_> = made.extract_if(|&(level, _), made| made.tiles.iter().all(|&key| far(level, key))).collect();
        if gone.is_empty() {
            return None;
        }
        self.forgotten.fetch_add(1, Ordering::Relaxed);
        Some(Box::new(gone))
    }
}

/** How bright the sky and the sun are, as multiples of a clear day's, and glowing kinds, of a street lamp's. */
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Brightness {
    pub sky: f32,
    pub sun: f32,
    pub glow: f32,
}

impl Brightness {
    /** The radiance of what glows, as a multiple of its colour (linear). */
    pub fn glowing(&self) -> f32 {
        GLOW * self.glow
    }
}

/** A clear day's sky: its radiance (1 would light a surface facing up to its colour), and the sun's, high in the south-west. */
const SKY: Rgb = [0.55, 0.62, 0.75];
const SUN: Rgb = [2.6, 2.4, 2.1];
const SUN_DIRECTION: [i64; 3] = [-1, 1, 0];
/** Radiance of a glowing kind at glow 1, times its colour. */
const GLOW: f32 = 250.0;

type Map<K, V> = fcc::FastMap<K, V>;

/**
 * A level's heights, and what a Light read of it is made in, by tile
 * (where in `held`), with the tile read last: most reads are in it. Its
 * own, so that it asks the others for each once; let go once any of what
 * is shared was forgotten (Shared's `forgotten`, as it was when it held
 * nothing), not to hold that.
 */
struct Level {
    heights: (i64, i64),
    last: Option<([i64; 3], usize)>,
    read: Map<[i64; 3], usize>,
    held: Vec<Arc<Made>>,
    forgotten: usize,
}

impl Level {
    fn new(heights: (i64, i64)) -> Level {
        Level { heights, last: None, read: Map::default(), held: Vec::new(), forgotten: 0 }
    }

    /** What the tile of this level, `i`, of `world`, that holds `site` is made in, from those shared. */
    fn made(&mut self, world: &World, shared: &Shared, i: usize, site: [i64; 3]) -> &Made {
        let forgotten = shared.forgotten.load(Ordering::Relaxed);
        if forgotten != self.forgotten {
            *self = Level { forgotten, ..Level::new(self.heights) };
        }
        let key = site.map(tile_of);
        let at = match self.last {
            Some((k, at)) if k == key => at,
            _ => {
                let at = *self.read.entry(key).or_insert_with(|| {
                    self.held.push(shared.made(world, i as u32, key));
                    self.held.len() - 1
                });
                self.last = Some((key, at));
                at
            }
        };
        &self.held[at]
    }
}

/**
 * What comes along the lines that leave a site by a face and does not
 * change as its light settles (`along`): along each, all but reflected
 * light; the sites met that reflect, each with its line (of the face's),
 * how many steps along it, and the share of the light that lands on its
 * face turned back that comes, its own and that of the sites it reflects
 * for (`along`).
 */
struct Paths {
    comes: [Packed; G],
    reflects: Box<[Reflects]>,
}

pub struct Light {
    world: Arc<World>,
    shared: Shared,
    /** By level, up to a renderer's coarsest grid's (GRID_LEVELS). */
    levels: Vec<Level>,
    /** Room for the sites that reflect of the paths read (`paths`), one for each call deep: kept from one to the next. */
    rooms: Vec<Vec<Reflects>>,
    /** Per line, the sum of its two steps: where it comes from. */
    sums: [[i64; 3]; N],
    /** What was seen of sites lately (`see`, `open`): what a site is seen as is the world's alone, so never changed. */
    seen: Box<[SeenAt]>,
    /** The radiance of a glow of GLOW_ONE. */
    glow: f32,
    /** The sky's radiance per line (the sun's within it). */
    sky: [Rgb; N],
    /** What the sky looks like, where a picture's rays miss. */
    background: Rgb,
}

/** The coarser level's light around a site, as `along` takes it, read at once (`near`): per line leaving by a face, past its steps, if the coarser level has room for it there (`past`). */
type Near = [Option<Rgb>; G];

/** How a site is seen (`see`): of each channel what it stops, of ALPHA, and the colour of that; what it glows. */
type Seen = (([u32; 3], Rgb), Rgb);

/** A site seen (`seen_at`): where, of which level and from which face, and how. */
type SeenAt = (([i64; 3], u8), Seen);

/** Sites seen that a Light keeps (`seen`): those of a few thousand faces' lines. */
const SEEN: usize = 1 << 16;

/** Where in a Light's `seen` how `site` of level `i` is seen from face `from` (F: what it stops of all its rays, `open`) is kept, and its key there. */
fn seen_at(i: usize, site: [i64; 3], from: usize) -> (([i64; 3], u8), usize) {
    let [x, y, z] = site.map(|c| c as u64);
    let hash = (x.wrapping_mul(0x9e37_79b9_7f4a_7c15) ^ y.rotate_left(21) ^ z.rotate_left(42) ^ (i * F + from) as u64)
        .wrapping_mul(0xff51_afd7_ed55_8ccd);
    ((site, (i * (F + 1) + from) as u8), (hash >> (64 - SEEN.trailing_zeros())) as usize)
}

impl Light {
    /** The light of `world`, found only where asked, and kept in `shared`. */
    pub fn new(world: impl Into<Arc<World>>, shared: Shared) -> Light {
        let world = world.into();
        let Brightness { sky, sun, .. } = shared.brightness;
        let coarsest = GRID_LEVELS as u32 - 1;
        let levels = (0..=coarsest).map(|level| Level::new(world.heights_at(level))).collect();
        let sums: [[i64; 3]; N] = std::array::from_fn(|l| {
            let [a, b] = line(l);
            add(FACES[a], FACES[b])
        });
        let skies = std::array::from_fn(|l| {
            let s = sums[l];
            let open = [0.0, 0.5, 1.0][(s[1].signum() + 1) as usize];
            // The sun comes along the lines from it, one on each face it lights, so that the face is lit by its cosine,
            // as by a sun a twelfth of the sphere wide: a third of its radiance on the face towards it.
            let towards = cross_i(s, SUN_DIRECTION) == [0; 3] && dot_i(s, SUN_DIRECTION) > 0;
            let cosine = dot_i(FACES[l / G], SUN_DIRECTION) as f32 / 2.0;
            let sun = if towards { sun * G as f32 * cosine / 3.0 } else { 0.0 };
            std::array::from_fn(|c| open * SKY[c] * sky + SUN[c] * sun)
        });
        let background = SKY.map(|c| c * sky);
        // A world with no heights has no sky: what glows is all its light.
        let (skies, background) = if world.heights.is_some() { (skies, background) } else { ([[0.0; 3]; N], [0.0; 3]) };
        let glow = shared.brightness.glowing() / GLOW_ONE as f32;
        let seen = vec![(([0; 3], u8::MAX), (([0; 3], [0.0; 3]), [0.0; 3])); SEEN].into();
        Light { world, shared, levels, rooms: Vec::new(), sums, seen, glow, sky: skies, background }
    }

    /** Whether a whole voxel, one that lets nothing through, is at `site` of level `i`. */
    fn whole(&mut self, i: usize, site: [i64; 3]) -> bool {
        self.open(i, site) <= 0.0
    }

    /**
     * How light sees `site` of level `i` from direction `from`: how much of
     * each channel it stops (of ALPHA), its pieces and its medium of the
     * rest, and their colour; what it glows.
     */
    fn see(&mut self, i: usize, site: [i64; 3], from: usize) -> Seen {
        let (key, at) = seen_at(i, site, from);
        if self.seen[at].0 == key {
            return self.seen[at].1;
        }
        let glowing = self.glow;
        let seen = match self.hold(i, site) {
            None => (([0; 3], [0.0; 3]), [0.0; 3]),
            Some(([x, y, z], grid)) if from == F => (([grid.alpha(x, y, z) as u32; 3], [0.0; 3]), [0.0; 3]),
            Some(([x, y, z], grid)) => {
                let ((pieces, p), (medium, m), glow) = grid.light(x, y, z, from);
                // What they show comes premultiplied, of COLOUR_ONE: as a share of the light on what it stops.
                let stops = medium.map(|m| pieces as u32 + m as u32);
                let colour = std::array::from_fn(|c| {
                    let (alpha, shows) = (stops[c] as f32 / ALPHA as f32, (p[c] + m[c]) as f32 / COLOUR_ONE as f32);
                    if alpha > 0.0 { (shows / alpha).min(1.0) } else { 0.0 }
                });
                ((stops, colour), glow.map(|g| g as f32 * glowing))
            }
        };
        self.seen[at] = (key, seen);
        seen
    }

    /** What a line through the middle of `site` of level `i` meets of it, coming from face `from` (`see`, `crossed`). */
    fn meets(&mut self, i: usize, site: [i64; 3], from: usize) -> ((Rgb, Rgb), Rgb) {
        crossed(self.see(i, site, from))
    }

    /** How much of what comes through `site` of level `i` it lets through. */
    fn open(&mut self, i: usize, site: [i64; 3]) -> f32 {
        1.0 - self.see(i, site, F).0.0[0] as f32 / ALPHA as f32
    }

    /** The sites of level `i`'s tile that holds `site`, generated if not yet; none above or below every voxel. */
    fn hold(&mut self, i: usize, site: [i64; 3]) -> Option<([i64; 3], &VoxelGrid)> {
        let level = &mut self.levels[i];
        let heights = level.heights;
        if !(heights.0..heights.1).contains(&site[1]) {
            return None;
        }
        let world = &self.world;
        let sites = &level.made(world, &self.shared, i, site).sites;
        Some((site, sites.get_or_init(|| make_sites(world, i as u32, site.map(tile_of)))))
    }

    /**
     * What is kept of the lines that leave site `p` of level `i` by face
     * `f`, with its tile, read by `with`: nothing yet, if it is begun here
     * (Shared::begun).
     */
    fn kept<T>(&mut self, i: usize, p: [i64; 3], f: usize, with: impl FnOnce(&Leaving) -> T) -> T {
        let mut begun = false;
        let value = {
            let site = self.levels[i].made(&self.world, &self.shared, i, p).site(p).get_or_init(|| {
                begun = true;
                Box::default()
            });
            with(site.faces[f].get_or_init(Box::default))
        };
        if begun {
            self.shared.begun();
        }
        value
    }

    /**
     * The light along the lines that leave site `p` of level `i` by face
     * `f`, as it settles round by round (`round`): found once what they
     * land with changes by less than Shared's tolerance, in a round or two
     * where little reflects back, or after ROUNDS. Where nothing met
     * reflects, round 0 is all there is.
     */
    fn light(&mut self, i: usize, p: [i64; 3], f: usize) -> [Packed; G] {
        if let Some(found) = self.kept(i, p, f, |l| l.found.get().copied()) {
            return found;
        }
        let mut reflects = self.rooms.pop().unwrap_or_default();
        let comes = self.paths(i, p, f, &mut reflects);
        let (mut lines, mut rounds) = (comes.map(unpack), 0);
        if !reflects.is_empty() {
            // The lines of the round last found here, not read kept: the light, if that round settles it.
            let mut last = None;
            let mut on = self.round(i, p, f, 0);
            while rounds < ROUNDS {
                rounds += 1;
                let next = match self.kept(i, p, f, |l| round_of(l, rounds)) {
                    Some(next) => unpack(next),
                    None => {
                        let lines = self.coming(i, p, f, rounds, &comes, &reflects);
                        last = Some((rounds, lines));
                        self.keep(i, p, f, rounds, &lines)
                    }
                };
                let settled = change(on, next) <= self.shared.tolerance;
                on = next;
                if settled {
                    break;
                }
            }
            lines = match last {
                Some((n, lines)) if n == rounds => lines,
                _ => self.coming(i, p, f, rounds, &comes, &reflects),
            };
        }
        self.rooms.push(reflects);
        let found = lines.map(pack);
        self.kept(i, p, f, |l| *l.found.get_or_init(|| found))
    }

    /**
     * The light along the lines that leave site `p` of level `i` by face
     * `f`, whose paths these are (`paths`), in round `n`: what comes, and what the
     * sites met reflect of what landed on their faces turned back in the
     * round before; none in round 0.
     */
    fn coming(&mut self, i: usize, p: [i64; 3], f: usize, n: usize, comes: &[Packed; G], reflects: &[Reflects]) -> [Rgb; G] {
        let mut light = comes.map(unpack);
        if n > 0 {
            for &r in reflects {
                let (k, step, reflects) = reflected_by(r);
                let (before, g) = reflected(p, f * G + k, step);
                let on = match self.kept(i, before, g, |l| round_of(l, n - 1)) {
                    Some(on) => unpack(on),
                    None => self.round(i, before, g, n - 1),
                };
                (0..3).for_each(|c| light[k][c] += reflects[c] * on[c]);
            }
        }
        light
    }

    /**
     * What the lines that leave site `p` of level `i` by face `f` land with
     * in round `n` (`lands`, of `coming`): packed, the same to whoever
     * finds it and whoever reads it kept.
     */
    fn round(&mut self, i: usize, p: [i64; 3], f: usize, n: usize) -> Rgb {
        if let Some(on) = self.kept(i, p, f, |l| round_of(l, n)) {
            return unpack(on);
        }
        let mut reflects = self.rooms.pop().unwrap_or_default();
        let comes = self.paths(i, p, f, &mut reflects);
        let lines = self.coming(i, p, f, n, &comes, &reflects);
        self.rooms.push(reflects);
        self.keep(i, p, f, n, &lines)
    }

    /** What `lines`, those that leave site `p` of level `i` by face `f` in round `n`, land with (`lands`), kept. */
    fn keep(&mut self, i: usize, p: [i64; 3], f: usize, n: usize, lines: &[Rgb; G]) -> Rgb {
        let on = pack(self.lands(i, p, f, lines));
        self.kept(i, p, f, |l| l.rounds[n].store(on.max(1), Ordering::Relaxed));
        unpack(on)
    }

    /**
     * What the light along the lines that leave site `p` of level `i` by
     * face `f`, `lines`, lands with on face `f` of the site past: their
     * mean, as much as `p` lets through of each, and what it glows.
     */
    fn lands(&mut self, i: usize, p: [i64; 3], f: usize, lines: &[Rgb; G]) -> Rgb {
        let ((alpha, _), glow) = self.meets(i, p, opposite(f));
        let mut sum = [0.0f32; 3];
        lines.iter().for_each(|l| (0..3).for_each(|c| sum[c] += l[c]));
        std::array::from_fn(|c| glow[c] + (1.0 - alpha[c]) * sum[c] / G as f32)
    }

    /** What comes to site `p` of level `i` (Paths) along each line that leaves it by face `f`, its sites that reflect into `reflects`. */
    fn paths(&mut self, i: usize, p: [i64; 3], f: usize, reflects: &mut Vec<Reflects>) -> [Packed; G] {
        reflects.clear();
        let read = |l: &Leaving| {
            let paths = l.paths.get()?;
            reflects.extend_from_slice(&paths.reflects);
            Some(paths.comes)
        };
        if let Some(comes) = self.kept(i, p, f, read) {
            return comes;
        }
        let mut near = None;
        let comes = std::array::from_fn(|k| pack(self.along(i, p, f, k, &mut near, reflects)));
        let paths = || Paths { comes, reflects: reflects.as_slice().into() };
        self.kept(i, p, f, |l| {
            l.paths.get_or_init(paths);
        });
        comes
    }

    /**
     * The light coming to `p`, a site of level `i`, along line `k` of
     * those that leave it by face `f`, all but reflected light: from the
     * sites its steps meet, what each glows; past them, the coarser
     * level's light, from Shared's steps on, where it has room for it
     * (`past`; `near`, read when first wanted, has it past the steps). At
     * the coarsest level, from the next `top_steps`, and past them the
     * sky. The sites met that reflect go in `reflects` (Paths): of what
     * each stops, its colour. Each has its own light found, so one that
     * reflects under Shared's tolerance is owed to the next that, with it,
     * reflects over: that one reflects for both, as if lit alike. What is
     * still owed where the line ends is left out.
     */
    fn along(&mut self, i: usize, p: [i64; 3], f: usize, k: usize, near: &mut Option<Near>, reflects: &mut Vec<Reflects>) -> Rgb {
        let l = f * G + k;
        let (steps, up, heights, tolerance) = (line(l), self.sums[l][1], self.levels[i].heights, self.shared.tolerance);
        let (coarser, handing) = (i + 1 < self.levels.len(), 2 * self.shared.steps);
        // What has come so far, and how much of what comes from further on gets through.
        let (mut sum, mut through) = ([0.0f32; 3], [1.0f32; 3]);
        // What the sites met so far reflect, under the tolerance: none reflects for them yet.
        let mut owed = [0.0f32; 3];
        let mut q = p;
        let most = 2 * if coarser { MOST_STEPS } else { top_steps(heights) };
        let beyond = 'march: {
            for step in 1..=most {
                let face = steps[(step - 1) % 2];
                q = add(q, FACES[face]);
                let ((alpha, colour), glow) = self.meets(i, q, opposite(face));
                (0..3).for_each(|c| sum[c] += through[c] * glow[c]);
                (0..3).for_each(|c| owed[c] += through[c] * alpha[c] * colour[c]);
                if owed.iter().any(|&r| r > tolerance) {
                    reflects.push(self::reflects(k, step, std::mem::take(&mut owed)));
                }
                (0..3).for_each(|c| through[c] *= 1.0 - alpha[c]);
                // Less gets through than the tolerance, or nothing more is met: below every voxel and not going up, or above them and not going down.
                if through.iter().all(|&t| t < tolerance) || q[1] < heights.0 - 2 && up <= 0 {
                    break 'march [0.0; 3];
                }
                if q[1] > heights.1 + 1 && up >= 0 {
                    break 'march self.sky[l];
                }
                if coarser && step >= handing {
                    let past = if step == handing {
                        near.get_or_insert_with(|| self.near(i + 1, p, f))[k]
                    } else {
                        self.past(i + 1, handover(q, steps[step % 2]), l)
                    };
                    if let Some(past) = past {
                        break 'march past;
                    }
                }
            }
            if !coarser && up >= 0 { self.sky[l] } else { [0.0; 3] }
        };
        std::array::from_fn(|c| sum[c] + through[c] * beyond[c])
    }

    /**
     * Level `i`'s light along line `l` at `at` (`reads`), if it has room
     * for light there, its site nearest empty. Where it fills what the
     * finer level leaves empty (a room whose walls, grown thick, close
     * it), the open sites near are not what lies past there.
     */
    fn past(&mut self, i: usize, at: [f64; 3], l: usize) -> Option<Rgb> {
        let mut reads = reads(at).peekable();
        let &(nearest, _) = reads.peek().expect("the nearest site");
        if self.open(i, nearest) <= 0.0 {
            return None;
        }
        let (mut sum, mut total) = ([0.0f32; 3], 0.0f32);
        for (site, w) in reads {
            let open = self.open(i, site);
            if open > 0.0 {
                let light = unpack(self.light(i, site, l / G)[l % G]);
                total += w * open;
                (0..3).for_each(|c| sum[c] += w * open * light[c]);
            }
        }
        Some(sum.map(|c| c / total))
    }

    /** `along`'s readings of level `i` around `p`, a site of level `i − 1`, for the lines that leave it by face `f`, as `past` would give them, read at once from the sites around (`around`). */
    fn near(&mut self, i: usize, p: [i64; 3], f: usize) -> Near {
        let (class, base) = class(p);
        let (sites, nearest) = &around(self.shared.steps)[class][f];
        let (mut past, mut totals) = ([[0.0f32; 3]; G], [0.0f32; G]);
        for (offset, weights) in sites {
            let site = add(base, *offset);
            let open = self.open(i, site);
            if open <= 0.0 {
                continue;
            }
            let light = self.light(i, site, f);
            for k in (0..G).filter(|&k| weights[k] > 0.0) {
                let (w, light) = (weights[k] * open, unpack(light[k]));
                totals[k] += w;
                (0..3).for_each(|c| past[k][c] += w * light[c]);
            }
        }
        let mean = |sum: Rgb, total: f32| if total > 0.0 { sum.map(|c| c / total) } else { sum };
        std::array::from_fn(|k| (self.open(i, add(base, nearest[k])) > 0.0).then(|| mean(past[k], totals[k])))
    }

    /**
     * The light from each face of the voxel at `site` of `level`: its
     * colour times the light on it, and what it glows. A voxel with no
     * neighbour light gets through (shown where a slice cuts it) takes the
     * mean of its neighbours' that have one, or none.
     */
    pub fn faces(&mut self, level: u32, site: [i64; 3]) -> FaceLight {
        let i = level as usize;
        if let Some(lit) = self.lit(i, site) {
            return lit;
        }
        let mut lit = Vec::new();
        for n in FACES.map(|f| add(site, f)) {
            if self.whole(i, n) {
                lit.extend(self.lit(i, n));
            }
        }
        std::array::from_fn(|f| std::array::from_fn(|c| lit.iter().map(|l| l[f][c]).sum::<f32>() / lit.len().max(1) as f32))
    }

    /** What the voxel at `site` of `level` glows. */
    pub fn glow(&mut self, level: u32, site: [i64; 3]) -> Rgb {
        self.see(level as usize, site, 0).1
    }

    /** What lights the voxel at `site` of `level` to its colour at 1: the light on its faces light gets to, on average; none if it gets to none (a ray meets it only where levels meet). */
    pub fn on_seen(&mut self, level: u32, site: [i64; 3]) -> Option<Rgb> {
        mean(self.on(level as usize, site).iter().flatten())
    }

    /** As `on_seen`, dark if light gets to none of its faces. */
    pub fn on_voxel(&mut self, level: u32, site: [i64; 3]) -> Rgb {
        self.on_seen(level, site).unwrap_or_default()
    }

    /** The light on each face of the voxel at `site` of level `i`: what lands on it from the site it faces (`lands`); none where that is whole. */
    fn on(&mut self, i: usize, site: [i64; 3]) -> [Option<Rgb>; F] {
        std::array::from_fn(|f| {
            let q = add(site, FACES[f]);
            (!self.whole(i, q)).then(|| {
                let lines = self.light(i, q, f).map(unpack);
                self.lands(i, q, f, &lines)
            })
        })
    }

    /** The light from each face of the voxel at `site` of level `i`, one a whole voxel hides lit as the others are on average; none if all are hidden. */
    fn lit(&mut self, i: usize, site: [i64; 3]) -> Option<FaceLight> {
        let on = self.on(i, site);
        let mean = mean(on.iter().flatten())?;
        Some(std::array::from_fn(|f| {
            let ((_, colour), glow) = self.see(i, site, f);
            std::array::from_fn(|c| colour[c] * on[f].unwrap_or(mean)[c] + glow[c])
        }))
    }

    /** `picture`, of `level`, lit: each voxel's colour times the light on it, glowing ones' own light added; the sky where it misses. */
    pub fn relight(&mut self, picture: &mut Picture, level: u32) {
        for pixel in 0..picture.looks.len() {
            let look = picture.looks[pixel];
            let rgb = if look == 0 {
                self.background
            } else {
                let site = picture.sites[pixel];
                let on = self.on_voxel(level, site);
                let glow = self.see(level as usize, site, 0).1;
                let albedo = linear(look);
                std::array::from_fn(|c| albedo[c] * on[c] + glow[c])
            };
            for (c, value) in rgb.into_iter().enumerate() {
                picture.rgb[3 * pixel + c] = (255.0 * linear_to_srgb(value.clamp(0.0, 1.0))).round() as u8;
            }
        }
    }
}

/**
 * What a line through the middle of a site `seen` so (`see`) meets of it:
 * what it stops and glows over CROSSING crossings (fcc's `over`).
 */
fn crossed(((stops, colour), glow): Seen) -> ((Rgb, Rgb), Rgb) {
    // By what a channel stops: a power each, the same for every site, read often.
    static OVER: OnceLock<[f32; ALPHA as usize + 1]> = OnceLock::new();
    let table = OVER.get_or_init(|| std::array::from_fn(|k| over((k as f32 / ALPHA as f32) as f64, CROSSING) as f32));
    let over = stops.map(|k| table[k as usize]);
    ((std::array::from_fn(|c| stops[c] as f32 / ALPHA as f32 * over[c]), colour), std::array::from_fn(|c| glow[c] * over[c]))
}

/** The mean of `lights`, none if there are none. */
fn mean<'a>(lights: impl Iterator<Item = &'a Rgb>) -> Option<Rgb> {
    let (sum, n) = lights.fold(([0.0f32; 3], 0), |(s, n), l| (std::array::from_fn(|c| s[c] + l[c]), n + 1));
    (n > 0).then(|| sum.map(|c| c / n as f32))
}

/**
 * Where a site met along line `l` of a site `p`, `step` steps along, is
 * seen from: the site before it, and the face the light that lands on
 * its face turned back leaves that by (`lands`).
 */
fn reflected(p: [i64; 3], l: usize, step: usize) -> ([i64; 3], usize) {
    let ([a, b], before) = (line(l), step - 1);
    let (along_a, along_b) = (before.div_ceil(2) as i64, (before / 2) as i64);
    (add(p, std::array::from_fn(|c| along_a * FACES[a][c] + along_b * FACES[b][c])), opposite([a, b][before % 2]))
}

/** A look's colour as a share of the light on it, linear, as light adds up; looks are sRGB. */
fn linear(look: Look) -> Rgb {
    // A power a channel, of only 256 bytes: kept.
    static LINEAR: OnceLock<[f32; 256]> = OnceLock::new();
    let table = LINEAR.get_or_init(|| std::array::from_fn(|b| srgb_to_linear(b as f32 / 255.0)));
    [0, 8, 16].map(|shift| table[((look >> shift) & 255) as usize])
}

/**
 * Steps a line takes at the coarsest level, of `heights`, before the
 * sky's light takes over: as many periods as the heights are sites high.
 * One that rises or falls has left them by then; one that keeps level
 * never does, and ends as far, shaded by nothing farther off. Chosen, for
 * a world that lies across y (docs/light.md).
 */
fn top_steps((low, high): (i64, i64)) -> usize {
    MOST_STEPS.min((high - low) as usize)
}

/** How much what lands on a face changed from `before` to `after`, of what it is: the most of any channel. */
fn change(before: Rgb, after: Rgb) -> f32 {
    (0..3).filter(|&c| after[c] > 0.0).map(|c| (after[c] - before[c]).abs() / after[c]).fold(0.0, f32::max)
}

fn add(a: [i64; 3], b: [i64; 3]) -> [i64; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn dot_i(a: [i64; 3], b: [i64; 3]) -> i64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross_i(a: [i64; 3], b: [i64; 3]) -> [i64; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

/**
 * The sites of a level that `at`, a point of it, reads light from, and
 * their weights: the site nearest first, and its neighbours, each 1.5 less
 * its distance. Any further would leak light through walls one site thick.
 */
fn reads(at: [f64; 3]) -> impl Iterator<Item = ([i64; 3], f32)> {
    let nearest = nearest_site(at);
    std::iter::once(nearest).chain(FACES.map(|f| add(nearest, f))).filter_map(move |site| {
        let r = sub(at, site);
        let w = 1.5 - dot(r, r).sqrt();
        (w > 0.0).then_some((site, w as f32))
    })
}

/**
 * `reads` at the places `near` reads around a site, by its class and the
 * face the lines leave by: as offsets from the class's base, each site's
 * weights at the G places past the steps along each line; and the site
 * nearest each.
 */
type Around = (Vec<([i64; 3], [f32; G])>, [[i64; 3]; G]);

/** A site's class, of the 8 that the sites around its centre come in (`around`), and the base they are from: half of it, rounded down. */
fn class(p: [i64; 3]) -> (usize, [i64; 3]) {
    let base = p.map(|c| c.div_euclid(2));
    // Its last coordinate's parity follows from the others'.
    let class = (p[0] & 1) | (p[1] & 1) << 1 | ((base[0] + base[1] + base[2]) & 1) << 2;
    (class as usize, base)
}

/** `Around` for each class and face, taking `steps` periods. */
fn around(steps: usize) -> &'static [[Around; F]; 8] {
    static AROUND: [OnceLock<[[Around; F]; 8]>; MOST_STEPS + 1] = [const { OnceLock::new() }; MOST_STEPS + 1];
    AROUND[steps].get_or_init(|| {
        std::array::from_fn(|class| {
            let (c, b) = (class as i64 & 3, class as i64 >> 2);
            // A site of the class: its base (b, 0, 0), its parities those of the class.
            let p = [2 * b + (c & 1), c >> 1, ((c & 1) + (c >> 1)) & 1];
            let base = [b, 0, 0];
            assert_eq!(self::class(p), (class, base));
            let offset = |site: [i64; 3]| add(site, base.map(|c| -c));
            std::array::from_fn(|f| {
                let at: [[f64; 3]; G] = std::array::from_fn(|k| {
                    let [a, b] = line(f * G + k);
                    handover(add(p, add(FACES[a], FACES[b]).map(|v| v * steps as i64)), a)
                });
                let mut sites = BTreeMap::new();
                for (k, at) in at.iter().enumerate() {
                    for (site, w) in reads(*at) {
                        sites.entry(offset(site)).or_insert([0.0; G])[k] = w;
                    }
                }
                (sites.into_iter().collect(), at.map(|at| offset(nearest_site(at))))
            })
        })
    })
}

/**
 * Where the next coarser level's line takes over from one at site `q` of
 * a level whose next step is by face `next`: half that step back, so that
 * the coarser line's first site begins where `q` ends.
 */
fn handover(q: [i64; 3], next: usize) -> [f64; 3] {
    std::array::from_fn(|c| (2 * q[c] - FACES[next][c]) as f64 / 4.0)
}

fn sub(a: [f64; 3], b: [i64; 3]) -> [f64; 3] {
    [a[0] - b[0] as f64, a[1] - b[1] as f64, a[2] - b[2] as f64]
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

#[cfg(test)]
mod reference;

#[cfg(test)]
mod tests;
