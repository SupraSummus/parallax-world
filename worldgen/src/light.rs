//! Light on the lattices 2^k·FCC: a prototype, for look's pictures and
//! native's `--light`, whose bakers ask for the light from the faces of
//! the voxels they show (fcc's light.rs).
//!
//! Light is kept at the sites it gets through, not in whole voxels. It
//! crosses a voxel from face to face: a line through two faces' middles
//! goes on through the middles of the next voxels' faces, by two steps in
//! turn, towards the neighbours across them (`line`). Of a voxel's 12
//! faces each sees the other 11 about alike (a rhombic dodecahedron's
//! form factors are all within a tenth of 1/11), so the 132 lines through
//! a site weigh alike, and the light on a face of a whole voxel is the
//! mean of the 11 lines that leave the site it faces through it
//! (`leaves`). No cosines: a surface is as bright as the lines that land
//! on it. So light is found by the face it leaves a site by, 11 lines at a
//! time, and only where asked: a face lit, a reflection, a finer level's
//! line read past its steps all want one face's lines.
//!
//! Along a line, light comes from the sites of its level for its two
//! steps, then from the next coarser level's line of the same steps, where
//! that level has room for light (`past`); or from more steps where the
//! coarser fills what the finer leaves empty. Two steps, as the lines of
//! two steps are the directions a level tells apart: more would see past
//! what the lines between miss, a lamp or a roof's edge.
//! Each site sends what it glows, and of what it stops (its alpha) its
//! colour times the light on it; the rest of what comes from past it gets
//! through. So what comes from far comes from coarser levels, the
//! coarsest's from the sky.
//!
//! The light on a site is what the sites near it reflect of the light on
//! them, so it settles round by round: round 0 has what glows, the sky and
//! the coarser level's light, each next round what the sites met reflect
//! of theirs in the round before; found once it changes by less than a
//! tolerance (Shared's, TOLERANCE unless set): in a round or two in the
//! open, in more where light bounces back and forth, as in a closed white
//! room.
//!
//! Light sees the voxels as the eye does, in the grids hosts draw (Grid):
//! a coarse site its pieces fill in part shows a whole voxel, or nothing,
//! as often (VoxelGrid::show). What glows glows shown or not: the coarse
//! sites around a small lamp glow as bright as the lamp, on average over
//! their faces. What fills a site in part, a line meets as much more
//! often than its share says as it is longer through the site than the
//! lines are on average (`meets`).

use std::collections::BTreeMap;
use std::rc::Rc;
use std::sync::{Arc, Mutex, OnceLock};

use fcc::light::{FaceLight, linear_to_srgb, srgb_to_linear};
use fcc::{GRID_LEVELS, Look, alpha_of};

use crate::World;
use crate::generator::{Generator, generate_shown};
use crate::grid::{ALPHA, BRICK, Box3, GLOW_ONE, Grid};
use crate::picture::{FACES, Picture, nearest_site};

type Rgb = [f32; 3];

/** A voxel's faces, and the lines through a site: one from each face to each but the one it is on (`line`). */
const F: usize = FACES.len();
const N: usize = F * G;
/** The lines that leave a site by one face (`leaves`): all but the one that would turn back. */
const G: usize = F - 1;

/** A line's steps before the coarser level's line may take over: one of each (Shared::stepping is unused: more steps want more lines). At most, at the coarsest, before the sky's light does. */
const STEPS: usize = 2;
const TOP_STEPS: usize = 64;

/** The face opposite face `f` (FACES come in fours, each with its opposite). */
const fn opposite(f: usize) -> usize {
    f ^ 3
}

/**
 * Line `l`'s two steps, as faces: light comes along it from the neighbour
 * across the first, where it came from the one across the second, and so
 * on in turn; it leaves towards the neighbour opposite the second. Lines
 * are in the order of the face they leave by, 11 each (`leaves`): all but
 * the line that would turn back.
 */
