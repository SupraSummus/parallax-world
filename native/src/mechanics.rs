//! Mechanics in the world (mechanics/, docs/mechanics.md), on a thread of
//! its own, which the thread that draws never waits for (threads.rs), and
//! shown as it moves.
//!
//! The world is one lattice, a part of it awake (`Field`). Its level-0
//! sites, as generated and edited since, are all there is (`View`): a
//! coarser site is the cube of level-0 sites it is, read by the same rules
//! at every level, and what moves is written back to level 0. A cut wakes
//! what bears over it and near it (`Around`). Loose matter (`Kind`) bears
//! nothing: it goes with what bears nearest it (`goes_with`).

use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};
use std::time::Duration;

use lattice::{ALPHA, FACE_KINDS, FastMap, FastSet, Holds, VOXEL_FACES, opposite};
use mechanics::{Cell, Lattice, Material, Matter, WOOD, about, faces};
use web_time::Instant;
use worldgen::edits::{Sites, holds};
use worldgen::grid::{Held, VoxelGrid};
use worldgen::sites::middle;
use worldgen::tiles::{TILE, made_with, sites_as};
use worldgen::units::{UNIT_SIZE, m};

use crate::threads::Post;
use crate::world::World;

/** What bears is all of one material: the worlds give none. */
const MATTER: Material = WOOD;
/** How heavy loose matter is, kg/m³: leaves, not wood. */
const LIGHT: f64 = 60.0;
/** How far across from a cut matter is woken, each way, and how far over it, lattice units. */
const REGION: i64 = m(10.0);
const ABOVE: i64 = m(30.0);
/** Cells a cut wakes at most, past which the next coarser level is tried; levels tried; cells awake at most. */
const BUDGET: usize = 6000;
const LEVELS: u32 = 4;
const MOST: usize = 2 * BUDGET;
/** How far from loose matter what bears, that it goes with, is looked for, lattice units. */
const REACH: i64 = m(2.0);
/** Seconds between what is told; how long what a cut woke is followed, at most. */
const EVERY: f64 = 1.0 / 20.0;
const LONGEST: f64 = 10.0;

/** What a site holds, as mechanics sees it, in the order of how much it bears. */
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Kind {
    Empty,
    Loose,
    Bearing,
}

/** What `held` is: what stops a walking body bears; a medium alone is nothing. */
pub fn kind(held: &Held) -> Kind {
    let ((pieces, _), _, solid, ..) = held;
    match () {
        _ if *pieces == 0 => Kind::Empty,
        _ if *solid > ALPHA / 2 => Kind::Bearing,
        _ => Kind::Loose,
    }
}

/** Of a site's cube: how many of its level-0 sites bear, whether any holds anything. */
#[derive(Clone, Copy)]
struct Cube {
    bears: u32,
    holds: bool,
}

/**
 * A world's level-0 sites, as generated and as edited (`edits`, as this
 * thread has them), and the sites of `level` as the cubes of them they are.
 */
struct View {
    world: Arc<worldgen::World>,
    edits: Sites,
    tiles: FastMap<[i64; 3], Arc<VoxelGrid>>,
    level: u32,
    cubes: FastMap<[i64; 3], Cube>,
    /** Sites of the level of cells awake, where each was shown last: no site asleep holds it. */
    awake: FastSet<[i64; 3]>,
    /** Sites of the level of the loose matter they carry, where it was shown last. */
    carried: FastSet<[i64; 3]>,
}

impl View {
    fn new(world: &Arc<worldgen::World>, edits: &Sites, level: u32) -> View {
        View {
            world: world.clone(),
            edits: edits.clone(),
            tiles: FastMap::default(),
            level,
            cubes: FastMap::default(),
            awake: FastSet::default(),
            carried: FastSet::default(),
        }
    }

    fn set_level(&mut self, level: u32) {
        self.level = level;
        self.cubes.clear();
    }

    /** Level-0 sites a site of the level is a side. */
    fn size(&self) -> i64 {
        1 << self.level
    }

    /** What a level-0 site holds. */
    fn held(&mut self, zero: [i64; 3]) -> Held {
        if let Some(e) = self.edits.get(&zero) {
            return e.0;
        }
        let key = zero.map(|v| v.div_euclid(TILE));
        if !self.tiles.contains_key(&key) {
            let sites = Arc::new(sites_as(&self.world, 0, key, &Sites::default()));
            self.tiles.extend(made_with(&self.world, 0, key).map(|with| (with, sites.clone())));
        }
        let [x, y, z] = zero;
        self.tiles[&key].held(x, y, z)
    }

