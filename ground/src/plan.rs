//! Plans: what is where, decided column by column (x, z), each at a
//! scale and from coarser ones only.
//! See "Composing" in docs/generators.md.

use std::cell::Cell;
use std::sync::{Arc, Mutex, OnceLock};

use worldgen::grid::Box3;
use worldgen::noise::{Fixed, Fx, Rng, hash64};
use worldgen::recent;
use worldgen::sites::{read_for, spacing, where_asked};

use crate::DOCS;

thread_local! {
    /** The plan being made on this thread and its scale (none: a layer, which may read any). */
    static MAKING: Cell<(&'static str, i64)> = const { Cell::new(("", 0)) };
}

/** Slots a plan keeps values in: as many as 32 tiles have columns, for the tiles the world's threads make at once and what their layers read around them. */
pub(crate) const SLOTS: usize = 1 << 15;
const _: () = assert!(SLOTS.is_power_of_two());
/** Column (x, z) takes slot `sum(x, z) >> SHIFT`: each a run of 2^SHIFT sums. */
const SHIFT: u32 = 64 - SLOTS.trailing_zeros();

/**
 * x and z times two odd constants, added. Columns whose sums differ by
 * 2^SHIFT or more never share a slot. Of random constants tried, these
 * keep near columns so, which a mixed hash would put together by chance:
 * a tile's at every level and Claims' keys (the test). Columns further
 * apart share as by chance, or more along a few directions nothing reads.
 */
fn sum(x: i64, z: i64) -> u64 {
    (x as u64).wrapping_mul(0xd548ecfaef79b2e7).wrapping_add((z as u64).wrapping_mul(0xf0d62a6465469d21))
}

/** Values by column (x, z), the last kept in each of SLOTS, for all the threads that read a world together. */
pub(crate) type Kept<T> = recent::Kept<[i64; 2], T>;

/** The slot of column (x, z). */
#[inline]
pub(crate) fn slot(x: i64, z: i64) -> usize {
    (sum(x, z) >> SHIFT) as usize
}

/**
 * What a plan is made and read under: its name, and its `scale`, the size
 * of the smallest features it decides, in lattice units. It reads only
 * plans at least as coarse, and panics otherwise, naming both.
 */
struct Rule {
    name: &'static str,
    scale: i64,
    /** Made from finer plans' samples: a filter, which may read any. */
    filter: bool,
}

impl Rule {
    /** Counts a read, and refuses one by a plan being made that is coarser. */
    #[inline]
    fn read(&self) {
        worldgen::cost::lookup();
        if self.scale < MAKING.get().1 {
            self.refuse();
        }
    }

    /** Panics: the plan being made reads this one, of a finer scale. */
    #[cold]
    #[inline(never)]
    fn refuse(&self) -> ! {
        let (reader, finest) = MAKING.get();
        panic!("plan {reader} (scale {finest}) reads {} of the finer scale {}; see \"Bands\" in {DOCS}", self.name, self.scale)
    }

    /** Makes column (x, z) of plan `of` with `make`: under its scale, and read where asked, as what a plan decides for a column is the same whoever reads it. */
    fn making<T, P>(&self, of: &P, x: i64, z: i64, make: impl FnOnce() -> T) -> T {
        let outer = MAKING.replace(if self.filter { ("", 0) } else { (self.name, self.scale) });
        let value = where_asked(|| worldgen::cost::plan(self.name, of, [x, 0, z], make));
        MAKING.set(outer);
        value
    }
}

/**
 * A decision for any column, made on demand and kept (the last made in
 * each slot): small, as every slot holds one; a large one is a `Large`.
 */
pub struct Plan<T> {
    rule: Rule,
    /** Read by layers at a point of their sites' footprint where they are further apart than `scale`. */
    dithered: bool,
    kept: Kept<T>,
    make: Box<dyn Fn(i64, i64) -> T + Send + Sync>,
}

impl<T: Copy + Default> Plan<T> {
    pub fn new(name: &'static str, scale: i64, make: impl Fn(i64, i64) -> T + Send + Sync + 'static) -> Plan<T> {
        Plan { rule: Rule { name, scale, filter: false }, dithered: false, kept: Kept::new(SLOTS), make: Box::new(make) }
    }

    /**
     * A plan of categories, which can't be averaged (what a town's column
     * is: street, pavement, yard). A field's line of spans further apart
     * than `scale` reads it at one point of the columns it stands for, the
     * same for every such plan it reads (spread evenly along rows of
     * lines: worldgen's generator/drawables.rs, draw_field), so each
     * category shows as often as it covers, its edges dithered; read where
     * the line is, it would alias. Things, and fields given at each site,
     * read it where they ask.
     */
    pub fn dithered(name: &'static str, scale: i64, make: impl Fn(i64, i64) -> T + Send + Sync + 'static) -> Plan<T> {
        Plan { dithered: true, ..Plan::new(name, scale, make) }
    }

    pub fn at(&self, x: i64, z: i64) -> T {
        self.fetch(x, z).0
    }

    /** Column (x, z), and whether it was made now. Making is out of line, so a kept column costs a few instructions. */
    #[inline]
    fn fetch(&self, x: i64, z: i64) -> (T, bool) {
        self.rule.read();
        // A dithered read keeps its point in the slot of the column it is read for: those of a tile's never share one.
        let at = slot(x, z);
        let (x, z) = match read_for() {
            Some((spacing, [dx, _, dz])) if self.dithered && spacing > self.rule.scale => (x + dx, z + dz),
            _ => (x, z),
        };
        match self.kept.get(at, [x, z]) {
            Some(value) => (value, false),
            None => (self.make(at, x, z), true),
        }
    }

    /** Makes column (x, z) and keeps it. */
    #[cold]
    #[inline(never)]
    fn make(&self, slot: usize, x: i64, z: i64) -> T {
        let value = self.rule.making(self, x, z, || (self.make)(x, z));
        self.kept.keep(slot, [x, z], value);
        value
    }

    /** Column (x, z), if kept: none is made. */
    fn kept(&self, x: i64, z: i64) -> Option<T> {
        self.kept.get(slot(x, z), [x, z])
    }
}

/** Values a Large holds at most: a curve's pieces are a kilometre long or more, so further than a world is crossed. */
const LARGE: usize = 1 << 10;
const _: () = assert!(LARGE.is_power_of_two());

/**
 * A plan of large values, a few of them, such as a curve's pieces: made
 * and read under its scale as a Plan's columns are, but each held for
 * good, in the first free slot from its column's on, so read where it is,
 * without waiting, by all the threads that read the world.
 */
pub struct Large<T> {
    rule: Rule,
    held: Box<[Held<T>]>,
    make: Box<dyn Fn(i64, i64) -> T + Send + Sync>,
}

/** A column and its large value, once made. */
type Held<T> = OnceLock<([i64; 2], T)>;

impl<T> Large<T> {
    pub fn new(name: &'static str, scale: i64, make: impl Fn(i64, i64) -> T + Send + Sync + 'static) -> Large<T> {
        Large { rule: Rule { name, scale, filter: false }, held: (0..LARGE).map(|_| OnceLock::new()).collect(), make: Box::new(make) }
    }

    pub fn at(&self, x: i64, z: i64) -> &T {
        self.rule.read();
        let first = (sum(x, z) >> (64 - LARGE.trailing_zeros())) as usize;
        let mut made = None;
        for slot in (first..first + LARGE).map(|i| &self.held[i % LARGE]) {
            if slot.get().is_none() {
                // Made outside the slot, which another column may take meanwhile, as making it may ask for others: it then goes in the next free.
                let value = made.take().unwrap_or_else(|| self.rule.making(self, x, z, || (self.make)(x, z)));
                made = slot.set(([x, z], value)).err().map(|(_, value)| value);
            }
            if let Some((_, value)) = slot.get().filter(|(at, _)| *at == [x, z]) {
                return value;
            }
        }
        panic!("plan {}: over {LARGE} large values", self.rule.name)
    }
}

/**
 * A plan of heights, as a world's ground, maybe with a promise: no
 * column's differs from a neighbour's along x or z by more than `slope`
 * units, checked whenever one is made and a neighbour is kept (a rule:
 * broken, it panics). So its least and greatest over a rectangle of
 * columns are near those of a grid of samples of it: within the slope
 * times how far a column may be from a sample. Layers drawn about it
 * (FieldAbout::about, PopulationOn::on) are asked where they may draw from that;
 * without a promise, everywhere.
 */
pub struct Sloped {
    pub plan: Arc<Plan<i64>>,
    slope: Option<i64>,
    /** The rectangles asked last and their ranges: a box's layers ask for the same few. */
    last: Mutex<Vec<Ranged>>,
}

/** A rectangle of columns, min x, min z, max x, max z, at a spacing, and the range of a plan over it. */
type Ranged = ([i64; 5], (i64, i64));

/** Samples a range takes along each axis, but one: more, the closer it is, and the costlier. */
const SAMPLES: i64 = 16;
/** Rectangles a Sloped keeps the range of: as many as a box's layers ask about, give or take. */
const KEPT: usize = 16;

impl Sloped {
    pub fn new(plan: Arc<Plan<i64>>, slope: i64) -> Sloped {
        Sloped { plan, slope: Some(slope), last: Mutex::default() }
    }

    /** `plan`, promising no slope: as steep as it likes. */
    pub fn any(plan: Arc<Plan<i64>>) -> Sloped {
        Sloped { plan, slope: None, last: Mutex::default() }
    }

    /**
     * The plan at column (x, z) on the sites read for now (`spacing`):
     * between the columns they read (worldgen's sites::middle),
     * interpolated from the four around. So a layer composes only its own
     * columns, and its things stand on the ground it draws. Where asked
     * (plans, points), the column itself.
     */
    #[inline]
    pub fn at(&self, x: i64, z: i64) -> i64 {
        let (s, m) = (spacing(), worldgen::sites::middle(spacing()));
        let (u, v) = ((x - m) & (s - 1), (z - m) & (s - 1));
        if u | v == 0 {
            return self.exact(x, z);
        }
        let (x0, z0) = (x - u, z - v);
        let h = |dx: i64, dz: i64| self.exact(x0 + dx * s, z0 + dz * s);
        let sum = h(0, 0) * (s - u) * (s - v) + h(1, 0) * u * (s - v) + h(0, 1) * (s - u) * v + h(1, 1) * u * v;
        (sum + s * s / 2).div_euclid(s * s)
    }

    /**
     * The plan at column (x, z), within its slope of its neighbours kept:
     * checked as it is made, so every two neighbours ever kept together are.
     */
    #[inline]
    fn exact(&self, x: i64, z: i64) -> i64 {
        let (h, made) = self.plan.fetch(x, z);
        let Some(slope) = self.slope.filter(|_| made) else { return h };
        for (nx, nz) in [(x - 1, z), (x + 1, z), (x, z - 1), (x, z + 1)] {
            if let Some(n) = self.plan.kept(nx, nz) {
                assert!(
                    (h - n).abs() <= slope,
                    "plan {}: columns ({x}, {z}) and ({nx}, {nz}) differ by {}, more than its slope {}; see \"Rules\" in {DOCS}",
                    self.plan.rule.name,
                    (h - n).abs(),
                    slope
                );
            }
        }
        h
    }

    /** The most a column differs from a neighbour, in units, if it promises so. */
    pub fn slope(&self) -> Option<i64> {
        self.slope
    }

    /**
     * The least and greatest it may be over columns `min` to `max` (x, z),
     * inclusive, as the level's sites read it (`at`), if it promises a
     * slope: between the columns a spacing apart round them, so within
     * theirs.
     */
    pub fn range(&self, min: [i64; 2], max: [i64; 2]) -> Option<(i64, i64)> {
        let (slope, s) = (self.slope?, spacing());
        let m = worldgen::sites::middle(s);
        let key = [min[0], min[1], max[0], max[1], s];
        if let Some((_, range)) = self.last.lock().expect("ranges").iter().find(|(k, _)| *k == key) {
            return Some(*range);
        }
        // Samples every `step` columns, a power of two and at least a spacing, from below min to past max: on one grid
        // for any rectangle, so rectangles side by side share theirs (the plan keeps them), and among the columns `at`
        // reads (`m` on from a spacing's multiple). One of those is at most step / 2 from a sample along each axis,
        // unless each is one.
        let step = [0, 1].map(|a| (((max[a] - min[a]) / SAMPLES).max(s) as u64).next_power_of_two() as i64);
        let along = |a: usize| ((min[a] - m).div_euclid(step[a])..=-(m - max[a]).div_euclid(step[a])).map(move |i| i * step[a] + m);
        let (mut lo, mut hi) = (i64::MAX, i64::MIN);
        for x in along(0) {
            for z in along(1) {
                let h = self.exact(x, z);
                (lo, hi) = (lo.min(h), hi.max(h));
            }
        }
        let far = slope * [0, 1].map(|a| if step[a] > s { step[a] / 2 } else { 0 }).iter().sum::<i64>();
        let range = (lo - far, hi + far);
        let mut last = self.last.lock().expect("ranges");
        if last.len() == KEPT {
            last.remove(0);
        }
        last.push((key, range));
        Some(range)
    }
}

impl Plan<Fx> {
    /**
     * A smooth plan made only every `step` columns and interpolated
     * between: a band `step` or more wide costs a column a sample's share.
     * Sampling is what makes a coarse band of finer ones, so `make` may
     * read finer plans; what it reads finer than `step` aliases. A
     * sample is read at the middle of its cell, as a site is (worldgen's
     * sites::middle): with `step` a level's spacing, where its sites read.
     */
    pub fn sampled(name: &'static str, scale: i64, step: i64, make: impl Fn(i64, i64) -> Fx + Send + Sync + 'static) -> Plan<Fx> {
        let m = worldgen::sites::middle(step);
        let mut samples = Plan::new(name, scale, move |i, k| make(i * step + m, k * step + m));
        samples.rule.filter = true;
        Plan::new(name, scale, move |x, z| {
            let (x, z) = (x - m, z - m);
            let (i, k) = (x.div_euclid(step), z.div_euclid(step));
            let (u, v) = ((x.rem_euclid(step) << 16) / step, (z.rem_euclid(step) << 16) / step);
            let lerp = |a: Fixed, b: Fixed, t: Fixed| a + (((b - a) * t) >> 16);
            let corner = |i, k| samples.at(i, k).to_bits();
            Fx::from_bits(lerp(lerp(corner(i, k), corner(i + 1, k), u), lerp(corner(i, k + 1), corner(i + 1, k + 1), u), v))
        })
    }
}

/** A cell proposes as many spots as thresholds (of 2^32) its hash reaches: Poisson, one on average, at most 8. */
const POISSON: [u32; 8] = [1580030169, 3160060337, 3950075422, 4213413783, 4279248374, 4292415292, 4294609778, 4294923276];
/** Keys a cell's spots take: k · SPOTS + n, room for the most POISSON proposes. */
const SPOTS: i64 = 16;

/** A thing claimed: where it stands, and what it is. */
pub type Claim<T> = Option<([i64; 2], T)>;

/**
 * Things at least `apart` from each other, anywhere (see "Claims" in
 * docs/generators.md). Each cell `cell` wide proposes spots evenly in it,
 * a Poisson number, one on average; `claim` makes each a thing at most a
 * cell from it, or none. A thing is kept unless one of higher hashed
 * priority stands closer than `apart`. The spots and things are plans
 * of scale `cell`: `claim` reads only plans as coarse.
 */
pub struct Claims<T> {
    apart: i64,
    /** Each spot's thing, by its key. */
    proposed: Arc<Plan<Claim<T>>>,
    /** The keys of the things kept near each stretch of columns `apart` / 2 wide, for `near`. */
    nearby: Plan<[(i32, i32); NEARBY]>,
}

/**
 * Things kept within 11/8 of `apart` of a point, at most: 10 would need
 * a circle 1.407 times as wide round it. A stretch's columns are within
 * 3/8 of `apart` of its centre (half its diagonal is under 0.354 of it).
 */
const NEARBY: usize = 9;

impl<T: Copy + Default + Send + Sync + 'static> Claims<T> {
    pub fn new(
        name: &'static str,
        seed: u64,
        cell: i64,
        apart: i64,
        claim: impl Fn([i64; 2], &mut Rng) -> Claim<T> + Send + Sync + 'static,
    ) -> Claims<T> {
        let proposed = Arc::new(Plan::new(name, cell, move |i, key| {
            let mut random = Rng::new(hash64(seed, i, key, 0));
            let spot = [i * cell + random.below(cell as u32) as i64, key.div_euclid(SPOTS) * cell + random.below(cell as u32) as i64];
            let made = claim(spot, &mut random);
            if let Some((at, _)) = made {
                assert!(
                    (0..2).all(|a| (at[a] - spot[a]).abs() <= cell),
                    "claims {name}: a thing at {at:?} stands over a cell from its spot {spot:?}; see \"Claims\" in {DOCS}"
                );
            }
            made
        }));
        let kept = {
            let proposed = proposed.clone();
            Plan::new(name, cell, move |i, key| {
                let Some(([x, z], _)) = proposed.at(i, key) else { return false };
                let priority = |i: i64, key: i64| (hash64(seed, i, key, 1), i, key);
                let outranks = |(j, other): (i64, i64)| {
                    priority(j, other) > priority(i, key)
                        && proposed.at(j, other).is_some_and(|([u, w], _)| (u - x).pow(2) + (w - z).pow(2) < apart * apart)
                };
                !spots(cell, seed, [x, z], apart).into_iter().any(outranks)
            })
        };
        let nearby = {
            let (proposed, stretch) = (proposed.clone(), (apart / 2).max(1));
            Plan::new(name, cell, move |a, b| {
                let (centre, reach) = ([a * stretch + stretch / 2, b * stretch + stretch / 2], apart + (apart * 3 + 7) / 8);
                let near = spots(cell, seed, centre, reach).into_iter().filter(|&(i, key)| {
                    kept.at(i, key)
                        && proposed.at(i, key).is_some_and(|([u, w], _)| (u - centre[0]).pow(2) + (w - centre[1]).pow(2) <= reach * reach)
                });
                let mut keys = [(i32::MIN, 0); NEARBY];
                for (n, (i, key)) in near.enumerate() {
                    assert!(n < NEARBY, "claims {name}: over {NEARBY} things {apart} apart near {centre:?}");
                    keys[n] = (i as i32, key as i32);
                }
                keys
            })
        };
        Claims { apart, proposed, nearby }
    }

    /** The things kept that stand in columns `min` to `max` (x, z), inclusive: of those kept for the stretches of columns there. */
    pub fn within(&self, min: [i64; 2], max: [i64; 2]) -> Vec<([i64; 2], T)> {
        let stretch = (self.apart / 2).max(1);
        let [lo, hi] = [min, max].map(|p| p.map(|v| v.div_euclid(stretch)));
        let mut keys: Vec<(i32, i32)> =
            (lo[0]..=hi[0]).flat_map(|a| (lo[1]..=hi[1]).flat_map(move |b| self.nearby.at(a, b))).filter(|k| k.0 != i32::MIN).collect();
        keys.sort_unstable();
        keys.dedup();
        let holds = |at: [i64; 2]| (0..2).all(|a| (min[a]..=max[a]).contains(&at[a]));
        keys.into_iter().filter_map(|(i, key)| self.proposed.at(i.into(), key.into())).filter(|(at, _)| holds(*at)).collect()
    }

    /** The things kept that stand in box `b`'s columns (lattice units): the points of a population of them (Population::at). */
    pub fn over(&self, b: &Box3) -> Vec<([i64; 2], T)> {
        self.within([b.min[0], b.min[2]], [b.max[0] - 1, b.max[2] - 1])
    }

    /**
     * The thing kept nearest (x, z) of those `keep` keeps, the least
     * position of those as near: looked for in squares twice as wide each
     * time, from `apart` round it, until one holds such a thing no further
     * than its edge; none if none is within `most`. What a world starts at
     * (its spawn) and names, it finds so: none is placed for it.
     */
    pub fn nearest(&self, x: i64, z: i64, most: i64, keep: impl Fn(&T) -> bool) -> Claim<T> {
        let distance = |p: [i64; 2]| (p[0] - x).pow(2) + (p[1] - z).pow(2);
        let mut r = self.apart;
        loop {
            let near = self.within([x - r, z - r], [x + r, z + r]).into_iter().filter(|(_, t)| keep(t));
            let found = near.min_by_key(|(p, _)| (distance(*p), *p));
            if let Some(found) = found.filter(|(p, _)| distance(*p) <= r * r) {
                return Some(found);
            }
            if r >= most {
                return None;
            }
            r *= 2;
        }
    }

    /** The thing kept nearest (x, z) within `within`, at most `apart`, if any: of those kept for its stretch of columns. */
    pub fn near(&self, x: i64, z: i64, within: i64) -> Claim<T> {
        assert!(within <= self.apart, "claims are near within {} at most, not {within}", self.apart);
        let stretch = (self.apart / 2).max(1);
        let keys = self.nearby.at(x.div_euclid(stretch), z.div_euclid(stretch));
        let distance = |[u, w]: [i64; 2]| (u - x).pow(2) + (w - z).pow(2);
        let near = keys.into_iter().take_while(|k| k.0 != i32::MIN).filter_map(|(i, key)| self.proposed.at(i.into(), key.into()));
        near.filter(|(at, _)| distance(*at) <= within * within).min_by_key(|(at, _)| (distance(*at), *at))
    }
}

