//! Light on the lattices 2^k·Z³: a prototype, for look's pictures and
//! native's `--light`, whose bakers ask for the light from the faces of
//! the voxels they show (lattice's light.rs).
//!
//! Light comes to a site from 18 directions, to the 6 neighbours across
//! its faces and the 12 across its edges, and only ever across faces: a
//! step along an edge's direction goes through one of the two sites beside
//! the edge, then the one past it, half the light each way (`HALVES`).
//! Along each direction from the sites of its level, as every level does:
//! a few steps (Shared's, STEPS unless set), then on till the line leaves
//! the cube of the next coarser level's site it is in, whose light from
//! that direction comes from past it, if that site has room for light
//! (`along`). So a wall thinner than a coarse cube in a level's steps may
//! let light through: more steps, less leaks.
//! Each site sends what it glows, and of what it stops (its alpha, a
//! medium's by channel) its colour times the light on the face it is seen
//! through (`meets`, `on_face`), a third of what it shows that way; the
//! rest of what comes from past it gets through. So what comes from far
//! comes from coarser levels, the coarsest's from the sky, once a line has gone
//! as far as the world is high (`top_steps`). Sites that reflect less than
//! the tolerance reflect together, as the next one met that brings them
//! over it, and nothing comes from past where less than the tolerance gets
//! through (`along`): the light on few sites is asked for, where many stop
//! little.
//!
//! The light on a site is what the sites near it reflect of the light on
//! them, so it settles round by round: round 0 has what glows, the sky and
//! the coarser level's light, each next round what the sites met reflect
//! of theirs in the round before, so round n has light bounced n times at
//! most; found once it changes by less than a tolerance (Shared's,
//! TOLERANCE unless set), in a round or two in the open, or at ROUNDS,
//! where light bounces back and forth. Of each round a site keeps the
//! light on its faces (`round`): what the sites it reflects on read, and
//! what is to settle. Between whole voxels that touch no light comes: one
//! beside a face is lit as the face is (`weighed`).
//!
//! What is found at a site is a function of the world alone, so the Lights
//! of all threads keep it together (Shared), each site's found once: no
//! level's sites apart from another's, each part of a site and each of
//! its rounds set once, so that Lights wait for each other only at the
//! same site, and read what is set without a lock. They read the
//! world's tiles (worldgen's tiles.rs), as hosts draw them: lit, a tile's sites are
//! made once, for what draws it and what lights it (Shared::make_tiles).
//!
//! Light sees each site as it is (VoxelGrid::light), as hosts take it: what
//! its pieces and its medium stop of its rays, a share. So the coarse
//! sites around a small lamp glow as bright as the lamp, on average over
//! their faces, and those small things fill in part stop as much as the
//! things do.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use lattice::light::{FaceLight, linear_to_srgb, srgb_to_linear};
use lattice::ray::FACES;
use lattice::{FACE_KINDS, GRID_LEVELS, Look};

use worldgen::World;
use worldgen::grid::{ALPHA, COLOUR_ONE, GLOW_ONE, Grid, Stops, VoxelGrid};
use worldgen::picture::Picture;
use worldgen::tiles::{TILE, made_with, make_sites, parted};

type Rgb = [f32; 3];

const N: usize = 18;

/**
 * The directions light comes along: towards the neighbours across a
 * voxel's faces (FACES), then across its edges, so that light comes
 * slanting too.
 */
const DIRECTIONS: [[i64; 3]; N] = {
    let mut directions = [[0; 3]; N];
    let mut d = 0;
    while d < FACE_KINDS {
        directions[d] = FACES[d];
        d += 1;
    }
    let (mut a, mut k) = (0, 0);
    while a < 3 {
        while k < 4 {
            directions[d][a] = if k & 1 == 0 { -1 } else { 1 };
            directions[d][(a + 1) % 3] = if k & 2 == 0 { -1 } else { 1 };
            d += 1;
            k += 1;
        }
        (a, k) = (a + 1, 0);
    }
    directions
};

/** A voxel's faces: its light kept by face, not by direction. */
const F: usize = FACE_KINDS;

/**
 * Per direction across an edge, the two faces of the site a step leaves
 * by, one to each site beside the edge (FACES' kinds), half the light each
 * way: their offsets are the step's two parts. None along an axis.
 */
const HALVES: [Option<[usize; 2]>; N] = {
    let mut halves = [None; N];
    let mut d = FACE_KINDS;
    while d < N {
        let (mut parts, mut n, mut k) = ([0; 2], 0, 0);
        while k < FACE_KINDS {
            let (f, v) = (FACES[k], DIRECTIONS[d]);
            if f[0] * v[0] + f[1] * v[1] + f[2] * v[2] == 1 {
                parts[n] = k;
                n += 1;
            }
            k += 1;
        }
        halves[d] = Some(parts);
        d += 1;
    }
    halves
};

/**
 * Per direction, the faces a line along it comes into a site by, one per
 * axis it goes along (FACES' kinds): along an axis one, as into a site
 * beside an edge (`along`), across an edge two.
 */