    /** Across which faces what a level-0 site holds holds on. */
    fn holds(&self, zero: [i64; 3]) -> Holds {
        holds(&self.edits, zero)
    }

    fn bears(&mut self, zero: [i64; 3]) -> bool {
        kind(&self.held(zero)) == Kind::Bearing
    }

    fn cube(&mut self, site: [i64; 3]) -> Cube {
        if let Some(&c) = self.cubes.get(&site) {
            return c;
        }
        let mut c = Cube { bears: 0, holds: false };
        for zero in cube(site, self.size()) {
            match kind(&self.held(zero)) {
                Kind::Bearing => (c.bears, c.holds) = (c.bears + 1, true),
                Kind::Loose => c.holds = true,
                Kind::Empty => {}
            }
        }
        self.cubes.insert(site, c);
        c
    }

    /** What a site of the level holds: it bears if any of its cube bears. */
    fn kind(&mut self, site: [i64; 3]) -> Kind {
        match self.cube(site) {
            Cube { bears: 1.., .. } => Kind::Bearing,
            Cube { holds: true, .. } => Kind::Loose,
            _ => Kind::Empty,
        }
    }

    /** Has each of `changes` (level-0 sites) hold what it says. */
    fn change(&mut self, changes: impl IntoIterator<Item = ([i64; 3], (Held, Holds))>) {
        let s = self.size();
        for (zero, held) in changes {
            self.edits.insert(zero, held);
            self.cubes.remove(&zero.map(|v| v.div_euclid(s)));
        }
    }

    /** The level-0 site a site of the level is read at, as hosts draw it. */
    fn read_at(&self, site: [i64; 3]) -> [i64; 3] {
        let s = self.size();
        site.map(|v| v * s + middle(s))
    }
}

impl Lattice for View {
    fn asleep(&mut self, site: [i64; 3]) -> Option<Material> {
        (!self.awake.contains(&site) && self.kind(site) == Kind::Bearing).then_some(MATTER)
    }

    /** The share of the face's level-0 pairs joined: both bear, both hold on across it. */
    fn joined(&mut self, site: [i64; 3], f: usize) -> f64 {
        let s = self.size();
        let (axis, step) = (f / 2, VOXEL_FACES[f].map(i64::from));
        let (u, w) = ((axis + 1) % 3, (axis + 2) % 3);
        let mut joined = 0;
        for k in 0..s * s {
            let mut zero = site.map(|v| v * s);
            zero[axis] += if step[axis] < 0 { 0 } else { s - 1 };
            (zero[u], zero[w]) = (zero[u] + k % s, zero[w] + k / s);
            let other = [zero[0] + step[0], zero[1] + step[1], zero[2] + step[2]];
            if self.holds(zero) & 1 << f != 0 && self.holds(other) & 1 << opposite(f) != 0 && self.bears(zero) && self.bears(other) {
                joined += 1;
            }
        }
        f64::from(joined) / (s * s) as f64
    }
}

/** What is looked at about a cut, in level-0 sites: its sites, the lowest's height, its middle across. */
struct Around {
    cut: FastSet<[i64; 3]>,
    lowest: i64,
    middle: [i64; 2],
}

impl Around {
    fn of(cut: &FastSet<[i64; 3]>) -> Option<Around> {
        let lowest = cut.iter().map(|s| s[1]).min()?;
        let n = cut.len() as i64;
        let middle = [0, 2].map(|k| cut.iter().map(|s| s[k]).sum::<i64>() / n);
        Some(Around { cut: cut.clone(), lowest, middle })
    }

    fn across(&self, s: [i64; 3]) -> i64 {
        (s[0] - self.middle[0]).abs().max((s[2] - self.middle[1]).abs())
    }

    /** Over the cut, not at the edge of what is looked at. */
    fn wakes(&self, s: [i64; 3]) -> bool {
        self.across(s) < REGION && (self.lowest..self.lowest + ABOVE).contains(&s[1])
    }
}

/** The level-0 sites of the cube of a site of the level whose sites are `s` across. */
fn cube(c: [i64; 3], s: i64) -> impl Iterator<Item = [i64; 3]> {
    (0..s * s * s).map(move |k| [c[0] * s + k % s, c[1] * s + k / s % s, c[2] * s + k / (s * s)])
}

