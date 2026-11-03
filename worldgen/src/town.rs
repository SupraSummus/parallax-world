//! Towns along the valley's path: blocks of houses on streets around a
//! square with a church and a fountain. A domain of the valley (valley.rs):
//! it bids for the land around its towns, levels the ground under them,
//! and draws only the columns it wins. Its trees are the forest's.

use std::rc::Rc;

use crate::forest::{self, K as F, Relief, Species, UNIT_SIZE, make_tree};
use crate::generator::{
    ANY_SIZE, Child, Children, Far, Field, FieldValues, Fill, Layer, Node, Op, Population, SOLID, Shape, Shapes, Span, ball, cuboid, gable,
    lathe,
};
use crate::grid::Box3;
use crate::kinds;
use crate::noise::{Fixed, ONE, Rng, fixed, hash64, smoothstep, stream};
use crate::plan::{Land, Plan};

kinds! {
    base: forest::KINDS.len() as u8;
    plasterOchre: 10, ["#d8b878", "#cfae6c"];
    plasterWhite: 10, ["#ece6d6", "#e2dccb"];
    plasterPink: 10, ["#d9a79a", "#cf9b8d"];
    plasterBlue: 10, ["#a9bccb", "#9fb3c3"];
    plasterSage: 10, ["#b5bf9a", "#aab58e"];
    brick: 10, ["#9a4a32", "#8a412c", "#a85538"];
    trim: 10, ["#f4f1e8"];
    shutterGreen: 4, ["#2f5a3a", "#356542"];
    shutterBlue: 4, ["#2f4a6a", "#36547a"];
    glass: 11, ["#5b7f97", "#668aa2"];
    door: 4, ["#5a3a24", "#6b4528", "#2f4f3a"];
    roofRed: 9, ["#a8432c", "#b44d32", "#9a3b27"];
    roofBrown: 9, ["#6e4a34", "#7a5238", "#64422e"];
    slate: 9, ["#4a4f57", "#52575f", "#43484f"];
    copper: 9, ["#5f9a86", "#6aa892"];
    churchStone: 3, ["#b8b0a0", "#aca492", "#c2baa9"];
    cobble: 12, ["#7a7670", "#6e6a65", "#85817a", "#66625d"];
    paving: 12, ["#b4ab98", "#a89f8c"];
    pavingDark: 12, ["#8f8676", "#857c6c"];
    slab: 12, ["#a6a39c", "#9c9992"];
    planks: 4, ["#9a6a3c", "#a8764a", "#8c5e34"];
    counter: 4, ["#5c3e26"];
    goods: 6, ["#c8483a", "#e0b040", "#4a7ab0", "#5a9a4a", "#e8e0d0"];
    iron: 3, ["#2b2d30"];
    lamp: 6, ["#fff3c4"];
    water: 13, ["#3f7fa8", "#4a8ab2"];
}

/** Metres in units, rounded. */
const fn m(metres: f64) -> i64 {
    (metres / UNIT_SIZE + 0.5) as i64
}

// The plan of a town, in units: a main street along z, blocks on either side of it in rows, side and cross streets between them.
const MAIN: i64 = m(10.0);
const STREET: i64 = m(7.0);
const BLOCK_W: i64 = m(36.0);
const BLOCK_D: i64 = m(40.0);
const PAVEMENT: i64 = m(2.5);
const STOREY: i64 = m(3.25);
const WALL: i64 = 3;
/** Towns are this far apart along the path, give or take JITTER. */
const SPACING: i64 = m(1200.0);
const JITTER: i64 = m(150.0);
/** Plan scales: where towns are; the bid for their land. */
pub const TOWNS_SCALE: i64 = m(400.0);
pub const BID_SCALE: i64 = m(4.0);
/** How far past its streets a town levels the ground, easing back to the valley's. */
const TERRACE: i64 = m(120.0);

/** A town: its main street's centre line x, its middle z, the height of its streets, and blocks per side and rows of them. */
#[derive(Clone, Copy, Default)]
pub struct Town {
    pub x: i64,
    pub z: i64,
    pub level: i64,
    cols: i64,
    rows: i64,
    seed: u64,
}

/** What a column of a town is. */
#[derive(Clone, Copy, PartialEq, Eq)]
enum Spot {
    Outside,
    Street,
    Square,
    Pavement,
    Yard,
}