/** The keys (i, k · SPOTS + n) of the spots whose things may stand within `reach` of `at`: at most a cell from cells reaching it. */
fn spots(cell: i64, seed: u64, at: [i64; 2], reach: i64) -> Vec<(i64, i64)> {
    let range = |a: usize| (at[a] - reach - 2 * cell).div_euclid(cell)..=(at[a] + reach + cell).div_euclid(cell);
    // A spot a cell on average: room for most without growing.
    let mut keys = Vec::with_capacity(2 * range(0).count() * range(1).count());
    for i in range(0) {
        for k in range(1) {
            let h = (hash64(seed, i, k, 2) >> 32) as u32;
            keys.extend((0..POISSON.iter().filter(|&&t| h >= t).count() as i64).map(|n| (i, k * SPOTS + n)));
        }
    }
    keys
}

#[cfg(test)]
mod tests {
    use super::*;
    use worldgen::noise::fx;
    use worldgen::sites::footprint;
    use worldgen::units::Units;

    #[test]
    fn a_sloped_plan_is_within_its_range_wherever_asked() {
        // Hills rising a few units a column at most: their range over rectangles holds every column in them as each
        // level's sites read it, between columns a spacing apart, not far past.
        let hills = |x: i64, z: i64| ((x as f64 / 9.0).sin() * 30.0 + (z as f64 / 13.0).cos() * 50.0).round() as i64;
        let sloped = Sloped::new(Arc::new(Plan::new("hills", 1, hills)), 9);
        for s in [1, 4, 16] {
            footprint(s, [0; 3], || {
                for (min, max) in [([0, 0], [0, 0]), ([3, 5], [3, 5]), ([3, 3], [6, 9]), ([-40, 7], [87, 300]), ([5, -5], [6, 1000])] {
                    let (lo, hi) = sloped.range(min, max).expect("a promise");
                    let columns = (min[0]..=max[0]).flat_map(|x| (min[1]..=max[1]).map(move |z| (x, z)));
                    let (least, most) =
                        columns.map(|(x, z)| sloped.at(x, z)).fold((i64::MAX, i64::MIN), |(lo, hi), h| (lo.min(h), hi.max(h)));
                    assert!(lo <= least && most <= hi, "{min:?} to {max:?}, spacing {s}: {least}..{most} in {lo}..{hi}");
                    assert!(
                        least - lo <= 9 * 64 && hi - most <= 9 * 64,
                        "{min:?} to {max:?}, spacing {s}: {lo}..{hi} far past {least}..{most}"
                    );
                }
            });
        }
    }

