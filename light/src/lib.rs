//! Light on the lattices 2^k·FCC: a prototype, for look's pictures and
//! native's `--light`, whose bakers ask for the light from the faces of
//! the voxels they show (fcc's light.rs).
//!
//! Light comes to a site from its 12 neighbours' directions: along each,
//! from the sites of its level, till the next coarser level can take over:
//! after a few steps (Shared's, STEPS unless set), where the coarser level
//! has room for light (`past`). So a wall thinner than about its distance
//! over the steps lets light through: more steps, less leaks.
//! Each site sends what it glows, and of what it stops (its alpha, a
//! medium's by channel) its colour times the light on its face
//! turned back (`on_face`), a third of what it shows that way; the rest
//! of what comes from past it gets through. So what comes from far comes
//! from coarser levels, the coarsest's from the sky, once a line has gone
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
//! level's sites apart from another's, each site under a lock of its own,
//! so that Lights wait for each other only at the same site. They read the
//! world's tiles (worldgen's tiles.rs), as hosts draw them: lit, a tile's sites are
//! made once, for what draws it and what lights it (Shared::make_tiles).
//!
//! Light sees each site as it is (VoxelGrid::light), as hosts take it: what
//! its pieces and its medium stop of its rays, a share. So the coarse
//! sites around a small lamp glow as bright as the lamp, on average over
//! their faces, and those small things fill in part stop as much as the
//! things do. What fills a site in part, a line through its middle meets
//! more often than its share says (fcc's CROSSING, `meets`).

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use fcc::light::{FaceLight, linear_to_srgb, srgb_to_linear};
use fcc::ray::{CROSSING, FACES, nearest_site, over};
use fcc::{GRID_LEVELS, Look};

use worldgen::World;
use worldgen::grid::{ALPHA, BRICK, COLOUR_ONE, GLOW_ONE, VoxelGrid};
use worldgen::picture::Picture;
use worldgen::tiles::{Made as Tile, TILE, made_with, make_sites, make_tile, parted};
use worldgen::time;

type Rgb = [f32; 3];

const N: usize = 12;

/** The directions light comes along: towards the neighbours. A voxel's face is across each, numbered alike. */
const DIRECTIONS: [[i64; 3]; N] = FACES;

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
 * site in it, each site under a lock of its own, nothing till begun; and
 * whether a Light read it since the tiles not read were last forgotten
 * (Shared::begun).
 */
struct Lit {
    read: AtomicBool,
    bricks: [OnceLock<Box<Sites>>; TILE_BRICKS],
}