/** A block: which side of the main street (±1), how far out (0 next to it), which row; its box of columns. */
#[derive(Clone, Copy)]
struct Block {
    side: i64,
    c: i64,
    r: i64,
    x0: i64,
    x1: i64,
    z0: i64,
    z1: i64,
}

impl Town {
    fn half_x(&self) -> i64 {
        MAIN / 2 + self.cols * BLOCK_W + (self.cols - 1) * STREET
    }

    fn half_z(&self) -> i64 {
        (self.rows * BLOCK_D + (self.rows - 1) * STREET) / 2
    }

    fn square_row(&self) -> i64 {
        self.rows / 2
    }

    /** Distance from the town's streets and blocks, in units: negative inside. */
    pub fn distance(&self, x: i64, z: i64) -> i64 {
        let (dx, dz) = ((x - self.x).abs() - self.half_x(), (z - self.z).abs() - self.half_z());
        if dx <= 0 && dz <= 0 {
            return dx.max(dz);
        }
        let (dx, dz) = (dx.max(0), dz.max(0));
        ((dx * dx + dz * dz) as u64).isqrt() as i64
    }

    /** Block `c` of side `side` in row `r`. */
    fn block(&self, side: i64, c: i64, r: i64) -> Block {
        let inner = MAIN / 2 + c * (BLOCK_W + STREET);
        let (x0, x1) = if side > 0 { (self.x + inner, self.x + inner + BLOCK_W) } else { (self.x - inner - BLOCK_W, self.x - inner) };
        let z0 = self.z - self.half_z() + r * (BLOCK_D + STREET);
        Block { side, c, r, x0, x1, z0, z1: z0 + BLOCK_D }
    }

    fn blocks(&self) -> impl Iterator<Item = Block> + '_ {
        (0..self.rows).flat_map(move |r| (0..self.cols).flat_map(move |c| [-1, 1].map(|side| self.block(side, c, r))))
    }

    fn spot(&self, x: i64, z: i64) -> Spot {
        let (a, f) = ((x - self.x).abs(), z - self.z + self.half_z());
        if a >= self.half_x() || f < 0 || f >= 2 * self.half_z() {
            return Spot::Outside;
        }
        let (r, g) = (f / (BLOCK_D + STREET), f % (BLOCK_D + STREET));
        if g >= BLOCK_D {
            return Spot::Street;
        }
        if a < MAIN / 2 {
            return if r == self.square_row() { Spot::Square } else { Spot::Street };
        }
        let (c, w) = ((a - MAIN / 2) / (BLOCK_W + STREET), (a - MAIN / 2) % (BLOCK_W + STREET));
        if w >= BLOCK_W {
            return Spot::Street;
        }
        if r == self.square_row() && c == 0 {
            return Spot::Square;
        }
        let along_street = w < PAVEMENT
            || (c < self.cols - 1 && w >= BLOCK_W - PAVEMENT)
            || (r > 0 && g < PAVEMENT)
            || (r < self.rows - 1 && g >= BLOCK_D - PAVEMENT);
        if along_street { Spot::Pavement } else { Spot::Yard }
    }

    /** Everything it draws: its blocks, and what grows over their edges, from under its streets to its church's spire. */
    fn bounds(&self) -> Box3 {
        let (hx, hz) = (self.half_x() + m(10.0), self.half_z() + m(10.0));
        Box3 { min: [self.x - hx, self.level - m(3.0), self.z - hz], max: [self.x + hx, self.level + m(40.0), self.z + hz] }
    }
}

/** The towns along the valley's path, by where they are. */
pub struct Towns {
    plan: Plan<Option<Town>>,
}