const fn line(l: usize) -> [usize; 2] {
    let (b, a) = (l / (F - 1), l % (F - 1));
    [if a < opposite(b) { a } else { a + 1 }, b]
}
/** How little the light on a site's faces may change, of what it is, for it to be found, unless set (Shared::settling); and rounds at most, should it never settle (a closed room that reflects all). */
const TOLERANCE: f32 = 1e-2;
const ROUNDS: usize = 256;
/** Sites a tile is a side, at each level, as native's (tile_box). */
pub const TILE: i64 = 128;
/** Sites a block is a side (Shared): small, though a site of one costs a few times one of a tile at the finest levels. */
const BLOCK: i64 = 32;
/** What a Light keeps at each level, of the light that leaves sites by a face (`Known`), all forgotten once full: the finer sites around read it soon after it is found, seldom later. */
const KEPT_SITES: usize = 1 << 18;

/** A grid made once: those that want it while it is made wait. */
type Made = Arc<OnceLock<Arc<Grid>>>;
/** Grids by level and key. */
type Grids = Map<(usize, [i64; 3]), Made>;

/**
 * What the Lights of one world under one brightness, and taking as many
 * steps along each level, share (a thread's each), and with its host: tiles of every level, generated once, by
 * whichever needs one first, kept till the host forgets them (`forget`);
 * the host draws them. Where no tile is made, a Light reads blocks, BLOCK
 * sites a side: light reads a few sites around what it lights, and in a
 * world that is not flat every tile around a tile has some of them.
 */
#[derive(Clone)]
pub struct Shared {
    brightness: Brightness,
    steps: usize,
    tolerance: f32,
    tiles: Arc<Mutex<Grids>>,
    blocks: Arc<Mutex<Grids>>,
}

impl Shared {
    /** Nothing made yet, for a world under `brightness`. */
    pub fn new(brightness: Brightness) -> Shared {
        Shared { brightness, steps: STEPS, tolerance: TOLERANCE, tiles: Arc::default(), blocks: Arc::default() }
    }

    pub fn brightness(&self) -> Brightness {
        self.brightness
    }

    /** The same world under `brightness`: its tiles kept. */
    pub fn under(&self, brightness: Brightness) -> Shared {
        Shared { brightness, ..self.clone() }
    }

    /** Steps along a direction before the coarser level's light may take over. */
    pub fn steps(&self) -> usize {
        self.steps
    }

    /** The same world, light taking `steps` steps along each level before the coarser's: its tiles kept. */
    pub fn stepping(&self, steps: usize) -> Shared {
        assert!((1..=TOP_STEPS).contains(&steps), "{steps} steps");
        Shared { steps, ..self.clone() }
    }

    /** How little the light on a site's faces may change, of what it is, for it to be found. */
    pub fn tolerance(&self) -> f32 {
        self.tolerance
    }

    /** The same world, a site's light found once the light on its faces changes by less than `tolerance` of what it is: its tiles kept. */
    pub fn settling(&self, tolerance: f32) -> Shared {
        assert!(tolerance > 0.0, "tolerance {tolerance}");
        Shared { tolerance, ..self.clone() }
    }

    /** Whether `other` lights the world as this does: as bright, as many steps, as little change. */
    pub fn lights_as(&self, other: &Shared) -> bool {
        (self.brightness, self.steps, self.tolerance) == (other.brightness, other.steps, other.tolerance)
    }

    /** Tile `key` of `level` of the world `generator` makes, of `heights` (World::heights_at). */
    pub fn tile(&self, generator: &Generator, level: u32, heights: (i64, i64), key: [i64; 3]) -> Arc<Grid> {
        let tile = self.tiles.lock().expect("tiles").entry((level as usize, key)).or_default().clone();
        tile.get_or_init(|| Arc::new(make_tile(generator, level, heights, key))).clone()
    }