/** The faces of its cell's cube, `s` sites a side, that a site carried at `d` from its least lies on: none, if it is not in the cube. */
fn on_face(d: [i64; 3], s: i64) -> Holds {
    if d.iter().any(|&v| !(0..s).contains(&v)) {
        return 0;
    }
    (0..FACE_KINDS).filter(|&f| d[f / 2] == if f % 2 == 0 { 0 } else { s - 1 }).fold(0, |h, f| h | 1 << f)
}

/**
 * What a cell carries: level-0 sites from its cube's least, what each
 * holds, and across which faces that holds on; the sites of loose matter
 * it carries, from its own; where it was told last to be, and holding on
 * across which faces (None till it is told).
 */
#[derive(Clone, Debug)]
struct Carried {
    sites: Vec<([i64; 3], Held, Holds)>,
    loose: Vec<[i64; 3]>,
    shown: Option<([i64; 3], Holds)>,
}

impl Carried {
    /** The sites of the level the loose matter it carries lies at, its cell at `site`. */
    fn loose_at(&self, site: [i64; 3]) -> impl Iterator<Item = [i64; 3]> + '_ {
        self.loose.iter().map(move |d| [site[0] + d[0], site[1] + d[1], site[2] + d[2]])
    }
}

/** Level-0 sites changed, what each holds now, and across which faces that holds on. */
type Changes = Vec<([i64; 3], (Held, Holds))>;

/** What the simulation tells: what cuts woke starts moving; what changes `t` seconds in; all is asleep, after so many cuts, and what was found. */
pub enum Told {
    Start,
    Frame(f64, Changes),
    Done(usize, String),
}

/** The world's lattice, and the part of it awake. */
pub struct Field {
    view: View,
    matter: Matter<Carried>,
    /** Changes not told yet; when they were told last; when a cut last woke anything. */
    untold: FastMap<[i64; 3], (Held, Holds)>,
    told: f64,
    woke: f64,
    /** Since all was asleep last: cuts, cells woken, sites changed; when it started. */
    cuts: usize,
    woken: usize,
    changed: usize,
    started: Instant,
}

impl Field {
    pub fn new(world: Arc<worldgen::World>) -> Field {
        let edits = world.edits.sites();
        Field {
            view: View::new(&world, &edits, 0),
            matter: Matter::new(0, UNIT_SIZE),
            untold: FastMap::default(),
            told: 0.0,
            woke: 0.0,
            cuts: 0,
            woken: 0,
            changed: 0,
            started: Instant::now(),
        }
    }

    /** Takes `cut`'s level-0 sites out of the world, and wakes what that sets moving. */
    pub fn cut(&mut self, cut: Vec<[i64; 3]>, tell: &mut dyn FnMut(Told)) {
        let cut: FastSet<[i64; 3]> = cut.into_iter().collect();
        self.view.change(cut.iter().map(|&s| (s, (Held::default(), 0))));
        if !self.matter.awake() {
            self.cuts = 0;
            self.started = Instant::now();
            tell(Told::Start);
        }
        self.cuts += 1;
        self.cut_awake(&cut);
        self.wake_about(&cut);
    }

    /** What is awake that `cut` took from: its sites gone from what carries them, cells holding nothing that bears asleep. */
    fn cut_awake(&mut self, cut: &FastSet<[i64; 3]>) {
        let s = self.view.size();
        for c in &mut self.matter.cells {
            let origin = c.site.map(|v| v * s);
            let before = c.what.sites.len();
            c.what.sites.retain(|(d, ..)| !cut.contains(&[origin[0] + d[0], origin[1] + d[1], origin[2] + d[2]]));
            if c.what.sites.len() < before {
                (c.mass, c.fill) = weigh(&c.what.sites, s);
            }
        }
        let touched: FastSet<[i64; 3]> = cut.iter().map(|z| z.map(|v| v.div_euclid(s))).collect();
        let gone = self.matter.changed(&touched, &mut self.view);
        self.fell_asleep(gone);
    }

    /**
     * Wakes what `cut` sets moving: at the finest level it is few enough
     * at (BUDGET), or the coarsest, or the level awake; nearest the cut
     * first, as much as BUDGET.
     */
    fn wake_about(&mut self, cut: &FastSet<[i64; 3]>) {
        let Some(around) = Around::of(cut) else { return };
        let levels = if self.matter.awake() { self.matter.level..self.matter.level + 1 } else { 0..LEVELS };
        let last = levels.end - 1;
        for level in levels {
            if level != self.view.level {
                self.view.set_level(level);
                self.matter = Matter::new(level, UNIT_SIZE);
            }
            let (sites, all) = self.region(&around);
            if all || level == last {
                self.woke = self.matter.time;
                return self.wake(sites);
            }
        }
    }