impl Towns {
    /**
     * Towns every SPACING along z, some of them, the first near the start.
     * Each lies on the path's coarse band (its mean, x = 0: the path bends
     * to meet it) at the mean height of the valley floor under it.
     */
    pub fn new(seed: u64, relief: Rc<Relief>) -> Towns {
        let s = stream(seed, 30);
        Towns {
            plan: Plan::new("towns", TOWNS_SCALE, move |_, n| {
                let mut random = Rng::new(hash64(s, n, 0, 0));
                if n != 0 && !random.chance(fixed(0.6)) {
                    return None;
                }
                let (z, cols, rows) = if n == 0 {
                    (m(200.0), 2, 4)
                } else {
                    (n * SPACING - JITTER + random.below(2 * JITTER as u32) as i64, 1 + random.below(3) as i64, 3 + random.below(3) as i64)
                };
                let mut town = Town { x: 0, z, level: 0, cols, rows, seed: random.next_u64() };
                // The floor's mean, not its bumps: a town's height is as coarse as the town.
                let (hx, hz) = (town.half_x(), town.half_z());
                let mut sum = 0;
                for (i, j) in (-1..=1).flat_map(|i| (-1..=1).map(move |j| (i, j))) {
                    sum += relief.ground(i * hx, z + j * hz, 0);
                }
                town.level = (sum / 9 + ONE / 2) >> 16;
                Some(town)
            }),
        }
    }

    /** The town nearest z along the path, if it has one. */
    pub fn near(&self, z: i64) -> Option<Town> {
        self.plan.at(0, (z + SPACING / 2).div_euclid(SPACING))
    }

    /** The path's centre line at z (fixed point), bent from `route` onto the main street of the town there. */
    pub fn street_x(&self, z: i64, route: Fixed) -> Fixed {
        let Some(t) = self.near(z) else { return route };
        let bend = ONE - smoothstep(0, m(60.0) * ONE, ((z - t.z).abs() - t.half_z()) * ONE);
        route + (((t.x * ONE - route) * bend) >> 16)
    }

    /** The town's bid for column (x, z), BID_SCALE: ahead inside its streets, behind a few metres out. */
    pub fn bid(&self, x: i64, z: i64) -> Fixed {
        self.near(z).map_or(-4 * ONE, |t| ((m(4.0) - t.distance(x, z)) * ONE / m(8.0)).clamp(-4 * ONE, 4 * ONE))
    }

    /** `ground` (fixed point) levelled to the town's streets, all the way under it and easing back over TERRACE. */
    pub fn terraced(&self, x: i64, z: i64, ground: Fixed) -> Fixed {
        let Some(t) = self.near(z) else { return ground };
        let weight = ONE - smoothstep(0, TERRACE * ONE, t.distance(x, z).max(0) * ONE);
        ground + (((t.level * ONE - ground) * weight) >> 16)
    }

    /** How far column (x, z) is from the nearest town, in units (negative inside); far if none. */
    pub fn distance(&self, x: i64, z: i64) -> i64 {
        self.near(z).map_or(i64::MAX / 4, |t| t.distance(x, z))
    }
}

/** The town's layers: its streets' surface, and the towns, both only where it owns the land (domain `me` of `land`). */
pub fn layers(towns: Rc<Towns>, land: Rc<Land>, me: usize, ground: Rc<Plan<i64>>) -> Vec<Layer> {
    let surface = {
        let towns = towns.clone();
        Field {
            name: "streets",
            order: 0.5,
            op: Op::Paint,
            values: FieldValues::Spans {
                axis: 1,
                spans: Box::new(move |x, z, spacing| {
                    let Some(t) = towns.near(z) else { return Vec::new() };
                    let spot = t.spot(x, z);
                    if spot == Spot::Outside || !land.owns(me, x, z) {
                        return Vec::new();
                    }
                    let h = ground.at(x, z);
                    let top = h + 1 - m(0.5).max(2 * spacing);
                    let surface = |kind| Span { from: top, to: h + 1, kind, extinction: None };
                    match spot {
                        Spot::Street => vec![surface(k(K::cobble))],
                        Spot::Square => {
                            let checker = ((x - t.x).div_euclid(m(1.0)) + (z - t.z).div_euclid(m(1.0))) & 1 == 0;
                            vec![surface(k(if checker { K::paving } else { K::pavingDark }))]
                        }
                        // A kerb's height above the street.
                        Spot::Pavement => vec![surface(k(K::slab)), Span { from: h + 1, to: h + 2, kind: k(K::slab), extinction: None }],
                        _ => vec![surface(forest::k(F::grass))],
                    }
                }),
            },
        }
    };
    let built = Population {
        name: "towns",
        order: 1.5,
        cell: m(256.0),
        reach: [m(140.0), m(45.0), m(140.0)],
        size: ANY_SIZE,
        anchor: {
            let towns = towns.clone();
            Box::new(move |cell| towns.near(cell.centre[2]).map(|t| [t.x as f64, t.level as f64, t.z as f64]))
        },
        make: Box::new(move |_, cell| towns.near(cell.centre[2]).map(town)),
        far: None,
    };
    vec![Layer::Field(surface), Layer::Population(built)]
}