    /** The grid a Light reads `site` of `level` from, and the box it holds: its tile if made, else its block, made if not yet. */
    fn holding(&self, generator: &Generator, level: u32, heights: (i64, i64), site: [i64; 3]) -> (Box3, Arc<Grid>) {
        let key = site.map(|v| v.div_euclid(TILE));
        let tile = self.tiles.lock().expect("tiles").get(&(level as usize, key)).and_then(|t| t.get().cloned());
        if let Some(tile) = tile {
            return (tile_box(heights, key), tile);
        }
        let key = site.map(|v| v.div_euclid(BLOCK));
        let block = self.blocks.lock().expect("blocks").entry((level as usize, key)).or_default().clone();
        let b = cube_box(heights, key, BLOCK);
        (b, block.get_or_init(|| Arc::new(generate_shown(generator, &b, level).into_grid(true))).clone())
    }

    /** Forgets the tiles `far` says are far, by level and key, and the blocks in them, to make again if asked. */
    pub fn forget(&self, far: impl Fn(usize, [i64; 3]) -> bool) {
        self.tiles.lock().expect("tiles").retain(|&(level, key), _| !far(level, key));
        self.blocks.lock().expect("blocks").retain(|&(level, key), _| !far(level, key.map(|k| k.div_euclid(TILE / BLOCK))));
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

/** A level's voxels, generated in tiles as they are needed, and the light found at its sites. */
struct Level {
    heights: (i64, i64),
    /** The grid read last, and the box it holds: most reads are in it. */
    last: Option<(Box3, Arc<Grid>)>,
    /** What is known of the light that leaves sites by a face, as found (KEPT_SITES). */
    leaving: Map<Leaving, Known>,
}

/** What is known of the light that leaves a site by a face: what comes (Paths), what leaves by round (`round`), and the lines' light once settled. */
struct Known {
    paths: Rc<Paths>,
    rounds: Vec<Option<Rgb>>,
    found: Option<[Rgb; G]>,
}

/** A site and a face it lets light out by: what light is found for, the lines that leave by it (`leaves`). */
type Leaving = ([i64; 3], u8);

/**
 * What comes to a site along the lines that leave by a face, and does not
 * change as its light settles (`along`): along each, all but reflected
 * light; and what reflects: the line (of the face's), the site before the
 * one met and the face of that one it leaves by, and the share of the
 * light that lands there that comes.
 */
struct Paths {
    comes: [Rgb; G],
    reflects: Vec<(usize, [i64; 3], usize, Rgb)>,
}

pub struct Light {
    world: World,
    shared: Shared,
    /** By level, up to a renderer's coarsest grid's (GRID_LEVELS). */
    levels: Vec<Level>,
    /** Per line, the sum of its two steps: where it comes from. */
    sums: [[i64; 3]; N],
    /** Per line, how long it is through a site, of the lines' mean (`meets`). */
    reach: [f32; N],
    /** The radiance of a glow of GLOW_ONE. */
    glow: f32,
    /** The sky's radiance per line (the sun's within it). */
    sky: [Rgb; N],
    /** What the sky looks like, where a picture's rays miss. */
    background: Rgb,
}

/** The coarser level's light around a site, as `along` takes it, read at once (`near`): per line leaving by a face, past its two steps, if the coarser level has room for it there (`past`). */
type Near = [Option<Rgb>; G];

impl Light {
    /** The light of `world`, found only where asked, its tiles and light in `shared`. */
    pub fn new(world: World, shared: Shared) -> Light {
        let Brightness { sky, sun, .. } = shared.brightness;
        let coarsest = GRID_LEVELS as u32 - 1;
        let levels = (0..=coarsest).map(|level| Level { heights: world.heights_at(level), last: None, leaving: Map::default() }).collect();
        let sums: [[i64; 3]; N] = std::array::from_fn(|l| add(FACES[line(l)[0]], FACES[line(l)[1]]));
        let lengths = sums.map(|s| (s.iter().map(|c| c * c).sum::<i64>() as f32).sqrt());
        let reach = lengths.map(|length| length * N as f32 / lengths.iter().sum::<f32>());
        // The sun's lines: those from its direction, which land on the face towards it and the four beside it, a
        // fifth of what lands on a surface towards a sun a twelfth of the sphere wide on each: 11/5 of its radiance.
        let skies = std::array::from_fn(|l| {
            let open = [0.0, 0.5, 1.0][(sums[l][1].signum() + 1) as usize];
            let sun = if [1, 2].iter().any(|k| sums[l] == SUN_DIRECTION.map(|c| k * c)) { sun * (F - 1) as f32 / 5.0 } else { 0.0 };
            std::array::from_fn(|c| open * SKY[c] * sky + SUN[c] * sun)
        });
        let background = SKY.map(|c| c * sky);
        // A world with no heights has no sky: what glows is all its light.
        let (skies, background) = if world.heights.is_some() { (skies, background) } else { ([[0.0; 3]; N], [0.0; 3]) };
        let glow = shared.brightness.glowing() / GLOW_ONE as f32;
        Light { world, shared, levels, sums, reach, glow, sky: skies, background }
    }

    /** Whether a whole voxel, one that lets nothing through, is at `site` of level `i`. */
    fn whole(&mut self, i: usize, site: [i64; 3]) -> bool {
        self.open(i, site) <= 0.0
    }

    /** How light sees `site` of level `i` from direction `from`: how much of it stops (alpha) and its colour; what it glows. */
    fn see(&mut self, i: usize, site: [i64; 3], from: usize) -> ((f32, Rgb), Rgb) {
        let Some(([x, y, z], grid)) = self.hold(i, site) else { return ((0.0, [0.0; 3]), [0.0; 3]) };
        let look = grid.face(x, y, z, from);
        ((alpha_of(look) as f32 / ALPHA as f32, linear(look)), grid.glow(x, y, z).map(|g| g as f32 * self.glow))
    }

    /**
     * What a line meets of `site` of level `i`, seen from direction `from`
     * (`see`): a whole voxel, as it is; pieces or a medium, `reach` as
     * much, of what they stop and of what they glow. Their share is of
     * the rays through the voxel: a line longer through the site than
     * most meets them more often.
     */
    fn meets(&mut self, i: usize, site: [i64; 3], from: usize, reach: f32) -> ((f32, Rgb), Rgb) {
        let ((alpha, colour), glow) = self.see(i, site, from);
        if alpha <= 0.0 || alpha >= 1.0 {
            return ((alpha, colour), glow);
        }
        ((1.0 - (1.0 - alpha).powf(reach), colour), glow.map(|g| g * reach))
    }

    /** How much of what comes through `site` of level `i` it lets through. */
    fn open(&mut self, i: usize, site: [i64; 3]) -> f32 {
        let Some(([x, y, z], grid)) = self.hold(i, site) else { return 1.0 };
        1.0 - alpha_of(grid.get(x, y, z)) as f32 / ALPHA as f32
    }

    /** The grid of level `i` that holds `site`, from those shared (Shared::holding), and `site` as it takes it; none above or below every voxel. */
    fn hold(&mut self, i: usize, site: [i64; 3]) -> Option<([i32; 3], &Grid)> {
        let level = &mut self.levels[i];
        if !(level.heights.0..level.heights.1).contains(&site[1]) {
            return None;
        }
        if level.last.as_ref().is_none_or(|(b, _)| !b.holds(site)) {
            level.last = Some(self.shared.holding(&self.world.generator, i as u32, level.heights, site));
        }
        Some((site.map(|v| v as i32), &level.last.as_ref().expect("held").1))
    }

    /**
     * The light coming to site `p` of level `i` along the lines that leave
     * it by face `f`, as it settles round by round (`round`): found once
     * what leaves it changes by less than Shared's tolerance, in a round or
     * two where little reflects back, in many in a closed white room.
     */
    fn light(&mut self, i: usize, p: [i64; 3], f: usize) -> [Rgb; G] {
        if let Some(light) = self.levels[i].leaving.get(&(p, f as u8)).and_then(|k| k.found) {
            return light;
        }
        let paths = self.paths(i, p, f);
        if paths.reflects.is_empty() {
            // Nothing met reflects: round 0 is all there is.
            self.known(i, p, f, &paths).found = Some(paths.comes);
            return paths.comes;
        }
        let (mut leaves, mut n) = (self.round(i, p, f, 0), 0);
        loop {
            n += 1;
            let next = self.round(i, p, f, n);
            let settled = change(&leaves, &next) <= self.shared.tolerance;
            leaves = next;
            if settled || n == ROUNDS {
                break;
            }
        }
        let light = self.lines(i, p, f, n);
        self.known(i, p, f, &paths).found = Some(light);
        light
    }

    /** What is known of the light that leaves site `p` of level `i` by face `f`, whose paths are `paths`: kept anew if forgotten since. */
    fn known(&mut self, i: usize, p: [i64; 3], f: usize, paths: &Rc<Paths>) -> &mut Known {
        let leaving = &mut self.levels[i].leaving;
        if !leaving.contains_key(&(p, f as u8)) {
            keep(leaving, (p, f as u8), Known { paths: paths.clone(), rounds: Vec::new(), found: None });
        }
        leaving.get_mut(&(p, f as u8)).expect("kept")
    }

    /** The light that leaves site `p` of level `i` by face `f` in round `n` (`leaves`, of `lines`). */
    fn round(&mut self, i: usize, p: [i64; 3], f: usize, n: usize) -> Rgb {
        if let Some(leaves) = self.levels[i].leaving.get(&(p, f as u8)).and_then(|k| k.rounds.get(n).copied().flatten()) {
            return leaves;
        }
        let paths = self.paths(i, p, f);
        let lines = self.lines_of(i, &paths, n);
        let leaves = self.leaves(i, p, &lines, f);
        let rounds = &mut self.known(i, p, f, &paths).rounds;
        if rounds.len() <= n {
            rounds.resize(n + 1, None);
        }
        rounds[n] = Some(leaves);
        leaves
    }

    /**
     * The light coming to site `p` of level `i` along the lines that leave
     * it by face `f`, in round `n`: what comes (`paths`), and what the
     * sites met reflect of the light that landed on them in the round
     * before; none in round 0.
     */
    fn lines(&mut self, i: usize, p: [i64; 3], f: usize, n: usize) -> [Rgb; G] {
        let paths = self.paths(i, p, f);
        self.lines_of(i, &paths, n)
    }

    /** As `lines`, of their `paths`. */
    fn lines_of(&mut self, i: usize, paths: &Paths, n: usize) -> [Rgb; G] {
        let mut light = paths.comes;
        if n > 0 {
            for &(k, before, face, reflects) in &paths.reflects {
                let lands = self.round(i, before, face, n - 1);
                (0..3).for_each(|c| light[k][c] += reflects[c] * lands[c]);
            }
        }
        light
    }

    /**
     * Of `lines`, the light coming to site `p` of level `i` along the lines
     * that leave it by face `f`, what leaves it by the face opposite and
     * lands on face `f` of the site there: their mean, as much as the site
     * lets through, and what it glows.
     */
    fn leaves(&mut self, i: usize, p: [i64; 3], lines: &[Rgb; G], f: usize) -> Rgb {
        let ((alpha, _), glow) = self.meets(i, p, f, 1.0);
        std::array::from_fn(|c| glow[c] + (1.0 - alpha) * lines.iter().map(|l| l[c]).sum::<f32>() / G as f32)
    }

    /** What comes to site `p` of level `i` (Paths), along each line that leaves it by face `f`. */
    fn paths(&mut self, i: usize, p: [i64; 3], f: usize) -> Rc<Paths> {
        if let Some(known) = self.levels[i].leaving.get(&(p, f as u8)) {
            return known.paths.clone();
        }
        let (mut near, mut reflects) = (None, Vec::new());
        let comes = std::array::from_fn(|k| self.along(i, p, f, k, &mut near, &mut reflects));
        let paths = Rc::new(Paths { comes, reflects });
        self.known(i, p, f, &paths);
        paths
    }

    /**
     * The light coming to `p`, a site of level `i`, along line `l`, all
     * but reflected light: from the sites its steps meet, what each
     * glows; past them, the coarser level's light, from its two steps on,
     * where it has room for it (`past`; `near`, read when first wanted,
     * has it past the two). At the coarsest level, from the next
     * TOP_STEPS, and past them the sky. The sites met that reflect go in
     * `reflects` (Paths): of what each stops, its colour.
     */
    fn along(
        &mut self,
        i: usize,
        p: [i64; 3],
        f: usize,
        k: usize,
        near: &mut Option<Near>,
        reflects: &mut Vec<(usize, [i64; 3], usize, Rgb)>,
    ) -> Rgb {
        let l = f * G + k;
        let (steps, up, heights, reach) = (line(l), self.sums[l][1], self.levels[i].heights, self.reach[l]);
        let coarser = i + 1 < self.levels.len();
        // What has come so far, and how much of what comes from further on gets through.
        let (mut sum, mut through) = ([0.0f32; 3], 1.0f32);
        let mut q = p;
        let beyond = 'march: {
            for step in 1..=TOP_STEPS {
                let (face, next) = (steps[(step - 1) % 2], steps[step % 2]);
                let before = q;
                q = add(q, FACES[face]);
                let ((alpha, colour), glow) = self.meets(i, q, opposite(face), reach);
                (0..3).for_each(|c| sum[c] += through * glow[c]);
                if alpha > 0.0 {
                    reflects.push((k, before, opposite(face), colour.map(|c| through * alpha * c)));
                }
                through *= 1.0 - alpha;
                // Nothing more gets through, or nothing more is met: below every voxel and not going up, or above them and not going down.
                if through < 1.0 / ALPHA as f32 || q[1] < heights.0 - 2 && up <= 0 {
                    break 'march [0.0; 3];
                }
                if q[1] > heights.1 + 1 && up >= 0 {
                    break 'march self.sky[l];
                }
                if coarser && step >= STEPS {
                    let past = if step == STEPS {
                        if near.is_none() {
                            *near = Some(self.near(i + 1, p, f));
                        }
                        near.as_ref().expect("read")[k]
                    } else {
                        self.past(i + 1, handover(q, next), l)
                    };
                    if let Some(past) = past {
                        break 'march past;
                    }
                }
            }
            if !coarser && up >= 0 { self.sky[l] } else { [0.0; 3] }
        };
        std::array::from_fn(|c| sum[c] + through * beyond[c])
    }

    /**
     * Level `i`'s light along line `d` at `at` (`reads`), if it has
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
                let light = self.light(i, site, d / G)[d % G];
                total += w * open;
                (0..3).for_each(|c| sum[c] += w * open * light[c]);
            }
        }
        Some(sum.map(|c| c / total))
    }

    /** `along`'s readings of level `i` around `p`, a site of level `i − 1`, for the lines that leave it by face `f`, as `past` would give them, read at once from the sites around (`around`). */
    fn near(&mut self, i: usize, p: [i64; 3], f: usize) -> Near {
        let (class, base) = class(p);
        let (sites, nearest) = &around()[class][f];
        let (mut past, mut totals) = ([[0.0f32; 3]; G], [0.0f32; G]);
        for (offset, weights) in sites {
            let site = add(base, *offset);
            let open = self.open(i, site);
            if open <= 0.0 {
                continue;
            }
            let light = self.light(i, site, f);
            for &(k, w) in weights {
                totals[k] += w * open;
                (0..3).for_each(|c| past[k][c] += w * open * light[k][c]);
            }
        }
        std::array::from_fn(|k| (self.open(i, add(base, nearest[k])) > 0.0).then(|| past[k].map(|c| c / totals[k])))
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

    /** What lights the voxel at `site` of `level` to its colour at 1: the light on its faces, those not hidden, on average; none if all are hidden (a ray meets it only where levels meet). */
    pub fn on_seen(&mut self, level: u32, site: [i64; 3]) -> Option<Rgb> {
        let on = self.on(level as usize, site);
        let lit: Vec<Rgb> = on.into_iter().flatten().collect();
        (!lit.is_empty()).then(|| std::array::from_fn(|c| lit.iter().map(|l| l[c]).sum::<f32>() / lit.len() as f32))
    }

    /** As `on_seen`, dark if all its faces are hidden. */
    pub fn on_voxel(&mut self, level: u32, site: [i64; 3]) -> Rgb {
        self.on_seen(level, site).unwrap_or_default()
    }

    /** The light on each face of the voxel at `site` of level `i`: what lands on it from the site it faces (`leaves`); none on a face a whole voxel hides. */
    fn on(&mut self, i: usize, site: [i64; 3]) -> [Option<Rgb>; F] {
        std::array::from_fn(|f| {
            let q = add(site, FACES[f]);
            (!self.whole(i, q)).then(|| {
                let lines = self.light(i, q, f);
                self.leaves(i, q, &lines, f)
            })
        })
    }

    /** The light from each face of the voxel at `site` of level `i`, a hidden face lit as its others are on average; none if all are hidden. */
    fn lit(&mut self, i: usize, site: [i64; 3]) -> Option<FaceLight> {
        let mean = self.on_seen(i as u32, site)?;
        let on = self.on(i, site);
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
    generate_shown(generator, &tile_box(heights, key), level).into_grid(true)
}

/** How much the light that leaves a site by a face changed from `before` to `after`, of what it is: the most of any channel. */
fn change(before: &Rgb, after: &Rgb) -> f32 {
    (0..3).filter(|&c| after[c] > 0.0).map(|c| (after[c] - before[c]).abs() / after[c]).fold(0.0, f32::max)
}

/** Keeps `value` for `site` in `map`, forgetting all it held once it holds KEPT_SITES. */
fn keep<K: std::hash::Hash + Eq, V>(map: &mut Map<K, V>, site: K, value: V) {
    if map.len() == KEPT_SITES {
        map.clear();
    }
    map.insert(site, value);
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
 * offsets from the class's base, each site's weights for the lines it is
 * read for; and the site nearest each line's place.
 */
type Around = (Vec<([i64; 3], Vec<(usize, f32)>)>, [[i64; 3]; G]);

/** A site's class, of the 8 that the sites around its centre come in (`around`), and the base they are from: half of it, rounded down. */
fn class(p: [i64; 3]) -> (usize, [i64; 3]) {
    let base = p.map(|c| c.div_euclid(2));
    // Its last coordinate's parity follows from the others'.
    let class = (p[0] & 1) | (p[1] & 1) << 1 | ((base[0] + base[1] + base[2]) & 1) << 2;
    (class as usize, base)
}

/** `Around` for each class, and the lines that leave by each face. */
fn around() -> &'static [[Around; F]; 8] {
    static AROUND: OnceLock<[[Around; F]; 8]> = OnceLock::new();
    AROUND.get_or_init(|| {
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
                    handover(add(add(p, FACES[a]), FACES[b]), a)
                });
                let mut sites = BTreeMap::new();
                for (k, at) in at.iter().enumerate() {
                    for (site, w) in reads(*at) {
                        sites.entry(offset(site)).or_insert_with(Vec::new).push((k, w));
                    }
                }
                (sites.into_iter().collect(), at.map(|at| offset(nearest_site(at))))
            })
        })
    })
}