    /**
     * What bears touching `around`'s cut across faces, held on or not,
     * and asleep: the sites of the level it holds, the cut's own, nearest
     * it first, as many as BUDGET; whether that is all of it.
     */
    fn region(&mut self, around: &Around) -> (Vec<[i64; 3]>, bool) {
        let s = self.view.size();
        let mut seen: FastSet<[i64; 3]> = around.cut.iter().map(|z| z.map(|v| v.div_euclid(s))).collect();
        let mut queue: VecDeque<[i64; 3]> = seen.iter().copied().collect();
        let mut region: Vec<[i64; 3]> = queue.iter().copied().filter(|&c| self.view.asleep(c).is_some()).collect();
        while let Some(site) = queue.pop_front() {
            for next in faces(site) {
                if seen.insert(next) && around.wakes(self.view.read_at(next)) && self.view.asleep(next).is_some() {
                    if region.len() >= BUDGET {
                        return (region, false);
                    }
                    region.push(next);
                    queue.push_back(next);
                }
            }
        }
        (region, true)
    }

    /** Wakes the sites `sites` of the level, asleep and bearing, and the loose matter that goes with them. */
    fn wake(&mut self, sites: Vec<[i64; 3]>) {
        let s = self.view.size();
        let index: FastMap<[i64; 3], u32> = sites.iter().enumerate().map(|(i, &c)| (c, i as u32)).collect();
        let loose = self.loose_with(&index);
        let mut carried: Vec<Carried> = vec![Carried { sites: Vec::new(), loose: Vec::new(), shown: None }; sites.len()];
        for (&site, &c) in index.iter().chain(&loose) {
            let owner = sites[c as usize];
            let origin = owner.map(|v| v * s);
            for zero in cube(site, s) {
                let held = self.view.held(zero);
                if kind(&held) != Kind::Empty {
                    let d = [zero[0] - origin[0], zero[1] - origin[1], zero[2] - origin[2]];
                    carried[c as usize].sites.push((d, held, self.view.holds(zero)));
                }
            }
            if site != owner {
                carried[c as usize].loose.push([site[0] - owner[0], site[1] - owner[1], site[2] - owner[2]]);
            }
        }
        let cells = sites
            .iter()
            .zip(carried)
            .map(|(&site, what)| {
                let (mass, fill) = weigh(&what.sites, s);
                Cell { site, mass, material: MATTER, fill, holds: 0, what }
            })
            .collect();
        self.woken += sites.len();
        self.matter.wake(cells, &mut self.view);
        for c in &mut self.matter.cells {
            if c.what.shown.is_none() {
                c.what.shown = Some((c.site, c.holds));
                self.view.awake.insert(c.site);
                self.view.carried.extend(c.what.loose_at(c.site));
            }
        }
    }

    /**
     * The loose matter that goes with the cells to wake (`cell`, by site):
     * that joined to them across faces, edges or corners, carried by
     * nothing awake, each with its cell (`goes_with`).
     */
    fn loose_with(&mut self, cell: &FastMap<[i64; 3], u32>) -> FastMap<[i64; 3], u32> {
        let reach = (REACH >> self.view.level).max(1);
        let mut loose = FastMap::default();
        let mut seen: FastSet<[i64; 3]> = cell.keys().copied().collect();
        let mut queue: VecDeque<[i64; 3]> = cell.keys().copied().collect();
        while let Some(site) = queue.pop_front() {
            for next in about(site) {
                if seen.insert(next)
                    && !self.view.carried.contains(&next)
                    && self.view.kind(next) == Kind::Loose
                    && let Some(c) = goes_with(next, cell, &mut self.view, reach)
                {
                    loose.insert(next, c);
                    queue.push_back(next);
                }
            }
        }
        loose
    }

