//! Towns along the valley's path: blocks of houses on streets around a
//! square with a church and a fountain. A domain of the valley (valley.rs):
//! it bids for the land around its towns, levels the ground under them,
//! and draws only the columns it wins. Its trees are the forest's.

use std::rc::Rc;

use worldgen::frame::{Frame, Part, U, V, W, back, walls};
use worldgen::generator::{
    ANY_SIZE, Child, Children, Far, Field, FieldValues, Fill, Layer, Node, Op, Population, SOLID, Shape, Shapes, Span,
};
use worldgen::grid::Box3;
use worldgen::kinds;
use worldgen::noise::{Fx, Rng, fx, hash64, smoothstep, stream};
use worldgen::plan::{Land, Plan};
use worldgen::units::{Units, m};

use crate::forest::{self, K as F, Relief, Species, make_tree};

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

// The plan of a town, in units: a main street along z, blocks on either side of it in rows, side and cross streets between them.
const MAIN: i64 = m(10.0);
const STREET: i64 = m(7.0);
const BLOCK_W: i64 = m(36.0);
const BLOCK_D: i64 = m(40.0);
const PAVEMENT: i64 = m(2.5);
const STOREY: i64 = m(3.25);
const WALL: i64 = 3;
/**
 * How much deeper a roof is than what it covers, measured straight down,
 * besides its slope: two sites of level 1, so that columns of its sites
 * cross it rather than step over it to the wall under it.
 */
const ROOF: i64 = 4;
/** Towns are this far apart along the path, give or take JITTER. */
const SPACING: i64 = m(1200.0);
const JITTER: i64 = m(150.0);
/** Plan scales: where towns are; the bid for their land. */
pub const TOWNS_SCALE: i64 = m(400.0);
pub const BID_SCALE: i64 = m(4.0);
/** What a column of a town is: its finest parts the square's paving, a checker of metres. */
const SPOT_SCALE: i64 = m(1.0);
/** How far past its streets a town levels the ground, easing back to the valley's. */
const TERRACE: Units = Units::m(120.0);

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
#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum Spot {
    #[default]
    Outside,
    Street,
    /** The square's paving, light and dark by turns. */
    Square,
    SquareDark,
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

    /** Distance from the town's streets and blocks: negative inside. */
    pub fn distance(&self, x: i64, z: i64) -> Units {
        let (dx, dz) = ((x - self.x).abs() - self.half_x(), (z - self.z).abs() - self.half_z());
        if dx <= 0 && dz <= 0 {
            return Units(dx.max(dz));
        }
        let (dx, dz) = (dx.max(0), dz.max(0));
        Units(((dx * dx + dz * dz) as u64).isqrt() as i64)
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
            return if r == self.square_row() { self.paving(x, z) } else { Spot::Street };
        }
        let (c, w) = ((a - MAIN / 2) / (BLOCK_W + STREET), (a - MAIN / 2) % (BLOCK_W + STREET));
        if w >= BLOCK_W {
            return Spot::Street;
        }
        if r == self.square_row() && c == 0 {
            return self.paving(x, z);
        }
        let along_street = w < PAVEMENT
            || (c < self.cols - 1 && w >= BLOCK_W - PAVEMENT)
            || (r > 0 && g < PAVEMENT)
            || (r < self.rows - 1 && g >= BLOCK_D - PAVEMENT);
        if along_street { Spot::Pavement } else { Spot::Yard }
    }

    /** The square's paving at column (x, z). */
    fn paving(&self, x: i64, z: i64) -> Spot {
        let light = ((x - self.x).div_euclid(m(1.0)) + (z - self.z).div_euclid(m(1.0))) & 1 == 0;
        if light { Spot::Square } else { Spot::SquareDark }
    }

    /** Everything it draws: its blocks, and what grows over their edges, from under its streets to its church's spire. */
    fn bounds(&self) -> Box3 {
        let (hx, hz) = (self.half_x() + m(10.0), self.half_z() + m(10.0));
        Box3 { min: [self.x - hx, self.level - m(3.0), self.z - hz], max: [self.x + hx, self.level + m(40.0), self.z + hz] }
    }
}