    #[test]
    #[should_panic(expected = "plan steps: columns (1, 0) and (0, 0) differ by 5, more than its slope 2")]
    fn refuses_a_sloped_plan_steeper_than_it_says() {
        let steps = Sloped::new(Arc::new(Plan::new("steps", 1, |x, _| 5 * x)), 2);
        steps.at(0, 0);
        steps.at(1, 0);
    }

    #[test]
    #[should_panic(expected = "plan coarse (scale 64) reads fine of the finer scale 1")]
    fn refuses_a_plan_reading_a_finer_one() {
        let fine = Arc::new(Plan::new("fine", 1, |x, _| x));
        let coarse = Plan::new("coarse", 64, move |x, z| fine.at(x, z));
        coarse.at(3, 4);
    }

    #[test]
    fn a_sampled_plan_may_read_finer_ones_and_interpolates() {
        let fine = Arc::new(Plan::new("fine", 1, |x, z| Units(x + 2 * z).to_fx()));
        let coarse = Plan::sampled("coarse", 64, 16, move |x, z| fine.at(x, z));
        assert_eq!((coarse.at(0, 0), coarse.at(8, 0), coarse.at(5, 3)), (Fx::ZERO, fx(8.0), fx(11.0)));
    }

    #[test]
    fn lookups_count_to_whoever_looks_up() {
        use worldgen::cost::{self, Work};
        let fine = Arc::new(Plan::new("fine", 8, |x, _| x));
        let coarse = Plan::new("coarse", 1, move |x, z| fine.at(x, z) + fine.at(x + 1, z));
        let counts = cost::counted(|| {
            coarse.at(0, 0);
            coarse.at(0, 0);
            cost::add("layer", Work::default());
        })
        .1;
        let of = |rows: &[(&str, Work)], name| rows.iter().find(|r| r.0 == name).map(|r| (r.1.lookups, r.1.plans));
        // (lookups, columns made): the layer's second lookup finds the column kept; making it looked up fine's two.
        assert_eq!(
            (of(&counts.layers, "layer"), of(&counts.plans, "coarse"), of(&counts.plans, "fine")),
            (Some((2, 0)), Some((2, 1)), Some((0, 2)))
        );
    }