    /** A step of what is awake, if anything is, and what it changed told when due. Whether anything is awake. */
    pub fn step(&mut self, tell: &mut dyn FnMut(Told)) -> bool {
        if self.matter.awake() {
            let mut slept = self.matter.step(&mut self.view);
            if self.matter.time - self.woke > LONGEST {
                slept.extend(self.matter.sleep());
            }
            self.fell_asleep(slept);
            let waking = self.matter.waking().into_iter();
            let waking: Vec<[i64; 3]> = waking.filter(|&w| self.view.asleep(w).is_some()).collect();
            if !waking.is_empty() && self.matter.cells.len() + waking.len() <= MOST {
                self.wake(waking);
            }
        }
        let awake = self.matter.awake();
        if (self.matter.time - self.told >= EVERY || !awake) && !self.untold.is_empty() {
            self.told = self.matter.time;
            self.changed += self.untold.len();
            tell(Told::Frame(self.matter.time, std::mem::take(&mut self.untold).into_iter().collect()));
        }
        if !awake && self.cuts > 0 {
            let found = format!(
                "mechanics: {} cells woken at level {}, {} welds broke; {} sites changed in {:.1} s simulated, {:.1} s taken, {} rounds",
                self.woken,
                self.matter.level,
                self.matter.broke,
                self.changed,
                self.matter.time,
                self.started.elapsed().as_secs_f64(),
                self.matter.rounds
            );
            tell(Told::Done(std::mem::take(&mut self.cuts), found));
            (self.woken, self.changed, self.told, self.woke) = (0, 0, 0.0, 0.0);
            self.matter = Matter::new(self.matter.level, UNIT_SIZE);
        }
        awake
    }

    /** What is awake shown as it is now, and `cells` fallen asleep with it: held still by the view from now on. */
    fn fell_asleep(&mut self, mut cells: Vec<Cell<Carried>>) {
        self.show(&mut cells);
        for c in &cells {
            self.view.awake.remove(&c.site);
            for site in c.what.loose_at(c.site) {
                self.view.carried.remove(&site);
            }
        }
    }

    /**
     * What changed of the cells awake and `slept` since each was told:
     * each moved or holding on otherwise is emptied where it was, then put
     * where it is, what it carries on its cube's faces letting go across
     * those it let go across; into the view, and to be told. Each empties
     * only what it put there, and the loose matter it carries is crushed
     * where it would lie on matter that bears.
     */
    fn show(&mut self, slept: &mut [Cell<Carried>]) {
        let s = self.view.size();
        let at = |site: [i64; 3], d: [i64; 3]| [site[0] * s + d[0], site[1] * s + d[1], site[2] * s + d[2]];
        let moved = |c: &Cell<Carried>| c.what.shown != Some((c.site, c.holds));
        let mut changes: FastMap<[i64; 3], (Held, Holds)> = FastMap::default();
        let cells = self.matter.cells.iter_mut().chain(slept.iter_mut()).filter(|c| moved(c));
        let cells: Vec<&mut Cell<Carried>> = cells.collect();
        for c in &cells {
            let (was, _) = c.what.shown.expect("told");
            self.view.awake.remove(&was);
            for site in c.what.loose_at(was) {
                self.view.carried.remove(&site);
            }
            for (d, held, _) in &c.what.sites {
                if self.view.held(at(was, *d)) == *held {
                    changes.insert(at(was, *d), (Held::default(), 0));
                }
            }
        }
        for c in cells {
            let view = &mut self.view;
            let mut bears = |zero: [i64; 3]| kind(&changes.get(&zero).map_or_else(|| view.held(zero), |h| h.0)) == Kind::Bearing;
            let site = c.site;
            c.what.loose.retain(|l| !cube([site[0] + l[0], site[1] + l[1], site[2] + l[2]], s).any(&mut bears));
            // What it carries is kept where it lies in its own cube, or with loose matter not crushed.
            let loose = &c.what.loose;
            c.what.sites.retain(|(d, ..)| {
                let l = d.map(|v| v.div_euclid(s));
                l == [0; 3] || loose.contains(&l)
            });
            for (d, held, holds) in &mut c.what.sites {
                let face = on_face(*d, s);
                *holds &= !(face & !c.holds);
                changes.insert(at(c.site, *d), (*held, *holds));
            }
            self.view.awake.insert(c.site);
            self.view.carried.extend(c.what.loose_at(c.site));
            c.what.shown = Some((c.site, c.holds));
        }
        self.view.change(changes.iter().map(|(&k, &v)| (k, v)));
        self.untold.extend(changes);
    }
}

