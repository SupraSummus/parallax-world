//! The learned town: the forest, with settlements along its path drawn by
//! a split grammar learned from the valley's towns seen from above (lab/,
//! grammar.rs), not written. See "Learned domains" in docs/generators.md.
//!
//! The grammar is a table of rules (data/town-grammar.bin, written by
//! lab's `export` example): rectangles of the towns' parses, each of a
//! kind (a cluster of label shares), either cut in two or a leaf. A
//! settlement's crop is drawn top down from a root rule: a rectangle of a
//! kind takes a rule of that kind of about its size (and its parent's kind
//! and side), one whose guides (strips such as cross streets, passed down
//! from the cuts above) fit those crossing it best, and cuts where the rule
//! does from the nearer end, snapping onto a guide's edge, or stops at a
//! leaf: its ground and, if it was mostly built, its house's box. Integers
//! only; a settlement is a function of its site, drawn once and kept.
//!
//! What the grammar draws becomes a domain on the forest's land, as the
//! valley's towns are: streets, paving and yards are the town's, painted
//! over the ground it levels; leaves of fields are the fields' (fields.rs);
//! canopy and texture rules (no town, not fields) are the forest's; houses
//! (town.rs's) stand on the leaves' boxes, facing the street beside them.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::OnceLock;

use worldgen::frame::{Frame, Part, U, V, W, back};
use worldgen::generator::{ANY_SIZE, Child, Children, Field, Generator, Layer, Node, Op, Population, Shapes, Span};
use worldgen::grid::Box3;
use worldgen::noise::{Fx, Rng, ease_towards, fx, hash64, stream};
use worldgen::plan::{Claims, Land, Plan};
use worldgen::units::{Units, m};
use worldgen::world::{Find, World};

use crate::fields;
use crate::forest::{self, GROUND_SCALE, Ground, K as F, Relief};
use crate::path::Path;
use crate::town::{self, House, K as T, Style, TOWNS_SCALE};

/** The rule table, learned in lab/ (examples/export.rs) from 54 of the valley's towns. */
const TABLE: &[u8] = include_bytes!("../data/town-grammar.bin");

/** The table's labels (lab/src/towns.rs): what a metre of a town shows from above. */
const STREET: u8 = 2;
const PAVING: u8 = 3;
const FITTINGS: u8 = 4;
const CANOPY: u8 = 5;
const GROUND: u8 = 6;
const FIELDS: u8 = 7;
/** What a layout holds besides: a house's footprint, a texture rule's rectangle. */
const HOUSE: u8 = 8;
const TEXTURE: u8 = 9;

/** Rule flags. */
const CUT: u8 = 1;
const ALONG_Y: u8 = 2;
const SNAP_LO: u8 = 4;
const SNAP_HI: u8 = 8;
const TEXTURE_RULE: u8 = 16;

/** Guides' centres this close are one; one this far or farther from any of the other's costs as much (lab's grammar.rs). */
const SLACK: usize = 2;
const MISMATCH: usize = 16;
/** How far a snapping cut moves to a guide's edge, metres. */
const SNAP: usize = 16;
/** Leaves this small (metres) get no house. */
const SMALLEST_HOUSE: usize = 5;

/** The leaf a rule of a rectangle no longer than the table's leaf size is: its ground, and its house's box, if any. */
#[derive(Clone, Copy, Default)]
struct Leaf {
    ground: u8,
    house: Option<[usize; 4]>,
}

struct Rule {
    w: usize,
    h: usize,
    kind: usize,
    flags: u8,
    /** A cut's place and its sides' kinds. */
    at: usize,
    kids: [usize; 2],
    /** Strips both sides of the cut hold, along the other axis, from the rectangle's start on it. */
    shared: Vec<(usize, usize)>,
    /** The guides that crossed it in training, per axis: their centres from its start. */
    crossing: [Vec<usize>; 2],
    leaf: Leaf,
}

/** Size class a rule is found by: leaves by kind and size only, cuts by their parent's kind and side too. */
const LEAVES: usize = usize::MAX;

struct Grammar {
    /** Rectangles no longer than this (metres) are leaves; a crop's side. */
    leaf: usize,
    crop: usize,
    rules: Vec<Rule>,
    roots: Vec<usize>,
    /** Rules by (context, kind, size in half-octaves of w and h): context 0 any, LEAVES for leaves. */
    by_size: HashMap<(usize, usize, i32, i32), Vec<usize>>,
}