static ENTERS: [&[usize]; N] = {
    // Each direction's faces, its one first along an axis, so that a slice of them can be handed out.
    static FACES_IN: [[usize; 2]; N] = {
        let mut faces = [[0; 2]; N];
        let mut d = 0;
        while d < N {
            faces[d] = match HALVES[d] {
                Some([a, b]) => [lattice::opposite(a), lattice::opposite(b)],
                None => [lattice::opposite(d); 2],
            };
            d += 1;
        }
        faces
    };
    let mut enters: [&[usize]; N] = [&[]; N];
    let mut d = 0;
    while d < N {
        enters[d] = if HALVES[d].is_some() { &FACES_IN[d] } else { FACES_IN[d].split_at(1).0 };
        d += 1;
    }
    enters
};

/** Steps along a direction before the coarser level's light may take over, unless set (Shared::stepping), and at most, where none does: in a closed place, or a world with no heights. */
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
 * Sites of a level begun between one forgetting of its tiles and the next
 * (Shared::begun): fewer hold less, and under what a view reads at once,
 * lighting slows several times over (docs/light.md).
 */
const KEPT_SITES: usize = 3 << 13;

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
 * What the Lights found in a tile: by cube (z-major, then y, then x) and
 * site in it, nothing till begun; and whether a Light read it since the
 * tiles not read were last forgotten (Shared::begun).
 */
struct Lit {
    read: AtomicBool,
    cubes: [OnceLock<Box<Sites>>; TILE_CUBES],
}

type Sites = [Room; CUBE_SITES];

/**
 * Sites a cube of a Lit is a side: smaller than a brick, as light begins
 * few sites of most of the bricks it reads, and a site's room is kept for
 * all its cube's (docs/light.md).
 */
const CUBE: i64 = 4;
/** Cubes a tile is a side, and in all; sites a cube holds. */
const SIDE: i64 = TILE / CUBE;
const TILE_CUBES: usize = (SIDE * SIDE * SIDE) as usize;
const CUBE_SITES: usize = (CUBE * CUBE * CUBE) as usize;

impl Made {
    /** The same tiles and sites, of what was found only the tiles' `keep` says to. */
    fn keeping(&self, keep: impl Fn(&Lit) -> bool) -> Made {
        let kept = |lit: &OnceLock<Arc<Lit>>| lit.get().filter(|lit| keep(lit)).cloned().map_or_else(OnceLock::new, OnceLock::from);
        Made { tiles: self.tiles.clone(), sites: self.sites.clone(), lit: self.lit.iter().map(kept).collect() }
    }

    /** What is kept of `site`, one of its tiles': its tile marked read. */
    fn room(&self, site: [i64; 3]) -> &Room {
        let tile = (site[1].div_euclid(TILE) - self.tiles[0][1]) as usize;
        let lit =
            self.lit[tile].get_or_init(|| Arc::new(Lit { read: AtomicBool::new(false), cubes: std::array::from_fn(|_| OnceLock::new()) }));
        // Written only when it changes: Lights reading the same tile do not take the mark from each other.
        if !lit.read.load(Ordering::Relaxed) {
            lit.read.store(true, Ordering::Relaxed);
        }
        let [x, y, z] = site.map(|v| v.rem_euclid(TILE) / CUBE);
        let cube = lit.cubes[((z * SIDE + y) * SIDE + x) as usize].get_or_init(|| Box::new(std::array::from_fn(|_| Room::default())));
        let [x, y, z] = site.map(|v| v.rem_euclid(CUBE));
        &cube[((z * CUBE + y) * CUBE + x) as usize]
    }
}

/**
 * A light in 16 bits a channel, as most of what a site keeps is: a float
 * of 10 bits past its first, from about 2⁻³² (less is none) to 2³¹ (more
 * is that), so within a thousandth.
 */
type Packed = [u16; 3];

/** How far a float's exponent and its first 10 bits are from a Packed's. */
const PACK: i32 = (127 - 33) << 10;

/** `light` packed: rounded to the nearest. */
fn pack(light: Rgb) -> Packed {
    light.map(|v| ((((v.max(0.0).to_bits() + (1 << 12)) >> 13) as i32) - PACK).clamp(0, u16::MAX as i32) as u16)
}

fn unpack(packed: Packed) -> Rgb {
    packed.map(|k| if k == 0 { 0.0 } else { f32::from_bits(((k as i32 + PACK) as u32) << 13) })
}

/**
 * What is kept of a site, in its cube: each part once found, the same
 * whichever Light finds it, so two finding it at once do no harm. What
 * its light is found from is kept only of a site that stops light: only
 * those reflect, so none reads the rounds of one that does not.
 */
#[derive(Default)]
struct Room {
    site: OnceLock<Arc<Site>>,
    /** The light coming to it from each direction, settled. */
    found: OnceLock<Box<[Packed; N]>>,
}

/**
 * What a site's light is found from, as it settles: what comes to it,
 * and its rounds so far, each packed, half of what floats take.
 */
struct Site {
    paths: Paths,
    /**
     * The light on each of its faces in each round (`round`): each kept
     * once found, the same whoever finds it, so read without a lock.
     */
    rounds: [OnceLock<[Packed; F]>; ROUNDS + 1],
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
    /** Sites whose light was begun, so far, by level (`begun`). */
    begun: Arc<[AtomicUsize; GRID_LEVELS]>,
    /** Times something made was forgotten or replaced, by level: a Light then lets go of what it read of it (Level::made). */
    forgotten: Arc<[AtomicUsize; GRID_LEVELS]>,
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

