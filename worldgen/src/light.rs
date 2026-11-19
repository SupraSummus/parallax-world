//! Light on the lattices 2^k·FCC: a prototype, for look's pictures and
//! native's `--light`, whose bakers ask for the light from the faces of
//! the voxels they show (fcc's light.rs).
//!
//! Light comes to a site from its 12 neighbours' directions: along each,
//! from the sites of its level, till the next coarser level can take over:
//! after a few steps (Shared's, STEPS unless set), where the coarser level
//! has room for light (`past`). So a wall thinner than about its distance
//! over the steps lets light through: more steps, less leaks.
//! Each site sends what it glows, and of what it stops (its alpha) its
//! colour times the light on its face turned back (`on_face`), a third of
//! what it shows that way; the rest of what comes from past it gets
//! through. So what comes from far comes from coarser levels, the
//! coarsest's from the sky.
//!
//! The light on a site is what the sites near it reflect of the light on
//! them, so it settles round by round: round 0 has what glows, the sky and
//! the coarser level's light, each next round what the sites met reflect
//! of theirs in the round before; found once it changes by less than a
//! tolerance (Shared's, TOLERANCE unless set): in a round or two in the
//! open, in more where light bounces back and forth, as in a closed white
//! room. Between whole voxels that touch no light comes: one beside a
//! face is lit as the face is (`weighed`).
//!
//! Light sees each site as it is (VoxelGrid::light): what its pieces and
//! its medium stop of its rays, a share, where hosts are shown its pieces
//! as a whole voxel or nothing (VoxelGrid::show). So the coarse sites
//! around a small lamp glow as bright as the lamp, on average over their
//! faces, and those small things fill in part stop as much as the things
//! do. What fills a site in part, a line through its middle meets more
//! often than its share says (picture's CROSSING, `meets`).
//! A voxel hosts show behind pieces they show nothing of is lit as if
//! those pieces were not there (`shown`): the ray that shows it met none.

use std::collections::BTreeMap;
use std::rc::Rc;
use std::sync::{Arc, Mutex, OnceLock};

use fcc::light::{FaceLight, linear_to_srgb, srgb_to_linear};
use fcc::{GRID_LEVELS, Look};

use crate::World;
use crate::generator::{Generator, generate_shown};
use crate::grid::{ALPHA, BRICK, Box3, COLOUR_ONE, GLOW_ONE, Grid, VoxelGrid};
use crate::picture::{CROSSING, FACES, Picture, nearest_site, over};
use crate::recent::Recent;

type Rgb = [f32; 3];

const N: usize = 12;

/** The directions light comes along: towards the neighbours. A voxel's face is across each, numbered alike. */
const DIRECTIONS: [[i64; 3]; N] = FACES;

/** Steps along a direction before the coarser level's light may take over, unless set (Shared::stepping), and at most: at the coarsest, before the sky's does. */
const STEPS: usize = 1;
const TOP_STEPS: usize = 64;
/** How little the light on a site's faces may change, of what it is, for it to be found, unless set (Shared::settling); and rounds at most, should it never settle (a closed room that reflects all). */
const TOLERANCE: f32 = 1e-2;
const ROUNDS: usize = 256;
/** Sites a tile is a side, at each level, as native's (tile_box). */
pub const TILE: i64 = 128;
/** Sites a block is a side (Shared): small, as light reads a few sites around what it lights. */
const BLOCK: i64 = 32;
/** Sites whose paths and light a Light keeps at each level, and as many used before them (Recent): the sites around read them, bricks side by side again. */
const KEPT_SITES: usize = 1 << 13;
/**
 * Sites whose rounds a Light keeps, of every level together, likewise: a site's rounds are read by every site it
 * reflects on, round after round. No fewer than a brick's light reads (the forest's, some thousands): a round
 * forgotten is found again from those before it, each forgotten too, and lighting all but stops.
 */
const KEPT_ROUNDS: usize = 1 << 14;

/** A grid made once: those that want it while it is made wait. */
type Made = Arc<OnceLock<Arc<VoxelGrid>>>;
/** Grids by level and key. */
type Grids = Map<(usize, [i64; 3]), Made>;

/**
 * What the Lights of one world under one brightness, and taking as many
 * steps along each level, share (a thread's each): the world's blocks,
 * BLOCK sites a side, of every level, generated once by whichever Light
 * needs one first, kept till the host forgets them (`forget`). Not the
 * host's tiles (make_tile), which keep of a site only what is drawn.
 * And all that is kept of the coarsest level's sites (`top`): no coarser
 * level takes over there, so a site's light reads more sites than a Light
 * keeps, as far as light goes in its rounds, the same for all that is lit.
 */
#[derive(Clone)]
pub struct Shared {
    brightness: Brightness,
    steps: usize,
    tolerance: f32,
    blocks: Arc<Mutex<Grids>>,
    top: Arc<Mutex<Top>>,
}

/** What is kept of the coarsest level's sites, all of it, till forgotten (Shared::forget). */
struct Top {
    kept: Kept,
    rounds: Rounds,
}

impl Default for Top {
    fn default() -> Top {
        Top { kept: Kept::new(usize::MAX), rounds: Recent::new(usize::MAX) }
    }
}

impl Shared {
    /** Nothing made yet, for a world under `brightness`. */
    pub fn new(brightness: Brightness) -> Shared {
        Shared { brightness, steps: STEPS, tolerance: TOLERANCE, blocks: Arc::default(), top: Arc::default() }
    }

    pub fn brightness(&self) -> Brightness {
        self.brightness
    }

    /** The same world under `brightness`: its blocks kept. */
    pub fn under(&self, brightness: Brightness) -> Shared {
        Shared { brightness, top: Arc::default(), ..self.clone() }
    }

    /** Steps along a direction before the coarser level's light may take over. */
    pub fn steps(&self) -> usize {
        self.steps
    }

    /** The same world, light taking `steps` steps along each level before the coarser's: its blocks kept. */
    pub fn stepping(&self, steps: usize) -> Shared {
        assert!((1..=TOP_STEPS).contains(&steps), "{steps} steps");
        Shared { steps, top: Arc::default(), ..self.clone() }
    }

