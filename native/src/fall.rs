//! What a cut sets moving, simulated on a thread of its own, which the
//! thread that draws never waits for (threads.rs), and shown as it moves.
//! It knows nothing of what things are: matter bears (it stops a walking
//! body, as a world marks its kinds) or is loose (`kind`).
//!
//! What bears, touching the cut across faces, is matter (mechanics/,
//! docs/mechanics.md), all of one material (MATTER): it bends, breaks,
//! falls and lands, all on the lattice, welded where it holds on (edits'
//! `holds`), and holding on after where it is welded. A cut bears what is
//! over it, so what lies lower holds still, as does what lies at the edge
//! of what is looked at (`Around`). Loose matter bears nothing: it goes
//! with the matter that bears nearest it (`goes_with`).
//!
//! Matter is simulated at the finest level of detail it is few enough at
//! (BUDGET): a cell is a voxel of that level that bears, and carries the
//! level-0 sites of its cube, and of the loose matter that goes with it,
//! moving them by its size. So what level 0 holds moves site by site, and
//! what is too large for it by the cells of a coarser level, as that level
//! draws it. A cell holds on across a face where most of what bears on
//! that face of its cube does, and those sites hold on after as it does
//! (`on_face`): at level 0, each site as itself.

use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};
use std::time::Duration;

use lattice::{ALPHA, FACE_KINDS, FastMap, FastSet, Holds};
use mechanics::{Cell, Material, Matter, WOOD, faces};
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
/** How far across from a cut matter is looked for, each way, and how far over it, lattice units. */
const REGION: i64 = m(10.0);
const ABOVE: i64 = m(30.0);
/** Cells past which a level is too fine to simulate, and the next coarser is; levels tried. */
const BUDGET: usize = 6000;
const LEVELS: u32 = 4;
/** How far from loose matter what bears, that it goes with, is looked for, lattice units. */
const REACH: i64 = m(2.0);
/** Seconds between what is shown; a fall is followed for, at most. */
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

/** A world's sites of a level as its edits had them at some time, made as asked; above level 0, level 0's too, which its cells carry. */
struct Voxels {
    world: Arc<worldgen::World>,
    edits: Arc<Sites>,
    level: u32,
    sites: FastMap<[i64; 3], Arc<VoxelGrid>>,
    fine: Option<Box<Voxels>>,
}

impl Voxels {
    fn new(world: &Arc<worldgen::World>, edits: &Arc<Sites>, level: u32) -> Voxels {
        let fine = (level > 0).then(|| Box::new(Voxels::new(world, edits, 0)));
        Voxels { world: world.clone(), edits: edits.clone(), level, sites: FastMap::default(), fine }
    }

    /** What holds a level-0 site. */
    fn fine(&mut self, zero: [i64; 3]) -> Held {
        match &mut self.fine {
            Some(fine) => fine.held(zero),
            None => self.held(zero),
        }
    }

    fn held(&mut self, site: [i64; 3]) -> Held {
        let key = site.map(|v| v.div_euclid(TILE));
        if !self.sites.contains_key(&key) {
            let sites = Arc::new(sites_as(&self.world, self.level, key, &self.edits));
            self.sites.extend(made_with(&self.world, self.level, key).map(|with| (with, sites.clone())));
        }
        let [x, y, z] = site;
        self.sites[&key].held(x, y, z)
    }

    fn kind(&mut self, site: [i64; 3]) -> Kind {
        kind(&self.held(site))
    }

    /** Across which faces what a level-0 site holds holds on. */
    fn holds(&self, zero: [i64; 3]) -> Holds {
        holds(&self.edits, zero)
    }

    /** The level-0 site a site of this level is read at. */
    fn read_at(&self, site: [i64; 3]) -> [i64; 3] {
        let s = 1i64 << self.level;
        site.map(|v| v * s + middle(s))
    }
}

/** What is looked at about a cut, in level-0 sites: its sites, the lowest's height, its middle across. */
struct Around {
    cut: FastSet<[i64; 3]>,
    lowest: i64,
    middle: [i64; 2],
}

impl Around {
    fn of(cut: &[[i64; 3]]) -> Option<Around> {
        let lowest = cut.iter().map(|s| s[1]).min()?;
        let n = cut.len() as i64;
        let middle = [0, 2].map(|k| cut.iter().map(|s| s[k]).sum::<i64>() / n);
        Some(Around { cut: cut.iter().copied().collect(), lowest, middle })
    }

    fn across(&self, s: [i64; 3]) -> i64 {
        (s[0] - self.middle[0]).abs().max((s[2] - self.middle[1]).abs())
    }

    fn inside(&self, s: [i64; 3]) -> bool {
        self.across(s) <= REGION && s[1] <= self.lowest + ABOVE
    }