type Sites = [Mutex<Option<Box<Site>>>; BRICK_SITES];

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
    fn site(&self, site: [i64; 3]) -> &Mutex<Option<Box<Site>>> {
        let tile = (site[1].div_euclid(TILE) - self.tiles[0][1]) as usize;
        let lit =
            self.lit[tile].get_or_init(|| Arc::new(Lit { read: AtomicBool::new(false), bricks: std::array::from_fn(|_| OnceLock::new()) }));
        // Written only when it changes: Lights reading the same tile do not take the mark from each other.
        if !lit.read.load(Ordering::Relaxed) {
            lit.read.store(true, Ordering::Relaxed);
        }
        let [x, y, z] = site.map(|v| v.rem_euclid(TILE) / BRICK);
        let brick = lit.bricks[((z * SIDE + y) * SIDE + x) as usize].get_or_init(|| Box::new(std::array::from_fn(|_| Mutex::default())));
        let [x, y, z] = site.map(|v| v.rem_euclid(BRICK));
        &brick[(((z * BRICK + y) * BRICK + x) >> 1) as usize]
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
 * What is kept of a site, each once found: the same whichever Light finds
 * it, so two finding it at once do no harm. What its rounds are found from
 * and the rounds are packed, half of what floats take; its settled light
 * not, read far more often than found.
 */
#[derive(Default)]
struct Site {
    /** What comes to it. */
    paths: Option<Arc<Paths>>,
    /** The light on each of its faces in each round (`round`), from the first on: most of what it holds. */
    rounds: Vec<[Packed; N]>,
    /** The light coming to it from each direction, settled. */
    found: Option<Box<[Rgb; N]>>,
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
     * Light made them: so what is drawn and lit is made once. A tile of
     * them that does not stay for the time it is made at (time.rs) is made
     * anew alone, and not kept: the light reads it where it was.
     */
    pub fn make_tiles(&self, world: &World, level: u32, key: [i64; 3]) -> Vec<Tile> {
        let made = self.made(world, level, key);
        let sites = made.sites.get_or_init(|| make_sites(world, level, key));
        let now = |(key, grid, stays): Tile| {
            if stays.contains(&time::now()) {
                return (key, grid, stays);
            }
            let (grid, stays) = make_tile(world, level, key);
            (key, grid, stays)
        };
        parted(sites, world, level, key).into_iter().map(now).collect()
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
    reflects: Box<[(u8, u8, Packed)]>,
    touches: Touches,
}

/** Per direction, the colour of the whole voxel a whole voxel touches that way, none (black) if it touches none; None if it touches none at all, as most sites. */
type Touches = Option<Box<[Rgb; N]>>;

pub struct Light {
    world: Arc<World>,
    shared: Shared,
    /** By level, up to a renderer's coarsest grid's (GRID_LEVELS). */
    levels: Vec<Level>,
    /** `paths`'s sites that reflect, while it finds them: its room kept from one site to the next. */
    reflecting: Vec<(u8, u8, Packed)>,
    unit: [[f64; 3]; N],
    /** Per face, how much the light from each direction weighs in the light on it (`on_face`). */
    cosines: [[f32; N]; N],
    /** Per direction, the one opposite: where a site met along it is seen from. */
    opposite: [usize; N],
    /** The radiance of a glow of GLOW_ONE. */
    glow: f32,
    /** The sky's radiance per direction (the sun's within it). */
    sky: [Rgb; N],
    /** What the sky looks like, where a picture's rays miss. */
    background: Rgb,
}

/** The coarser level's light around a site, as `along` takes it, read at once (`near`): per direction, past the steps, if the coarser level has room for it there (`past`). */
type Near = [Option<Rgb>; N];

impl Light {
    /** The light of `world`, found only where asked, and kept in `shared`. */
    pub fn new(world: impl Into<Arc<World>>, shared: Shared) -> Light {
        let world = world.into();
        let Brightness { sky, sun, .. } = shared.brightness;
        let coarsest = GRID_LEVELS as u32 - 1;
        let levels = (0..=coarsest).map(|level| Level::new(world.heights_at(level))).collect();
        let unit = DIRECTIONS.map(|d| normalize(d.map(|v| v as f64)));
        let cosines = unit.map(|normal| {
            let weights = unit.map(|u| dot(u, normal).max(0.0));
            let total: f64 = weights.iter().sum();
            weights.map(|w| (w / total) as f32)
        });
        let opposite = std::array::from_fn(fcc::opposite);
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
        Light { world, shared, levels, reflecting: Vec::new(), unit, cosines, opposite, glow, sky: skies, background }
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
     * How light sees `site` of level `i` from direction `from`: how much of
     * each channel it stops (of ALPHA), its pieces and its medium of the
     * rest, and their colour; what it glows.
     */
    fn see(&mut self, i: usize, site: [i64; 3], from: usize) -> (([u32; 3], Rgb), Rgb) {
        let glowing = self.glow;
        let Some(([x, y, z], grid)) = self.hold(i, site) else { return (([0; 3], [0.0; 3]), [0.0; 3]) };
        let ((pieces, p), (medium, m), glow) = grid.light(x, y, z, from);
        // What they show comes premultiplied, of COLOUR_ONE: as a share of the light on what it stops.
        let stops = medium.map(|m| pieces as u32 + m as u32);
        let colour = std::array::from_fn(|c| {
            let (alpha, shows) = (stops[c] as f32 / ALPHA as f32, (p[c] + m[c]) as f32 / COLOUR_ONE as f32);
            if alpha > 0.0 { (shows / alpha).min(1.0) } else { 0.0 }
        });
        ((stops, colour), glow.map(|g| g as f32 * glowing))
    }

    /**
     * What a line through the middle of `site` of level `i` meets of it,
     * coming from direction `from` (`see`): what it stops and glows over
     * CROSSING crossings (fcc's `over`).
     */
    fn meets(&mut self, i: usize, site: [i64; 3], from: usize) -> ((Rgb, Rgb), Rgb) {
        // By what a channel stops: a power each, the same for every site, read often.
        static OVER: OnceLock<[f32; ALPHA as usize + 1]> = OnceLock::new();
        let table = OVER.get_or_init(|| std::array::from_fn(|k| over((k as f32 / ALPHA as f32) as f64, CROSSING) as f32));
        let ((stops, colour), glow) = self.see(i, site, from);
        let over = stops.map(|k| table[k as usize]);
        ((std::array::from_fn(|c| stops[c] as f32 / ALPHA as f32 * over[c]), colour), std::array::from_fn(|c| glow[c] * over[c]))
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

    /**
     * What is kept of site `p` of level `i`, with its tile, read or changed
     * by `with`: nothing yet, if it is begun here (Shared::begun).
     */
    fn site<T>(&mut self, i: usize, p: [i64; 3], with: impl FnOnce(&mut Site) -> T) -> T {
        let (value, begun) = {
            let mut site = self.levels[i].made(&self.world, &self.shared, i, p).site(p).lock().expect("site");
            let begun = site.is_none();
            (with(site.get_or_insert_default()), begun)
        };
        if begun {
            self.shared.begun();
        }
        value
    }

    /**
     * The light coming to site `p` of level `i` from every direction, as
     * it settles round by round (`round`): found once the light on its
     * faces changes by less than Shared's tolerance, in a round or two
     * where little reflects back, or after ROUNDS.
     */
    fn light(&mut self, i: usize, p: [i64; 3]) -> [Rgb; N] {
        if let Some(light) = self.site(i, p, |site| site.found.as_deref().copied()) {
            return light;
        }
        let (mut on, mut rounds) = (self.round(i, p, 0), 0);
        while rounds < ROUNDS {
            let next = self.round(i, p, rounds + 1);
            let settled = change(&on, &next) <= self.shared.tolerance;
            (on, rounds) = (next, rounds + 1);
            if settled {
                break;
            }
        }
        let paths = self.paths(i, p);
        let light = self.coming(i, p, rounds, &paths);
        self.site(i, p, |site| site.found = Some(Box::new(light)));
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
            for &(d, step, reflects) in &paths.reflects {
                let (d, q) = (d as usize, add(p, DIRECTIONS[d as usize].map(|v| v * step as i64)));
                let back = self.opposite[d];
                let on = match self.site(i, q, |site| site.rounds.get(n - 1).map(|on| on[back])) {
                    Some(on) => unpack(on),
                    None => self.round(i, q, n - 1)[back],
                };
                let reflects = unpack(reflects);
                (0..3).for_each(|c| light[d][c] += reflects[c] * on[c]);
            }
        }
        light
    }

    /**
     * The light on each face of site `p` of level `i` in round `n`
     * (`on_face`), from what comes to it then (`coming`): packed, the
     * same to whoever finds it and whoever reads it kept. Kept only of
     * a site that stops light: no site reads another's.
     */
    fn round(&mut self, i: usize, p: [i64; 3], n: usize) -> [Rgb; N] {
        if let Some(on) = self.site(i, p, |site| site.rounds.get(n).copied()) {
            return on.map(unpack);
        }
        let paths = self.paths(i, p);
        let light = self.coming(i, p, n, &paths);
        let on: [Packed; N] = std::array::from_fn(|f| pack(self.on_face(&light, &paths.touches, f)));
        if self.open(i, p) < 1.0 {
            // A site's rounds are asked for in turn: one whose earlier ones were forgotten since, or that another Light kept meanwhile, is not kept.
            self.site(i, p, |site| {
                if site.rounds.len() == n {
                    site.rounds.push(on);
                }
            });
        }
        on.map(unpack)
    }

    /** What comes to site `p` of level `i` (Paths), along each direction. */
    fn paths(&mut self, i: usize, p: [i64; 3]) -> Arc<Paths> {
        if let Some(paths) = self.site(i, p, |site| site.paths.clone()) {
            return paths;
        }
        let near = (i + 1 < self.levels.len()).then(|| self.near(i + 1, p));
        let whole = self.whole(i, p);
        let (mut reflects, mut touches): (_, Touches) = (std::mem::take(&mut self.reflecting), None);
        reflects.clear();
        let comes = std::array::from_fn(|d| {
            let q = add(p, DIRECTIONS[d]);
            if whole && self.whole(i, q) {
                touches.get_or_insert_with(|| Box::new([[0.0; 3]; N]))[d] = self.see(i, q, self.opposite[d]).0.1;
                return [0; 3];
            }
            pack(self.along(i, p, d, near.as_ref(), &mut reflects))
        });
        let paths = Arc::new(Paths { comes, reflects: reflects.as_slice().into(), touches });
        self.reflecting = reflects;
        self.site(i, p, |site| site.paths.insert(paths).clone())
    }

    /**
     * The light coming to `p`, a site of level `i`, from direction `d`,
     * all but reflected light: from the sites that way, what each glows;
     * past them, the coarser level's light, from the steps on, where it
     * has room for it (`past`). At the coarsest level (no `near`), from
     * the next `top_steps`, and past them the sky. The sites met that
     * reflect go in `reflects` (Paths): of what each stops, its colour.
     * Each has its own light found, so one that reflects under Shared's
     * tolerance is owed to the next that, with it, reflects over: that
     * one reflects for both, as if lit alike. What is still owed where
     * the line ends is left out.
     */
    fn along(&mut self, i: usize, p: [i64; 3], d: usize, near: Option<&Near>, reflects: &mut Vec<(u8, u8, Packed)>) -> Rgb {
        let (v, heights, steps) = (DIRECTIONS[d], self.levels[i].heights, self.shared.steps);
        // What has come so far, and how much of what comes from further on gets through.
        let (mut sum, mut through) = ([0.0f32; 3], [1.0f32; 3]);
        // What the sites met so far reflect, under the tolerance: none reflects for them yet.
        let mut owed = [0.0f32; 3];
        let mut q = p;
        let most = if near.is_some() { MOST_STEPS } else { top_steps(heights) };
        let beyond = 'march: {
            for step in 1..=most {
                let from = q;
                q = add(q, v);
                let ((alpha, colour), glow) = self.meets(i, q, self.opposite[d]);
                (0..3).for_each(|c| sum[c] += through[c] * glow[c]);
                (0..3).for_each(|c| owed[c] += through[c] * alpha[c] * colour[c]);
                if owed.iter().any(|&r| r > self.shared.tolerance) {
                    reflects.push((d as u8, step as u8, pack(std::mem::take(&mut owed))));
                }
                (0..3).for_each(|c| through[c] *= 1.0 - alpha[c]);
                // Less gets through than the tolerance, or nothing more is met: below every voxel and not going up, or above them and not going down.
                if through.iter().all(|&t| t < self.shared.tolerance) || q[1] < heights.0 - 1 && v[1] <= 0 {
                    break 'march [0.0; 3];
                }
                if q[1] > heights.1 && v[1] >= 0 {
                    break 'march self.sky[d];
                }
                if let Some(near) = near
                    && step >= steps
                    && let Some(past) = if step == steps { near[d] } else { self.past(i + 1, middle(from, q), d) }
                {
                    break 'march past;
                }
            }
            if near.is_none() && v[1] >= 0 { self.sky[d] } else { [0.0; 3] }
        };
        std::array::from_fn(|c| sum[c] + through[c] * beyond[c])
    }

    /**
     * Level `i`'s light from direction `d` at `at` (`reads`), if it has
     * room for light there, its site nearest empty. Where it fills what
     * the finer level leaves empty (a room whose walls, grown thick, close
     * it), the open sites near are not what lies past there.
     */
    fn past(&mut self, i: usize, at: [f64; 3], d: usize) -> Option<Rgb> {
        let mut reads = reads(at).peekable();
        let &(nearest, _) = reads.peek().expect("the nearest site");
        if self.open(i, nearest) <= 0.0 {
            return None;
        }
        let (mut sum, mut total) = ([0.0f32; 3], 0.0f32);
        for (site, w) in reads {
            let open = self.open(i, site);
            if open > 0.0 {
                let light = self.light(i, site)[d];
                total += w * open;
                (0..3).for_each(|c| sum[c] += w * open * light[c]);
            }
        }
        Some(sum.map(|c| c / total))
    }

    /** `along`'s readings of level `i` around `p`, a site of level `i − 1`, as `past` would give them, read at once from the sites around (`around`). */
    fn near(&mut self, i: usize, p: [i64; 3]) -> Near {
        let (class, base) = class(p);
        let (sites, nearest) = &around(self.shared.steps)[class];
        let (mut past, mut totals) = ([[0.0f32; 3]; N], [0.0f32; N]);
        for (offset, weights) in sites {
            let site = add(base, *offset);
            let open = self.open(i, site);
            if open <= 0.0 {
                continue;
            }
            let light = self.light(i, site);
            for d in 0..N {
                let w = weights[d] * open;
                totals[d] += w;
                (0..3).for_each(|c| past[d][c] += w * light[d][c]);
            }
        }
        let mean = |sum: Rgb, total: f32| if total > 0.0 { sum.map(|c| c / total) } else { sum };
        std::array::from_fn(|d| (self.open(i, add(base, nearest[d])) > 0.0).then(|| mean(past[d], totals[d])))
    }

    /** Over π, the light on face `f` from `light`, its directions weighed by their cosines (`weighed`). */
    fn on_face(&self, light: &[Rgb; N], touches: &Touches, f: usize) -> Rgb {
        weighed(&self.cosines[f], light, touches)
    }

    /** Over π, the light on a surface facing `normal` from `light`, as on a face (`on_face`). */
    fn on(&self, light: &[Rgb; N], touches: &Touches, normal: [f64; 3]) -> Rgb {
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

    /** The light from each face of the voxel at `site` of level `i`, on its surface. */
    fn lit(&mut self, i: usize, site: [i64; 3]) -> FaceLight {
        let (light, paths) = (self.light(i, site), self.paths(i, site));
        std::array::from_fn(|f| {
            let on = self.on_face(&light, &paths.touches, f);
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
        let (light, paths) = (self.light(i, site), self.paths(i, site));
        let touches = &paths.touches;
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
fn weighed(weights: &[f32; N], light: &[Rgb; N], touches: &Touches) -> Rgb {
    let (mut sum, mut kept) = ([0.0f32; 3], [1.0f32; 3]);
    for (d, (w, light)) in weights.iter().zip(light).enumerate() {
        (0..3).for_each(|c| sum[c] += w * light[c]);
        if let Some(touches) = touches {
            (0..3).for_each(|c| kept[c] -= w * touches[d][c]);
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
fn change(before: &[Rgb; N], after: &[Rgb; N]) -> f32 {
    let mut most = 0.0f32;
    for (b, a) in before.iter().zip(after) {
        (0..3).filter(|&c| a[c] > 0.0).for_each(|c| most = most.max((a[c] - b[c]).abs() / a[c]));
    }
    most
}

fn add(a: [i64; 3], b: [i64; 3]) -> [i64; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
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
 * `reads` at the places `near` reads around a site, by its class: as
 * offsets from the class's base, each site's weights at the N places past
 * the steps along each direction; and the site nearest each.
 */
type Around = (Vec<([i64; 3], [f32; N])>, [[i64; 3]; N]);

/** A site's class, of the 8 that the sites around its centre come in (`around`), and the base they are from: half of it, rounded down. */
fn class(p: [i64; 3]) -> (usize, [i64; 3]) {
    let base = p.map(|c| c.div_euclid(2));
    // Its last coordinate's parity follows from the others'.
    let class = (p[0] & 1) | (p[1] & 1) << 1 | ((base[0] + base[1] + base[2]) & 1) << 2;
    (class as usize, base)
}

/** `Around` for each class, taking `steps` steps. */
fn around(steps: usize) -> &'static [Around; 8] {
    static AROUND: [OnceLock<[Around; 8]>; MOST_STEPS + 1] = [const { OnceLock::new() }; MOST_STEPS + 1];
    AROUND[steps].get_or_init(|| {
        std::array::from_fn(|class| {
            let (c, b) = (class as i64 & 3, class as i64 >> 2);
            // A site of the class: its base (b, 0, 0), its parities those of the class.
            let p = [2 * b + (c & 1), c >> 1, ((c & 1) + (c >> 1)) & 1];
            let base = [b, 0, 0];
            assert_eq!(self::class(p), (class, base));
            let along = |d: usize, steps: usize| add(p, DIRECTIONS[d].map(|v| v * steps as i64));
            let at: [[f64; 3]; N] = std::array::from_fn(|k| middle(along(k, steps - 1), along(k, steps)));
            let offset = |site: [i64; 3]| add(site, base.map(|c| -c));
            let mut sites = BTreeMap::new();
            for (k, at) in at.iter().enumerate() {
                for (site, w) in reads(*at) {
                    sites.entry(offset(site)).or_insert([0.0; N])[k] = w;
                }
            }
            let nearest = std::array::from_fn(|d| offset(nearest_site(at[d])));
            (sites.into_iter().collect(), nearest)
        })
    })
}

/** Where the middle of sites `a` and `b` of a level is at the next coarser one. */
fn middle(a: [i64; 3], b: [i64; 3]) -> [f64; 3] {
    std::array::from_fn(|c| (a[c] + b[c]) as f64 / 4.0)
}

fn sub(a: [f64; 3], b: [i64; 3]) -> [f64; 3] {
    [a[0] - b[0] as f64, a[1] - b[1] as f64, a[2] - b[2] as f64]
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