/**
 * Where the next coarser level's line takes over from one at site `q` of
 * a level whose next step is by face `next`: half that step back, so
 * that the coarser line's first site begins where `q` ends.
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
    fn blocks_light_as_tiles_do() {
        // A roof over ground and a lamp under it: its light read from blocks alone, and from tiles made first.
        let make = || world(&[[8.0, 12.0, -6.0]], |x, y, z, s| y < 0 || x.abs() < 40 && z.abs() < 40 && (24..24 + s).contains(&y));
        let brightness = Brightness { sky: 1.0, sun: 1.0, glow: 1.0 };
        let tiles = Shared::new(brightness);
        let world = make();
        for level in 0..GRID_LEVELS as u32 {
            for key in [-1, 0].into_iter().flat_map(|x| (-1..=8).flat_map(move |y| [-1, 0].map(|z| [x, y, z]))) {
                tiles.tile(&world.generator, level, world.heights_at(level), key);
            }
        }
        let (mut blocks, mut tiles) = (Light::new(make(), Shared::new(brightness)), Light::new(make(), tiles));
        for x in (-63..64).step_by(6) {
            let site = ground(x, 0);
            assert!(blocks.faces(0, site) == tiles.faces(0, site), "at {site:?}");
        }
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
                0.35,
            ),
        ];
        // A roof 2 sites thick and 128 units wide at y = 24, turned about y, probed across it: the same light at every turn.
        for (turn, bound) in [(0.0, 0.3), (22.5, 0.2), (45.0, 0.3)] {
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
        scenes.push(scene("yard, past a wall", yard, sky, probes, 1 << 16, 2.0));
        // A tunnel 40 wide and high through a hill 384 long, under a roof 16 thick at every level, thinner than the coarse
        // levels' sites: probed along its floor, from its middle out past a portal. The sun is side on: none of its light comes in.
        let tunnel = world(&[], |x, y, z, _| y < 0 || z.abs() < 192 && y < 56 && !(x.abs() < 20 && y < 40));
        let probes = [0.0, 96.0, 160.0, 224.0].map(|z| ground_near(0.0, z)).to_vec();
        scenes.push(scene("tunnel under a thin roof", tunnel, day, probes, 1 << 16, 2.2));
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
        scenes.push(scene("blocks over the ground", canopy, sky, [0.0, 24.0, 48.0].map(|x| ground_near(x, 0.0)).to_vec(), 4096, 0.45));
        // A closed room, white, a lamp in it: most of the light on its floor has bounced, many times.
        let room = world_of(&["#f0f0f0"], &[[0.0, 24.0, 0.0]], |x, y, z, s| {
            let wall = |c: i64| (32..32 + s).contains(&c.abs());
            let within = |c: i64| c.abs() < 32 + s;
            y < 0 || within(x) && within(z) && (0..48 + s).contains(&y) && (wall(x) || wall(z) || y >= 48)
        });
        let probes = vec![ground(1, 0), ground(16, 7), ground(29, 0), ground(28, 27)];
        scenes.push(scene("white room, lamp", room, night, probes, 1 << 16, 0.15));
        // A cave, a ball in rock the cheese's colour, a lamp by its wall: probed across its floor.
        let ball = |x: i64, y: i64, z: i64| x * x + (y - CAVE.1) * (y - CAVE.1) + z * z < CAVE.0 * CAVE.0;
        let cave = world_of(&["#f3d46c"], &[[0.0, 20.0, 20.0]], move |x, y, z, _| y < 80 && !ball(x, y, z));
        let probes = [0, 12, 24].map(|x| {
            let y = (0..CAVE.1).find(|&y| ball(x, y, 0)).expect("the cave's floor") - 1;
            [x + (x + y).rem_euclid(2), y, 0]
        });
        scenes.push(scene("yellow cave, lamp", cave, night, probes.to_vec(), 1 << 16, 0.7));
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
        let open: Vec<usize> = (0..F).filter(|&f| !light.whole(0, add(site, FACES[f]))).collect();
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