    /** Lower than the cut, or at the edge. */
    fn held(&self, s: [i64; 3]) -> bool {
        s[1] < self.lowest || self.across(s) >= REGION || s[1] >= self.lowest + ABOVE
    }
}

/**
 * What bears, touching the cut (`cut`, sites of `voxels`' level) across
 * faces, holding on or not, before it: the sites that move, the cut's
 * among them; None past BUDGET.
 */
fn moving(voxels: &mut Voxels, around: &Around, cut: &FastSet<[i64; 3]>) -> Option<FastSet<[i64; 3]>> {
    let mut moving: FastSet<[i64; 3]> = FastSet::default();
    let mut queue: VecDeque<[i64; 3]> = VecDeque::new();
    for &site in cut {
        if voxels.kind(site) == Kind::Bearing && moving.insert(site) {
            queue.push_back(site);
        }
    }
    let mut seen = moving.clone();
    while let Some(site) = queue.pop_front() {
        for next in faces(site) {
            let zero = voxels.read_at(next);
            if seen.insert(next) && around.inside(zero) && !around.held(zero) && voxels.kind(next) == Kind::Bearing {
                if moving.len() >= BUDGET {
                    return None;
                }
                moving.insert(next);
                queue.push_back(next);
            }
        }
    }
    Some(moving)
}

/**
 * The loose matter that goes with the cells (`cell`, by site): that
 * joined to them across faces, edges or corners, `inside` what is looked
 * at, each with its cell (`goes_with`).
 */
fn loose(cell: &FastMap<[i64; 3], u32>, voxels: &mut Voxels, around: &Around, reach: i64) -> FastMap<[i64; 3], u32> {
    let is_loose = |voxels: &mut Voxels, site: [i64; 3]| voxels.kind(site) == Kind::Loose;
    let mut loose = FastMap::default();
    let mut seen: FastSet<[i64; 3]> = cell.keys().copied().collect();
    let mut queue: VecDeque<[i64; 3]> = cell.keys().copied().collect();
    while let Some(site) = queue.pop_front() {
        for next in about(site) {
            if seen.insert(next)
                && around.inside(voxels.read_at(next))
                && is_loose(voxels, next)
                && let Some(c) = goes_with(next, cell, voxels, reach)
            {
                loose.insert(next, c);
                queue.push_back(next);
            }
        }
    }
    loose
}

/**
 * The cell loose matter at `site` goes with: that of the most of the
 * matter that bears nearest it (in the first cube about it holding any,
 * within `reach`), if most of that moves.
 */
fn goes_with(site: [i64; 3], cell: &FastMap<[i64; 3], u32>, voxels: &mut Voxels, reach: i64) -> Option<u32> {
    for r in 1..=reach {
        let (mut still, mut moves) = (0, FastMap::<u32, usize>::default());
        let cube = (-r..=r).flat_map(|dx| (-r..=r).flat_map(move |dy| (-r..=r).map(move |dz| [dx, dy, dz])));
        for d in cube.filter(|d| d.iter().any(|v| v.abs() == r)) {
            let near = [site[0] + d[0], site[1] + d[1], site[2] + d[2]];
            match cell.get(&near) {
                Some(&c) => *moves.entry(c).or_default() += 1,
                None if voxels.kind(near) == Kind::Bearing => still += 1,
                None => {}
            }
        }
        let moved: usize = moves.values().sum();
        if moved + still > 0 {
            return (moved > still).then(|| moves.into_iter().max_by_key(|&(c, n)| (n, std::cmp::Reverse(c))).expect("some").0);
        }
    }
    None
}

/** The level-0 sites of the cube of a site of the level whose sites are `s` across. */
fn cube(c: [i64; 3], s: i64) -> impl Iterator<Item = [i64; 3]> {
    (0..s * s * s).map(move |k| [c[0] * s + k % s, c[1] * s + k / s % s, c[2] * s + k / (s * s)])
}

/** The sites about a site, across its faces, edges and corners. */
fn about(s: [i64; 3]) -> impl Iterator<Item = [i64; 3]> {
    (0..27).filter(|&d| d != 13).map(move |d| [s[0] + d % 3 - 1, s[1] + d / 3 % 3 - 1, s[2] + d / 9 - 1])
}

/** A cut, as mechanics is asked of it: the level-0 sites it took out, and the world's edits just before. */
pub struct Job {
    pub cut: Vec<[i64; 3]>,
    pub before: Arc<Sites>,
}

/** What a cell carries: level-0 sites from its cube's first, what each holds, and across which faces that holds on. */
type Carried = Vec<([i64; 3], Held, Holds)>;