    #[test]
    fn claims_are_apart_and_prefer_no_places() {
        use std::collections::BTreeSet;

        use worldgen::checks::places::{SIDE, preference};
        use worldgen::grid::Box3;
        // Things at least 100 apart, where x + z is even (a ground they suit); in cells 64 wide.
        let claims = |seed| Claims::new("claims", seed, 64, 100, |[x, z], _| ((x + z) & 1 == 0).then_some(([x, z], ())));
        let area = Box3 { min: [-SIDE / 2, 0, -SIDE / 2], max: [SIDE / 2, 1, SIDE / 2] };
        let places: Vec<(u64, Vec<[i64; 3]>)> = (0..4)
            .map(|seed| {
                let claims = claims(seed);
                let near = |[x, z]: [i64; 2]| claims.near(x, z, 100).map(|([x, z], ())| [x, 0, z]);
                // All those in a square, each the nearest to a point of a grid finer than they are apart.
                let square: BTreeSet<_> = (-20..20).flat_map(|i| (-20..20).map(move |k| [25 * i, 25 * k])).filter_map(near).collect();
                // The nearest the origin is the nearest of those in the square.
                let nearest = claims.nearest(0, 0, 1000, |_| true).map(|([x, z], ())| [x, 0, z]);
                assert_eq!(nearest, square.iter().copied().min_by_key(|p| (p[0].pow(2) + p[2].pow(2), [p[0], p[2]])));
                for a in &square {
                    for b in square.range(..*a) {
                        assert!((a[0] - b[0]).pow(2) + (a[2] - b[2]).pow(2) >= 100 * 100, "{a:?} and {b:?}");
                    }
                }
                let mut random = Rng::new(seed);
                let anywhere: BTreeSet<_> =
                    (0..4000).filter_map(|_| near([0, 2].map(|a| area.min[a] + random.below(SIDE as u32) as i64))).collect();
                (seed, anywhere.into_iter().collect())
            })
            .collect();
        // Periods from the cells' width up.
        let p = preference("claims", 16, &area, &places);
        assert!(!p.broken(), "{p}");
    }