    /** How little the light on a site's faces may change, of what it is, for it to be found. */
    pub fn tolerance(&self) -> f32 {
        self.tolerance
    }

    /** The same world, a site's light found once the light on its faces changes by less than `tolerance` of what it is: its blocks kept. */
    pub fn settling(&self, tolerance: f32) -> Shared {
        assert!(tolerance > 0.0, "tolerance {tolerance}");
        Shared { tolerance, top: Arc::default(), ..self.clone() }
    }

    /** Whether `other` lights the world as this does: as bright, as many steps, as little change. */
    pub fn lights_as(&self, other: &Shared) -> bool {
        (self.brightness, self.steps, self.tolerance) == (other.brightness, other.steps, other.tolerance)
    }

    /** The block a Light reads `site` of `level` from, and the box it holds, made if not yet. */
    fn holding(&self, generator: &Generator, level: u32, heights: (i64, i64), site: [i64; 3]) -> (Box3, Arc<VoxelGrid>) {
        let key = site.map(|v| v.div_euclid(BLOCK));
        let block = self.blocks.lock().expect("blocks").entry((level as usize, key)).or_default().clone();
        let b = cube_box(heights, key, BLOCK);
        (b, block.get_or_init(|| Arc::new(generate_shown(generator, &b, level))).clone())
    }