    /** Steps along a direction before the coarser level's light may take over. */
    pub fn steps(&self) -> usize {
        self.steps
    }

    /** The same world, light taking `steps` steps along each level before the coarser's: its tiles' sites kept. */
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
     * Counts a site of `level` whose light is begun. Every KEPT_SITES of
     * that level, the light of its tiles no Light read since the last time
     * is forgotten, to find again if asked: what is being lit, and what it
     * reads, is read all the while. Each level by its own: finer sites
     * read a coarse site's light once, so its tiles go unread while finer
     * levels work around them.
     */
    fn begun(&self, level: usize) {
        if (self.begun[level].fetch_add(1, Ordering::Relaxed) + 1).is_multiple_of(KEPT_SITES) {
            // Aside, none waiting for all of them meanwhile: what was forgotten or made anew since is left as it is.
            let made: Vec<_> = self
                .made
                .boxes()
                .lock()
                .expect("made")
                .iter()
                .filter(|((l, _), _)| *l == level)
                .map(|(key, made)| (*key, made.clone()))
                .collect();
            let kept: Vec<_> = made.iter().map(|(_, made)| Arc::new(made.keeping(|lit| lit.read.swap(false, Ordering::Relaxed)))).collect();
            // What they were is let go once none waits for the lock: after `now`, as `made` goes.
            let mut now = self.made.boxes().lock().expect("made");
            for ((key, was), kept) in made.iter().zip(kept) {
                if let Some(made) = now.get_mut(key).filter(|made| Arc::ptr_eq(made, was)) {
                    *made = kept;
                }
            }
            self.forgotten[level].fetch_add(1, Ordering::Relaxed);
        }
    }

    /**
     * Forgets what was made whose every tile `far` says is far, by level
     * and key, and its light with it, to make again if asked. Returns
     * what it forgot, if anything, for its caller to let go where it has
     * the time: freeing it is long.
     */
    pub fn forget(&self, far: impl Fn(usize, [i64; 3]) -> bool) -> Option<Box<dyn Send>> {
        self.forget_made(|level, made| made.tiles.iter().all(|&key| far(level, key)))
    }

    /**
     * Forgets what was made whose any tile `near` says is near an edit
     * (worldgen's Edits), by level and key, and its light with it: its
     * sites are made again with the edit carved, and the light around it
     * found again. Returns what it forgot, as `forget`.
     */
    pub fn edited(&self, near: impl Fn(usize, [i64; 3]) -> bool) -> Option<Box<dyn Send>> {
        self.forget_made(|level, made| made.tiles.iter().any(|&key| near(level, key)))
    }