/** Round(2 log2 s): a size in half-octaves, in integers (lab's `octaves`, which is the same for every s). */
pub fn octaves(s: usize) -> i32 {
    let s2 = (s * s) as u128;
    let n = 127 - s2.leading_zeros() as i32;
    // Up where s² ≥ 2^(n + ½), that is s⁴ ≥ 2^(2n + 1); never equal for whole s.
    n + i32::from(s2 * s2 >= 1u128 << (2 * n + 1))
}

/** Reads the table (see lab's grammar.rs, `export`). */
fn read(table: &[u8]) -> Grammar {
    assert_eq!(&table[..4], b"TGR2", "a rule table");
    let mut at = 4;
    let mut next = || {
        let (mut v, mut shift) = (0usize, 0);
        loop {
            let b = table[at];
            at += 1;
            v |= ((b & 0x7f) as usize) << shift;
            shift += 7;
            if b < 0x80 {
                return v;
            }
        }
    };
    let (_labels, _kinds, leaf, crop, count, roots) = (next(), next(), next(), next(), next(), next());
    let roots: Vec<usize> = (0..roots).map(|_| next()).collect();
    let mut rules = Vec::with_capacity(count);
    let mut contexts = Vec::with_capacity(count);
    for _ in 0..count {
        let (w, h, kind, context, flags) = (next(), next(), next(), next(), next() as u8);
        let mut rule =
            Rule { w, h, kind, flags, at: 0, kids: [0; 2], shared: Vec::new(), crossing: [Vec::new(), Vec::new()], leaf: Leaf::default() };
        if flags & CUT != 0 {
            (rule.at, rule.kids) = (next(), [next(), next()]);
            rule.shared = (0..next()).map(|_| (next(), next())).collect();
            for c in &mut rule.crossing {
                *c = (0..next()).map(|_| next()).collect();
            }
        } else if flags & TEXTURE_RULE == 0 {
            let ground = next() as u8;
            let b = [next(), next(), next(), next()];
            rule.leaf = Leaf { ground, house: (b[2] > b[0]).then_some(b) };
        }
        rules.push(rule);
        contexts.push(context);
    }
    let mut by_size: HashMap<(usize, usize, i32, i32), Vec<usize>> = HashMap::new();
    for (i, r) in rules.iter().enumerate() {
        let size = (octaves(r.w), octaves(r.h));
        if r.w.max(r.h) <= leaf {
            by_size.entry((LEAVES, r.kind, size.0, size.1)).or_default().push(i);
        } else {
            by_size.entry((0, r.kind, size.0, size.1)).or_default().push(i);
            if contexts[i] > 0 {
                by_size.entry((contexts[i], r.kind, size.0, size.1)).or_default().push(i);
            }
        }
    }
    Grammar { leaf, crop, rules, roots, by_size }
}

fn grammar() -> &'static Grammar {
    static GRAMMAR: OnceLock<Grammar> = OnceLock::new();
    GRAMMAR.get_or_init(|| read(TABLE))
}

/** Guides a rectangle inherits: per axis, strips (lo, hi) in the crop's metres. */
type Guides = [Vec<(usize, usize)>; 2];

/** The centres of `guides` within (start, len), from start, sorted, those within SLACK of one before left out. */
fn crossing(guides: &[(usize, usize)], (start, len): (usize, usize)) -> Vec<usize> {
    let mut c: Vec<usize> = guides.iter().map(|g| (g.0 + g.1) / 2).filter(|c| *c >= start && *c < start + len).map(|c| c - start).collect();
    c.sort_unstable();
    c.dedup_by(|a, b| a.abs_diff(*b) <= SLACK);
    c
}

/** How badly guides `seen` crossing a rule `from` long fit those `wanted` crossing a rectangle `to` long: a chamfer distance both ways. */
fn mismatch(seen: &[usize], from: usize, wanted: &[usize], to: usize) -> usize {
    let placed: Vec<usize> = seen.iter().filter_map(|p| if 2 * p <= from { Some(*p) } else { (p + to).checked_sub(from) }).collect();
    let near = |p: usize, of: &[usize]| of.iter().map(|q| q.abs_diff(p)).min().unwrap_or(MISMATCH).min(MISMATCH);
    wanted.iter().map(|p| near(*p, &placed)).sum::<usize>() + placed.iter().map(|p| near(*p, wanted)).sum::<usize>()
}