    #[test]
    fn claims_within_a_rectangle_are_those_near_its_columns() {
        use std::collections::BTreeSet;
        let claims = Claims::new("claims", 7, 64, 100, |[x, z], _| ((x + z) & 1 == 0).then_some(([x, z], ())));
        for (min, max) in [([-300, -200], [400, 350]), ([17, 17], [40, 90]), ([-1000, 0], [-1000, 0])] {
            let within: BTreeSet<[i64; 2]> = claims.within(min, max).into_iter().map(|(at, ())| at).collect();
            assert!(within.iter().all(|at| (0..2).all(|a| (min[a]..=max[a]).contains(&at[a]))), "{within:?} in {min:?} to {max:?}");
            // Each nearest to a column of a grid finer than they are apart, and standing in it.
            let near: BTreeSet<[i64; 2]> = (min[0]..=max[0])
                .step_by(10)
                .flat_map(|x| (min[1]..=max[1]).step_by(10).map(move |z| [x, z]))
                .filter_map(|[x, z]| claims.near(x, z, 100).map(|(at, ())| at))
                .filter(|at| (0..2).all(|a| (min[a]..=max[a]).contains(&at[a])))
                .collect();
            assert!(near.is_subset(&within), "{near:?} not all in {within:?}");
        }
    }

    #[test]
    fn near_keys_take_a_slot_each() {
        // Keys w by h, s apart, anywhere: their differences' sums are 2^SHIFT or more from zero.
        let apart = |w: i64, h: i64, s: i64| {
            (1 - w..w).all(|dx| (1 - h..h).all(|dz| (dx, dz) == (0, 0) || (sum(dx * s, dz * s) as i64).unsigned_abs() >> SHIFT > 0))
        };
        // A host's tile of 128 by 128 sites, at each level it draws (lattice's GRID_LEVELS).
        for level in 0..9 {
            assert!(apart(128, 128, 1 << level), "a tile at level {level}");
        }
        // Claims' keys (i, k · SPOTS + n) of 36 by 36 cells.
        assert!(apart(36, 36 * SPOTS, 1), "claims' keys");
    }
}