/** Level-0 sites changed, what each holds now, and across which faces that holds on. */
type Changes = Vec<([i64; 3], (Held, Holds))>;

/** Across which faces a cell holds on that carries `carried`, its cube `s` sites a side: those most of what bears on does. */
fn holding(carried: &Carried, s: i64) -> Holds {
    let holds = |f: usize| {
        let on = carried.iter().filter(|(d, held, _)| on_face(*d, s) & (1 << f) != 0 && kind(held) == Kind::Bearing);
        let (all, holding) = on.fold((0, 0), |(a, h), (_, _, holds)| (a + 1, h + usize::from(holds & (1 << f) != 0)));
        2 * holding > all
    };
    (0..FACE_KINDS).filter(|&f| holds(f)).fold(0, |h, f| h | 1 << f)
}

/** The faces of its cell's cube, `s` sites a side, that a site carried at `d` from its first lies on: none, if it is not in the cube. */
fn on_face(d: [i64; 3], s: i64) -> Holds {
    if d.iter().any(|&v| !(0..s).contains(&v)) {
        return 0;
    }
    (0..FACE_KINDS).filter(|&f| d[f / 2] == if f % 2 == 0 { 0 } else { s - 1 }).fold(0, |h, f| h | 1 << f)
}

/** What a cut sets moving: matter, the sites it was at (the cut's among them), the world at its level. */
struct Moving {
    matter: Matter<Carried>,
    sites: FastSet<[i64; 3]>,
    voxels: Voxels,
}

/** What the simulation tells: a cut's motion starts; what changes `t` seconds in; it is at rest, and what was found. */
pub enum Told {
    Start,
    Frame(f64, Changes),
    Done(String),
}

/** What a cut sets moving, as cells of the finest level few enough, carrying their level-0 sites; why not, if it is not looked at. */
fn moving_matter(world: &Arc<worldgen::World>, job: &Job, around: &Around) -> Result<Moving, String> {
    for level in 0..LEVELS {
        let mut voxels = Voxels::new(world, &job.before, level);
        let s = 1i64 << level;
        // The cut as this level reads it: the sites read at a site cut (edits.rs).
        let cut: FastSet<[i64; 3]> =
            around.cut.iter().map(|c| c.map(|v| v.div_euclid(s))).filter(|&c| around.cut.contains(&voxels.read_at(c))).collect();
        let Some(moving) = moving(&mut voxels, around, &cut) else { continue };
        let sites: Vec<[i64; 3]> = moving.iter().copied().collect();
        let index: FastMap<[i64; 3], u32> = sites.iter().enumerate().map(|(i, &s)| (s, i as u32)).collect();
        let loose = loose(&index, &mut voxels, around, (REACH >> level).max(1));
        let mut carried: Vec<Carried> = vec![Vec::new(); sites.len()];
        for (&site, &c) in index.iter().chain(&loose) {
            let origin = sites[c as usize].map(|v| v * s);
            for zero in cube(site, s) {
                let held = voxels.fine(zero);
                if kind(&held) != Kind::Empty && !around.cut.contains(&zero) {
                    let d = [zero[0] - origin[0], zero[1] - origin[1], zero[2] - origin[2]];
                    carried[c as usize].push((d, held, voxels.holds(zero)));
                }
            }
        }
        let cells = sites
            .iter()
            .zip(carried)
            .map(|(&site, carried)| {
                // As much as what it carries fills its sites, what bears as heavy as MATTER, what is loose as LIGHT.
                let weighs =
                    |held: &Held| f64::from(held.0.0) / f64::from(ALPHA) * if kind(held) == Kind::Bearing { MATTER.rho } else { LIGHT };
                let mass = carried.iter().map(|(_, held, _)| weighs(held)).sum::<f64>().max(LIGHT) * UNIT_SIZE.powi(3);
                Cell { site, mass, material: MATTER, holds: holding(&carried, s), what: carried }
            })
            .collect();
        let mut matter = Matter::new(level, UNIT_SIZE, cells, &mut |site| {
            (voxels.kind(site) == Kind::Bearing).then(|| {
                let carried: Carried =
                    cube(site, s).map(|zero| (zero.map(|v| v.rem_euclid(s)), voxels.fine(zero), voxels.holds(zero))).collect();
                (MATTER, holding(&carried, s))
            })
        });
        matter.take_out(&cut);
        return Ok(Moving { matter, sites: moving, voxels });
    }
    Err(format!("mechanics: over {BUDGET} cells about the cut at every level, not looked at"))
}