/** A rectangle to draw: its seed, rule if given, context, kind and extent (x, y, w, h), metres of the crop. */
struct Rect {
    h: u64,
    rule: Option<usize>,
    context: usize,
    kind: usize,
    r: (usize, usize, usize, usize),
}

/** A settlement's crop as drawn: a label per metre (x fastest, rows along z), and its houses' boxes (x0, z0, x1, z1). */
struct Layout {
    labels: Vec<u8>,
    houses: Vec<[usize; 4]>,
}

impl Grammar {
    /** A rule for a rectangle of `kind`, w × h, in `context`, chosen by `h` (lab's `Grammar::rule`, with leaves apart). */
    fn rule(&self, context: usize, kind: usize, (w, hh): (usize, usize), wanted: &[Vec<usize>; 2], h: u64) -> Option<usize> {
        let (ow, oh) = (octaves(w), octaves(hh));
        let leaf = w.max(hh) <= self.leaf;
        let tries: &[(usize, i32)] = if leaf {
            &[(LEAVES, 12)]
        } else if context > 0 {
            &[(context, 2), (0, 12)]
        } else {
            &[(0, 12)]
        };
        let guided = !leaf && wanted.iter().any(|g| !g.is_empty());
        let mut found: Vec<usize> = Vec::new();
        for &(c, radius) in tries {
            let mut first = None;
            for r in 0..=radius {
                if first.is_some_and(|f| r > f + i32::from(guided)) {
                    break;
                }
                for dw in -r..=r {
                    for dh in -r..=r {
                        if dw.abs().max(dh.abs()) == r
                            && let Some(v) = self.by_size.get(&(c, kind, ow + dw, oh + dh))
                        {
                            found.extend(v);
                        }
                    }
                }
                if first.is_none() && !found.is_empty() {
                    first = Some(r);
                }
            }
            if !found.is_empty() {
                break;
            }
        }
        if found.is_empty() {
            return None;
        }
        found.sort_unstable();
        if guided {
            let cost = |e: usize| {
                let r = &self.rules[e];
                mismatch(&r.crossing[0], r.w, &wanted[0], w) + mismatch(&r.crossing[1], r.h, &wanted[1], hh)
            };
            let best = found.iter().map(|e| cost(*e)).min().unwrap_or(0);
            found.retain(|e| cost(*e) == best);
        }
        Some(found[(h % found.len() as u64) as usize])
    }

    /** The crop of `seed`, whole. */
    fn layout(&self, seed: u64) -> Layout {
        let mut out = Layout { labels: vec![TEXTURE; self.crop * self.crop], houses: Vec::new() };
        let root = self.roots[(hash64(seed, 0, 0, 1) % self.roots.len() as u64) as usize];
        let at =
            Rect { h: hash64(seed, 0, 0, 2), rule: Some(root), context: 0, kind: self.rules[root].kind, r: (0, 0, self.crop, self.crop) };
        self.draw(at, &mut [Vec::new(), Vec::new()], &mut out);
        out
    }