    /**
     * Forgets the blocks in the tiles `far` says are far, by level and key,
     * to make again if asked; and, when one of the coarsest level goes,
     * what is kept of that level's sites there: seldom, as a host asks
     * every frame, and they are many.
     */
    pub fn forget(&self, far: impl Fn(usize, [i64; 3]) -> bool) {
        let top = GRID_LEVELS - 1;
        let mut gone = false;
        self.blocks.lock().expect("blocks").retain(|&(level, key), _| {
            let far = far(level, key.map(|k| k.div_euclid(TILE / BLOCK)));
            gone |= far && level == top;
            !far
        });
        if gone {
            let near = |site: &[i64; 3]| !far(top, site.map(|v| v.div_euclid(TILE)));
            let mut kept = self.top.lock().expect("light");
            kept.kept.paths.retain(near);
            kept.kept.found.retain(near);
            kept.rounds.retain(|(_, site)| near(site));
        }
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

/** A level's blocks, as they are needed, and the light found at its sites. */
struct Level {
    heights: (i64, i64),
    /** The block read last, and the box it holds: most reads are in it. */
    last: Option<(Box3, Arc<VoxelGrid>)>,
    /** The coarsest level's are Shared's, here while its light is found (`light`). */
    kept: Kept,
}

/** What is kept of a level's sites, of each the ones used last (KEPT_SITES). */
struct Kept {
    /** What comes to sites. */
    paths: Recent<[i64; 3], Arc<Paths>>,
    /** The light coming to sites from each direction, settled. */
    found: Recent<[i64; 3], [Rgb; N]>,
}

impl Kept {
    fn new(sites: usize) -> Kept {
        Kept { paths: Recent::new(sites), found: Recent::new(sites) }
    }
}

/** By level and site, the light coming to it from each direction in each round (`round`), from the first on. */
type Rounds = Recent<(usize, [i64; 3]), Vec<[Rgb; N]>>;

/**
 * What comes to a site and does not change as its light settles (`along`):
 * from each direction, all but reflected light; the sites met that
 * reflect, each with its direction, how many steps along it, and the share
 * of the light on its face that comes; and, for a whole voxel, the colours of the whole voxels it
 * touches (`weighed`).
 */
struct Paths {
    comes: [Rgb; N],
    reflects: Box<[(u8, u8, Rgb)]>,
    touches: Touches,
}

/** Per direction, the colour of the whole voxel a whole voxel touches that way; none if it touches none. */
type Touches = [Rgb; N];

pub struct Light {
    world: Rc<World>,
    shared: Shared,
    /** By level, up to a renderer's coarsest grid's (GRID_LEVELS). */
    levels: Vec<Level>,
    /** KEPT_ROUNDS; the coarsest level's are Shared's, as its `kept`. */
    rounds: Rounds,
    /** `paths`'s sites that reflect, while it finds them: its room kept from one site to the next. */
    reflecting: Vec<(u8, u8, Rgb)>,
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
    /** The light of `world`, found only where asked, its blocks in `shared`. */
    pub fn new(world: impl Into<Rc<World>>, shared: Shared) -> Light {
        let world = world.into();
        let Brightness { sky, sun, .. } = shared.brightness;
        let coarsest = GRID_LEVELS as u32 - 1;
        let levels =
            (0..=coarsest).map(|level| Level { heights: world.heights_at(level), last: None, kept: Kept::new(KEPT_SITES) }).collect();
        let unit = DIRECTIONS.map(|d| normalize(d.map(|v| v as f64)));
        let cosines = unit.map(|normal| {
            let weights = unit.map(|u| dot(u, normal).max(0.0));
            let total: f64 = weights.iter().sum();
            weights.map(|w| (w / total) as f32)
        });
        let opposite = DIRECTIONS.map(|d| DIRECTIONS.iter().position(|f| *f == d.map(|c| -c)).expect("directions come in pairs"));
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
        Light {
            world,
            shared,
            levels,
            rounds: Recent::new(KEPT_ROUNDS),
            reflecting: Vec::new(),
            unit,
            cosines,
            opposite,
            glow,
            sky: skies,
            background,
        }
    }

    /** Whether a whole voxel, one that lets nothing through, is at `site` of level `i`. */
    fn whole(&mut self, i: usize, site: [i64; 3]) -> bool {
        self.open(i, site) <= 0.0
    }

    /** Whether hosts are shown a voxel at `site` of level `i` (VoxelGrid::show). */
    fn drawn(&mut self, i: usize, site: [i64; 3]) -> bool {
        self.hold(i, site).is_some_and(|([x, y, z], grid)| grid.get(x, y, z) != 0)
    }

    /**
     * How light sees `site` of level `i` from direction `from`: how much of
     * it stops (alpha), its pieces and its medium, and their colour; what
     * it glows.
     */
    fn see(&mut self, i: usize, site: [i64; 3], from: usize) -> ((f32, Rgb), Rgb) {
        let glowing = self.glow;
        let Some(([x, y, z], grid)) = self.hold(i, site) else { return ((0.0, [0.0; 3]), [0.0; 3]) };
        let ([(pieces, p), (medium, m)], glow) = grid.light(x, y, z, from);
        let alpha = (u32::from(pieces) + u32::from(medium)) as f32 / ALPHA as f32;
        // Their colours come premultiplied, of COLOUR_ONE: as a share of the light on what it stops.
        let colour = std::array::from_fn(|c| if alpha > 0.0 { ((p[c] + m[c]) as f32 / COLOUR_ONE as f32 / alpha).min(1.0) } else { 0.0 });
        ((alpha, colour), glow.map(|g| g as f32 * glowing))
    }

    /**
     * What a line through the middle of `site` of level `i` meets of it,
     * coming from direction `from` (`see`): what it stops and glows over
     * CROSSING crossings (picture's `over`).
     */
    fn meets(&mut self, i: usize, site: [i64; 3], from: usize) -> ((f32, Rgb), Rgb) {
        let ((alpha, colour), glow) = self.see(i, site, from);
        let over = over(alpha as f64, CROSSING) as f32;
        ((alpha * over, colour), glow.map(|g| g * over))
    }

    /** How much of what comes through `site` of level `i` it lets through. */
    fn open(&mut self, i: usize, site: [i64; 3]) -> f32 {
        let Some(([x, y, z], grid)) = self.hold(i, site) else { return 1.0 };
        1.0 - grid.alpha(x, y, z) as f32 / ALPHA as f32
    }

    /** The block of level `i` that holds `site`, from those shared (Shared::holding); none above or below every voxel. */
    fn hold(&mut self, i: usize, site: [i64; 3]) -> Option<([i64; 3], &VoxelGrid)> {
        let level = &mut self.levels[i];
        if !(level.heights.0..level.heights.1).contains(&site[1]) {
            return None;
        }
        if level.last.as_ref().is_none_or(|(b, _)| !b.holds(site)) {
            level.last = Some(self.shared.holding(&self.world.generator, i as u32, level.heights, site));
        }
        Some((site, &level.last.as_ref().expect("held").1))
    }

    /**
     * The light coming to site `p` of level `i` from every direction, as
     * it settles round by round (`round`): found once it changes by less
     * than Shared's tolerance, in a round or two where little reflects
     * back, in many in a closed white room.
     */
    fn light(&mut self, i: usize, p: [i64; 3]) -> [Rgb; N] {
        if i + 1 < self.levels.len() {
            return self.settled(i, p);
        }
        // The coarsest level's sites are Shared's: this Light's while it finds the light of one.
        let top = self.shared.top.clone();
        let mut top = top.lock().expect("light");
        let mut swap = |light: &mut Light| {
            std::mem::swap(&mut top.kept, &mut light.levels[i].kept);
            std::mem::swap(&mut top.rounds, &mut light.rounds);
        };
        swap(self);
        let light = self.settled(i, p);
        swap(self);
        light
    }

    /** `light`, from the sites the level keeps. */
    fn settled(&mut self, i: usize, p: [i64; 3]) -> [Rgb; N] {
        if let Some(light) = self.levels[i].kept.found.get(&p) {
            return *light;
        }
        let touches = self.paths(i, p).touches;
        let mut light = self.round(i, p, 0);
        for n in 1..=ROUNDS {
            let next = self.round(i, p, n);
            let settled = self.change(&light, &next, &touches) <= self.shared.tolerance;
            light = next;
            if settled {
                break;
            }
        }
        self.levels[i].kept.found.insert(p, light);
        light
    }

    /**
     * The light coming to site `p` of level `i` from every direction in
     * round `n`: what comes (`paths`), and what the sites met reflect of
     * the light on them in the round before; none in round 0.
     */
    fn round(&mut self, i: usize, p: [i64; 3], n: usize) -> [Rgb; N] {
        if let Some(light) = self.rounds.get(&(i, p)).and_then(|rounds| rounds.get(n)) {
            return *light;
        }
        let paths = self.paths(i, p);
        let mut light = paths.comes;
        if n > 0 {
            for &(d, step, reflects) in &paths.reflects {
                let (d, q) = (d as usize, add(p, DIRECTIONS[d as usize].map(|v| v * step as i64)));
                let (before, touches) = (self.round(i, q, n - 1), self.paths(i, q).touches);
                let on = self.on_face(&before, &touches, self.opposite[d]);
                (0..3).for_each(|c| light[d][c] += reflects[c] * on[c]);
            }
        }
        // A site's rounds are asked for in turn; where the earlier ones were forgotten since, this one is not kept.
        match self.rounds.get(&(i, p)) {
            Some(rounds) if rounds.len() == n => rounds.push(light),
            None if n == 0 => self.rounds.insert((i, p), vec![light]),
            _ => {}
        }
        light
    }

    /** What comes to site `p` of level `i` (Paths), along each direction. */
    fn paths(&mut self, i: usize, p: [i64; 3]) -> Arc<Paths> {
        if let Some(paths) = self.levels[i].kept.paths.get(&p) {
            return paths.clone();
        }
        let near = (i + 1 < self.levels.len()).then(|| self.near(i + 1, p));
        let whole = self.whole(i, p);
        let (mut reflects, mut touches) = (std::mem::take(&mut self.reflecting), [[0.0; 3]; N]);
        reflects.clear();
        let comes = std::array::from_fn(|d| {
            let q = add(p, DIRECTIONS[d]);
            if whole && self.whole(i, q) {
                touches[d] = self.see(i, q, self.opposite[d]).0.1;
                return [0.0; 3];
            }
            self.along(i, p, d, near.as_ref(), &mut reflects)
        });
        let paths = Arc::new(Paths { comes, reflects: reflects.as_slice().into(), touches });
        self.reflecting = reflects;
        self.levels[i].kept.paths.insert(p, paths.clone());
        paths
    }

    /**
     * The light coming to `p`, a site of level `i`, from direction `d`,
     * all but reflected light: from the sites that way, what each glows;
     * past them, the coarser level's light, from the steps on, where it
     * has room for it (`past`). At the coarsest level (no `near`), from
     * the next TOP_STEPS, and past them the sky. The sites met that
     * reflect go in `reflects` (Paths): of what each stops, its colour.
     */
    fn along(&mut self, i: usize, p: [i64; 3], d: usize, near: Option<&Near>, reflects: &mut Vec<(u8, u8, Rgb)>) -> Rgb {
        let (v, heights, steps) = (DIRECTIONS[d], self.levels[i].heights, self.shared.steps);
        // What has come so far, and how much of what comes from further on gets through.
        let (mut sum, mut through) = ([0.0f32; 3], 1.0f32);
        let mut q = p;
        let beyond = 'march: {
            for step in 1..=TOP_STEPS {
                let from = q;
                q = add(q, v);
                let ((alpha, colour), glow) = self.meets(i, q, self.opposite[d]);
                (0..3).for_each(|c| sum[c] += through * glow[c]);
                if alpha > 0.0 {
                    reflects.push((d as u8, step as u8, colour.map(|c| through * alpha * c)));
                }
                through *= 1.0 - alpha;
                // Nothing more gets through, or nothing more is met: below every voxel and not going up, or above them and not going down.
                if through < 1.0 / ALPHA as f32 || q[1] < heights.0 - 1 && v[1] <= 0 {
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
        std::array::from_fn(|c| sum[c] + through * beyond[c])
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

    /** How much a site's light changed from `before` to `after`, of what it is: the most of any channel of the light on any face (`on_face`). */
    fn change(&self, before: &[Rgb; N], after: &[Rgb; N], touches: &Touches) -> f32 {
        let mut most = 0.0f32;
        for f in 0..N {
            let (b, a) = (self.on_face(before, touches, f), self.on_face(after, touches, f));
            (0..3).filter(|&c| a[c] > 0.0).for_each(|c| most = most.max((a[c] - b[c]).abs() / a[c]));
        }
        most
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

    /**
     * The light coming to the voxel hosts show at `site` of level `i`: its
     * site's (`light`), but through a neighbour whose pieces they show
     * nothing of, what comes to that neighbour. A ray that shows the voxel
     * through them met none of them.
     */
    fn shown(&mut self, i: usize, site: [i64; 3]) -> [Rgb; N] {
        let mut light = self.light(i, site);
        for d in 0..N {
            let q = add(site, DIRECTIONS[d]);
            let from = self.opposite[d];
            let pieces = self.hold(i, q).is_some_and(|([x, y, z], grid)| grid.light(x, y, z, from).0[0].0 > 0);
            if pieces && !self.drawn(i, q) {
                light[d] = self.light(i, q)[d];
            }
        }
        light
    }

    /** The light from each face of the voxel at `site` of level `i`, on its surface. */
    fn lit(&mut self, i: usize, site: [i64; 3]) -> FaceLight {
        let (light, touches) = (self.shown(i, site), self.paths(i, site).touches);
        std::array::from_fn(|f| {
            let on = self.on_face(&light, &touches, f);
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
        let (light, touches) = (self.shown(i, site), self.paths(i, site).touches);
        if normal == [0.0; 3] {
            // No surface, or all of it: the mean of all its faces'.
            let all = FACES.map(|f| self.on(&light, &touches, normalize(f.map(|v| v as f64))));
            return std::array::from_fn(|c| all.iter().map(|l| l[c]).sum::<f32>() / all.len() as f32);
        }
        self.on(&light, &touches, normalize(normal))
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
    [0, 8, 16].map(|shift| srgb_to_linear(((look >> shift) & 255) as f32 / 255.0))
}

/**
 * Tile `key`'s box: a cube TILE sites a side, holding x in
 * [key[0] · TILE, (key[0] + 1) · TILE), y and z likewise, cut to `heights`
 * rounded out to whole bricks, so that every tile's bricks are on one
 * grid; empty (no y) if it is all above or below them.
 */
pub fn tile_box(heights: (i64, i64), key: [i64; 3]) -> Box3 {
    cube_box(heights, key, TILE)
}

/** As tile_box, of cubes `side` sites a side (blocks too). */
fn cube_box((low, high): (i64, i64), key: [i64; 3], side: i64) -> Box3 {
    let [x, y, z] = key.map(|k| k * side);
    let low = low.div_euclid(BRICK) * BRICK;
    let high = (high + BRICK - 1).div_euclid(BRICK) * BRICK;
    let (y0, y1) = (low.clamp(y, y + side), high.clamp(y, y + side));
    Box3 { min: [x, y0, z], max: [x + side, y1.max(y0), z + side] }
}

/** Tile `key` of `level` of the world `generator` makes, of `heights`, as hosts make and draw it (generate_shown), see-through ones too. */
pub fn make_tile(generator: &Generator, level: u32, heights: (i64, i64), key: [i64; 3]) -> Grid {
    make_tiles(generator, level, heights, key, key[1]).remove(0)
}

/**
 * Tile `key` and those above it up to y = `last`: generated as one box
 * and parted, so what they share is made once (their fields' lines, the
 * things reaching several). Each shows as made alone; what is buried may
 * differ in shade, rock being of one from a box's bottom up.
 */
pub fn make_tiles(generator: &Generator, level: u32, heights: (i64, i64), key: [i64; 3], last: i64) -> Vec<Grid> {
    let boxes: Vec<Box3> = (key[1]..=last).map(|y| tile_box(heights, [key[0], y, key[2]])).collect();
    let all = Box3 { min: boxes[0].min, max: boxes[boxes.len() - 1].max };
    let grid = generate_shown(generator, &all, level).into_grid(true);
    // What was made may end short of a tile, or before it.
    let (low, high) = (grid.min[1] as i64, (grid.min[1] + grid.n[1]) as i64);
    let part = |b: &Box3| {
        let (y0, y1) = (b.min[1].clamp(low, high), b.max[1].clamp(low, high));
        grid.part([grid.min[0], y0 as i32, grid.min[2]], [grid.n[0], (y1 - y0) as i32, grid.n[2]])
    };
    boxes.iter().map(part).collect()
}

/**
 * The light on a face, its directions weighed by `weights` (summing to 1).
 * A voxel the face touches is taken to be lit as the face is, so it sends
 * back its colour times that light: the light on the face is what comes
 * along the other directions, over what the touched voxels do not send.
 */
fn weighed(weights: &[f32; N], light: &[Rgb; N], touches: &Touches) -> Rgb {
    let (mut sum, mut kept) = ([0.0f32; 3], [1.0f32; 3]);
    for ((w, light), touch) in weights.iter().zip(light).zip(touches) {
        (0..3).for_each(|c| {
            sum[c] += w * light[c];
            kept[c] -= w * touch[c];
        });
    }
    std::array::from_fn(|c| if kept[c] > 0.0 { sum[c] / kept[c] } else { 0.0 })
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
    static AROUND: [OnceLock<[Around; 8]>; TOP_STEPS + 1] = [const { OnceLock::new() }; TOP_STEPS + 1];
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
mod tests {
    use super::*;
    use crate::generator::{Field, Generator, KindDef, Layer, Node, Op, Population, SOLID, cuboid};
    use fcc::light::colour;

    #[test]
    fn tiles_are_cubes_cut_to_the_heights_at_bricks() {
        let heights = (-70, 140);
        let cube = |key: [i64; 3]| Box3 { min: key.map(|k| k * TILE), max: key.map(|k| (k + 1) * TILE) };
        assert_eq!(tile_box(heights, [2, 1, -3]), Box3 { max: [3 * TILE, 144, -2 * TILE], ..cube([2, 1, -3]) });
        assert_eq!(tile_box(heights, [0, -1, 0]), Box3 { min: [0, -72, 0], ..cube([0, -1, 0]) });
        assert_eq!(tile_box((-500, 500), [1, 2, 3]), cube([1, 2, 3]), "whole within them");
        for key in [[0, 2, 0], [0, -2, 0]] {
            let b = tile_box(heights, key);
            assert!(b.min[1] == b.max[1], "{key:?}: none of it in the heights");
        }
    }

    const STONE: u8 = 0;
    const LAMP_KIND: u8 = 1;
    /** Lamps are cubes 4 units wide, as the towns' are: many sites, its glow on those inside. */
    const LAMP: f64 = 4.0;

    /**
     * Stone where `stone(x, y, z, spacing)` says (the sites' spacing), and white lamps at
     * `lamps`, as the towns' are; from y = −64 up to what the coarsest
     * levels' sites hold.
     */
    fn world(lamps: &[[f64; 3]], stone: impl Fn(i64, i64, i64, i64) -> bool + 'static) -> World {
        world_of(&["#808080"], lamps, stone)
    }

    /** As `world`, its stone coloured `colour`. */
    fn world_of(colour: &'static [&'static str], lamps: &[[f64; 3]], stone: impl Fn(i64, i64, i64, i64) -> bool + 'static) -> World {
        world_with(colour, lamps, &[], stone)
    }

    /** As `world_of`, with blocks of stone the lamps' size at `blocks`. */
    fn world_with(
        colour: &'static [&'static str],
        lamps: &[[f64; 3]],
        blocks: &[[f64; 3]],
        stone: impl Fn(i64, i64, i64, i64) -> bool + 'static,
    ) -> World {
        let kinds = vec![
            KindDef { name: "stone", kind_type: 1, colors: colour, walk_through: false, glows: false, medium: false },
            KindDef { name: "lamp", kind_type: 6, colors: &["#ffffff"], walk_through: false, glows: true, medium: false },
        ];
        let cube = |p: &[f64; 3], kind| cuboid(p.map(|c| c - LAMP / 2.0), p.map(|c| c + LAMP / 2.0), kind, SOLID, Op::Fill);
        let shapes: Vec<_> = lamps.iter().map(|p| cube(p, LAMP_KIND)).chain(blocks.iter().map(|p| cube(p, STONE))).collect();
        let lamps = Population::new("lamps", 1.0, 1024)
            .reach(1024)
            .anchor(|cell| (cell.centre == [0; 3]).then_some([0.0; 3]))
            .make(move |_, _| Some(Node::of(shapes.clone())));
        let ground = Field::at("stone", 0.0, Op::Fill, move |x, y, z| stone(x, y, z, crate::plan::spacing()).then_some((STONE, None)));
        let layers = vec![Layer::Field(ground), Layer::Population(lamps)];
        World { generator: Generator { kinds, seed: 1, layers }, spawn: [0; 3], heights: Some((-64, 1024)), points: Vec::new() }
    }

    /** A site on top of ground below y = 0: at y = −1, x + z odd. */
    fn ground(x: i64, z: i64) -> [i64; 3] {
        assert!((x + z) % 2 != 0, "({x}, {z}) is not on top");
        [x, -1, z]
    }

    #[test]
    fn walls_keep_the_sky_out() {
        // A closed box whose walls are one site thick at every level, as generators draw sheets.
        let world = world(&[], |x, y, z, s| {
            let wall = |c: i64| (48..48 + s).contains(&c.abs());
            let within = |c: i64| c.abs() < 48 + s;
            y < 0 || within(x) && within(z) && (0..48 + s).contains(&y) && (wall(x) || wall(z) || y >= 48)
        });
        let mut light = Light::new(world, Shared::new(Brightness { sky: 1.0, sun: 1.0, glow: 0.0 }));
        let inside = light.on_voxel(0, ground(1, 0));
        assert!(inside.iter().all(|c| *c < 0.02), "inside {inside:?}");
    }

    #[test]
    fn ground_seen_through_pieces_not_shown_is_lit_as_in_the_open() {
        // Blocks a quarter of a site of level 3 across, on the ground: pieces of their sites, some not shown.
        let blocks: Vec<[f64; 3]> = (0..64).map(|k| [16.0 * k as f64, 4.0, 0.0]).collect();
        let world = world_with(&["#808080"], &[], &blocks, |_, y, _, _| y < 0);
        let mut light = Light::new(world, Shared::new(Brightness { sky: 1.0, sun: 1.0, glow: 0.0 }));
        let open = light.on_voxel(3, ground(-9, 0))[0];
        let mut under = 0;
        for site in (0..128).map(|k| ground(2 * k + 1, 0)) {
            let above = FACES.map(|f| add(site, f)).into_iter().filter(|q| q[1] == 0);
            // Not beside a block shown: that one does shade it.
            if above.clone().any(|q| light.open(3, q) < 1.0) && !above.clone().any(|q| light.drawn(3, q)) {
                under += 1;
                let lit = light.on_voxel(3, site)[0];
                // Less by what the blocks around stop.
                assert!((lit / open - 1.0).abs() < 0.08, "at {site:?} {lit}, in the open {open}");
            }
        }
        assert!(under > 0, "no ground under pieces not shown");
    }

    #[test]
    fn a_lamp_glows_as_much_at_every_level_it_is_smaller_than_a_site() {
        // Its glow is its share of its site, which is its area over the site's: times the site's area, the same.
        let world = world(&[[5.0, 31.0, 6.0]], |_, _, _, _| false);
        let glows: Vec<f64> = (3..=8)
            .map(|level| {
                let grid = crate::generator::generate(&world.generator, &Box3 { min: [-8; 3], max: [8; 3] }, level);
                grid.glow.values().map(|g| g[0] as f64).sum::<f64>() * (1u64 << (2 * level)) as f64
            })
            .collect();
        assert!(glows.iter().all(|g| (g / glows[0] - 1.0).abs() < 0.1), "glow times the site's area, by level from 3: {glows:?}");
    }

    #[test]
    fn a_closed_room_keeps_its_lamps_light_in() {
        // A box whose walls are one site thick at every level, a lamp inside, and the ground outside.
        let room = |x: i64, y: i64, z: i64, s: i64| {
            let wall = |c: i64| (16..16 + s).contains(&c.abs());
            let within = |c: i64| c.abs() < 16 + s;
            y < 0 || within(x) && within(z) && (0..32 + s).contains(&y) && (wall(x) || wall(z) || y >= 32)
        };
        let outside = |closed: bool| {
            let world = world(&[[0.0, 24.0, 0.0]], move |x, y, z, s| if closed { room(x, y, z, s) } else { y < 0 });
            let mut light = Light::new(world, Shared::new(Brightness { sky: 0.0, sun: 0.0, glow: 1.0 }));
            light.on_voxel(0, ground(25, 0))[0]
        };
        let (open, closed) = (outside(false), outside(true));
        assert!(open > 0.0 && closed < 0.01 * open, "outside, open {open}, closed {closed}");
    }

    #[test]
    fn a_wall_is_lit_on_the_lamps_side() {
        // A wall one site thick across x, the lamp on its +x side.
        let world = world(&[[80.0, 10.0, 64.0]], |x, y, z, s| y < 0 || (64..64 + s).contains(&x) && y < 32 && (32..96).contains(&z));
        let light = Light::new(world, Shared::new(Brightness { sky: 0.0, sun: 0.0, glow: 1.0 })).faces(0, [64, 10, 64]);
        let side = |sign: i64| FACES.iter().zip(&light).filter(|(f, _)| f[0] == sign).map(|(_, l)| l[0]).sum::<f32>();
        assert!(side(1) > 100.0 * side(-1), "towards the lamp {}, away {}", side(1), side(-1));
    }

    /** A scene to hold Light to the reference: its world, how bright, the ground sites to probe, rays per face, and the bound on Light's error there. */
    struct Scene {
        name: String,
        world: World,
        brightness: Brightness,
        probes: Vec<[i64; 3]>,
        samples: u64,
        bound: f64,
    }

    fn scenes() -> Vec<Scene> {
        let day = Brightness { sky: 1.0, sun: 1.0, glow: 0.0 };
        let sky = Brightness { sky: 1.0, sun: 0.0, glow: 0.0 };
        let night = Brightness { sky: 0.0, sun: 0.0, glow: 1.0 };
        let scene =
            |name: &str, world, brightness, probes, samples, bound| Scene { name: name.into(), world, brightness, probes, samples, bound };
        let mut scenes = vec![
            scene("open ground", world(&[], |_, y, _, _| y < 0), sky, vec![ground(1, 0)], 4096, 0.1),
            scene(
                "wall, sun",
                world(&[], |x, y, z, s| y < 0 || (0..2 * s).contains(&x) && y < 32 && z.abs() < 96),
                day,
                [-9, -1, 3, 17, 29, 41].map(|x| ground(x, 0)).to_vec(),
                4096,
                0.2,
            ),
            scene(
                "lamp",
                world(&[[0.0, 24.0, 0.0]], |_, y, _, _| y < 0),
                night,
                vec![ground(1, 0), ground(16, 7), ground(33, 0)],
                1 << 16,
                0.75,
            ),
        ];
        // A roof 2 sites thick and 128 units wide at y = 24, turned about y, probed across it: the same light at every turn.
        for (turn, bound) in [(0.0, 0.7), (22.5, 0.35), (45.0, 0.4)] {
            let (sin, cos) = f64::to_radians(turn).sin_cos();
            let roof = world(&[], move |x, y, z, s| {
                let (u, v) = (cos * x as f64 + sin * z as f64, cos * z as f64 - sin * x as f64);
                y < 0 || u.abs() <= 64.0 && v.abs() <= 64.0 && (24..24 + 2 * s).contains(&y)
            });
            let probes = [0.0, 40.0, 56.0, 72.0, 96.0].map(|d: f64| ground_near(d * cos, d * sin)).to_vec();
            scenes.push(scene(&format!("roof turned {turn}°"), roof, sky, probes, 4096, bound));
        }
        // A yard walled 48 high, open to the sky, and past a wall with a low door a room roofed at 48: from 32 past the wall, the sky does not show through the door, so only light that bounced comes there.
        let yard = world(&[], |x, y, z, s| {
            let inside = x.abs() < 64 && z.abs() < 48 && y < 48;
            let shell = x.abs() < 64 + 2 * s && z.abs() < 48 + 2 * s && y < 48 + if x >= 0 { 2 * s } else { 0 };
            let wall = (0..2 * s).contains(&x) && !(z.abs() < 12 && y < 16);
            y < 0 || shell && (!inside || wall)
        });
        let probes = [-48.0, -24.0, 32.0, 44.0, 56.0].map(|x| ground_near(x, 0.0)).to_vec();
        scenes.push(scene("yard, past a wall", yard, sky, probes, 1 << 16, 1.1));
        // A tunnel 40 wide and high through a hill 384 long, under a roof 16 thick at every level, thinner than the coarse
        // levels' sites: probed along its floor, from its middle out past a portal. The sun is side on: none of its light comes in.
        let tunnel = world(&[], |x, y, z, _| y < 0 || z.abs() < 192 && y < 56 && !(x.abs() < 20 && y < 40));
        let probes = [0.0, 96.0, 160.0, 224.0].map(|z| ground_near(0.0, z)).to_vec();
        scenes.push(scene("tunnel under a thin roof", tunnel, day, probes, 1 << 16, 4.0));
        // Blocks the lamps' size strewn over the ground, two to a square 8 wide, from 24 to 40 up: whole voxels near, a
        // share of a site each far, stopping about two thirds of the sky's light.
        let strewn = |k: i64, v: i64| (crate::noise::hash(k as u64, v, 0, 0) % 1000) as f64 / 1000.0;
        let blocks: Vec<[f64; 3]> = (0..2 * 23 * 23)
            .map(|k| {
                [
                    8.0 * (k % 23 - 11) as f64 + 4.0 * strewn(1, k),
                    26.0 + 12.0 * strewn(2, k),
                    8.0 * (k / 23 % 23 - 11) as f64 + 4.0 * strewn(3, k),
                ]
            })
            .collect();
        let canopy = world_with(&["#808080"], &[], &blocks, |_, y, _, _| y < 0);
        scenes.push(scene("blocks over the ground", canopy, sky, [0.0, 24.0, 48.0].map(|x| ground_near(x, 0.0)).to_vec(), 4096, 0.1));
        // A closed room, white, a lamp in it: most of the light on its floor has bounced, many times.
        let room = world_of(&["#f0f0f0"], &[[0.0, 24.0, 0.0]], |x, y, z, s| {
            let wall = |c: i64| (32..32 + s).contains(&c.abs());
            let within = |c: i64| c.abs() < 32 + s;
            y < 0 || within(x) && within(z) && (0..48 + s).contains(&y) && (wall(x) || wall(z) || y >= 48)
        });
        let probes = vec![ground(1, 0), ground(16, 7), ground(29, 0), ground(28, 27)];
        scenes.push(scene("white room, lamp", room, night, probes, 1 << 16, 1.1));
        // A cave, a ball in rock the cheese's colour, a lamp by its wall: probed across its floor.
        let ball = |x: i64, y: i64, z: i64| x * x + (y - CAVE.1) * (y - CAVE.1) + z * z < CAVE.0 * CAVE.0;
        let cave = world_of(&["#f3d46c"], &[[0.0, 20.0, 20.0]], move |x, y, z, _| y < 80 && !ball(x, y, z));
        let probes = [0, 12, 24].map(|x| {
            let y = (0..CAVE.1).find(|&y| ball(x, y, 0)).expect("the cave's floor") - 1;
            [x + (x + y).rem_euclid(2), y, 0]
        });
        scenes.push(scene("yellow cave, lamp", cave, night, probes.to_vec(), 1 << 16, 0.95));
        scenes
    }

    /** The cave's radius and how high its middle is. */
    const CAVE: (i64, i64) = (32, 40);

    /** The ground site nearest (x, z). */
    fn ground_near(x: f64, z: f64) -> [i64; 3] {
        let (x, z) = (x.round() as i64, z.round() as i64);
        ground(x + (x + z + 1).rem_euclid(2), z)
    }

    /** What the open faces of the voxel at `site` (level 0) send out by Light, on average. */
    fn lit(light: &mut Light, site: [i64; 3]) -> Rgb {
        let open: Vec<usize> = (0..N).filter(|&f| !light.whole(0, add(site, FACES[f]))).collect();
        let faces = light.faces(0, site);
        std::array::from_fn(|c| open.iter().map(|&f| faces[f][c]).sum::<f32>() / open.len().max(1) as f32)
    }

    /** Where the reference's light at the scenes' probes is kept (`reference_values`), as lines of scene, site, light and its standard error. */
    const VALUES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/data/light-reference.txt");

    fn line(name: &str, [x, y, z]: [i64; 3]) -> String {
        format!("{name}; {x} {y} {z}")
    }

    /** The reference's light at the scenes' probes, written to VALUES: run by hand when the scenes change, as it takes a while. It finds only the probes VALUES lacks; delete a changed scene's lines first. */
    #[test]
    #[ignore]
    fn reference_values() {
        let kept = std::fs::read_to_string(VALUES).unwrap_or_default();
        let mut lines = String::new();
        for Scene { name, world, brightness, probes, samples, .. } in scenes() {
            let reference = reference::Reference::new(&world, brightness);
            for site in probes {
                let key = format!("{}; ", line(&name, site));
                lines += &match kept.lines().find(|l| l.starts_with(&key)) {
                    Some(l) => format!("{l}\n"),
                    None => {
                        let ([r, g, b], error) = reference.voxel(site, samples);
                        format!("{key}{r} {g} {b}; {error}\n")
                    }
                };
            }
        }
        std::fs::write(VALUES, lines).expect("written");
    }

    /**
     * Light against the reference (reference.rs, as `reference_values`
     * kept it), scene by scene: how far off the light from each probe's
     * open faces is, as the root mean square of ln(light / reference)
     * over the probes. The bounds are Light's errors as they were; lower
     * one as Light gets closer. `-- --nocapture` prints each probe.
     */
    #[test]
    fn against_the_reference() {
        let kept = std::fs::read_to_string(VALUES).expect("the reference's values: run reference_values");
        let value = |name: &str, site| {
            let key = line(name, site);
            let found = kept
                .lines()
                .find_map(|l| l.strip_prefix(&key)?.strip_prefix("; "))
                .unwrap_or_else(|| panic!("{key}: run reference_values"));
            let [light, error] = [0, 1].map(|k| found.split("; ").nth(k).expect("light; error"));
            (light.split(' ').map(|v| v.parse::<f32>().expect("a number")).sum::<f32>(), error.parse::<f32>().expect("a number"))
        };
        let mut over = Vec::new();
        for Scene { name, world, brightness, probes, bound, .. } in scenes() {
            let mut light = Light::new(world, Shared::new(brightness));
            let mut squares = 0.0;
            for &site in &probes {
                let (m, (r, error)) = (lit(&mut light, site).iter().sum::<f32>(), value(&name, site));
                println!("{name}, {site:?}: light {m:.4}, reference {r:.4} ± {error:.4}, light / reference {:.2}", m / r);
                squares += ((m / r) as f64).ln().powi(2);
            }
            let rms = (squares / probes.len() as f64).sqrt();
            println!("{name}: Light is off by {rms:.2} (ln), at most {bound}");
            if rms > bound {
                over.push(name);
            }
        }
        assert!(over.is_empty(), "Light is further off than it was in {over:?}");
    }

    /**
     * Maps of the ground around each scene, from above, as PNGs in the
     * directory OUT: Light's, the reference's (a sixty-fourth of the rays,
     * kept in OUT for the next run) and where Light is brighter (red) or
     * darker (blue), up to e^1.5 times; and how far off Light is over
     * each map, as `against_the_reference` reckons.
     * Run by hand: OUT=dir cargo test --release maps -- --ignored
     */
    #[test]
    #[ignore]
    fn maps() {
        const HALF: i64 = 96;
        let out = std::env::var("OUT").expect("OUT, a directory");
        let (w, scale, gap) = (HALF as usize, 4, 4);
        for Scene { name, world, brightness, samples, .. } in scenes() {
            let file = format!("{out}/{}", name.replace([' ', ',', '°'], ""));
            let sites: Vec<[i64; 3]> =
                (0..w * w).map(|k| ground_near(2.0 * (k % w) as f64 - HALF as f64, HALF as f64 - 2.0 * (k / w) as f64)).collect();
            let kept: Vec<f32> = std::fs::read(format!("{file}.reference"))
                .unwrap_or_default()
                .as_chunks::<4>()
                .0
                .iter()
                .map(|b| f32::from_le_bytes(*b))
                .collect();
            let references: Vec<Rgb> = if kept.len() == 3 * sites.len() {
                kept.as_chunks::<3>().0.to_vec()
            } else {
                let reference = reference::Reference::new(&world, brightness);
                let found: Vec<Rgb> = sites.iter().map(|&site| reference.voxel(site, samples / 64).0).collect();
                std::fs::write(format!("{file}.reference"), found.iter().flatten().flat_map(|v| v.to_le_bytes()).collect::<Vec<u8>>())
                    .expect("kept");
                found
            };
            let mut light = Light::new(world, Shared::new(brightness));
            let lights: Vec<Rgb> = sites.iter().map(|&site| lit(&mut light, site)).collect();
            let logs: Vec<f64> = lights
                .iter()
                .zip(&references)
                .map(|(m, r)| (m.iter().sum::<f32>() / r.iter().sum::<f32>()).ln() as f64)
                .filter(|v| v.is_finite())
                .collect();
            println!(
                "{name}: Light is off by {:.2} (ln, over the map)",
                (logs.iter().map(|v| v * v).sum::<f64>() / logs.len() as f64).sqrt()
            );
            // Exposed so that the reference's brightest tenth is white.
            let mut sums: Vec<f32> = references.iter().map(|r| r.iter().sum()).collect();
            sums.sort_by(f32::total_cmp);
            let exposure = 3.0 / sums[sums.len() * 9 / 10].max(1e-9);
            let shown = |l: Rgb| colour(l.map(|c| c * exposure)).to_le_bytes();
            let off = |m: Rgb, r: Rgb| {
                let ratio = (m.iter().sum::<f32>() / r.iter().sum::<f32>()).ln();
                let ratio = if ratio.is_nan() { 0.0 } else { ratio.clamp(-1.5, 1.5) / 1.5 };
                let pale = (255.0 * (1.0 - ratio.abs())) as u8;
                if ratio > 0.0 { [255, pale, pale, 255] } else { [pale, pale, 255, 255] }
            };
            let width = 3 * w + 2 * gap;
            let mut rgb = vec![255; width * w * 3];
            for (k, (&m, &r)) in lights.iter().zip(&references).enumerate() {
                for (panel, pixel) in [shown(m), shown(r), off(m, r)].into_iter().enumerate() {
                    let at = 3 * ((k / w) * width + panel * (w + gap) + k % w);
                    rgb[at..at + 3].copy_from_slice(&pixel[..3]);
                }
            }
            let rgb = (0..w * scale)
                .flat_map(|y| (0..width * scale).flat_map(move |x| [0, 1, 2].map(|c| ((y / scale) * width + x / scale) * 3 + c)))
                .map(|at| rgb[at])
                .collect();
            let picture = Picture { width: width * scale, height: w * scale, rgb, ..Picture::default() };
            std::fs::write(format!("{file}.png"), picture.png()).expect("written");
        }
    }
}
