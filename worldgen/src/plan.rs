//! Plans: what is where, decided column by column (x, z), each at a
//! scale and from coarser ones only; and land that domains bid for.
//! See "Composing" in docs/generators.md.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use crate::noise::{Fixed, Fx, Rng, hash64, smoothstep};

const DOCS: &str = "docs/generators.md";

thread_local! {
    /** The plan being made on this thread and its scale (none: a layer, which may read any). */
    static MAKING: Cell<(&'static str, i64)> = const { Cell::new(("", 0)) };
    /**
     * The sites a layer reads plans for: their spacing, and how far from
     * where it asks it reads dithered plans finer than that (`footprint`);
     * none while plans or things are made.
     */
    static FOOTPRINT: Cell<Option<(i64, [i64; 2])>> = const { Cell::new(None) };
}

/**
 * Runs `read`, a layer's reads for sites `spacing` apart, with dithered
 * plans finer than the spacing read `offset` (x, z) from where asked:
 * how a line of sites reads them at one point of its voxels' footprint
 * (see Plan::dithered).
 */
pub(crate) fn footprint<R>(spacing: i64, offset: [i64; 2], read: impl FnOnce() -> R) -> R {
    debug_assert!((spacing as u64).is_power_of_two(), "sites are a power of two apart");
    let outer = FOOTPRINT.replace(Some((spacing, offset)));
    let value = read();
    FOOTPRINT.set(outer);
    value
}

/** The spacing of the sites a layer reads for now (`footprint`); 1 where asked, as plans and points are made. */
#[inline]
pub fn spacing() -> i64 {
    FOOTPRINT.get().map_or(1, |(spacing, _)| spacing)
}

/** Runs `read` with plans read where asked, whoever asks: as a plan's columns are made, and points' chances (points.rs). */
pub(crate) fn where_asked<R>(read: impl FnOnce() -> R) -> R {
    let outer = FOOTPRINT.take();
    let value = read();
    FOOTPRINT.set(outer);
    value
}

/** Slots a plan keeps values in: as many as two tiles have columns, so a layer finds what the one before made in its tile. */
const SLOTS: usize = 1 << 15;
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

/** Values by column, the last kept in each of SLOTS. */
pub(crate) struct Kept<T>(RefCell<Vec<(i64, i64, T)>>);

impl<T: Clone + Default> Kept<T> {
    pub(crate) fn new() -> Kept<T> {
        Kept(RefCell::new(vec![(i64::MIN, 0, T::default()); SLOTS]))
    }

    /** Column (x, z), if kept. */
    #[inline]
    pub(crate) fn get(&self, x: i64, z: i64) -> Option<T> {
        let (kx, kz, value) = &self.0.borrow()[(sum(x, z) >> SHIFT) as usize];
        ((*kx, *kz) == (x, z)).then(|| value.clone())
    }

    pub(crate) fn keep(&self, x: i64, z: i64, value: T) {
        self.0.borrow_mut()[(sum(x, z) >> SHIFT) as usize] = (x, z, value);
    }
}

/**
 * A decision for any column, made on demand and kept (the last made in
 * each slot; a large one, such as a curve's points, as an `Rc`). Its
 * `scale` is the size of the smallest features it decides, in lattice
 * units; it reads only plans at least as coarse, and panics otherwise,
 * naming both.
 */
pub struct Plan<T> {
    name: &'static str,
    scale: i64,
    /** Made from finer plans' samples: a filter, which may read any. */
    filter: bool,
    /** Read by layers at a point of their sites' footprint where they are further apart than `scale`. */
    dithered: bool,
    kept: Kept<T>,
    make: Box<dyn Fn(i64, i64) -> T>,
}

impl<T: Clone + Default> Plan<T> {
    pub fn new(name: &'static str, scale: i64, make: impl Fn(i64, i64) -> T + 'static) -> Plan<T> {
        Plan { name, scale, filter: false, dithered: false, kept: Kept::new(), make: Box::new(make) }
    }

    /**
     * A plan of categories, which can't be averaged (what a town's column
     * is: street, pavement, yard). A line of sites further apart than
     * `scale` reads it at one point of the columns it stands for, the
     * same for every such plan it reads (spread evenly along rows of
     * lines: generator/drawables.rs, draw_field), so each category shows as often
     * as it covers, its edges dithered; read where the line is, it would
     * alias. Things read it where they ask.
     */
    pub fn dithered(name: &'static str, scale: i64, make: impl Fn(i64, i64) -> T + 'static) -> Plan<T> {
        Plan { dithered: true, ..Plan::new(name, scale, make) }
    }

    pub fn at(&self, x: i64, z: i64) -> T {
        self.fetch(x, z).0
    }

    /** Column (x, z), and whether it was made now. Making is out of line, so a kept column costs a few instructions. */
    #[inline]
    fn fetch(&self, x: i64, z: i64) -> (T, bool) {
        crate::cost::lookup();
        let finest = MAKING.get().1;
        if self.scale < finest {
            self.refuse();
        }
        let (x, z) = match FOOTPRINT.get() {
            Some((spacing, [dx, dz])) if self.dithered && spacing > self.scale => (x + dx, z + dz),
            _ => (x, z),
        };
        match self.kept.get(x, z) {
            Some(value) => (value, false),
            None => (self.make(x, z), true),
        }
    }

    /** Makes column (x, z) and keeps it. */
    #[cold]
    #[inline(never)]
    fn make(&self, x: i64, z: i64) -> T {
        let outer = MAKING.replace(if self.filter { ("", 0) } else { (self.name, self.scale) });
        // What a plan decides for a column is the same whoever reads it.
        let value = where_asked(|| crate::cost::plan(self.name, self, [x, 0, z], || (self.make)(x, z)));
        MAKING.set(outer);
        self.kept.keep(x, z, value.clone());
        value
    }

    /** Panics: the plan being made reads this one, of a finer scale. */
    #[cold]
    #[inline(never)]
    fn refuse(&self) -> ! {
        let (reader, finest) = MAKING.get();
        panic!("plan {reader} (scale {finest}) reads {} of the finer scale {}; see \"Bands\" in {DOCS}", self.name, self.scale)
    }

    /** Column (x, z), if kept: none is made. */
    fn kept(&self, x: i64, z: i64) -> Option<T> {
        self.kept.get(x, z)
    }
}

/**
 * A plan of heights, as a world's ground, maybe with a promise: no
 * column's differs from a neighbour's along x or z by more than `slope`
 * units, checked whenever one is made and a neighbour is kept (a rule:
 * broken, it panics). So its least and greatest over a rectangle of
 * columns are near those of a grid of samples of it: within the slope
 * times how far a column may be from a sample. Layers drawn about it
 * (Field::about, Population::on) are asked where they may draw from that;
 * without a promise, everywhere.
 */
pub struct Sloped {
    pub plan: Rc<Plan<i64>>,
    slope: Option<i64>,
    /** The rectangles asked last and their ranges: a box's layers ask for the same few. */
    last: RefCell<Vec<Ranged>>,
}

/** A rectangle of columns, min x, min z, max x, max z, and the range of a plan over it. */
type Ranged = ([i64; 4], (i64, i64));

/** Samples a range takes along each axis, but one: more, the closer it is, and the costlier. */
const SAMPLES: i64 = 16;
/** Rectangles a Sloped keeps the range of: as many as a box's layers ask about, give or take. */
const KEPT: usize = 16;

impl Sloped {
    pub fn new(plan: Rc<Plan<i64>>, slope: i64) -> Sloped {
        Sloped { plan, slope: Some(slope), last: RefCell::new(Vec::new()) }
    }

    /** `plan`, promising no slope: as steep as it likes. */
    pub fn any(plan: Rc<Plan<i64>>) -> Sloped {
        Sloped { plan, slope: None, last: RefCell::new(Vec::new()) }
    }

    /**
     * The plan at column (x, z) on the sites read for now (`spacing`):
     * between their columns, interpolated from the four around. So a
     * layer composes only its own columns, and its things stand on the
     * ground it draws. Where asked (plans, points), the column itself.
     */
    #[inline]
    pub fn at(&self, x: i64, z: i64) -> i64 {
        let s = spacing();
        let (u, v) = (x & (s - 1), z & (s - 1));
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
                    self.plan.name,
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

    /** The least and greatest it may be over columns `min` to `max` (x, z), inclusive, if it promises a slope. */
    pub fn range(&self, min: [i64; 2], max: [i64; 2]) -> Option<(i64, i64)> {
        let slope = self.slope?;
        let key = [min[0], min[1], max[0], max[1]];
        if let Some((_, range)) = self.last.borrow().iter().find(|(k, _)| *k == key) {
            return Some(*range);
        }
        // Samples every `step` columns, a power of two, from below min to past max: on one grid for any rectangle, so
        // rectangles side by side share theirs (the plan keeps them). A column is at most step / 2 from one along each.
        let step = [0, 1].map(|a| (((max[a] - min[a]) / SAMPLES).max(1) as u64).next_power_of_two() as i64);
        let along = |a: usize| (min[a].div_euclid(step[a])..=-(-max[a]).div_euclid(step[a])).map(move |i| i * step[a]);
        let (mut lo, mut hi) = (i64::MAX, i64::MIN);
        for x in along(0) {
            for z in along(1) {
                let h = self.exact(x, z);
                (lo, hi) = (lo.min(h), hi.max(h));
            }
        }
        let far = slope * (step[0] / 2 + step[1] / 2);
        let range = (lo - far, hi + far);
        let mut last = self.last.borrow_mut();
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
     * read finer plans; what it reads finer than `step` aliases.
     */
    pub fn sampled(name: &'static str, scale: i64, step: i64, make: impl Fn(i64, i64) -> Fx + 'static) -> Plan<Fx> {
        let mut samples = Plan::new(name, scale, move |i, k| make(i * step, k * step));
        samples.filter = true;
        Plan::new(name, scale, move |x, z| {
            let (i, k) = (x.div_euclid(step), z.div_euclid(step));
            let (u, v) = ((x.rem_euclid(step) << 16) / step, (z.rem_euclid(step) << 16) / step);
            let lerp = |a: Fixed, b: Fixed, t: Fixed| a + (((b - a) * t) >> 16);
            let corner = |i, k| samples.at(i, k).to_bits();
            Fx::from_bits(lerp(lerp(corner(i, k), corner(i + 1, k), u), lerp(corner(i, k + 1), corner(i + 1, k + 1), u), v))
        })
    }
}

/** The most domains a land is shared by. */
pub const DOMAINS: usize = 6;

/**
 * Domains' shares of a column, fractions adding up to one, and the one
 * with the most. Kept in every slot of the land's plan, so small: a
 * share is its `Fx`'s bits, which fit in 32 for a fraction.
 */
#[derive(Clone, Copy, Default)]
struct Shares {
    share: [i32; DOMAINS],
    winner: u8,
}

/**
 * A domain's bid for column (x, z), a fraction: read once a column, by
 * the land's plan, so it reads only plans as coarse as the land's.
 */
pub type Bid = Box<dyn Fn(i64, i64) -> Fx>;

/**
 * Land shared by domains that never see each other: each bids for every
 * column, and gets a share by how far it outbids the best of the rest:
 * all of it `width` ahead, none as far behind, eased between. Whatever
 * the domains propose for a column (heights, densities) blends by these
 * shares.
 */
pub struct Land {
    shares: Plan<Shares>,
}

impl Land {
    /** Land of the domains bidding `bids`, in that order (their indices, see `holding`), decided at `scale`. */
    pub fn new(scale: i64, width: Fx, bids: Vec<Bid>) -> Land {
        assert!(bids.len() <= DOMAINS, "a land holds at most {DOMAINS} domains; see \"Composing\" in {DOCS}");
        Land {
            shares: Plan::dithered("land", scale, move |x, z| {
                let mut bid = [Fx::MIN; DOMAINS];
                for (d, of) in bids.iter().enumerate() {
                    bid[d] = of(x, z);
                }
                let mut raw = [Fx::ZERO; DOMAINS];
                for d in 0..bids.len() {
                    let rest = (0..bids.len()).filter(|e| *e != d).map(|e| bid[e]).max().unwrap_or(Fx::from_bits(Fixed::MIN / 2));
                    raw[d] = smoothstep(-width, width, bid[d].saturating_sub(rest));
                }
                // The best bid is ahead of the rest or level with them, so the sum is at least a half.
                let sum: Fx = raw.into_iter().sum();
                let share = raw.map(|r| r / sum);
                let winner = (0..bids.len()).max_by_key(|d| (share[*d], -(*d as i64))).unwrap_or(0) as u8;
                Shares { share: share.map(|s| s.to_bits() as i32), winner }
            }),
        }
    }

    /** Domain `d`'s share of column (x, z), a fraction. */
    fn share(&self, d: usize, x: i64, z: i64) -> Fx {
        Fx::from_bits(self.shares.at(x, z).share[d].into())
    }

    /** Whether domain `d` has the most of column (x, z): the first of those with the most. */
    fn owns(&self, d: usize, x: i64, z: i64) -> bool {
        self.shares.at(x, z).winner as usize == d
    }

    /** Domain `d`'s hold on it. */
    pub fn holding(self: &Rc<Self>, d: usize) -> Holding {
        Holding(Some((self.clone(), d)))
    }
}

/** A domain's hold on land: its share of each column, and whether it owns it; or all of every column, holding none shared. */
#[derive(Clone)]
pub struct Holding(Option<(Rc<Land>, usize)>);

impl Holding {
    /** All of every column. */
    pub fn all() -> Holding {
        Holding(None)
    }

    /** Its share of column (x, z), a fraction. */
    pub fn share(&self, x: i64, z: i64) -> Fx {
        self.0.as_ref().map_or(Fx::ONE, |(land, d)| land.share(*d, x, z))
    }

    /** Whether it has the most of column (x, z). */
    pub fn owns(&self, x: i64, z: i64) -> bool {
        self.0.as_ref().is_none_or(|(land, d)| land.owns(*d, x, z))
    }
}

/** A cell proposes as many spots as thresholds (of 2^32) its hash reaches: Poisson, one on average, at most 8. */
const POISSON: [u32; 8] = [1580030169, 3160060337, 3950075422, 4213413783, 4279248374, 4292415292, 4294609778, 4294923276];
/** Keys a cell's spots take, its own and the first's: k · SPOTS + n. */
const SPOTS: i64 = 16;

/** A thing claimed: where it stands, and what it is. */
pub type Claim<T> = Option<([i64; 2], T)>;

/**
 * Things at least `apart` from each other, anywhere (see "Claims" in
 * docs/generators.md). Each cell `cell` wide proposes spots evenly in it,
 * a Poisson number, one on average; `claim` makes each a thing at most a
 * cell from it, or none. A thing is kept unless one of higher hashed
 * priority stands closer than `apart`. The spot `first`, if any, is
 * proposed too, with the highest. The spots and things are plans of
 * scale `cell`: `claim` reads only plans as coarse.
 */
pub struct Claims<T> {
    apart: i64,
    /** Each spot's thing, by its key. */
    proposed: Rc<Plan<Claim<T>>>,
    /** The keys of the things kept near each stretch of columns `apart` / 2 wide, for `near`. */
    nearby: Plan<[(i32, i32); NEARBY]>,
}

/**
 * Things kept within 11/8 of `apart` of a point, at most: 10 would need
 * a circle 1.407 times as wide round it. A stretch's columns are within
 * 3/8 of `apart` of its centre (half its diagonal is under 0.354 of it).
 */
const NEARBY: usize = 9;

impl<T: Copy + Default + 'static> Claims<T> {
    pub fn new(
        name: &'static str,
        seed: u64,
        cell: i64,
        apart: i64,
        first: Option<[i64; 2]>,
        claim: impl Fn([i64; 2], &mut Rng) -> Claim<T> + 'static,
    ) -> Claims<T> {
        let proposed = Rc::new(Plan::new(name, cell, move |i, key| {
            let mut random = Rng::new(hash64(seed, i, key, 0));
            let spot = match first {
                Some(spot) if key.rem_euclid(SPOTS) == SPOTS - 1 => spot,
                _ => [i * cell + random.below(cell as u32) as i64, key.div_euclid(SPOTS) * cell + random.below(cell as u32) as i64],
            };
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
                let priority =
                    |i: i64, key: i64| (if key.rem_euclid(SPOTS) == SPOTS - 1 { u64::MAX } else { hash64(seed, i, key, 1) }, i, key);
                let outranks = |(j, other): (i64, i64)| {
                    priority(j, other) > priority(i, key)
                        && proposed.at(j, other).is_some_and(|([u, w], _)| (u - x).pow(2) + (w - z).pow(2) < apart * apart)
                };
                !spots(cell, seed, first, [x, z], apart).into_iter().any(outranks)
            })
        };
        let nearby = {
            let (proposed, stretch) = (proposed.clone(), (apart / 2).max(1));
            Plan::new(name, cell, move |a, b| {
                let (centre, reach) = ([a * stretch + stretch / 2, b * stretch + stretch / 2], apart + (apart * 3 + 7) / 8);
                let near = spots(cell, seed, first, centre, reach).into_iter().filter(|&(i, key)| {
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
fn spots(cell: i64, seed: u64, first: Option<[i64; 2]>, at: [i64; 2], reach: i64) -> Vec<(i64, i64)> {
    let range = |a: usize| (at[a] - reach - 2 * cell).div_euclid(cell)..=(at[a] + reach + cell).div_euclid(cell);
    let first = first.map(|f| f.map(|v| v.div_euclid(cell)));
    let mut keys = Vec::new();
    for i in range(0) {
        for k in range(1) {
            let h = (hash64(seed, i, k, 2) >> 32) as u32;
            keys.extend((0..POISSON.iter().filter(|&&t| h >= t).count() as i64).map(|n| (i, k * SPOTS + n)));
            if first == Some([i, k]) {
                keys.push((i, k * SPOTS + SPOTS - 1));
            }
        }
    }
    keys
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::noise::{ONE, fx};
    use crate::units::Units;

    #[test]
    fn a_sloped_plan_is_within_its_range_wherever_asked() {
        // Hills rising a few units a column at most: their range over rectangles holds every column in them, not far past.
        let hills = |x: i64, z: i64| ((x as f64 / 9.0).sin() * 30.0 + (z as f64 / 13.0).cos() * 50.0).round() as i64;
        let sloped = Sloped::new(Rc::new(Plan::new("hills", 1, hills)), 9);
        for (min, max) in [([0, 0], [0, 0]), ([-40, 7], [87, 300]), ([5, -5], [6, 1000])] {
            let (lo, hi) = sloped.range(min, max).expect("a promise");
            let columns = (min[0]..=max[0]).flat_map(|x| (min[1]..=max[1]).map(move |z| hills(x, z)));
            let (least, most) = columns.fold((i64::MAX, i64::MIN), |(lo, hi), h| (lo.min(h), hi.max(h)));
            assert!(lo <= least && most <= hi, "{min:?} to {max:?}: {least}..{most} in {lo}..{hi}");
            assert!(least - lo <= 9 * 64 && hi - most <= 9 * 64, "{min:?} to {max:?}: {lo}..{hi} far past {least}..{most}");
        }
    }

    #[test]
    #[should_panic(expected = "plan steps: columns (1, 0) and (0, 0) differ by 5, more than its slope 2")]
    fn refuses_a_sloped_plan_steeper_than_it_says() {
        let steps = Sloped::new(Rc::new(Plan::new("steps", 1, |x, _| 5 * x)), 2);
        steps.at(0, 0);
        steps.at(1, 0);
    }

    #[test]
    #[should_panic(expected = "plan coarse (scale 64) reads fine of the finer scale 1")]
    fn refuses_a_plan_reading_a_finer_one() {
        let fine = Rc::new(Plan::new("fine", 1, |x, _| x));
        let coarse = Plan::new("coarse", 64, move |x, z| fine.at(x, z));
        coarse.at(3, 4);
    }

    #[test]
    fn a_sampled_plan_may_read_finer_ones_and_interpolates() {
        let fine = Rc::new(Plan::new("fine", 1, |x, z| Units(x + 2 * z).to_fx()));
        let coarse = Plan::sampled("coarse", 64, 16, move |x, z| fine.at(x, z));
        assert_eq!((coarse.at(0, 0), coarse.at(8, 0), coarse.at(5, 3)), (Fx::ZERO, fx(8.0), fx(11.0)));
    }

    #[test]
    fn lookups_count_to_whoever_looks_up() {
        use crate::cost::{self, Work};
        let fine = Rc::new(Plan::new("fine", 8, |x, _| x));
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
    fn shares_add_up_and_ease_across_the_edge() {
        let land = Land::new(8, fx(0.1), vec![Box::new(|x, _| Fx::from_bits(x * ONE / 100)), Box::new(|_, _| Fx::ZERO)]);
        for x in -30..30 {
            let (sa, sb) = (land.share(0, x, 0), land.share(1, x, 0));
            assert!((sa + sb - Fx::ONE).abs() <= Fx::from_bits(1), "{x}: {sa:?} + {sb:?}");
            assert_eq!(land.owns(0, x, 0), sa >= sb, "{x}");
        }
        assert_eq!((land.share(0, -20, 0), land.share(0, 20, 0)), (Fx::ZERO, Fx::ONE));
    }

    #[test]
    fn claims_are_apart_and_prefer_no_places() {
        use std::collections::BTreeSet;

        use crate::checks::places::{SIDE, preference};
        use crate::grid::Box3;
        // Things at least 100 apart, where x + z is even (a ground they suit), the first at (5, 5); in cells 64 wide.
        let claims = |seed| Claims::new("claims", seed, 64, 100, Some([5, 5]), |[x, z], _| ((x + z) & 1 == 0).then_some(([x, z], ())));
        let area = Box3 { min: [-SIDE / 2, 0, -SIDE / 2], max: [SIDE / 2, 1, SIDE / 2] };
        let places: Vec<(u64, Vec<[i64; 3]>)> = (0..4)
            .map(|seed| {
                let claims = claims(seed);
                let near = |[x, z]: [i64; 2]| claims.near(x, z, 100).map(|([x, z], ())| [x, 0, z]);
                // All those in a square, each the nearest to a point of a grid finer than they are apart.
                let square: BTreeSet<_> = (-20..20).flat_map(|i| (-20..20).map(move |k| [25 * i, 25 * k])).filter_map(near).collect();
                assert!(square.contains(&[5, 0, 5]), "the first is kept");
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
    fn near_keys_take_a_slot_each() {
        // Keys w by h, s apart, anywhere: their differences' sums are 2^SHIFT or more from zero.
        let apart = |w: i64, h: i64, s: i64| {
            (1 - w..w).all(|dx| (1 - h..h).all(|dz| (dx, dz) == (0, 0) || (sum(dx * s, dz * s) as i64).unsigned_abs() >> SHIFT > 0))
        };
        // A host's tile of 128 by 128 sites, at each level it draws (fcc's GRID_LEVELS).
        for level in 0..9 {
            assert!(apart(128, 128, 1 << level), "a tile at level {level}");
        }
        // Claims' keys (i, k · SPOTS + n) of 36 by 36 cells.
        assert!(apart(36, 36 * SPOTS, 1), "claims' keys");
    }
}