    fn draw(&self, at: Rect, guides: &mut Guides, out: &mut Layout) {
        let Rect { h, rule, context, kind, r: (rx, ry, rw, rh) } = at;
        let wanted = [crossing(&guides[0], (rx, rw)), crossing(&guides[1], (ry, rh))];
        let Some(e) = rule.or_else(|| self.rule(context, kind, (rw, rh), &wanted, h)) else { return };
        let rule = &self.rules[e];
        let crop = self.crop;
        let mut fill = |(x0, y0, x1, y1): (usize, usize, usize, usize), label: u8| {
            (y0..y1).for_each(|y| out.labels[y * crop + x0..y * crop + x1].fill(label));
        };
        if rule.flags & TEXTURE_RULE != 0 {
            fill((rx, ry, rx + rw, ry + rh), TEXTURE);
            return;
        }
        if rule.flags & CUT != 0 {
            let axis = usize::from(rule.flags & ALONG_Y != 0);
            let (le, lt, start) = if axis == 0 { (rule.w, rw, rx) } else { (rule.h, rh, ry) };
            if lt >= 2 {
                let mut cut = if rule.at * 2 <= le { rule.at } else { lt.saturating_sub(le - rule.at) }.clamp(1, lt - 1);
                let snap = rule.flags & (SNAP_LO | SNAP_HI);
                if snap != 0 {
                    let edges = guides[axis].iter().map(|g| if snap == SNAP_LO { g.0 } else { g.1 });
                    let near = edges.filter(|p| *p > start && *p < start + lt).map(|p| p - start).min_by_key(|p| (p.abs_diff(cut), *p));
                    if let Some(p) = near.filter(|p| p.abs_diff(cut) <= SNAP) {
                        cut = p;
                    }
                }
                // The rule's own strips, placed as its cut is, from the nearer end; only where none are inherited.
                let other = 1 - axis;
                let (ol, ot, ostart) = if other == 0 { (rule.w, rw, rx) } else { (rule.h, rh, ry) };
                let inherited = guides[other].iter().any(|g| g.1 > ostart && g.0 < ostart + ot);
                let before = guides[other].len();
                if !inherited {
                    for &(lo, hi) in &rule.shared {
                        let shift = |p: usize| if lo + hi <= ol { Some(p) } else { (p + ot).checked_sub(ol) };
                        if let (Some(lo), Some(hi)) = (shift(lo), shift(hi))
                            && lo >= 1
                            && hi < ot
                        {
                            guides[other].push((ostart + lo, ostart + hi));
                        }
                    }
                }
                let sides = if axis == 0 {
                    [(rx, ry, cut, rh), (rx + cut, ry, rw - cut, rh)]
                } else {
                    [(rx, ry, rw, cut), (rx, ry + cut, rw, rh - cut)]
                };
                for (side, (kid, part)) in rule.kids.iter().zip(sides).enumerate() {
                    let kid = Rect { h: hash64(h, side as i64, 0, 3), rule: None, context: 1 + kind * 2 + side, kind: *kid, r: part };
                    self.draw(kid, guides, out);
                }
                guides[other].truncate(before);
                return;
            }
        }
        // A leaf: its ground, and its house scaled to the rectangle, if there is room for one.
        fill((rx, ry, rx + rw, ry + rh), rule.leaf.ground);
        if let Some([x0, y0, x1, y1]) = rule.leaf.house {
            let scale = |p: usize, of: usize, to: usize| p * to / of;
            let b = [rx + scale(x0, rule.w, rw), ry + scale(y0, rule.h, rh), rx + scale(x1, rule.w, rw), ry + scale(y1, rule.h, rh)];
            if b[2] - b[0] >= SMALLEST_HOUSE && b[3] - b[1] >= SMALLEST_HOUSE {
                fill((b[0], b[1], b[2], b[3]), HOUSE);
                out.houses.push(b);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// The world

/**
 * Settlements stand at least APART along the path, claimed from spots one
 * every CLAIM of it on average (as the valley's towns); a column's is the
 * nearest within WITHIN. The first's middle is at FIRST.
 */
const APART: i64 = m(900.0);
const CLAIM: i64 = m(800.0);
const WITHIN: i64 = m(600.0);
const FIRST: i64 = m(260.0);
/** How far past its crop's middle a settlement levels the ground, and how far beyond it eases back. */
const LEVELLED: i64 = m(170.0);
const TERRACE: Units = Units::m(120.0);
/** What a column is decided at: a metre, the grammar's. */
const SPOT_SCALE: i64 = m(1.0);

/** The domains, by their index in the land. */
const FOREST: usize = 0;
const TOWN: usize = 1;
const FIELD: usize = 2;

/** A settlement: its crop's middle, its streets' height, its seed. */
#[derive(Clone, Copy, Default)]
struct Site {
    x: i64,
    z: i64,
    level: i64,
    seed: u64,
}

impl Site {
    /** Its crop's first column (units). */
    fn origin(&self, crop: usize) -> [i64; 2] {
        let half = m(1.0) * crop as i64 / 2;
        [self.x - half, self.z - half]
    }

    /** How far column (x, z) is outside the ground it levels. */
    fn distance(&self, x: i64, z: i64) -> Units {
        let (dx, dz) = (((x - self.x).abs() - LEVELLED).max(0), ((z - self.z).abs() - LEVELLED).max(0));
        Units(((dx * dx + dz * dz) as u64).isqrt() as i64)
    }
}

/** What a column is. */
#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum Spot {
    #[default]
    Forest,
    Street,
    Paving,
    Fittings,
    House,
    Yard,
    Fields,
}

impl Spot {
    fn town(self) -> bool {
        matches!(self, Spot::Street | Spot::Paving | Spot::Fittings | Spot::House | Spot::Yard)
    }
}

/** The settlements along the path: where they are, and each one's layout, drawn once and kept (the last few). */
struct Settlements {
    sites: Claims<Site>,
    layouts: RefCell<Vec<(i64, Rc<Layout>)>>,
    spots: Plan<Spot>,
}

/** Layouts kept: a host's tiles are near one or two settlements at a time. */
const KEPT: usize = 4;

impl Settlements {
    fn new(seed: u64, relief: Rc<Relief>) -> Rc<Settlements> {
        let s = stream(seed, 60);
        let crop = grammar().crop;
        let sites = Claims::new("settlements", s, CLAIM, APART, Some([0, FIRST]), move |[x, z], random| {
            let seed = if [x, z] == [0, FIRST] {
                Rng::new(hash64(s, 0, 0, 0)).next_u64()
            } else if x.abs() < CLAIM / 2 {
                random.next_u64()
            } else {
                return None;
            };
            // The floor's mean, as coarse as the settlement.
            let half = m(1.0) * crop as i64 / 4;
            let around = (-1..=1).flat_map(|i| (-1..=1).map(move |j| (i, j)));
            let level = Fx::mean(around.map(|(i, j)| relief.ground(i * half, z + j * half, Fx::ZERO))).round_units().0;
            Some(([0, z], Site { x: 0, z, level, seed }))
        });
        Rc::new_cyclic(|me: &std::rc::Weak<Settlements>| {
            let me = me.clone();
            Settlements {
                sites,
                layouts: RefCell::new(Vec::new()),
                spots: Plan::dithered("learned spots", SPOT_SCALE, move |x, z| me.upgrade().map_or(Spot::Forest, |s| s.spot_of(x, z))),
            }
        })
    }

    fn near(&self, z: i64) -> Option<Site> {
        self.sites.near(0, z, WITHIN).map(|(_, site)| site)
    }

    fn layout(&self, site: &Site) -> Rc<Layout> {
        let mut kept = self.layouts.borrow_mut();
        if let Some((_, l)) = kept.iter().find(|(z, _)| *z == site.z) {
            return l.clone();
        }
        let layout = Rc::new(grammar().layout(site.seed));
        if kept.len() >= KEPT {
            kept.remove(0);
        }
        kept.push((site.z, layout.clone()));
        layout
    }

    /** The label of column (x, z) in its settlement's crop, if in one. */
    fn label(&self, x: i64, z: i64) -> Option<u8> {
        let site = self.near(z)?;
        let crop = grammar().crop;
        let [ox, oz] = site.origin(crop);
        let (i, j) = ((x - ox).div_euclid(m(1.0)), (z - oz).div_euclid(m(1.0)));
        let inside = |v: i64| (0..crop as i64).contains(&v);
        (inside(i) && inside(j)).then(|| self.layout(&site).labels[j as usize * crop + i as usize])
    }

    fn spot_of(&self, x: i64, z: i64) -> Spot {
        match self.label(x, z) {
            Some(STREET) => Spot::Street,
            Some(PAVING) => Spot::Paving,
            Some(FITTINGS) => Spot::Fittings,
            Some(HOUSE) => Spot::House,
            Some(GROUND) => Spot::Yard,
            Some(FIELDS) => Spot::Fields,
            Some(CANOPY | TEXTURE) | None => Spot::Forest,
            Some(_) => Spot::Paving,
        }
    }

    fn spot(&self, x: i64, z: i64) -> Spot {
        self.spots.at(x, z)
    }

    /** The path's centre line at z, with its fraction, bent from `route` onto the settlement's middle there. */
    fn path_x(&self, z: i64, route: Fx) -> Fx {
        let Some(s) = self.near(z) else { return route };
        ease_towards(route, Units(s.x).to_fx(), Units::m(60.0), Units((z - s.z).abs() - LEVELLED))
    }

    /** `ground`, with its fraction, levelled to the settlement's streets under its crop and easing back over TERRACE. */
    fn terraced(&self, x: i64, z: i64, ground: Fx) -> Fx {
        let Some(s) = self.near(z) else { return ground };
        ease_towards(ground, Units(s.level).to_fx(), TERRACE, s.distance(x, z))
    }
}

/** The learned town of `seed`; the camera starts in the forest, on the path to the first settlement. */
pub fn learned(seed: u64) -> World {
    let relief = Rc::new(Relief::new(seed));
    let settlements = Settlements::new(seed, relief.clone());
    let first = settlements.clone();
    let path = {
        let (relief, settlements) = (relief.clone(), settlements.clone());
        Rc::new(Path::new(move |z| settlements.path_x(z, relief.path_x(z))))
    };
    let ground = {
        let (settlements, path) = (settlements.clone(), path.clone());
        Rc::new(Plan::new("ground", GROUND_SCALE, move |x, z| {
            settlements.terraced(x, z, relief.ground(x, z, path.across(x, z))).round_units().0
        }))
    };
    // What the grammar drew bids for each column, outright: the forest bids nothing.
    let bid = |name: &'static str, wins: fn(Spot) -> bool| {
        let settlements = settlements.clone();
        Rc::new(Plan::new(name, SPOT_SCALE, move |x, z| if wins(settlements.spot(x, z)) { Fx::ONE } else { -Fx::ONE }))
    };
    let bids = vec![
        Rc::new(Plan::new("forest's bid", TOWNS_SCALE, |_, _| Fx::ZERO)),
        bid("town's bid", Spot::town),
        bid("fields' bid", |s| s == Spot::Fields),
    ];
    let land = Rc::new(Land::new(SPOT_SCALE, fx(0.125), bids));

    let share = {
        let land = land.clone();
        Rc::new(move |x, z| land.share(FOREST, x, z))
    };
    let forest_ground = Ground { ground: ground.clone(), path: path.clone(), share };
    let spawn = forest_ground.on_path(m(-31.0), m(3.0));
    let mut layers = forest::layers(seed, forest_ground, spawn);
    layers.extend(town_layers(settlements, land.clone(), ground.clone()));
    layers.extend(fields::layers(seed, land, FIELD, ground, path));
    let kinds = [forest::KINDS, town::KINDS, fields::KINDS].concat();
    let first: Find = Box::new(move || first.near(FIRST).map(|s| [s.x, s.level, s.z]).expect("the first settlement"));
    World {
        generator: Generator { kinds, seed: stream(seed, 4), layers },
        spawn,
        heights: forest::HEIGHTS,
        points: vec![("settlement", first)],
    }
}

/** The town's layers: its streets, paving and yards where it owns the land, and its settlements' houses. */
fn town_layers(settlements: Rc<Settlements>, land: Rc<Land>, ground: Rc<Plan<i64>>) -> Vec<Layer> {
    let surface = {
        let (settlements, ground) = (settlements.clone(), ground.clone());
        Field::columns("learned streets", 0.5, Op::Paint, move |x, z, spacing| {
            let spot = settlements.spot(x, z);
            if !spot.town() || !land.owns(TOWN, x, z) {
                return Vec::new();
            }
            let h = ground.at(x, z);
            let kind = match spot {
                Spot::Street => town::k(T::cobble),
                Spot::Paving => town::k(T::paving),
                Spot::Fittings => town::k(T::pavingDark),
                Spot::House => town::k(T::slab),
                _ => forest::k(F::grass),
            };
            vec![Span::solid(h + 1 - m(0.5).max(2 * spacing), h + 1, kind)]
        })
    };
    let crop = grammar().crop as i64;
    let built = {
        let anchor = settlements.clone();
        Population::new("settlements", 1.5, m(256.0))
            .reach([m(200.0), m(32.0), m(200.0)])
            .anchor_along(1, move |line| anchor.near(line.centre[1]).map(|s| [s.x as f64, s.level as f64, s.z as f64]))
            .make(move |_, cell| {
                let site = settlements.near(cell.centre[2])?;
                let layout = settlements.layout(&site);
                let [ox, oz] = site.origin(crop as usize);
                let bounds = Box3 {
                    min: [ox - m(2.0), site.level - m(3.0), oz - m(2.0)],
                    max: [ox + m(1.0) * crop + m(2.0), site.level + HOUSE_TOP, oz + m(1.0) * crop + m(2.0)],
                };
                Some(Node {
                    shapes: Shapes::Now(Vec::new()),
                    bounds: Some(bounds),
                    children: Some(Children::Reaching(Box::new(move |units: &Box3| {
                        let mut children = Vec::new();
                        for (i, b) in layout.houses.iter().enumerate() {
                            let footprint = house_bounds(&site, b);
                            if footprint.overlaps(units) {
                                children.push(Child::new(format!("house {i}"), ANY_SIZE, house(&site, &layout, i, crop as usize)));
                            }
                        }
                        children
                    }))),
                    far: None,
                })
            })
    };
    vec![Layer::Field(surface), Layer::Population(built)]
}

/** How far a house reaches above the streets: two storeys and the steepest roof on the widest leaf, with its chimney. */
const HOUSE_TOP: i64 = m(24.0);
/** A house stands this far inside its box (units): its plinth and eaves stay in it. */
const INSET: i64 = 4;

/** What house `b` (metres of the crop) of `site` may draw in. */
fn house_bounds(site: &Site, b: &[usize; 4]) -> Box3 {
    let [ox, oz] = site.origin(grammar().crop);
    let at = |o: i64, p: usize| o + m(1.0) * p as i64;
    Box3 { min: [at(ox, b[0]), site.level - m(1.0), at(oz, b[1])], max: [at(ox, b[2]), site.level + HOUSE_TOP, at(oz, b[3])] }
}

/** House `i` of a settlement, town.rs's, facing the side of its box with the most street and paving beside it. */
fn house(site: &Site, layout: &Layout, i: usize, crop: usize) -> Node {
    let b = layout.houses[i];
    let open = |x: usize, y: usize| matches!(layout.labels[y * crop + x], STREET | PAVING | FITTINGS);
    let reach = 6;
    // Street beside each side: towards −z, +z, −x, +x.
    let count = |xs: std::ops::Range<usize>, ys: std::ops::Range<usize>| {
        xs.flat_map(|x| ys.clone().map(move |y| (x, y))).filter(|(x, y)| open(*x, *y)).count()
    };
    let beside = [
        count(b[0]..b[2], b[1].saturating_sub(reach)..b[1]),
        count(b[0]..b[2], b[3]..(b[3] + reach).min(crop)),
        count(b[0].saturating_sub(reach)..b[0], b[1]..b[3]),
        count(b[2]..(b[2] + reach).min(crop), b[1]..b[3]),
    ];
    let front = (0..4).max_by_key(|s| (beside[*s], std::cmp::Reverse(*s))).unwrap_or(0);
    let bounds = house_bounds(site, &b);
    let (x0, z0, x1, z1) = (bounds.min[0] + INSET, bounds.min[2] + INSET, bounds.max[0] - INSET, bounds.max[2] - INSET);
    let base = site.level + 1;
    let (frame, length, depth) = match front {
        0 => (Frame::new([x0, base, z0], [U, V, W]), x1 - x0, z1 - z0),
        1 => (Frame::new([x0, base, z1 - 1], [U, V, back(W)]), x1 - x0, z1 - z0),
        2 => (Frame::new([x0, base, z0], [W, V, U]), z1 - z0, x1 - x0),
        _ => (Frame::new([x1 - 1, base, z0], [W, V, back(U)]), z1 - z0, x1 - x0),
    };
    let mut random = Rng::new(hash64(site.seed, i as i64, 0, 7));
    let storeys = 1 + random.below(2) as i64;
    let mut shapes = Vec::new();
    House::new(length, depth, storeys, Style::of(site.seed)).build(&mut Part::new(frame, &mut shapes), &mut random);
    town::thing(shapes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn octaves_in_integers_round_twice_the_log() {
        for s in 1..=2048usize {
            assert_eq!(octaves(s), ((s as f64).log2() * 2.0).round() as i32, "{s}");
        }
    }

    /** Over a dozen settlements (some are hamlets, a few nothing), a town's shares: streets, paving, houses, a few fields, mostly forest. */
    #[test]
    fn layouts_are_towns_in_the_forest() {
        let g = grammar();
        let (mut shares, mut houses) = ([0.0; 10], 0);
        for seed in 0..12 {
            let l = g.layout(seed);
            l.labels.iter().for_each(|v| shares[*v as usize] += 1.0 / (12 * l.labels.len()) as f64);
            houses += l.houses.len();
        }
        let town = shares[STREET as usize] + shares[PAVING as usize] + shares[HOUSE as usize];
        assert!(
            town > 0.1 && shares[HOUSE as usize] > 0.03 && shares[TEXTURE as usize] > 0.5 && houses > 600,
            "{shares:.2?}, {houses} houses"
        );
    }
}