// ---------------------------------------------------------------------------
// Things

/** A cuboid of `kind` from min to max, in units. */
fn bx(min: [i64; 3], max: [i64; 3], kind: K, op: Op) -> Shape {
    cuboid(min.map(|v| v as f64), max.map(|v| v as f64), k(kind), SOLID, op)
}

fn child(name: String, size: i64, node: Node) -> Child {
    Child { name, size, node }
}

/** A town's node: its blocks, lamps and fountain, each made only for the boxes it reaches. */
fn town(t: Town) -> Node {
    Node {
        shapes: Shapes::Now(Vec::new()),
        bounds: Some(t.bounds()),
        children: Some(Children::Reaching(Box::new(move |units: &Box3| {
            let mut children = Vec::new();
            for b in t.blocks() {
                let bounds = block_bounds(&t, &b);
                if bounds.overlaps(units) {
                    children.push(child(format!("block {} {} {}", b.side, b.c, b.r), ANY_SIZE, block(&t, &b, bounds)));
                }
            }
            for r in (0..t.rows).filter(|r| *r != t.square_row()) {
                let lamps = lamps(&t, r);
                if lamps.bounds.is_some_and(|b| b.overlaps(units)) {
                    children.push(child(format!("lamps {r}"), m(1.0), lamps));
                }
            }
            let fountain = fountain(&t);
            if fountain.shapes.get().iter().any(|s| s.bounds.overlaps(units)) {
                children.push(child("fountain".into(), ANY_SIZE, fountain));
            }
            children
        }))),
        far: None,
    }
}

/** A block's box, with room for its trees' crowns. */
fn block_bounds(t: &Town, b: &Block) -> Box3 {
    Box3 { min: [b.x0 - m(9.0), t.level - m(3.0), b.z0 - m(9.0)], max: [b.x1 + m(9.0), t.level + m(40.0), b.z1 + m(9.0)] }
}

/** Where columns w0 to w1 of a block's width are along x, w from its inner edge (towards the main street). */
fn across(b: &Block, w0: i64, w1: i64) -> (i64, i64) {
    if b.side > 0 { (b.x0 + w0, b.x0 + w1) } else { (b.x1 - w1, b.x1 - w0) }
}

/** The x of column w of a block's width. */
fn column(b: &Block, w: i64) -> i64 {
    across(b, w, w + 1).0
}