    /** Forgets what was made that `gone` says to, by level. */
    fn forget_made(&self, gone: impl Fn(usize, &Made) -> bool) -> Option<Box<dyn Send>> {
        let mut made = self.made.boxes().lock().expect("made");
        let gone: Vec<_> = made.extract_if(|&(level, _), made| gone(level, made)).collect();
        if gone.is_empty() {
            return None;
        }
        for &((level, _), _) in &gone {
            self.forgotten[level].fetch_add(1, Ordering::Relaxed);
        }
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

type Map<K, V> = lattice::FastMap<K, V>;

/**
 * A level's heights, and what a Light read of it is made in, by tile
 * (where in `held`), with the tile read last: most reads are in it. Its
 * own, so that it asks the others for each once; let go once any of the
 * level's that is shared was forgotten (Shared's `forgotten`, as it was
 * when it held nothing), not to hold that.
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
        let forgotten = shared.forgotten[i].load(Ordering::Relaxed);
        if forgotten != self.forgotten {
            *self = Level { forgotten, ..Level::new(self.heights) };
        }
        let key = site.map(|v| v.div_euclid(TILE));
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
 * What comes to a site and does not change as its light settles (`along`):
 * from each direction, all but reflected light; the sites met that
 * reflect, each with its direction, how many steps along it, and the share
 * of the light on its face that comes, its own and that of the sites it
 * reflects for (`along`); and, for a whole voxel, the colours of the whole
 * voxels it touches (`weighed`).
 */
struct Paths {
    comes: [Packed; N],
    reflects: Box<[Reflects]>,
    touches: Touches,
}

/**
 * A site met that reflects (Paths): in 16 bits, the direction, the steps
 * along it (MOST_STEPS at most), which site of the last step (0 the one
 * past it, 1 or 2 one beside its edge, HALVES) and the face seen
 * (`meets`); and the share of the light on it that comes. Most of what
 * a site keeps, so as small as it goes.
 */
#[derive(Clone, Copy)]
struct Reflects {
    at: u16,
    share: Packed,
}

impl Reflects {
    fn new(d: usize, step: usize, part: usize, face: usize, share: Packed) -> Reflects {
        debug_assert!(d < N && (1..=MOST_STEPS).contains(&step) && part < 3 && face < F);
        Reflects { at: (d | (step - 1) << 5 | part << 11 | face << 13) as u16, share }
    }

    /** The site it is, met from `p`; its direction, and the face seen. */
    fn site(self, p: [i64; 3]) -> ([i64; 3], usize, usize) {
        let at = self.at as usize;
        let (d, step, part, face) = (at & 31, (at >> 5 & 63) as i64, at >> 11 & 3, at >> 13);
        // Past the steps before the last, the one past it, or the one beside its edge (HALVES) by `part`.
        let before = add(p, DIRECTIONS[d].map(|v| v * step));
        let last = match HALVES[d] {
            Some(faces) if part > 0 => FACES[faces[part - 1]],
            _ => DIRECTIONS[d],
        };
        (add(before, last), d, face)
    }
}

/** What a line meets in a site (`meets`): the face it is seen through, what it stops and the colour of that, what it glows. */
type Met = (usize, (Rgb, Rgb), Rgb);

/**
 * The sites across a site's faces, as `begin` finds them: each met by
 * five of its lines in their first step, along its axis and beside its 4
 * edges, so seen once. What a line coming into each by its face meets,
 * and whether the site touches it: both whole voxels.
 */
struct Around {
    met: [Met; F],
    touching: [bool; F],
}

/** Per face, the colour of the whole voxel a whole voxel touches across it, none (black) if it touches none; None if it touches none at all, as most sites. */
type Touches = Option<Box<[Rgb; F]>>;

pub struct Light {
    world: Arc<World>,
    shared: Shared,
    /** By level, up to a renderer's coarsest grid's (GRID_LEVELS). */
    levels: Vec<Level>,
    /** `begin`'s sites that reflect, while it finds them: its room kept from one site to the next. */
    reflecting: Vec<Reflects>,
    unit: [[f64; 3]; N],
    /** Per face, how much the light from each direction weighs in the light on it (`on_face`). */
    cosines: [[f32; N]; F],
    /** The radiance of a glow of GLOW_ONE. */
    glow: f32,
    /** The sky's radiance per direction (the sun's within it). */
    sky: [Rgb; N],
    /** What the sky looks like, where a picture's rays miss. */
    background: Rgb,
}

impl Light {
    /** The light of `world`, found only where asked, and kept in `shared`. */
    pub fn new(world: impl Into<Arc<World>>, shared: Shared) -> Light {
        let world = world.into();
        let Brightness { sky, sun, .. } = shared.brightness;
        let coarsest = GRID_LEVELS as u32 - 1;
        let levels = (0..=coarsest).map(|level| Level::new(world.heights_at(level))).collect();
        let unit = DIRECTIONS.map(|d| normalize(d.map(|v| v as f64)));
        let cosines = FACES.map(|normal| {
            let normal = normal.map(|v| v as f64);
            let weights = unit.map(|u| dot(u, normal).max(0.0));
            let total: f64 = weights.iter().sum();
            weights.map(|w| (w / total) as f32)
        });
        let sun_at = DIRECTIONS.iter().position(|d| *d == SUN_DIRECTION).expect("the sun is in one of them");
        let skies = std::array::from_fn(|d| {
            let up = unit[d][1];
            let open = if up > 1e-9 {
                1.0
            } else if up > -1e-9 {
                0.5
            } else {
                0.0
            };
            std::array::from_fn(|c| open * SKY[c] * sky + if d == sun_at { SUN[c] * sun } else { 0.0 })
        });
        let background = SKY.map(|c| c * sky);
        // A world with no heights has no sky: what glows is all its light.
        let (skies, background) = if world.heights.is_some() { (skies, background) } else { ([[0.0; 3]; N], [0.0; 3]) };
        let glow = shared.brightness.glowing() / GLOW_ONE as f32;
        Light { world, shared, levels, reflecting: Vec::new(), unit, cosines, glow, sky: skies, background }
    }

    /** Whether a whole voxel, one that lets nothing through, is at `site` of level `i`. */
    fn whole(&mut self, i: usize, site: [i64; 3]) -> bool {
        self.open(i, site) <= 0.0
    }

    /** Whether hosts are shown a whole voxel at `site` of level `i` (VoxelGrid::shows_whole). */
    fn drawn(&mut self, i: usize, site: [i64; 3]) -> bool {
        self.hold(i, site).is_some_and(|([x, y, z], grid)| grid.shows_whole(x, y, z))
    }

    /**
     * How light sees `site` of level `i` through face `from`: how much of
     * each channel it stops, its pieces and its medium of the rest, and
     * their colour; what it glows.
     */
    fn see(&mut self, i: usize, site: [i64; 3], from: usize) -> ((Rgb, Rgb), Rgb) {
        let (_, stops, glow) = self.meets(i, site, &[from]);
        (stops, glow)
    }

    /**
     * What a line coming into `site` of level `i` by the faces `enters`
     * (ENTERS) meets: the face it is seen through, the first of those that
     * stop most, and through it what it stops, its colour and its glow.
     * A line across an edge crosses a wall in one of the sites it meets
     * there, so each stops it as the wall stops a line straight through
     * it (docs/light.md, "Thin walls").
     */
    fn meets(&mut self, i: usize, site: [i64; 3], enters: &[usize]) -> Met {
        let glowing = self.glow;
        let Some(([x, y, z], grid)) = self.hold(i, site) else { return (enters[0], ([0.0; 3], [0.0; 3]), [0.0; 3]) };
        let stops = |((pieces, _), (medium, _), _): &Seen| medium.iter().map(|&m| *pieces as u32 + m as u32).sum::<u32>();
        let mut most = (enters[0], grid.light(x, y, z, enters[0]));
        for &face in &enters[1..] {
            let next = grid.light(x, y, z, face);
            if stops(&next) > stops(&most.1) {
                most = (face, next);
            }
        }
        let (stops, glow) = seen(most.1, glowing);
        (most.0, stops, glow)
    }

    /** How much of what comes through `site` of level `i` it lets through. */
    fn open(&mut self, i: usize, site: [i64; 3]) -> f32 {
        let Some(([x, y, z], grid)) = self.hold(i, site) else { return 1.0 };
        1.0 - grid.alpha(x, y, z) as f32 / ALPHA as f32
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
        Some((site, sites.get_or_init(|| make_sites(world, i as u32, site.map(|v| v.div_euclid(TILE))))))
    }

    /** What is kept of site `p` of level `i`, read by `with`: its tile marked read. */
    fn room<T>(&mut self, i: usize, p: [i64; 3], with: impl FnOnce(&Room) -> T) -> T {
        with(self.levels[i].made(&self.world, &self.shared, i, p).room(p))
    }

    /**
     * The light coming to site `p` of level `i` from every direction, as
     * it settles round by round (`round`): found once the light on its
     * faces changes by less than Shared's tolerance, in a round or two
     * where little reflects back, or after ROUNDS: packed, the same to
     * whoever finds it and whoever reads it kept.
     */
    fn light(&mut self, i: usize, p: [i64; 3]) -> [Packed; N] {
        if let Some(light) = self.room(i, p, |room| room.found.get().map(|found| **found)) {
            return light;
        }
        let site = self.site(i, p);
        let (mut on, mut rounds) = (self.round(i, p, 0, &site), 0);
        while rounds < ROUNDS {
            let next = self.round(i, p, rounds + 1, &site);
            let settled = change(&on, &next) <= self.shared.tolerance;
            (on, rounds) = (next, rounds + 1);
            if settled {
                break;
            }
        }
        let light = self.coming(i, p, rounds, &site.paths).map(pack);
        // Unless another Light found it meanwhile: the same.
        _ = self.room(i, p, |room| room.found.set(Box::new(light)));
        light
    }

    /**
     * The light coming to site `p` of level `i`, whose `paths` these are,
     * from every direction in round `n`: what comes, and what the sites
     * met reflect of the light on their faces turned back in the round
     * before; none in round 0.
     */
    fn coming(&mut self, i: usize, p: [i64; 3], n: usize, paths: &Paths) -> [Rgb; N] {
        let mut light = paths.comes.map(unpack);
        if n > 0 {
            for &reflects in &paths.reflects {
                let (q, d, back) = reflects.site(p);
                let on = match self.room(i, q, |room| room.site.get().and_then(|site| site.rounds[n - 1].get().copied())) {
                    Some(on) => unpack(on[back]),
                    None => {
                        let site = self.site(i, q);
                        self.round(i, q, n - 1, &site)[back]
                    }
                };
                let share = unpack(reflects.share);
                (0..3).for_each(|c| light[d][c] += share[c] * on[c]);
            }
        }
        light
    }

    /**
     * The light on each face of site `p` of level `i` in round `n`
     * (`on_face`), from what comes to it then (`coming`), with what it is
     * found from, `site`: packed, the same to whoever finds it and whoever
     * reads it kept.
     */
    fn round(&mut self, i: usize, p: [i64; 3], n: usize, site: &Site) -> [Rgb; F] {
        if let Some(on) = site.rounds[n].get() {
            return on.map(unpack);
        }
        let light = self.coming(i, p, n, &site.paths);
        let on: [Packed; F] = std::array::from_fn(|f| pack(self.on_face(&light, site.paths.touches.as_deref(), f)));
        // Unless another Light kept it meanwhile: the same.
        _ = site.rounds[n].set(on);
        on.map(unpack)
    }

    /**
     * What the light of site `p` of level `i` is found from, begun if not
     * yet: kept if the site stops light (Room), else for its caller alone.
     */
    fn site(&mut self, i: usize, p: [i64; 3]) -> Arc<Site> {
        if let Some(site) = self.room(i, p, |room| room.site.get().cloned()) {
            return site;
        }
        let site = Arc::new(self.begin(i, p));
        if self.open(i, p) == 1.0 {
            return site;
        }
        // Unless another Light began it meanwhile: the same, so either does.
        self.room(i, p, |room| room.site.get_or_init(|| site).clone())
    }

    /** What the light of site `p` of level `i` is found from: what comes to it along each direction (Paths), and no rounds yet. */
    fn begin(&mut self, i: usize, p: [i64; 3]) -> Site {
        self.shared.begun(i);
        let whole = self.whole(i, p);
        let around = Around {
            met: std::array::from_fn(|k| self.meets(i, add(p, FACES[k]), ENTERS[k])),
            touching: std::array::from_fn(|k| whole && self.whole(i, add(p, FACES[k]))),
        };
        let (mut reflects, mut touches): (_, Touches) = (std::mem::take(&mut self.reflecting), None);
        reflects.clear();
        let comes = std::array::from_fn(|d| {
            // Across a face, a whole voxel touches the next; across an edge, light comes between them.
            if HALVES[d].is_none() && around.touching[d] {
                touches.get_or_insert_with(|| Box::new([[0.0; 3]; F]))[d] = around.met[d].1.1;
                return [0; 3];
            }
            pack(self.along(i, p, &around, d, &mut reflects))
        });
        let paths = Paths { comes, reflects: reflects.as_slice().into(), touches };
        self.reflecting = reflects;
        Site { paths, rounds: Default::default() }
    }

    /**
     * The light coming to `p`, a site of level `i`, from direction `d`,
     * all but reflected light: from the sites that way, what each glows;
     * past them, the coarser level's light. After Shared's steps, once the
     * line is to leave the cube of the coarser site it is in, the coarser
     * level's light from `d` through the face it leaves by (`coarser`), if
     * that site has room for light: it comes from past the cube, all the
     * line has not met. Every level alike, so each has its
     * light so from the next, the coarsest from the next `top_steps`, and
     * past them the sky. The sites met that reflect go in `reflects`
     * (Paths): of what each stops, its colour. Each has its own light
     * found, so one that reflects under Shared's tolerance is owed to the
     * next that, with it, reflects over: that one reflects for both, as if
     * lit alike. What is still owed where the line ends is left out.
     */
    fn along(&mut self, i: usize, p: [i64; 3], around: &Around, d: usize, reflects: &mut Vec<Reflects>) -> Rgb {
        let (v, heights, steps) = (DIRECTIONS[d], self.levels[i].heights, self.shared.steps);
        let coarser = i + 1 < self.levels.len();
        // What has come so far, and how much of what comes from further on gets through.
        let (mut sum, mut through) = ([0.0f32; 3], [1.0f32; 3]);
        // What the sites met so far reflect, under the tolerance: none reflects for them yet.
        let mut owed = [0.0f32; 3];
        let mut q = p;
        let most = if coarser { MOST_STEPS } else { top_steps(heights) };
        let beyond = 'march: {
            for step in 1..=most {
                let next = add(q, v);
                let cube = q.map(|c| c.div_euclid(2));
                let leaves = (0..3).find(|&a| next[a].div_euclid(2) != cube[a]);
                if let Some(across) = leaves.filter(|_| coarser && step > steps && self.open(i + 1, cube) > 0.0) {
                    break 'march self.coarser(i + 1, q, across, d);
                }
                // The parts of the step, each with its share of the light: across an edge, a site beside it, half
                // the light each, come in by one face, then the one past it, all of it, by two (ENTERS). Next to
                // `q` across its face `k`: in the first step, as `around` met it.
                let next_to = |light: &mut Self, k: usize| {
                    if step == 1 { around.met[k] } else { light.meets(i, add(q, FACES[k]), ENTERS[k]) }
                };
                let mut met = |light: &mut Self, part: usize, (face, (alpha, colour), glow): Met, share: f32, through: &mut [f32; 3]| {
                    (0..3).for_each(|c| sum[c] += through[c] * share * glow[c]);
                    (0..3).for_each(|c| owed[c] += through[c] * share * alpha[c] * colour[c]);
                    if owed.iter().any(|&r| r > light.shared.tolerance) {
                        reflects.push(Reflects::new(d, step, part, face, pack(std::mem::take(&mut owed))));
                    }
                    alpha.map(|a| share * a)
                };
                if let Some(beside) = HALVES[d] {
                    // Off a whole voxel, none comes through a whole one beside it, which touches it: all through the other.
                    let shares = if step == 1 {
                        let open = beside.map(|k| if around.touching[k] { 0.0 } else { 1.0 });
                        if open == [0.0; 2] {
                            break 'march [0.0; 3];
                        }
                        open.map(|o| o / (open[0] + open[1]))
                    } else {
                        [0.5; 2]
                    };
                    let mut stops = [0.0f32; 3];
                    for (part, k) in beside.into_iter().enumerate().filter(|&(part, _)| shares[part] > 0.0) {
                        let seen = next_to(self, k);
                        let alpha = met(self, part + 1, seen, shares[part], &mut through);
                        (0..3).for_each(|c| stops[c] += alpha[c]);
                    }
                    (0..3).for_each(|c| through[c] *= 1.0 - stops[c]);
                }
                // Along an axis, the site across its face; across an edge, the one past it.
                let ahead = if HALVES[d].is_none() { next_to(self, d) } else { self.meets(i, next, ENTERS[d]) };
                let alpha = met(self, 0, ahead, 1.0, &mut through);
                (0..3).for_each(|c| through[c] *= 1.0 - alpha[c]);
                q = next;
                // Less gets through than the tolerance, or nothing more is met: below every voxel and not going up, or above them and not going down.
                if through.iter().all(|&t| t < self.shared.tolerance) || q[1] < heights.0 - 1 && v[1] <= 0 {
                    break 'march [0.0; 3];
                }
                if q[1] > heights.1 && v[1] >= 0 {
                    break 'march self.sky[d];
                }
            }
            if !coarser && v[1] >= 0 { self.sky[d] } else { [0.0; 3] }
        };
        std::array::from_fn(|c| sum[c] + through[c] * beyond[c])
    }

    /**
     * Level `i`'s light from direction `d` through the face across axis
     * `across` of the cube of `q`, a site of level `i − 1`, where the line
     * through the middle of `q` crosses it: its sites' light from `d` comes
     * through their faces on that side, so it is bilinear between the face
     * and the 3 beside it in its plane nearest the line, and shows no edges
     * of their cubes. A finer cube's middle is a quarter of a coarser cube
     * from the middles of the two nearest along each axis: weights of 3/4
     * and 1/4. Each is read as far as it lets light through, and the finer
     * site beside `q` towards it, if it is past `q`'s cube (`through`): so
     * none is read past a wall.
     */
    fn coarser(&mut self, i: usize, q: [i64; 3], across: usize, d: usize) -> Rgb {
        let cube = q.map(|c| c.div_euclid(2));
        let axes = [(across + 1) % 3, (across + 2) % 3];
        // Along each axis of the face, the two coarser sites nearest the line, each with its weight.
        let sides = axes.map(|a| {
            let (first, near) = ((q[a] - 1).div_euclid(2), if q[a] & 1 == 0 { 0.75 } else { 0.25 });
            [(first, 1.0 - near), (first + 1, near)]
        });
        // Along each axis, how much gets through from `q` to the side past its cube: found once, if a site
        // there lets light through.
        let mut past: [Option<f32>; 2] = [None; 2];
        let (mut sum, mut total) = ([0.0f32; 3], 0.0f32);
        for (y, wy) in sides[1] {
            for (x, wx) in sides[0] {
                let mut site = cube;
                (site[axes[0]], site[axes[1]]) = (x, y);
                let mut open = self.open(i, site);
                if open > 0.0 {
                    let mut through = 1.0;
                    for (k, a) in axes.into_iter().enumerate().filter(|&(_, a)| site[a] != cube[a]) {
                        let face = FACES.iter().position(|face| face[a] == site[a] - cube[a]).expect("a face");
                        through *= *past[k].get_or_insert_with(|| self.through(i - 1, q, face));
                    }
                    open *= through;
                }
                if open > 0.0 {
                    let (w, light) = (wx * wy, unpack(self.light(i, site)[d]));
                    total += w * open;
                    (0..3).for_each(|ch| sum[ch] += w * open * light[ch]);
                }
            }
        }
        sum.map(|ch| ch / total)
    }

    /** How much of what comes from `q`, a site of level `i`, gets through the site across its face `face`, on average over the channels. */
    fn through(&mut self, i: usize, q: [i64; 3], face: usize) -> f32 {
        let stops = self.meets(i, add(q, FACES[face]), ENTERS[face]).1.0;
        1.0 - stops.iter().sum::<f32>() / 3.0
    }

    /** Over π, the light on face `f` from `light`, its directions weighed by their cosines (`weighed`). */
    fn on_face(&self, light: &[Rgb; N], touches: Option<&[Rgb; F]>, f: usize) -> Rgb {
        weighed(&self.cosines[f], light, touches)
    }

    /** Over π, the light on a surface facing `normal` from `light`, as on a face (`on_face`). */
    fn on(&self, light: &[Rgb; N], touches: Option<&[Rgb; F]>, normal: [f64; 3]) -> Rgb {
        let weights = self.unit.map(|u| dot(u, normal).max(0.0));
        let total: f64 = weights.iter().sum();
        weighed(&weights.map(|w| (w / total) as f32), light, touches)
    }

    /**
     * The light from each face of the voxel at `site` of `level`: its
     * colour times the light on it, and what it glows. A voxel with no
     * neighbour light gets through (shown where a slice cuts it) takes the
     * mean of its neighbours' that have one, or none.
     */
    pub fn faces(&mut self, level: u32, site: [i64; 3]) -> FaceLight {
        let i = level as usize;
        if self.surface(i, site) {
            return self.lit(i, site);
        }
        let mut lit = Vec::new();
        for n in FACES.map(|f| add(site, f)) {
            if self.whole(i, n) && self.surface(i, n) {
                lit.push(self.lit(i, n));
            }
        }
        std::array::from_fn(|f| std::array::from_fn(|c| lit.iter().map(|l| l[f][c]).sum::<f32>() / lit.len().max(1) as f32))
    }

    /** What the voxel at `site` of `level` glows. */
    pub fn glow(&mut self, level: u32, site: [i64; 3]) -> Rgb {
        self.see(level as usize, site, 0).1
    }

    /** What lights the voxel at `site` of `level` (on_voxel), if light gets out of it: none if a ray meets it only where levels meet. */
    pub fn on_seen(&mut self, level: u32, site: [i64; 3]) -> Option<Rgb> {
        let i = level as usize;
        (self.open(i, site) > 0.0 || self.surface(i, site)).then(|| self.on_voxel(level, site))
    }

    /** Whether the voxel at `site` of level `i` has a neighbour light gets through. */
    fn surface(&mut self, i: usize, site: [i64; 3]) -> bool {
        FACES.iter().any(|f| !self.whole(i, add(site, *f)))
    }

    /** The colours of the whole voxels a whole voxel at `site` of level `i` touches (Paths), if any. */
    fn touches(&mut self, i: usize, site: [i64; 3]) -> Option<[Rgb; F]> {
        // Only a whole voxel touches others, and it stops light, so what its light is found from is kept.
        if self.whole(i, site) { self.site(i, site).paths.touches.as_deref().copied() } else { None }
    }

    /** The light from each face of the voxel at `site` of level `i`, on its surface. */
    fn lit(&mut self, i: usize, site: [i64; 3]) -> FaceLight {
        let (light, touches) = (self.light(i, site).map(unpack), self.touches(i, site));
        std::array::from_fn(|f| {
            let on = self.on_face(&light, touches.as_ref(), f);
            let ((_, colour), glow) = self.see(i, site, f);
            std::array::from_fn(|c| colour[c] * on[c] + glow[c])
        })
    }

    /** What lights the voxel at `site` of `level` to its colour at 1: the light on its surface, which faces the neighbours hosts are shown no voxel at. */
    pub fn on_voxel(&mut self, level: u32, site: [i64; 3]) -> Rgb {
        let i = level as usize;
        let mut normal = [0.0; 3];
        for face in FACES {
            if !self.drawn(i, add(site, face)) {
                (0..3).for_each(|c| normal[c] += face[c] as f64);
            }
        }
        let (light, touches) = (self.light(i, site).map(unpack), self.touches(i, site));
        let touches = touches.as_ref();
        if normal == [0.0; 3] {
            // No surface, or all of it: the mean of all its faces'.
            let all = FACES.map(|f| self.on(&light, touches, normalize(f.map(|v| v as f64))));
            return std::array::from_fn(|c| all.iter().map(|l| l[c]).sum::<f32>() / all.len() as f32);
        }
        self.on(&light, touches, normalize(normal))
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

/** How light sees a site, as VoxelGrid::light has it: how much of each channel it stops, of ALPHA, and the colour of that; what it glows. */
type Seen = (Stops<u8>, Stops<[u8; 3]>, [u32; 3]);

/** What light sees of a site (`see`), from what VoxelGrid::light has of it, where `glowing` is a glow of GLOW_ONE. */
fn seen(((pieces, p), (medium, m), glow): Seen, glowing: f32) -> ((Rgb, Rgb), Rgb) {
    let glow = glow.map(|g| g as f32 * glowing);
    // What they show comes premultiplied, of COLOUR_ONE: as a share of the light on what it stops.
    let alpha = medium.map(|m| (pieces as u32 + m as u32) as f32 / ALPHA as f32);
    let colour = std::array::from_fn(|c| {
        let shows = (p[c] + m[c]) as f32 / COLOUR_ONE as f32;
        if alpha[c] > 0.0 { (shows / alpha[c]).min(1.0) } else { 0.0 }
    });
    ((alpha, colour), glow)
}

/** A look's colour as a share of the light on it, linear, as light adds up; looks are sRGB. */
fn linear(look: Look) -> Rgb {
    // A power a channel, of only 256 bytes: kept.
    static LINEAR: OnceLock<[f32; 256]> = OnceLock::new();
    let table = LINEAR.get_or_init(|| std::array::from_fn(|b| srgb_to_linear(b as f32 / 255.0)));
    [0, 8, 16].map(|shift| table[((look >> shift) & 255) as usize])
}

/**
 * The light on a face, its directions weighed by `weights` (summing to 1).
 * A voxel the face touches is taken to be lit as the face is, so it sends
 * back its colour times that light: the light on the face is what comes
 * along the other directions, over what the touched voxels do not send.
 */
fn weighed(weights: &[f32; N], light: &[Rgb; N], touches: Option<&[Rgb; F]>) -> Rgb {
    let (mut sum, mut kept) = ([0.0f32; 3], [1.0f32; 3]);
    for (w, light) in weights.iter().zip(light) {
        (0..3).for_each(|c| sum[c] += w * light[c]);
    }
    if let Some(touches) = touches {
        // Across the faces: DIRECTIONS' first.
        for (w, touches) in weights.iter().zip(touches.iter()) {
            (0..3).for_each(|c| kept[c] -= w * touches[c]);
        }
    }
    std::array::from_fn(|c| if kept[c] > 0.0 { sum[c] / kept[c] } else { 0.0 })
}

/**
 * Steps a line takes at the coarsest level, of `heights`, before the
 * sky's light takes over: as many as the heights are sites high. One that
 * rises or falls has left them by then; one that keeps level never does,
 * and ends as far, shaded by nothing farther off. Chosen, for a world
 * that lies across y (docs/light.md).
 */
fn top_steps((low, high): (i64, i64)) -> usize {
    MOST_STEPS.min((high - low) as usize)
}

/** How much the light on a site's faces changed from `before` to `after`, of what it is: the most of any channel on any face. */
fn change(before: &[Rgb; F], after: &[Rgb; F]) -> f32 {
    let mut most = 0.0f32;
    for (b, a) in before.iter().zip(after) {
        (0..3).filter(|&c| a[c] > 0.0).for_each(|c| most = most.max((a[c] - b[c]).abs() / a[c]));
    }
    most
}

fn add(a: [i64; 3], b: [i64; 3]) -> [i64; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn normalize(v: [f64; 3]) -> [f64; 3] {
    let length = dot(v, v).sqrt();
    v.map(|c| c / length)
}

#[cfg(test)]
mod reference;

#[cfg(test)]
mod tests;