/** A cell's mass, and the share of its cube that bears, of what it carries, its cube `s` sites a side. */
fn weigh(sites: &[([i64; 3], Held, Holds)], s: i64) -> (f64, f64) {
    // As much as what it carries fills its sites, what bears as heavy as MATTER, what is loose as LIGHT.
    let weighs = |held: &Held| f64::from(held.0.0) / f64::from(ALPHA) * if kind(held) == Kind::Bearing { MATTER.rho } else { LIGHT };
    let mass = sites.iter().map(|(_, held, _)| weighs(held)).sum::<f64>().max(LIGHT) * UNIT_SIZE.powi(3);
    let bears = sites.iter().filter(|(d, held, _)| d.iter().all(|&v| (0..s).contains(&v)) && kind(held) == Kind::Bearing).count();
    (mass, bears as f64 / (s * s * s) as f64)
}

/**
 * The cell loose matter at `site` goes with: the nearest of those in the
 * first cube about it holding any matter that bears (within `reach`), if
 * most of that is to wake.
 */
fn goes_with(site: [i64; 3], cell: &FastMap<[i64; 3], u32>, view: &mut View, reach: i64) -> Option<u32> {
    for r in 1..=reach {
        let (mut still, mut moved, mut nearest) = (0, 0, None::<(i64, u32)>);
        let cube = (-r..=r).flat_map(|dx| (-r..=r).flat_map(move |dy| (-r..=r).map(move |dz| [dx, dy, dz])));
        for d in cube.filter(|d| d.iter().any(|v| v.abs() == r)) {
            let near = [site[0] + d[0], site[1] + d[1], site[2] + d[2]];
            match cell.get(&near) {
                Some(&c) => {
                    moved += 1;
                    let this = (d.iter().map(|v| v * v).sum::<i64>(), c);
                    nearest = Some(nearest.map_or(this, |n| n.min(this)));
                }
                None if view.kind(near) == Kind::Bearing => still += 1,
                None => {}
            }
        }
        if moved + still > 0 {
            return nearest.filter(|_| moved > still).map(|(_, c)| c);
        }
    }
    None
}

/** A cut, as mechanics is asked of it: the level-0 sites it took out. */
pub struct Job {
    pub cut: Vec<[i64; 3]>,
}

/**
 * Mechanics on a thread of its own: a cut is posted, what it sets moving
 * comes back as it is found, and is shown as it moves, each change once
 * it is due and the world shows the last (World::editing), so a slow
 * world skips to what is due rather than lags.
 */
pub struct Mechanics {
    post: Post<Job>,
    told: Receiver<Told>,
    /** Cuts posted, not at rest yet. */
    pub waiting: usize,
    /** Since when the motion now told of is shown; what changes, when. */
    since: Instant,
    due: VecDeque<(Instant, Changes)>,
    /** What the last at rest found. */
    pub found: String,
}

impl Mechanics {
    pub fn new(world: Arc<worldgen::World>) -> Mechanics {
        let (send, told): (Sender<Told>, _) = channel();
        let post = Post::spawn(|jobs: Receiver<Job>| {
            vec![move || {
                crate::world::behind_drawing();
                let mut field = Field::new(world);
                let mut tell = |t| _ = send.send(t);
                loop {
                    loop {
                        match jobs.try_recv() {
                            Ok(job) => field.cut(job.cut, &mut tell),
                            Err(TryRecvError::Empty) => break,
                            Err(TryRecvError::Disconnected) => return,
                        }
                    }
                    if !field.step(&mut tell) {
                        std::thread::park();
                    }
                }
            }]
        });
        Mechanics { post, told, waiting: 0, since: Instant::now(), due: VecDeque::new(), found: String::new() }
    }

    pub fn post(&mut self, job: Job) {
        self.waiting += 1;
        self.post.send(job);
    }

    /** Takes what was told, and has `world` show what is due by now. Whether anything still moves or is to be shown. */
    pub fn moving(&mut self, world: &mut World) -> bool {
        for told in self.told.try_iter() {
            match told {
                Told::Start => self.since = Instant::now().max(self.due.back().map_or(self.since, |d| d.0)),
                Told::Frame(t, changes) => self.due.push_back((self.since + Duration::from_secs_f64(t), changes)),
                Told::Done(cuts, found) => {
                    self.waiting -= cuts;
                    self.found = found;
                }
            }
        }
        if !world.editing() {
            let now = Instant::now();
            let mut changes = Vec::new();
            while self.due.front().is_some_and(|d| d.0 <= now) {
                changes.extend(self.due.pop_front().expect("due").1);
            }
            world.change(changes);
        }
        self.waiting > 0 || !self.due.is_empty()
    }
}

#[cfg(test)]
mod tests;