fn block(t: &Town, b: &Block, bounds: Box3) -> Node {
    let mut random = Rng::new(hash64(t.seed, b.side, b.c, b.r));
    let mut children = Vec::new();
    let base = t.level + 1;
    if b.r == t.square_row() && b.c == 0 {
        if b.side > 0 {
            children.push(child("church".into(), ANY_SIZE, church(t, b)));
        } else {
            // Trees at the square's corners, the forest's own.
            for (i, (w, g)) in
                [(m(6.0), m(6.0)), (m(6.0), BLOCK_D - m(6.0)), (BLOCK_W - m(8.0), m(6.0)), (BLOCK_W - m(8.0), BLOCK_D - m(6.0))]
                    .into_iter()
                    .enumerate()
            {
                let x = column(b, w);
                let species = if random.chance(ONE / 2) { Species::Oak } else { Species::Birch };
                let mut grow = Rng::new(random.next_u64());
                let level = t.level;
                children.push(child(
                    format!("tree {i}"),
                    ANY_SIZE,
                    make_tree(species, x as f64, level as f64, (b.z0 + g) as f64, &mut grow, &|_, _| level),
                ));
            }
        }
        return Node { shapes: Shapes::Now(Vec::new()), bounds: Some(bounds), children: Some(Children::Fixed(children)), far: None };
    }
    // Two rows of lots back to back, each facing its street; along z, lots 7 to 12 m wide.
    for back in [false, true] {
        let mut g = PAVEMENT;
        let mut n = 0;
        while g < BLOCK_D - PAVEMENT {
            let mut width = m(7.0) + random.below(m(5.0) as u32) as i64;
            if BLOCK_D - PAVEMENT - (g + width) < m(5.0) {
                width = BLOCK_D - PAVEMENT - g;
            }
            let gap = if b.c == 0 { 0 } else { m(1.0) + random.below(m(1.0) as u32) as i64 };
            let setback = if b.c == 0 { 0 } else { random.below(m(2.0) as u32) as i64 };
            let depth = m(8.0) + random.below(m(3.0) as u32) as i64;
            let (w0, w1) = if back {
                (BLOCK_W - PAVEMENT - setback - depth, BLOCK_W - PAVEMENT - setback)
            } else {
                (PAVEMENT + setback, PAVEMENT + setback + depth)
            };
            let (x0, x1) = across(b, w0, w1);
            // Facing its street: away from the block's middle.
            let facing = if back == (b.side > 0) { 1 } else { -1 };
            let storeys = if b.c == 0 { 2 + random.below(2) as i64 } else { 1 + random.below(2) as i64 };
            let house = House { x0, x1, z0: b.z0 + g + gap, z1: b.z0 + g + width - gap, base, storeys, facing, shop: b.c == 0 && !back };
            children.push(child(format!("house {} {n}", u8::from(back)), ANY_SIZE, house.node(&mut random)));
            // A tree in the yard behind, now and then.
            let yard = BLOCK_W / 2 - (PAVEMENT + setback + depth);
            if yard >= m(4.0) && random.chance(fixed(0.3)) {
                let x = column(b, BLOCK_W / 2 + if back { yard / 2 } else { -yard / 2 });
                let species = if random.chance(fixed(0.6)) { Species::Birch } else { Species::Oak };
                let mut grow = Rng::new(random.next_u64());
                let level = t.level;
                let z = (b.z0 + g + width / 2) as f64;
                children.push(child(
                    format!("tree {} {n}", u8::from(back)),
                    ANY_SIZE,
                    make_tree(species, x as f64, level as f64, z, &mut grow, &|_, _| level),
                ));
            }
            g += width;
            n += 1;
        }
    }
    Node { shapes: Shapes::Now(Vec::new()), bounds: Some(bounds), children: Some(Children::Fixed(children)), far: None }
}

/** A house on [x0, x1) × [z0, z1) from `base` up, its front on x1 (facing +1) or x0 (−1). */
struct House {
    x0: i64,
    x1: i64,
    z0: i64,
    z1: i64,
    base: i64,
    storeys: i64,
    facing: i64,
    shop: bool,
}

const PLASTERS: [K; 6] = [K::plasterOchre, K::plasterWhite, K::plasterPink, K::plasterBlue, K::plasterSage, K::brick];

/** A facade: the plane `axis` (x or z, 0 or 2) = `face`, its outside towards `out` (±1). */
struct Facade {
    axis: usize,
    face: i64,
    out: i64,
}

impl Facade {
    /** A box on it: from `a` to `b` deep into it (negative: in front of it), `along` it the other way, `y` up. */
    fn bx(&self, a: i64, b: i64, along: (i64, i64), y: (i64, i64), kind: K, op: Op) -> Shape {
        let (d0, d1) = if self.out > 0 { (self.face - b, self.face - a) } else { (self.face + a, self.face + b) };
        let (mut min, mut max) = ([along.0, y.0, along.0], [along.1, y.1, along.1]);
        (min[self.axis], max[self.axis]) = (d0, d1);
        bx(min, max, kind, op)
    }
}

/** A window from y up, centred on zc, `half` wide each way and `tall`: frame, reveal, glass, sill, and shutters if any. */
fn window(f: &Facade, zc: i64, y: i64, half: i64, tall: i64, shutters: Option<K>, shapes: &mut Vec<Shape>) {
    shapes.push(f.bx(0, 1, (zc - half - 1, zc + half + 1), (y - 1, y + tall + 1), K::trim, Op::Paint));
    shapes.push(f.bx(0, 2, (zc - half, zc + half), (y, y + tall), K::trim, Op::Carve));
    shapes.push(f.bx(2, 3, (zc - half, zc + half), (y, y + tall), K::glass, Op::Paint));
    shapes.push(f.bx(-1, 0, (zc - half - 1, zc + half + 1), (y - 1, y), K::trim, Op::Fill));
    if let Some(kind) = shutters {
        shapes.push(f.bx(-1, 0, (zc - half - 5, zc - half - 1), (y, y + tall), kind, Op::Fill));
        shapes.push(f.bx(-1, 0, (zc + half + 1, zc + half + 5), (y, y + tall), kind, Op::Fill));
    }
}

