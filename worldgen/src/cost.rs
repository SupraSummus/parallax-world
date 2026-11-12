//! What generating costs: work counted by layer while `counted` runs,
//! the same on every machine, weighed into a rough estimate of its time;
//! a report of a world's host tiles at several levels, what in them looks
//! costly, and how they differ from a baseline (docs/generators.md,
//! "Cost").

use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::fmt::Write;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use crate::World;
use crate::generator::{generate, shown};
use crate::grid::Box3;

thread_local! {
    /** Whether a `counted` is running on this thread: generating unwatched, the counts are only made. */
    static COUNTING: Cell<bool> = const { Cell::new(false) };
    /** Its counts. */
    static COUNTS: RefCell<Counts> = const { RefCell::new(Counts { layers: Vec::new(), plans: Vec::new() }) };
    /**
     * Plans looked up (`Plan::at`) since the last were counted to a layer
     * or a plan: kept unwatched too, as cheap as asking whether to.
     */
    static LOOKUPS: Cell<u64> = const { Cell::new(0) };
    /** The columns plans made while counting, each by its plan's address: to tell those made again. */
    static MADE: RefCell<HashSet<(usize, [i64; 3])>> = RefCell::new(HashSet::new());
}

/** Defines `Work`, a count of each of what time goes on, and its counts' names in order. */
macro_rules! work {
    ($($(#[$doc:meta])* $name:ident,)*) => {
        /** Work done for a layer, or a plan: each a count of what its time goes on. */
        #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
        pub struct Work {
            $($(#[$doc])* pub $name: u64,)*
        }

        /** The names of `Work`'s counts, in order. */
        const NAMES: [&str; [$(stringify!($name)),*].len()] = [$(stringify!($name)),*];

        impl Work {
            fn counts(&self) -> [u64; NAMES.len()] {
                [$(self.$name),*]
            }

            fn of(counts: [u64; NAMES.len()]) -> Work {
                let [$($name),*] = counts;
                Work { $($name),* }
            }
        }
    };
}

work! {
    /** Cells of a population whose anchor was asked for, or lines of them (`Anchors::Lines`). */
    cells,
    /** Cells whose anchor is in them: those asked to make a thing. */
    owned,
    /** Nodes drawn: things and their children, near or far. */
    nodes,
    /** Sites tested against a shape: the inner loop of drawing things. */
    tests,
    /** Sites of the boxes things are drawn in whole, to be measured (`Far::measured`). */
    measured,
    /** A field's values: at a site, or a line of sites' spans. */
    reads,
    /** Plans' columns looked up, found kept or made: counted to the layer or plan looking them up. */
    lookups,
    /** Columns a plan decided (made, not found kept); the lookups making them are its `lookups`. */
    plans,
    /** Of those, columns made for the first time: fewer where some were made again. */
    distinct,
    /** The most sites tested for one thing in one box. */
    largest,
}

/**
 * Rough nanoseconds per count, to weigh counts into one estimate:
 * fitted once, by least squares, to the times of every world's tiles
 * at levels 0 to 6 on one busy machine; `measured` scaled since by how
 * much less time measuring a site took once it was made faster;
 * `lookups` set since between what one took in two changes that added
 * many (claims making their keys afresh for every column, a canopy
 * reading a plan around each), as it varies with what it finds, and
 * `plans` refitted with it. They rank work more than they time it; a
 * report's "took here" shows how far off they are elsewhere. `distinct`
 * is among `plans` and `largest` among `tests`, so weigh nothing of
 * their own.
 */
const NS: Work =
    Work { cells: 100, owned: 1300, nodes: 1800, tests: 50, measured: 35, reads: 20, lookups: 25, plans: 45, distinct: 0, largest: 0 };

impl Work {
    /** Adds `w`'s counts, but for the largest, the larger. */
    pub(crate) fn add(&mut self, w: &Work) {
        let (a, b, largest) = (self.counts(), w.counts(), self.largest.max(w.largest));
        *self = Work { largest, ..Work::of(std::array::from_fn(|i| a[i] + b[i])) };
    }

    /** Its time estimated with NS, in nanoseconds. */
    pub fn ns(&self) -> u64 {
        self.counts().iter().zip(NS.counts()).map(|(c, ns)| c * ns).sum()
    }

    /** How many times each column was made, on average: a plan's (0 for a layer). */
    pub fn times_made(&self) -> f64 {
        self.plans as f64 / self.distinct.max(1) as f64
    }
}

/** Work by layer (each field and population by name) and by plan, in the order first counted. */
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    pub layers: Vec<(&'static str, Work)>,
    pub plans: Vec<(&'static str, Work)>,
}

impl Counts {
    /** All of it but what is done once a world (SHADOWS). */
    pub fn total(&self) -> Work {
        let mut total = Work::default();
        self.rows().filter(|(name, _)| name != SHADOWS).for_each(|(_, w)| total.add(w));
        total
    }

    /** Plans making each column over REMADE times, and how many. */
    pub fn remade(&self) -> impl Iterator<Item = (&'static str, f64)> {
        self.plans.iter().map(|(name, w)| (*name, w.times_made())).filter(|(_, times)| *times > REMADE)
    }

    /** Layers by name, then plans as "plan <name>". */
    fn rows(&self) -> impl Iterator<Item = (String, &Work)> {
        let layers = self.layers.iter().map(|(name, w)| (name.to_string(), w));
        layers.chain(self.plans.iter().map(|(name, w)| (format!("plan {name}"), w)))
    }
}

fn add_to(rows: &mut Vec<(&'static str, Work)>, name: &'static str, work: &Work) {
    match rows.iter_mut().find(|(n, _)| *n == name) {
        Some((_, w)) => w.add(work),
        None => rows.push((name, *work)),
    }
}

/** Runs `f`, counting the work of what it generates on this thread. */
pub fn counted<R>(f: impl FnOnce() -> R) -> (R, Counts) {
    let outer = (COUNTING.replace(true), COUNTS.take(), LOOKUPS.take(), MADE.take());
    let value = f();
    COUNTING.set(outer.0);
    LOOKUPS.set(outer.2);
    MADE.set(outer.3);
    (value, COUNTS.replace(outer.1))
}

/** Adds `work` to `layer`'s, if counting, with the plans looked up since work was last added: the layer's. */
#[inline]
pub(crate) fn add(layer: &'static str, work: Work) {
    let lookups = LOOKUPS.take();
    if COUNTING.get() {
        let work = Work { lookups: work.lookups + lookups, ..work };
        COUNTS.with_borrow_mut(|counts| add_to(&mut counts.layers, layer, &work));
    }
}

/** Counts a plan looked up. */
#[inline]
pub(crate) fn lookup() {
    LOOKUPS.set(LOOKUPS.get() + 1);
}

/**
 * Makes column `at` of `plan` with `make`, counting it and the lookups
 * making it, if counting; `of` is the plan, told by its address from
 * others of its name.
 */
#[inline]
pub(crate) fn plan<T, P>(plan: &'static str, of: &P, at: [i64; 3], make: impl FnOnce() -> T) -> T {
    let outer = LOOKUPS.take();
    let value = make();
    let lookups = LOOKUPS.replace(outer);
    if COUNTING.get() {
        let distinct = MADE.with_borrow_mut(|made| made.insert((std::ptr::from_ref(of).addr(), at))).into();
        let work = Work { lookups, plans: 1, distinct, ..Work::default() };
        COUNTS.with_borrow_mut(|counts| add_to(&mut counts.plans, plan, &work));
    }
    value
}

/** What `Shadows` draws counts to: things measured once a world, not per tile. */
pub(crate) const SHADOWS: &str = "shadows";

// ---------------------------------------------------------------------------
// Reports

/**
 * Places to report on: a world's spawn and the centres of `regions`
 * (lattice units), by name.
 */
pub fn places(world: &World, regions: &[Box3]) -> Vec<(String, [i64; 3])> {
    let centres = regions.iter().enumerate().map(|(i, r)| (format!("region{}", i + 1), [0, 1, 2].map(|a| (r.min[a] + r.max[a]) / 2)));
    [("spawn".to_string(), world.spawn)].into_iter().chain(centres).collect()
}

/** Sites of the web's tile along x and z (TILE in src/lib/world.ts; native's are cubes, light::tile_box). */
const TILE: i64 = 128;

/** The host tile of `level` holding `at` (lattice units): TILE sites of the level along x and z, the world's heights along y, what may show of it (shown). */
fn tile(world: &World, at: [i64; 3], level: u32) -> Box3 {
    let (size, (y0, y1)) = (TILE << level, world.heights_at(level));
    let [x, _, z] = at.map(|v| v.div_euclid(size) * TILE);
    shown(&world.generator, &Box3 { min: [x, y0, z], max: [x + TILE, y1, z + TILE] }, level)
}

/** A tile of a report: where and at what level, its work, and how long it took here. */
#[derive(Clone, Debug)]
pub struct Tile {
    pub place: String,
    pub level: u32,
    pub b: Box3,
    pub counts: Counts,
    /** What the counts estimate, but on this machine, with the other tiles on all cores and whatever else ran. */
    pub took: Duration,
}

/**
 * A world's work in host tiles at several levels: shown, its estimated
 * time by layer and level, largest first, with what looks costly (`flags`).
 */
pub struct Report {
    pub name: String,
    pub tiles: Vec<Tile>,
}

/**
 * The work of the host tile of each of `levels` holding each of `places`
 * (named, lattice units), each generated by a world of its own (`make`),
 * as a host's first tile is; once where places share a tile. Tiles are
 * generated on all cores.
 */
pub fn report(name: &str, make: &(dyn Fn() -> World + Sync), places: &[(String, [i64; 3])], levels: &[u32]) -> Report {
    let world = make();
    let mut tiles: Vec<Tile> = Vec::new();
    for (place, at) in places {
        for &level in levels {
            let b = tile(&world, *at, level);
            if !tiles.iter().any(|t| t.level == level && t.b == b) {
                tiles.push(Tile { place: place.clone(), level, b, counts: Counts::default(), took: Duration::ZERO });
            }
        }
    }
    let next = AtomicUsize::new(0);
    let done: Vec<(usize, Counts, Duration)> = std::thread::scope(|scope| {
        let threads: Vec<_> = (0..std::thread::available_parallelism().map_or(1, |n| n.get()))
            .map(|_| {
                scope.spawn(|| {
                    let mut done = Vec::new();
                    loop {
                        let i = next.fetch_add(1, Ordering::Relaxed);
                        let Some(t) = tiles.get(i) else { break done };
                        // A World isn't Send: a world each, which starts cold, as a host's first tile does.
                        let world = make();
                        let start = Instant::now();
                        let counts = counted(|| generate(&world.generator, &t.b, t.level)).1;
                        done.push((i, counts, start.elapsed()));
                    }
                })
            })
            .collect();
        threads.into_iter().flat_map(|t| t.join().expect("generated")).collect()
    });
    for (i, counts, took) in done {
        (tiles[i].counts, tiles[i].took) = (counts, took);
    }
    Report { name: name.to_string(), tiles }
}

/** How many times as much a layer may take per tile from one level of a report to the next before it is flagged. */
const GROWTH: u64 = 4;
/** How many cells a population may ask per anchor in them before it is flagged. */
const CELLS_PER_ANCHOR: u64 = 8;
/** The share of a tile's time any of these must take to be flagged. */
const SHARE: f64 = 0.1;
/** The share of a tile's time one thing, or things drawn whole to be measured, may take before it is flagged. */
const LARGE: f64 = 0.25;
/** The time a tile may take, estimated, before it is flagged (and fails worlds/tests/cost.rs): nanoseconds. */
pub const BUDGET: u64 = 200_000_000;
/**
 * How many times a plan may make each column of a tile, on average,
 * before it is flagged (and fails worlds/tests/cost.rs): dithered or
 * coarse plans make some again where a tile reads more than their slots
 * keep apart, up to about 1.8 times; a cache that fails makes them
 * several times each.
 */
pub const REMADE: f64 = 2.5;

fn ms(ns: u64) -> f64 {
    ns as f64 / 1e6
}

impl Report {
    /**
     * What looks costly, by tile: a tile over BUDGET; a plan making its
     * columns over REMADE times each; a layer taking
     * GROWTH times as much as at the level before, cells asked for anchors
     * in vain, a field's values read at every site, one huge thing, or
     * things drawn whole to be measured.
     */
    pub fn flags(&self) -> Vec<String> {
        let mut flags = Vec::new();
        for t in &self.tiles {
            let total = t.counts.total().ns();
            let at = format!("{} level {}", t.place, t.level);
            // Work of note: a millisecond or more, and `share` of the tile.
            let flag = |flags: &mut Vec<String>, ns: u64, share: f64, what: String| {
                if ns >= 1_000_000 && ns as f64 >= share * total as f64 {
                    flags.push(format!("{at}: {what}, {:.0} ms, {:.0} % of the tile", ms(ns), 100.0 * ns as f64 / total as f64));
                }
            };
            if total > BUDGET {
                flags.push(format!("{at}: the tile takes {:.0} ms, over the budget of {:.0}", ms(total), ms(BUDGET)));
            }
            for (plan, times) in t.counts.remade() {
                flags.push(format!("{at}: plan {plan} makes each column {times:.1} times"));
            }
            let [x, y, z] = [0, 1, 2].map(|i| (t.b.max[i] - t.b.min[i]) as u64);
            for (layer, w) in t.counts.rows().filter(|(name, _)| name != SHADOWS) {
                if w.cells > CELLS_PER_ANCHOR * w.owned.max(1) {
                    let what = format!("{layer} asks {} cells for {} anchors in them", w.cells, w.owned);
                    flag(&mut flags, Work { cells: w.cells, ..Work::default() }.ns(), SHARE, what);
                }
                // Read at half the tile's sites or more (x·y·z / 2 of them), not once per line of them.
                if 4 * w.reads >= x * y * z && w.reads > x * z {
                    let what = format!("{layer} reads a value at each of {} sites", w.reads);
                    flag(&mut flags, Work { reads: w.reads, ..Work::default() }.ns(), SHARE, what);
                }
                let what = format!("one thing of {layer} tests {} sites", w.largest);
                flag(&mut flags, Work { tests: w.largest, ..Work::default() }.ns(), LARGE, what);
                let what = format!("{layer} draws things whole to measure them, {} sites", w.measured);
                flag(&mut flags, Work { measured: w.measured, ..Work::default() }.ns(), LARGE, what);
                // The same place's tile at the level before; a coarser tile is wider, and may hold more of what the
                // layer draws, so only where the layer took a share of that one too.
                let before = self.tiles.iter().filter(|u| u.place == t.place && u.level < t.level).max_by_key(|u| u.level);
                let Some(before) = before else { continue };
                let was = before.counts.rows().find(|(name, _)| *name == layer).map_or(0, |(_, w)| w.ns());
                if w.ns() > GROWTH * was && was as f64 >= SHARE * before.counts.total().ns() as f64 {
                    let what = format!("{layer} grew from {:.1} ms at level {}", ms(was), before.level);
                    flag(&mut flags, w.ns(), SHARE, what);
                }
            }
        }
        flags
    }

    /** Every count of every tile, a line each under a line of their names: what `changes` reads. */
    pub fn baseline(&self) -> String {
        let mut text = format!("# {}: work by tile and layer (worldgen::cost)\n{}\n", self.name, header());
        for t in &self.tiles {
            for (layer, w) in t.counts.rows() {
                write!(text, "{:<12} {:>5}", t.place, t.level).expect("written");
                w.counts().iter().for_each(|c| write!(text, " {c:>9}").expect("written"));
                writeln!(text, " {layer}").expect("written");
            }
        }
        text
    }

    /**
     * Counts that differ from those of `baseline` (a `baseline`'s text) by
     * more than `tolerance` (0.25: a quarter), up or down, where the
     * difference weighs at least a hundredth of the tile.
     */
    pub fn changes(&self, baseline: &str, tolerance: f64) -> Vec<String> {
        let mut lines = baseline.lines().filter(|l| !l.starts_with('#'));
        if lines.next().map(|l| l.split_whitespace().eq(header().split_whitespace())) != Some(true) {
            return vec!["the baseline counts other things".to_string()];
        }
        // (tile, layer, work); a layer's name comes last, and may have spaces.
        let old: Vec<(String, String, Work)> = lines
            .map(|line| {
                let words: Vec<&str> = line.split_whitespace().collect();
                let count = |i: usize| words[2 + i].parse().unwrap_or_else(|_| panic!("a count at {i} of {line:?}"));
                (format!("{} level {}", words[0], words[1]), words[2 + NAMES.len()..].join(" "), Work::of(std::array::from_fn(count)))
            })
            .collect();
        let new: Vec<(String, String, Work)> = (self.tiles.iter())
            .flat_map(|t| t.counts.rows().map(move |(layer, w)| (format!("{} level {}", t.place, t.level), layer, *w)))
            .collect();
        let find = |rows: &[(String, String, Work)], tile: &str, layer: &str| {
            rows.iter().find(|r| r.0 == tile && r.1 == layer).map_or(Work::default(), |r| r.2)
        };
        let total = |rows: &[(String, String, Work)], tile: &str| {
            let mut total = Work::default();
            rows.iter().filter(|r| r.0 == tile && r.1 != SHADOWS).for_each(|r| total.add(&r.2));
            total.ns()
        };
        // What a difference of `n` in count `i` weighs: the largest thing's sites are tests.
        let weight = |i: usize, n: u64| {
            let mut by = [0; NAMES.len()];
            by[i] = n;
            let by = Work::of(by);
            Work { tests: by.tests + by.largest, ..by }.ns()
        };
        let mut changes = Vec::new();
        let gone = old.iter().filter(|o| !new.iter().any(|n| n.0 == o.0 && n.1 == o.1));
        for (tile, layer, _) in new.iter().chain(gone) {
            let least = 0.01 * total(&new, tile).max(total(&old, tile)) as f64;
            let (was, now) = (find(&old, tile, layer).counts(), find(&new, tile, layer).counts());
            for (i, (was, now)) in was.into_iter().zip(now).enumerate() {
                if now.abs_diff(was) as f64 > tolerance * was as f64 && weight(i, now.abs_diff(was)) as f64 >= least {
                    let by = match (was, now) {
                        (0, _) => "new".to_string(),
                        (_, 0) => "gone".to_string(),
                        _ => format!("{:+.0} %", 100.0 * (now as f64 / was as f64 - 1.0)),
                    };
                    let how = if now > was { "rose" } else { "fell" };
                    changes.push(format!("{tile}, {layer}: {} {how} from {was} to {now} ({by})", NAMES[i]));
                }
            }
        }
        changes
    }
}

/** The line naming a baseline's columns. */
fn header() -> String {
    let names: String = NAMES.iter().map(|n| format!(" {n:>9}")).collect();
    format!("{:<12} {:>5}{names} layer", "place", "level")
}

impl std::fmt::Display for Report {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        let mut places: Vec<&str> = Vec::new();
        self.tiles.iter().for_each(|t| {
            if !places.contains(&t.place.as_str()) {
                places.push(&t.place)
            }
        });
        writeln!(f, "{}: estimated ms per tile, by level", self.name)?;
        for place in places {
            let tiles: Vec<&Tile> = self.tiles.iter().filter(|t| t.place == place).collect();
            let levels: String = tiles.iter().map(|t| format!(" {:>8}", format!("level {}", t.level))).collect();
            // And a plan's columns made per distinct one, the most of any level.
            writeln!(f, "  {place:<24}{levels}  times made")?;
            let mut rows: Vec<String> = Vec::new();
            for (name, _) in tiles.iter().flat_map(|t| t.counts.rows()) {
                if !rows.contains(&name) {
                    rows.push(name);
                }
            }
            let work = |t: &Tile, row: &str| t.counts.rows().find(|(name, _)| name == row).map_or(Work::default(), |(_, w)| *w);
            let ns = |t: &Tile, row: &str| work(t, row).ns();
            let most = |row: &str| tiles.iter().map(|t| ns(t, row)).max().unwrap_or(0);
            // By the most a row takes at any level, largest first; those under a hundredth of the most a tile takes left out.
            rows.sort_by_key(|row| std::cmp::Reverse(most(row)));
            let least = tiles.iter().map(|t| t.counts.total().ns()).max().unwrap_or(0) / 100;
            let line = |f: &mut std::fmt::Formatter, row: &str, ms: &dyn Fn(&Tile) -> f64, after: &str| {
                let values: String = tiles.iter().map(|t| format!(" {:>8.1}", ms(t))).collect();
                writeln!(f, "    {row:<22}{values}{after}")
            };
            for row in rows.iter().filter(|row| most(row) >= least) {
                let name = if row == SHADOWS { format!("{row} (once a world)") } else { row.clone() };
                let times = tiles.iter().map(|t| work(t, row).times_made()).fold(0.0, f64::max);
                let made = if times > 0.0 { format!(" {times:>11.2}") } else { String::new() };
                line(f, &name, &|t| ms(ns(t, row)), &made)?;
            }
            line(f, "total (no shadows)", &|t| ms(t.counts.total().ns()), "")?;
            line(f, "took here", &|t| t.took.as_secs_f64() * 1e3, "")?;
        }
        for flag in self.flags() {
            writeln!(f, "  ! {flag}")?;
        }
        Ok(())
    }
}