/** What `job`'s cut sets moving in `world`, simulated till it rests, each change told by `tell` as found. */
pub fn simulate(world: Arc<worldgen::World>, job: &Job, tell: &mut dyn FnMut(Told)) {
    let started = Instant::now();
    tell(Told::Start);
    let Some(around) = Around::of(&job.cut) else { return tell(Told::Done(String::new())) };
    let Moving { mut matter, sites: moving, mut voxels } = match moving_matter(&world, job, &around) {
        Ok(found) => found,
        Err(told) => return tell(Told::Done(told)),
    };
    let (cells, welds) = (matter.cells.len(), matter.welds());
    let s = 1i64 << matter.level;
    // The world's sites that stop matter: what bears and was not set moving.
    let mut still = |site: [i64; 3]| !moving.contains(&site) && voxels.kind(site) == Kind::Bearing;
    // Each cell as the world shows it: where, holding on to what.
    let mut shown: Vec<([i64; 3], Holds)> = matter.cells.iter().map(|c| (c.site, holding(&c.what, s))).collect();
    let (mut last, mut changed) = (0.0, 0);
    let mut show = |matter: &mut Matter<Carried>, shown: &mut [([i64; 3], Holds)]| {
        let changes = changes(matter, shown);
        changed += changes.len();
        if !changes.is_empty() {
            tell(Told::Frame(matter.time, changes));
        }
    };
    while !matter.cells.is_empty() && matter.time < LONGEST && !matter.at_rest() {
        matter.step(&mut still);
        if matter.time - last >= EVERY {
            last = matter.time;
            show(&mut matter, &mut shown);
        }
    }
    show(&mut matter, &mut shown);
    tell(Told::Done(format!(
        "mechanics: {cells} cells at level {}, {welds} welds, {} broke; {changed} sites changed in {:.1} s simulated, {:.1} s taken, {} rounds",
        matter.level,
        welds - matter.welds(),
        matter.time,
        started.elapsed().as_secs_f64(),
        matter.rounds
    )));
}

/**
 * What changed since `shown` (each cell's site and what it held on to, as
 * told last), and `shown` as now: each cell moved or holding on otherwise
 * is emptied where it was, then put where it is, what it carries on its
 * cube's faces holding on as it does.
 */
fn changes(matter: &mut Matter<Carried>, shown: &mut [([i64; 3], Holds)]) -> Changes {
    let s = 1i64 << matter.level;
    let at = |site: [i64; 3], d: [i64; 3]| [site[0] * s + d[0], site[1] * s + d[1], site[2] * s + d[2]];
    let changed: Vec<usize> = (0..shown.len()).filter(|&i| shown[i] != (matter.cells[i].site, matter.cells[i].holds)).collect();
    let mut changes = FastMap::default();
    for &i in &changed {
        for &(d, ..) in &matter.cells[i].what {
            changes.insert(at(shown[i].0, d), (Held::default(), 0));
        }
    }
    for &i in &changed {
        let cell = &mut matter.cells[i];
        for (d, held, holds) in &mut cell.what {
            let face = on_face(*d, s);
            *holds = *holds & !face | cell.holds & face;
            changes.insert(at(cell.site, *d), (*held, *holds));
        }
        shown[i] = (cell.site, cell.holds);
    }
    changes.into_iter().collect()
}

/**
 * Mechanics after cuts, on a thread of its own: a cut is posted, what it
 * sets moving comes back as it is found, and is shown as it moves, each
 * change once it is due and the world shows the last (World::editing),
 * so a slow world skips to what is due rather than lags.
 */
pub struct Falls {
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

impl Falls {
    pub fn new(world: Arc<worldgen::World>) -> Falls {
        let (send, told): (Sender<Told>, _) = channel();
        let post = Post::spawn(|jobs: Receiver<Job>| {
            vec![move || {
                crate::world::behind_drawing();
                loop {
                    match jobs.try_recv() {
                        Ok(job) => simulate(world.clone(), &job, &mut |t| _ = send.send(t)),
                        Err(TryRecvError::Empty) => std::thread::park(),
                        Err(TryRecvError::Disconnected) => return,
                    }
                }
            }]
        });
        Falls { post, told, waiting: 0, since: Instant::now(), due: VecDeque::new(), found: String::new() }
    }

    pub fn post(&mut self, job: Job) {
        self.waiting += 1;
        self.post.send(job);
    }

    /** Takes what was told, and has `world` show what is due by now. Whether anything still moves or is to be shown. */
    pub fn falling(&mut self, world: &mut World) -> bool {
        for told in self.told.try_iter() {
            match told {
                Told::Start => self.since = Instant::now().max(self.due.back().map_or(self.since, |d| d.0)),
                Told::Frame(t, changes) => self.due.push_back((self.since + Duration::from_secs_f64(t), changes)),
                Told::Done(found) => {
                    self.waiting -= 1;
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