impl House {
    fn node(&self, random: &mut Rng) -> Node {
        let House { x0, x1, z0, z1, base, storeys, facing, .. } = *self;
        let kinds = if random.chance(fixed(0.1)) { 6 } else { 5 };
        let plaster = PLASTERS[random.below(kinds) as usize];
        let roof_kind = [K::roofRed, K::roofRed, K::roofBrown, K::slate][random.below(4) as usize];
        let top = base + storeys * STOREY;
        // Its ridge along the street, eaves to it; now and then gable-fronted.
        let ridge = if random.chance(fixed(0.75)) { 2 } else { 0 };
        let across = 2 - ridge;
        let (lo, hi) = if across == 0 { (x0, x1) } else { (z0, z1) };
        let eaves = 3;
        let rise = ((hi - lo) as f64 / 2.0 + eaves as f64) * (0.6 + random.next() * 0.4);
        let widen = |v: [f64; 3], by: f64| {
            let mut v = v;
            v[across] += by;
            v
        };
        let (min, max) = ([x0 as f64, top as f64, z0 as f64], [x1 as f64, top as f64, z1 as f64]);
        let wall_rise = rise * (hi - lo) as f64 / (hi - lo + 2 * eaves) as f64;
        let mut body = vec![
            bx([x0 - 1, base - 3, z0 - 1], [x1 + 1, base + 4, z1 + 1], K::churchStone, Op::Fill),
            bx([x0, base, z0], [x1, top, z1], plaster, Op::Fill),
            gable(min, [max[0], top as f64 + wall_rise, max[2]], ridge, k(plaster), SOLID, Op::Fill),
            gable(
                widen(min, -(eaves as f64)),
                widen([max[0], top as f64 + rise, max[2]], eaves as f64),
                ridge,
                k(roof_kind),
                SOLID,
                Op::Fill,
            ),
        ];
        // Eaves boarded along both sides: a thin wedge's edge would leave gaps.
        for (from, to) in [(lo - eaves, lo), (hi, hi + eaves)] {
            let (mut min, mut max) = ([x0, top - 2, z0], [x1, top + 1, z1]);
            (min[across], max[across]) = (from, to);
            body.push(bx(min, max, roof_kind, Op::Fill));
        }
        let (cx, cz) =
            (x0 + 4 + random.below((x1 - x0 - 9).max(1) as u32) as i64, z0 + 4 + random.below((z1 - z0 - 9).max(1) as u32) as i64);
        body.push(bx([cx, top, cz], [cx + 5, top + rise as i64 + m(0.8), cz + 5], K::brick, Op::Fill));

        // Windows on both long sides, a door in the front's middle, a shop's open door and room on the ground floor.
        let shutters = random.chance(fixed(0.4)).then(|| if random.chance(ONE / 2) { K::shutterGreen } else { K::shutterBlue });
        let shop = self.shop && random.chance(ONE / 2);
        let slots = 1.max((z1 - z0 - m(1.0)) / m(2.4));
        let slot = |j: i64| z0 + (z1 - z0) * (2 * j + 1) / (2 * slots);
        let door = slots / 2;
        let mut openings = Vec::new();
        let mut bands = Vec::new();
        for front in [true, false] {
            let out = if front { facing } else { -facing };
            let f = Facade { axis: 0, face: if out > 0 { x1 } else { x0 }, out };
            for storey in 0..storeys {
                let floor = base + storey * STOREY;
                let sill = floor + m(0.9);
                // Far away, a band of the facade as much glass as its windows.
                let glazed = (slots * 8) as f64 / (z1 - z0) as f64;
                bands.push(f.bx(0, 1, (z0, z1), (sill, sill + 11), K::glass, Op::Paint));
                bands.last_mut().expect("one").fill = Fill::Const(glazed);
                for j in 0..slots {
                    let zc = slot(j);
                    if front && storey == 0 && j == door {
                        openings.push(f.bx(0, 1, (zc - 5, zc + 6), (floor, floor + 19), K::trim, Op::Paint));
                        if shop {
                            openings.push(f.bx(0, WALL, (zc - 4, zc + 5), (floor, floor + 18), K::trim, Op::Carve));
                        } else {
                            openings.push(f.bx(0, 2, (zc - 4, zc + 5), (floor, floor + 18), K::trim, Op::Carve));
                            openings.push(f.bx(2, 3, (zc - 4, zc + 5), (floor, floor + 18), K::door, Op::Paint));
                        }
                    } else if front && storey == 0 && shop {
                        window(&f, zc, floor + 3, ((z1 - z0) / slots / 2 - 3).min(9), 15, None, &mut openings);
                    } else {
                        window(&f, zc, sill, 4, 11, shutters, &mut openings);
                    }
                }
            }
        }
        if shop {
            let (r0, r1) = ([x0 + WALL, base + 1, z0 + WALL], [x1 - WALL, base + STOREY - 2, z1 - WALL]);
            openings.push(bx(r0, r1, K::trim, Op::Carve));
            openings.push(bx([r0[0], base, r0[2]], [r1[0], base + 1, r1[2]], K::planks, Op::Paint));
            // Shelves of goods along the back wall, a counter across the room.
            let back = Facade { axis: 0, face: if facing > 0 { x0 } else { x1 }, out: -facing };
            openings.push(back.bx(WALL, WALL + 4, (r0[2] + 2, r1[2] - 2), (base + 1, base + 17), K::counter, Op::Fill));
            let mut goods = back.bx(WALL - 1, WALL + 4, (r0[2] + 2, r1[2] - 2), (base + 3, base + 16), K::goods, Op::Paint);
            goods.fill = Fill::Const(0.5);
            openings.push(goods);
            let mid = Facade { axis: 0, face: (x0 + x1) / 2, out: -facing };
            openings.push(mid.bx(0, 3, (r0[2] + 4, (r0[2] + r1[2]) / 2), (base + 1, base + 8), K::counter, Op::Fill));
        }
        let bounds = crate::generator::bounds_of_shapes(body.iter().chain(&openings).map(|s| &s.bounds));
        let mut far = body.clone();
        far.extend(bands);
        Node {
            shapes: Shapes::Now(body),
            bounds: Some(bounds),
            children: Some(Children::Fixed(vec![child("openings".into(), ANY_SIZE, Node::of(openings))])),
            far: Some(Far { detail: fixed(2.0), shapes: far }),
        }
    }
}