/** The town of `towns` nearest z along the path, if it has one. */
fn nearest(towns: &Plan<Option<Town>>, z: i64) -> Option<Town> {
    towns.at(0, (z + SPACING / 2).div_euclid(SPACING))
}

/** The towns along the valley's path, by where they are. */
pub struct Towns {
    plan: Rc<Plan<Option<Town>>>,
    /** What each column is, a plan of categories: dithered far away, so streets show as often as they are. */
    spots: Plan<Spot>,
}

impl Towns {
    /**
     * Towns every SPACING along z, some of them, the first near the start.
     * Each lies on the path's coarse band (its mean, x = 0: the path bends
     * to meet it) at the mean height of the valley floor under it.
     */
    pub fn new(seed: u64, relief: Rc<Relief>) -> Towns {
        let s = stream(seed, 30);
        let plan = Rc::new(Plan::new("towns", TOWNS_SCALE, move |_, n| {
            let mut random = Rng::new(hash64(s, n, 0, 0));
            if n != 0 && !random.chance(fx(0.6)) {
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
            let around = (-1..=1).flat_map(|i| (-1..=1).map(move |j| (i, j)));
            town.level = Fx::mean(around.map(|(i, j)| relief.ground(i * hx, z + j * hz, Fx::ZERO))).round_units().0;
            Some(town)
        }));
        let spots = {
            let plan = plan.clone();
            Plan::dithered("town spots", SPOT_SCALE, move |x, z| nearest(&plan, z).map_or(Spot::Outside, |t| t.spot(x, z)))
        };
        Towns { plan, spots }
    }

    /** The town nearest z along the path, if it has one. */
    pub fn near(&self, z: i64) -> Option<Town> {
        nearest(&self.plan, z)
    }

    /** What column (x, z) is of the town nearest it. */
    fn spot(&self, x: i64, z: i64) -> Spot {
        self.spots.at(x, z)
    }

    /** The path's centre line at z, with its fraction, bent from `route` onto the main street of the town there. */
    pub fn street_x(&self, z: i64, route: Fx) -> Fx {
        let Some(t) = self.near(z) else { return route };
        let bend = Fx::ONE - smoothstep(Fx::ZERO, Units::m(60.0).to_fx(), Units((z - t.z).abs() - t.half_z()).to_fx());
        route + (Units(t.x).to_fx() - route) * bend
    }

    /** The town's bid for column (x, z), BID_SCALE: ahead inside its streets, behind a few metres out. */
    pub fn bid(&self, x: i64, z: i64) -> Fx {
        let most = fx(4.0);
        self.near(z).map_or(-most, |t| ((Units::m(4.0) - t.distance(x, z)).to_fx() / Units::m(8.0)).clamp(-most, most))
    }

    /** `ground`, with its fraction, levelled to the town's streets, all the way under it and easing back over TERRACE. */
    pub fn terraced(&self, x: i64, z: i64, ground: Fx) -> Fx {
        let Some(t) = self.near(z) else { return ground };
        let weight = Fx::ONE - smoothstep(Fx::ZERO, TERRACE.to_fx(), t.distance(x, z).max(Units(0)).to_fx());
        ground + (Units(t.level).to_fx() - ground) * weight
    }

    /** How far column (x, z) is from the nearest town (negative inside); far if none. */
    pub fn distance(&self, x: i64, z: i64) -> Units {
        self.near(z).map_or(Units(i64::MAX / 4), |t| t.distance(x, z))
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
                    let spot = towns.spot(x, z);
                    if spot == Spot::Outside || !land.owns(me, x, z) {
                        return Vec::new();
                    }
                    let h = ground.at(x, z);
                    let top = h + 1 - m(0.5).max(2 * spacing);
                    let surface = |kind| Span { from: top, to: h + 1, kind, extinction: None };
                    match spot {
                        Spot::Street => vec![surface(k(K::cobble))],
                        Spot::Square => vec![surface(k(K::paving))],
                        Spot::SquareDark => vec![surface(k(K::pavingDark))],
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

/** A block's own coordinates: u across it from its inner edge (towards the main street), v up from the streets, w along it. */
fn block_frame(t: &Town, b: &Block) -> Frame {
    let base = t.level + 1;
    if b.side > 0 { Frame::new([b.x0, base, b.z0], [U, V, W]) } else { Frame::new([b.x1 - 1, base, b.z0], [back(U), V, W]) }
}

/** A tree of the forest's, its foot on the streets' level at (u, w) of `frame`. */
fn tree(t: &Town, frame: &Frame, u: i64, w: i64, oak: bool, random: &mut Rng) -> Node {
    let [x, y, z] = frame.site([u, -1, w]);
    let level = t.level;
    make_tree(if oak { Species::Oak } else { Species::Birch }, x as f64, y as f64, z as f64, &mut Rng::new(random.next_u64()), &|_, _| {
        level
    })
}

fn block(t: &Town, b: &Block, bounds: Box3) -> Node {
    let mut random = Rng::new(hash64(t.seed, b.side, b.c, b.r));
    let frame = block_frame(t, b);
    let mut children = Vec::new();
    let node =
        |children| Node { shapes: Shapes::Now(Vec::new()), bounds: Some(bounds), children: Some(Children::Fixed(children)), far: None };
    if b.r == t.square_row() && b.c == 0 {
        if b.side > 0 {
            let mut shapes = Vec::new();
            church(&mut Part::new(frame.then(Frame::at([m(4.0), 0, BLOCK_D / 2 - m(6.0)])), &mut shapes));
            children.push(child("church".into(), ANY_SIZE, thing(shapes)));
        } else {
            // Trees at the square's corners, the forest's own.
            for (i, (u, w)) in
                [(m(6.0), m(6.0)), (m(6.0), BLOCK_D - m(6.0)), (BLOCK_W - m(8.0), m(6.0)), (BLOCK_W - m(8.0), BLOCK_D - m(6.0))]
                    .into_iter()
                    .enumerate()
            {
                let oak = random.chance(Fx::HALF);
                children.push(child(format!("tree {i}"), ANY_SIZE, tree(t, &frame, u, w, oak, &mut random)));
            }
        }
        return node(children);
    }
    // Two rows of lots back to back, each facing its street; along the block, lots 7 to 12 m wide.
    for back_row in [false, true] {
        let mut w = PAVEMENT;
        let mut n = 0;
        while w < BLOCK_D - PAVEMENT {
            let mut width = m(7.0) + random.below(m(5.0) as u32) as i64;
            if BLOCK_D - PAVEMENT - (w + width) < m(5.0) {
                width = BLOCK_D - PAVEMENT - w;
            }
            let gap = if b.c == 0 { 0 } else { m(1.0) + random.below(m(1.0) as u32) as i64 };
            let setback = if b.c == 0 { 0 } else { random.below(m(2.0) as u32) as i64 };
            let depth = m(8.0) + random.below(m(3.0) as u32) as i64;
            // The house's own frame: u along the street, w back from its front.
            let front = PAVEMENT + setback;
            let at = if back_row {
                Frame::new([BLOCK_W - 1 - front, 0, w + gap], [W, V, back(U)])
            } else {
                Frame::new([front, 0, w + gap], [W, V, U])
            };
            let storeys = if b.c == 0 { 2 + random.below(2) as i64 } else { 1 + random.below(2) as i64 };
            let house = House { length: width - 2 * gap, depth, storeys, shop: b.c == 0 && !back_row };
            let mut shapes = Vec::new();
            house.build(&mut Part::new(frame.then(at), &mut shapes), &mut random);
            children.push(child(format!("house {} {n}", u8::from(back_row)), ANY_SIZE, thing(shapes)));
            // A tree in the yard behind, now and then.
            let yard = BLOCK_W / 2 - (front + depth);
            if yard >= m(4.0) && random.chance(fx(0.3)) {
                let u = BLOCK_W / 2 + if back_row { yard / 2 } else { -yard / 2 };
                let oak = !random.chance(fx(0.6));
                children.push(child(
                    format!("tree {} {n}", u8::from(back_row)),
                    ANY_SIZE,
                    tree(t, &frame, u, w + width / 2, oak, &mut random),
                ));
            }
            w += width;
            n += 1;
        }
    }
    node(children)
}

/** A thing of `shapes`, its far form measured from them (see "Levels" in docs/generators.md). */
fn thing(shapes: Vec<Shape>) -> Node {
    Node { far: Some(Far::measured(fx(2.0))), ..Node::of(shapes) }
}

/** A house in its own frame: u along its front, v up from its floor, w back from its front. */
struct House {
    length: i64,
    depth: i64,
    storeys: i64,
    shop: bool,
}

const PLASTERS: [K; 6] = [K::plasterOchre, K::plasterWhite, K::plasterPink, K::plasterBlue, K::plasterSage, K::brick];

/**
 * A window on a wall (its own frame: u along it, v up, w into it), from
 * (u − half, v) to (u + half, v + tall): a frame round it, glass set back
 * in a reveal, a sill, and shutters if any.
 */
fn window(wall: &mut Part, u: i64, v: i64, half: i64, tall: i64, shutters: Option<K>) {
    wall.paint([u - half - 1, v - 1, 0], [u + half + 1, v + tall + 1, 1], k(K::trim));
    wall.cut([u - half, v, 0], [u + half, v + tall, 2]);
    wall.paint([u - half, v, 2], [u + half, v + tall, 3], k(K::glass));
    wall.add([u - half - 1, v - 1, -1], [u + half + 1, v, 0], k(K::trim));
    if let Some(kind) = shutters {
        wall.add([u - half - 5, v, -1], [u - half - 1, v + tall, 0], k(kind));
        wall.add([u + half + 1, v, -1], [u + half + 5, v + tall, 0], k(kind));
    }
}

/** A door on a wall from (u − half, 0) up, `tall`: framed, its leaf set back, or open (`open`) through `thick`. */
fn door(wall: &mut Part, u: i64, half: i64, tall: i64, open: Option<i64>) {
    wall.paint([u - half - 1, 0, 0], [u + half + 1, tall + 1, 1], k(K::trim));
    match open {
        Some(thick) => {
            wall.cut([u - half, 0, 0], [u + half, tall, thick]);
        }
        None => {
            wall.cut([u - half, 0, 0], [u + half, tall, 2]);
            wall.paint([u - half, 0, 2], [u + half, tall, 3], k(K::door));
        }
    }
}

impl House {
    fn build(&self, part: &mut Part, random: &mut Rng) {
        let House { length, depth, storeys, .. } = *self;
        let kinds = if random.chance(fx(0.1)) { 6 } else { 5 };
        let plaster = k(PLASTERS[random.below(kinds) as usize]);
        let roof = k([K::roofRed, K::roofRed, K::roofBrown, K::slate][random.below(4) as usize]);
        let top = storeys * STOREY;
        part.add([-1, -3, -1], [length + 1, 4, depth + 1], k(K::churchStone));
        part.add([0, 0, 0], [length, top, depth], plaster);
        // Its ridge along the street, eaves to it; now and then gable-fronted.
        let ridge = if random.chance(fx(0.75)) { 0 } else { 2 };
        let across = 2 - ridge;
        let (width, eaves) = (if across == 0 { length } else { depth }, 3);
        let rise = (width as f64 / 2.0 + eaves as f64) * (0.6 + random.next() * 0.4);
        let roofed = |rise: f64, eaves: i64| {
            let (mut min, mut max) = ([0.0, top as f64, 0.0], [length as f64, top as f64 + rise, depth as f64]);
            (min[across], max[across]) = (-eaves as f64, (width + eaves) as f64);
            (min, max)
        };
        let (mut min, mut max) = roofed(rise * width as f64 / (width + 2 * eaves) as f64, 0);
        (min[1], max[1]) = (min[1] - ROOF as f64, max[1] - ROOF as f64);
        part.gable(min, max, ridge, plaster, Op::Fill);
        let (min, max) = roofed(rise, eaves);
        part.gable(min, max, ridge, roof, Op::Fill);
        // Eaves boarded along both sides: a thin wedge's edge would leave gaps.
        for (from, to) in [(-eaves, 0), (width, width + eaves)] {
            let (mut min, mut max) = ([0, top - 2, 0], [length, top + 1, depth]);
            (min[across], max[across]) = (from, to);
            part.add(min, max, roof);
        }
        let (cu, cw) = (4 + random.below((length - 9).max(1) as u32) as i64, 4 + random.below((depth - 9).max(1) as u32) as i64);
        part.add([cu, top, cw], [cu + 5, top + rise as i64 + m(0.8), cw + 5], k(K::brick));

        // Windows front and back, a door in the front's middle; a shop's open door, wide windows and room on the ground floor.
        let shutters = random.chance(fx(0.4)).then(|| if random.chance(Fx::HALF) { K::shutterGreen } else { K::shutterBlue });
        let shop = self.shop && random.chance(Fx::HALF);
        let slots = 1.max((length - m(1.0)) / m(2.4));
        let slot = |j: i64| length * (2 * j + 1) / (2 * slots);
        for (i, (wall, _)) in walls(length, depth).into_iter().enumerate().filter(|(i, _)| i % 2 == 0) {
            let mut wall = part.within(wall);
            for storey in 0..storeys {
                let floor = storey * STOREY;
                for j in 0..slots {
                    let u = slot(j);
                    if i == 0 && storey == 0 && j == slots / 2 {
                        door(&mut wall, u, 4, 18, shop.then_some(WALL));
                    } else if i == 0 && storey == 0 && shop {
                        window(&mut wall, u, 3, (length / slots / 2 - 3).min(9), 15, None);
                    } else {
                        window(&mut wall, u, floor + m(0.9), 4, 11, shutters);
                    }
                }
            }
        }
        if shop {
            let room = ([WALL, 1, WALL], [length - WALL, STOREY - 2, depth - WALL]);
            part.cut(room.0, room.1);
            part.paint([room.0[0], 0, room.0[2]], [room.1[0], 1, room.1[2]], k(K::planks));
            // Shelves of goods along the back wall, a counter across the room.
            let shelves = ([room.0[0] + 2, 1, room.1[2] - 4], [room.1[0] - 2, 17, room.1[2]]);
            part.add(shelves.0, shelves.1, k(K::counter));
            part.paint([shelves.0[0], 3, shelves.0[2] - 1], [shelves.1[0], 16, shelves.1[2]], k(K::goods)).fill = Fill::Const(0.5);
            part.add([room.0[0] + 4, 1, depth / 2], [length / 2, 8, depth / 2 + 3], k(K::counter));
        }
    }
}

/** The church, in its own frame: its tower's front on u from 0 to TOWER, a nave behind it, CHURCH wide across w. */
fn church(part: &mut Part) {
    const TOWER: i64 = m(5.0);
    const NAVE: i64 = m(12.0);
    let (nave_length, nave_top, tower_top) = (BLOCK_W - m(7.0) - TOWER, m(9.0), m(22.0));
    let stone = k(K::churchStone);
    // The nave: u along it (from the tower), w across.
    let mut nave = part.within(Frame::new([TOWER, 0, 0], [U, V, W]));
    nave.add([0, -3, 0], [nave_length, nave_top, NAVE], stone);
    let rise = m(5.5) as f64;
    let gable = (nave_top - ROOF) as f64;
    nave.gable([0.0, gable, 0.0], [nave_length as f64, gable + rise * 0.9, NAVE as f64], 0, stone, Op::Fill);
    nave.gable(
        [0.0, nave_top as f64, -3.0],
        [nave_length as f64 + 2.0, nave_top as f64 + rise, (NAVE + 3) as f64],
        0,
        k(K::slate),
        Op::Fill,
    );
    // Tall windows down its two long walls.
    let windows = (nave_length - m(2.0)) / m(3.0);
    for (wall, length) in walls(nave_length, NAVE).into_iter().step_by(2) {
        let mut wall = nave.within(wall);
        for i in 0..windows {
            let u = m(1.5) + (length - m(3.0)) * (2 * i + 1) / (2 * windows);
            wall.cut([u - 3, m(2.0), 0], [u + 3, m(7.0), 2]);
            wall.paint([u - 3, m(2.0), 2], [u + 3, m(7.0), 3], k(K::glass));
        }
    }
    // The tower: square, across the nave's middle, its door onto the square, a clock above, belfry openings on every side, a spire.
    let mut tower = part.within(Frame::at([0, 0, NAVE / 2 - TOWER / 2]));
    tower.add([0, -3, 0], [TOWER, tower_top, TOWER], stone);
    for (q, (wall, _)) in walls(TOWER, TOWER).into_iter().enumerate() {
        let mut wall = tower.within(wall);
        if q == 3 {
            door(&mut wall, TOWER / 2, 7, m(3.5), None);
            wall.ball([(TOWER / 2) as f64, (tower_top - m(5.0)) as f64, 0.0], 7.0, k(K::trim), Op::Paint);
        }
        wall.cut([TOWER / 2 - 6, tower_top - m(3.0), 0], [TOWER / 2 + 6, tower_top - m(1.0), m(1.0)]);
    }
    let (c, r) = ((TOWER / 2) as f64, (TOWER / 2 + 1) as f64);
    tower.lathe(
        [c, tower_top as f64, c],
        [c, (tower_top + m(12.0)) as f64, c],
        move |t| r * (1.0 - t) + 1.0,
        k(K::copper),
        SOLID,
        Op::Fill,
    );
}

/** Street lamps down both sides of the main street in row r, by turns. */
fn lamps(t: &Town, r: i64) -> Node {
    let z0 = t.z - t.half_z() + r * (BLOCK_D + STREET);
    let mut shapes = Vec::new();
    let (mut z, mut side) = (z0 + m(4.0), 1);
    while z < z0 + BLOCK_D - m(2.0) {
        // On the pavement's kerb, a step above the street.
        let mut lamp = Part::new(Frame::at([t.x + side * (MAIN / 2 + m(0.75)), t.level + 2, z]), &mut shapes);
        let top = m(3.5);
        lamp.add([-2, 0, -2], [2, 2, 2], k(K::iron));
        lamp.add([-1, 0, -1], [1, top, 1], k(K::iron));
        lamp.add([-2, top, -2], [2, top + 4, 2], k(K::lamp));
        lamp.add([-3, top + 4, -3], [3, top + 5, 3], k(K::iron));
        z += m(16.0);
        side = -side;
    }
    Node::of(shapes)
}

/** A fountain in the middle of the square: a basin, a column and a bowl, water in both. */
fn fountain(t: &Town) -> Node {
    let z0 = t.z - t.half_z() + t.square_row() * (BLOCK_D + STREET);
    let mut shapes = Vec::new();
    let mut f = Part::new(Frame::at([t.x, t.level + 1, z0 + BLOCK_D / 2]), &mut shapes);
    let (stone, water) = (k(K::churchStone), k(K::water));
    let up = |y: f64| [0.0, y, 0.0];
    f.lathe(up(-1.0), up(5.0), |_| m(3.0) as f64, stone, SOLID, Op::Fill);
    f.lathe(up(3.0), up(5.0), |_| (m(3.0) - 3) as f64, water, SOLID, Op::Paint);
    f.lathe(up(5.0), up(15.0), |_| 3.0, stone, SOLID, Op::Fill);
    f.lathe(up(14.0), up(18.0), |t| 4.0 + 6.0 * t, stone, SOLID, Op::Fill);
    f.lathe(up(17.0), up(18.0), |_| 7.0, water, SOLID, Op::Paint);
    f.ball(up(20.0), 2.0, stone, Op::Fill);
    Node::of(shapes)
}