/** The church on the square: a nave along x, a tower with a spire at its front onto the main street. */
fn church(t: &Town, b: &Block) -> Node {
    let base = t.level + 1;
    let (zc, half) = ((b.z0 + b.z1) / 2, m(6.0));
    let (tower_lo, tower_hi) = across(b, m(4.0), m(9.0));
    let (nave_lo, nave_hi) = across(b, m(9.0), BLOCK_W - m(3.0));
    let nave_top = base + m(9.0);
    let rise = m(5.5) as f64;
    let tower_top = base + m(22.0);
    let tz = (zc - m(2.5), zc + m(2.5));
    let mut shapes = vec![
        bx([nave_lo, base - 3, zc - half], [nave_hi, nave_top, zc + half], K::churchStone, Op::Fill),
        gable(
            [nave_lo as f64, nave_top as f64, (zc - half) as f64],
            [nave_hi as f64, nave_top as f64 + rise * 0.9, (zc + half) as f64],
            0,
            k(K::churchStone),
            SOLID,
            Op::Fill,
        ),
        gable(
            [nave_lo as f64, nave_top as f64, (zc - half - 3) as f64],
            [nave_hi as f64 + 2.0, nave_top as f64 + rise, (zc + half + 3) as f64],
            0,
            k(K::slate),
            SOLID,
            Op::Fill,
        ),
        bx([tower_lo, base - 3, tz.0], [tower_hi, tower_top, tz.1], K::churchStone, Op::Fill),
    ];
    let (cx, r) = (((tower_lo + tower_hi) / 2) as f64, (m(2.5) + 1) as f64);
    let tip = m(12.0) as f64;
    shapes.push(lathe(
        [cx, tower_top as f64, zc as f64],
        [cx, tower_top as f64 + tip, zc as f64],
        move |t| r * (1.0 - t) + 1.0,
        k(K::copper),
        SOLID,
        Op::Fill,
    ));
    let far = shapes.clone();
    // Tall windows down the nave's sides, belfry openings and a clock on the tower, a door at its foot.
    let mut openings = Vec::new();
    let windows = (nave_hi - nave_lo - m(2.0)) / m(3.0);
    for side in [-1, 1] {
        let f = Facade { axis: 2, face: if side > 0 { zc + half } else { zc - half }, out: side };
        for i in 0..windows {
            let x = nave_lo + m(1.5) + (nave_hi - nave_lo - m(3.0)) * (2 * i + 1) / (2 * windows);
            openings.push(f.bx(0, 2, (x - 3, x + 3), (base + m(2.0), base + m(7.0)), K::trim, Op::Carve));
            openings.push(f.bx(2, 3, (x - 3, x + 3), (base + m(2.0), base + m(7.0)), K::glass, Op::Paint));
        }
    }
    let front = Facade { axis: 0, face: if b.side > 0 { tower_lo } else { tower_hi }, out: -b.side };
    openings.push(front.bx(0, 3, (zc - 7, zc + 7), (base, base + m(3.5)), K::trim, Op::Carve));
    openings.push(front.bx(3, 4, (zc - 7, zc + 7), (base, base + m(3.5)), K::door, Op::Paint));
    openings.push(ball([front.face as f64, (tower_top - m(5.0)) as f64, zc as f64], 7.0, k(K::trim), SOLID, Op::Paint));
    for (face, out) in [(tower_lo, -1), (tower_hi, 1)] {
        let f = Facade { axis: 0, face, out };
        openings.push(f.bx(0, m(5.0), (zc - 6, zc + 6), (tower_top - m(3.0), tower_top - m(1.0)), K::trim, Op::Carve));
    }
    let bounds = crate::generator::bounds_of_shapes(far.iter().chain(&openings).map(|s| &s.bounds));
    Node {
        shapes: Shapes::Now(shapes),
        bounds: Some(bounds),
        children: Some(Children::Fixed(vec![child("openings".into(), ANY_SIZE, Node::of(openings))])),
        far: Some(Far { detail: fixed(2.0), shapes: far }),
    }
}

/** Street lamps down both sides of the main street in row r, by turns. */
fn lamps(t: &Town, r: i64) -> Node {
    let base = t.level + 2;
    let z0 = t.z - t.half_z() + r * (BLOCK_D + STREET);
    let mut shapes = Vec::new();
    let mut z = z0 + m(4.0);
    let mut side = 1;
    while z < z0 + BLOCK_D - m(2.0) {
        let x = t.x + side * (MAIN / 2 + m(0.75));
        let top = base + m(3.5);
        shapes.push(bx([x - 2, base, z - 2], [x + 2, base + 2, z + 2], K::iron, Op::Fill));
        shapes.push(bx([x - 1, base, z - 1], [x + 1, top, z + 1], K::iron, Op::Fill));
        shapes.push(bx([x - 2, top, z - 2], [x + 2, top + 4, z + 2], K::lamp, Op::Fill));
        shapes.push(bx([x - 3, top + 4, z - 3], [x + 3, top + 5, z + 3], K::iron, Op::Fill));
        z += m(16.0);
        side = -side;
    }
    Node::of(shapes)
}

/** A fountain in the middle of the square: a basin, a column and a bowl, water in both. */
fn fountain(t: &Town) -> Node {
    let base = (t.level + 1) as f64;
    let z0 = t.z - t.half_z() + t.square_row() * (BLOCK_D + STREET);
    let (x, z) = (t.x as f64, (z0 + BLOCK_D / 2) as f64);
    let stone = k(K::churchStone);
    let at = |y: f64| [x, base + y, z];
    Node::of(vec![
        lathe(at(-1.0), at(5.0), |_| m(3.0) as f64, stone, SOLID, Op::Fill),
        lathe(at(3.0), at(5.0), |_| (m(3.0) - 3) as f64, k(K::water), SOLID, Op::Paint),
        lathe(at(5.0), at(15.0), |_| 3.0, stone, SOLID, Op::Fill),
        lathe(at(14.0), at(18.0), |t| 4.0 + 6.0 * t, stone, SOLID, Op::Fill),
        lathe(at(17.0), at(18.0), |_| 7.0, k(K::water), SOLID, Op::Paint),
        ball(at(20.0), 2.0, stone, SOLID, Op::Fill),
    ])
}
