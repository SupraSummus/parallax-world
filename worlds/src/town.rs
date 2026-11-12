//! Towns along the valley's path: houses on streets around a square, of a
//! size and in a style each town's seed decides. Hamlets of a few houses
//! with gardens and barns; villages round a green or a paved square with
//! their church and well; market towns of shops round a market hall, some
//! walled with gates on their main street. A style is how its houses are
//! built: southern plaster under low tiles, northern walls under steep
//! slate, timber frames, brick with stepped gables. A domain of the valley
//! (valley.rs): it bids for the land around its towns, levels the ground
//! under them, and draws only the columns it wins. Its trees are the forest's.

use std::rc::Rc;

use worldgen::frame::{Frame, Part, U, V, W, back, walls};
use worldgen::generator::{
    ANY_SIZE, About, Anchors, Child, Children, Far, Field, FieldValues, Fill, Kind, Layer, Node, Op, Population, SOLID, Shape, Shapes,
    Span, bounds_of_shapes,
};
use worldgen::grid::Box3;
use worldgen::kinds;
use worldgen::noise::{Fx, Rng, ease_towards, fx, hash64, stream};
use worldgen::plan::{Claims, Land, Plan, Sloped};
use worldgen::units::{Units, m};

use crate::forest::{self, K as F, Relief, Species, make_tree};

kinds! {
    base: forest::KINDS.len() as u8;
    plasterOchre: 10, ["#d8b878", "#cfae6c"];
    plasterWhite: 10, ["#ece6d6", "#e2dccb"];
    plasterPink: 10, ["#d9a79a", "#cf9b8d"];
    plasterBlue: 10, ["#a9bccb", "#9fb3c3"];
    plasterSage: 10, ["#b5bf9a", "#aab58e"];
    plasterGrey: 10, ["#c2c0b8", "#b6b4ac"];
    faluRed: 10, ["#8e3526", "#80301f"];
    brick: 10, ["#9a4a32", "#8a412c", "#a85538"];
    timber: 4, ["#3e2c1f", "#4a3525"];
    trim: 10, ["#f4f1e8"];
    shutterGreen: 4, ["#2f5a3a", "#356542"];
    shutterBlue: 4, ["#2f4a6a", "#36547a"];
    glass: 11, ["#5b7f97", "#668aa2"];
    door: 4, ["#5a3a24", "#6b4528", "#2f4f3a"];
    roofRed: 9, ["#a8432c", "#b44d32", "#9a3b27"];
    roofTile: 9, ["#c8693c", "#ba5f35"];
    roofBrown: 9, ["#6e4a34", "#7a5238", "#64422e"];
    slate: 9, ["#4a4f57", "#52575f", "#43484f"];
    thatch: 9, ["#a08a56", "#91794a"];
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
    lamp: 6, ["#fff3c4"], glows;
    water: 13, ["#3f7fa8", "#4a8ab2"], walk_through;
}

// The plan of a town, in units: a main street along z, blocks on either side of it in rows, side and cross streets between them.
const MAIN: i64 = m(10.0);
const STREET: i64 = m(7.0);
const BLOCK_W: i64 = m(36.0);
const BLOCK_D: i64 = m(40.0);
const PAVEMENT: i64 = m(2.5);
const STOREY: i64 = m(3.25);
const WALL: i64 = 3;
/** How far under a roof's top its gable ends stop (`gable_ends`), besides its slope: the roof's edge seen at them. */
pub const ROOF: i64 = 4;
/** A walled town's wall stands this far out from its blocks, a lane inside it; it is TOWN_WALL thick. */
const RING: i64 = m(6.0);
const TOWN_WALL: i64 = m(1.5);
/**
 * Towns stand at least APART along the path, claimed (Claims) from spots
 * one every CLAIM of it on average: about one every 2 km. A column's town
 * is the nearest within WITHIN. The first's middle is at FIRST.
 */
const APART: i64 = m(900.0);
const CLAIM: i64 = m(800.0);
const WITHIN: i64 = m(600.0);
const FIRST: i64 = m(200.0);
/** Plan scale of the land they bid for. */
pub const BID_SCALE: i64 = m(4.0);
/** What a column of a town is: its finest parts the square's paving, a checker of metres. */
const SPOT_SCALE: i64 = m(1.0);
/** How far past its streets a town levels the ground, easing back to the valley's. */
const TERRACE: Units = Units::m(120.0);
/** The most blocks a side of the main street, and so the furthest a town reaches across from it, its wall included. */
const COLS: i64 = 3;
pub const ACROSS: i64 = MAIN / 2 + COLS * BLOCK_W + (COLS - 1) * STREET + RING;

/** How a town builds: its houses' walls, roofs and windows, its church, its fences and its wall. */
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub enum Style {
    /** Ochre, pink and white plaster under low roofs of red tiles, shutters on every window, balconies. */
    #[default]
    Southern,
    /** White, grey and red-painted walls under steep slate, dormers, white fences, a white church. */
    Northern,
    /** Dark beams over white infill under steep roofs, often gables to the street. */
    Timbered,
    /** Brick under slate and red tiles, stepped gables to the street, hedges. */
    Brick,
}

/** What a style builds houses of: kinds to pick from, roofs' rise over run, chances of shutters, of gables to the street and of dormers. */
struct Palette {
    walls: &'static [K],
    roofs: &'static [K],
    pitch: [f64; 2],
    shutters: f64,
    shutter_kinds: &'static [K],
    gables: f64,
    dormers: f64,
    /** Storeys more than the town's own. */
    storeys: i64,
}

impl Style {
    /** The style of a settlement of `seed`. */
    pub fn of(seed: u64) -> Style {
        [Style::Southern, Style::Northern, Style::Timbered, Style::Brick][(hash64(seed, 0, 0, 11) % 4) as usize]
    }

    fn palette(self) -> Palette {
        match self {
            Style::Southern => Palette {
                walls: &[K::plasterOchre, K::plasterOchre, K::plasterPink, K::plasterWhite, K::plasterSage],
                roofs: &[K::roofTile, K::roofTile, K::roofRed],
                pitch: [0.3, 0.45],
                shutters: 0.85,
                shutter_kinds: &[K::shutterGreen, K::shutterGreen, K::shutterBlue],
                gables: 0.05,
                dormers: 0.0,
                storeys: 1,
            },
            Style::Northern => Palette {
                walls: &[K::plasterWhite, K::plasterGrey, K::faluRed, K::faluRed, K::plasterOchre, K::plasterBlue],
                roofs: &[K::slate, K::slate, K::slate, K::roofRed, K::roofBrown],
                pitch: [0.95, 1.25],
                shutters: 0.1,
                shutter_kinds: &[K::shutterBlue],
                gables: 0.15,
                dormers: 0.5,
                storeys: 0,
            },
            Style::Timbered => Palette {
                walls: &[K::plasterWhite, K::plasterWhite, K::plasterWhite, K::plasterOchre, K::plasterPink],
                roofs: &[K::roofBrown, K::roofRed, K::roofRed, K::slate],
                pitch: [1.1, 1.5],
                shutters: 0.3,
                shutter_kinds: &[K::shutterGreen, K::faluRed],
                gables: 0.6,
                dormers: 0.5,
                storeys: 1,
            },
            Style::Brick => Palette {
                walls: &[K::brick],
                roofs: &[K::roofRed, K::slate, K::slate, K::roofBrown],
                pitch: [0.8, 1.1],
                shutters: 0.25,
                shutter_kinds: &[K::shutterGreen],
                gables: 0.5,
                dormers: 0.0,
                storeys: 1,
            },
        }
    }

    /** Its church's walls, roof and spire, and the spire's height. */
    fn church(self) -> (K, K, K, i64) {
        match self {
            Style::Southern => (K::churchStone, K::roofTile, K::roofTile, m(3.0)),
            Style::Northern => (K::plasterWhite, K::slate, K::slate, m(14.0)),
            Style::Timbered => (K::churchStone, K::slate, K::copper, m(12.0)),
            Style::Brick => (K::brick, K::slate, K::copper, m(12.0)),
        }
    }
}

/** A kind of `kinds`, picked evenly. */
fn pick(random: &mut Rng, kinds: &[K]) -> K {
    kinds[random.below(kinds.len() as u32) as usize]
}

/** How large a town is, and what it has. */
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
enum Size {
    /** A few houses with gardens and barns along the main street, a well. */
    Hamlet,
    /** Houses with gardens round a square or a green, a church, a well or a fountain. */
    Village,
    /** Shops round a market hall or a fountain, stalls, a church, sometimes a wall with gates. */
    #[default]
    Market,
}

/** What stands in the middle of a town's square. */
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
enum Centre {
    #[default]
    Fountain,
    Well,
    Hall,
}

/** A town: its main street's centre line x, its middle z, the height of its streets, blocks per side and rows of them, and what it is like. */
#[derive(Clone, Copy, Default, Debug)]
pub struct Town {
    pub x: i64,
    pub z: i64,
    pub level: i64,
    cols: i64,
    rows: i64,
    seed: u64,
    style: Style,
    size: Size,
    centre: Centre,
    /** The side of the main street its church is on (±1), 0 without one. */
    church: i64,
    /** Its square a green, grass, not paving. */
    green: bool,
    /** Trees down its main street instead of lamps. */
    avenue: bool,
    walled: bool,
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
    /** A town of `size` at z, its blocks and square drawn from `random`, its style from its seed. */
    fn of(size: Size, z: i64, random: &mut Rng) -> Town {
        let seed = random.next_u64();
        let side = if random.chance(Fx::HALF) { 1 } else { -1 };
        let mut town = Town { z, style: Style::of(seed), size, church: side, seed, ..Town::default() };
        match size {
            Size::Hamlet => {
                (town.cols, town.rows, town.centre, town.church) = (1, 1 + random.below(2) as i64, Centre::Well, 0);
            }
            Size::Village => {
                (town.cols, town.rows) = (1, 3 + random.below(2) as i64);
                town.green = random.chance(fx(0.6));
                town.centre = if town.green || random.chance(Fx::HALF) { Centre::Well } else { Centre::Fountain };
                town.avenue = random.chance(fx(0.5));
            }
            Size::Market => {
                (town.cols, town.rows) = (2 + random.below(COLS as u32 - 1) as i64, 4 + random.below(2) as i64);
                town.centre = if random.chance(fx(0.65)) { Centre::Hall } else { Centre::Fountain };
                town.walled = random.chance(fx(0.6));
            }
        }
        town
    }

    pub(crate) fn half_x(&self) -> i64 {
        MAIN / 2 + self.cols * BLOCK_W + (self.cols - 1) * STREET
    }

    pub(crate) fn half_z(&self) -> i64 {
        (self.rows * BLOCK_D + (self.rows - 1) * STREET) / 2
    }

    /** How far past its blocks it reaches: to the outside of its wall. */
    fn ring(&self) -> i64 {
        if self.walled { RING } else { 0 }
    }

    /** Its square's row; a hamlet has none. */
    fn square_row(&self) -> Option<i64> {
        (self.size != Size::Hamlet).then_some(self.rows / 2)
    }

    /** Where its square's middle is along z. */
    fn square_z(&self) -> i64 {
        self.z - self.half_z() + self.rows / 2 * (BLOCK_D + STREET) + BLOCK_D / 2
    }

    /** Distance from the town's streets and blocks, and wall if any: negative inside. */
    pub fn distance(&self, x: i64, z: i64) -> Units {
        let (dx, dz) = ((x - self.x).abs() - self.half_x() - self.ring(), (z - self.z).abs() - self.half_z() - self.ring());
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
        let (hx, dz, ring) = (self.half_x(), 2 * self.half_z(), self.ring());
        if a >= hx + ring || f < -ring || f >= dz + ring {
            return Spot::Outside;
        }
        if a >= hx || f < 0 || f >= dz {
            // Inside the wall: a lane round the blocks, grass at the wall's foot, the main street out through its gates.
            let out = (a - hx).max(-1 - f).max(f - dz);
            return if a < MAIN / 2 || out < ring - TOWN_WALL { Spot::Street } else { Spot::Yard };
        }
        let (r, g) = (f / (BLOCK_D + STREET), f % (BLOCK_D + STREET));
        if g >= BLOCK_D {
            return Spot::Street;
        }
        let square = self.square_row() == Some(r);
        if a < MAIN / 2 {
            return if square && !self.green { self.paving(x, z) } else { Spot::Street };
        }
        let (c, w) = ((a - MAIN / 2) / (BLOCK_W + STREET), (a - MAIN / 2) % (BLOCK_W + STREET));
        if w >= BLOCK_W {
            return Spot::Street;
        }
        if square && c == 0 {
            return if self.green { Spot::Yard } else { self.paving(x, z) };
        }
        let along_street = w < PAVEMENT
            || (c < self.cols - 1 && w >= BLOCK_W - PAVEMENT)
            || (r > 0 && g < PAVEMENT)
            || (r < self.rows - 1 && g >= BLOCK_D - PAVEMENT);
        // A hamlet's verges are grass.
        if along_street && self.size != Size::Hamlet { Spot::Pavement } else { Spot::Yard }
    }

    /** The square's paving at column (x, z). */
    fn paving(&self, x: i64, z: i64) -> Spot {
        let light = ((x - self.x).div_euclid(m(1.0)) + (z - self.z).div_euclid(m(1.0))) & 1 == 0;
        if light { Spot::Square } else { Spot::SquareDark }
    }

    /** Everything it draws: its blocks and wall, and what grows over their edges, from under its streets to its church's spire. */
    fn bounds(&self) -> Box3 {
        let (hx, hz) = (self.half_x() + self.ring() + m(10.0), self.half_z() + self.ring() + m(10.0));
        Box3 { min: [self.x - hx, self.level - m(3.0), self.z - hz], max: [self.x + hx, self.level + m(40.0), self.z + hz] }
    }
}

/** The town of `towns` nearest z along the path, if it has one. */
fn nearest(towns: &Claims<Town>, z: i64) -> Option<Town> {
    towns.near(0, z, WITHIN).map(|(_, town)| town)
}

/** The towns along the valley's path, by where they are. */
pub struct Towns {
    plan: Rc<Claims<Town>>,
    /** What each column is, a plan of categories: dithered far away, so streets show as often as they are. */
    spots: Plan<Spot>,
}

impl Towns {
    /**
     * Towns along the path, anywhere at least APART, the first near the
     * start: hamlets, villages and market towns. Each lies on the path's
     * coarse band (its mean, x = 0: the path bends to meet it), claimed
     * from spots within half a cell of it, at the mean height of the
     * valley floor under it.
     */
    pub fn new(seed: u64, relief: Rc<Relief>) -> Towns {
        let s = stream(seed, 30);
        let plan = Rc::new(Claims::new("towns", s, CLAIM, APART, Some([0, FIRST]), move |[x, z], random| {
            let mut town = if [x, z] == [0, FIRST] {
                // The first, which the contract's regions look at: two blocks a side, four rows, a fountain, its church east.
                let town = Town::of(Size::Market, FIRST, &mut Rng::new(hash64(s, 0, 0, 0)));
                Town { cols: 2, rows: 4, centre: Centre::Fountain, church: 1, walled: false, ..town }
            } else if x.abs() < CLAIM / 2 {
                let size = match random.below(10) {
                    0..3 => Size::Hamlet,
                    3..7 => Size::Village,
                    _ => Size::Market,
                };
                Town::of(size, z, random)
            } else {
                return None;
            };
            // The floor's mean, not its bumps: a town's height is as coarse as the town.
            let (hx, hz, z) = (town.half_x(), town.half_z(), town.z);
            let around = (-1..=1).flat_map(|i| (-1..=1).map(move |j| (i, j)));
            town.level = Fx::mean(around.map(|(i, j)| relief.ground(i * hx, z + j * hz, Fx::ZERO))).round_units().0;
            Some(([town.x, town.z], town))
        }));
        let spots = {
            let plan = plan.clone();
            Plan::dithered("town spots", SPOT_SCALE, move |x, z| nearest(&plan, z).map_or(Spot::Outside, |t| t.spot(x, z)))
        };
        Towns { plan, spots }
    }

    /** The first town, near the start: every seed has it. */
    pub fn first(&self) -> Town {
        self.near(FIRST).expect("the first town")
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
        ease_towards(route, Units(t.x).to_fx(), Units::m(60.0), Units((z - t.z).abs() - t.half_z() - t.ring()))
    }

    /** The town's bid for column (x, z): ahead inside its streets, behind a few metres out. */
    pub fn bid(&self, x: i64, z: i64) -> Fx {
        let most = fx(4.0);
        self.near(z).map_or(-most, |t| ((Units::m(4.0) - t.distance(x, z)).to_fx() / Units::m(8.0)).clamp(-most, most))
    }

    /** `ground`, with its fraction, levelled to the town's streets, all the way under it and easing back over TERRACE. */
    pub fn terraced(&self, x: i64, z: i64, ground: Fx) -> Fx {
        let Some(t) = self.near(z) else { return ground };
        ease_towards(ground, Units(t.level).to_fx(), TERRACE, t.distance(x, z))
    }

    /** How far column (x, z) is from the nearest town (negative inside); far if none. */
    pub fn distance(&self, x: i64, z: i64) -> Units {
        self.near(z).map_or(Units(i64::MAX / 4), |t| t.distance(x, z))
    }
}

/** The town's layers: its streets' surface, and the towns, both only where it owns the land (domain `me` of `land`). */
pub fn layers(towns: Rc<Towns>, land: Rc<Land>, me: usize, ground: Rc<Sloped>) -> Vec<Layer> {
    let surface = {
        let (towns, surface_ground) = (towns.clone(), ground.clone());
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
                    let h = surface_ground.at(x, z);
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
            about: Some(About { of: ground.clone(), below: m(0.5), above: 2, under: None }),
        }
    };
    let built = Population {
        name: "towns",
        order: 1.5,
        cell: m(256.0),
        // The largest town's blocks, wall and what overhangs them, past its cell's reach.
        reach: [m(150.0), m(45.0), m(150.0)],
        size: ANY_SIZE,
        anchor: {
            let towns = towns.clone();
            Anchors::Lines {
                axis: 1,
                anchor: Box::new(move |line| towns.near(line.centre[1]).map(|t| [t.x as f64, t.level as f64, t.z as f64])),
            }
        },
        make: Box::new(move |_, cell| towns.near(cell.centre[2]).map(town)),
        far: None,
        on: None,
    };
    vec![Layer::Field(surface), Layer::Population(built)]
}

// ---------------------------------------------------------------------------
// Things

fn child(name: String, size: i64, node: Node) -> Child {
    Child { name, size, node }
}

/** Children of `nodes` whose shapes reach `units`. */
fn reaching(units: &Box3, nodes: impl IntoIterator<Item = (String, Node)>, children: &mut Vec<Child>) {
    for (name, node) in nodes {
        if node.shapes.get().iter().any(|s| s.bounds.overlaps(units)) {
            children.push(child(name, ANY_SIZE, node));
        }
    }
}

/** A town's node: its blocks, lamps, square's middle and wall, each made only for the boxes it reaches. */
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
            if t.size != Size::Hamlet && !t.avenue {
                for r in (0..t.rows).filter(|r| Some(*r) != t.square_row()) {
                    let lamps = lamps(&t, r);
                    if lamps.bounds.is_some_and(|b| b.overlaps(units)) {
                        // At every level: too small to see from afar, they still light the street.
                        children.push(child(format!("lamps {r}"), ANY_SIZE, lamps));
                    }
                }
            }
            let centre = match t.centre {
                Centre::Fountain => ("fountain", fountain(&t)),
                Centre::Hall => ("market hall", hall(&t)),
                // A hamlet's at the side of its main street; a village's on its square, on the green if any.
                Centre::Well if t.size == Size::Hamlet => ("well", well(&t, [t.x - MAIN / 2 + m(2.0), t.z])),
                Centre::Well if t.green => ("well", well(&t, [t.x - t.church * (MAIN / 2 + BLOCK_W / 2), t.square_z()])),
                Centre::Well => ("well", well(&t, [t.x, t.square_z()])),
            };
            reaching(units, [(centre.0.to_string(), centre.1)], &mut children);
            if t.walled {
                reaching(units, town_wall(&t), &mut children);
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
fn tree(t: &Town, frame: &Frame, u: i64, w: i64, species: Species, random: &mut Rng) -> Node {
    let [x, y, z] = frame.site([u, -1, w]);
    let level = t.level;
    make_tree(species, x as f64, y as f64, z as f64, &mut Rng::new(random.next_u64()), &|_, _| level)
}

/** An oak if `oak`, else a birch. */
fn broadleaf(oak: bool) -> Species {
    if oak { Species::Oak } else { Species::Birch }
}

fn block(t: &Town, b: &Block, bounds: Box3) -> Node {
    let mut random = Rng::new(hash64(t.seed, b.side, b.c, b.r));
    let frame = block_frame(t, b);
    let mut children = Vec::new();
    let node =
        |children| Node { shapes: Shapes::Now(Vec::new()), bounds: Some(bounds), children: Some(Children::Fixed(children)), far: None };
    if Some(b.r) == t.square_row() && b.c == 0 {
        square(t, b, &frame, &mut random, &mut children);
        return node(children);
    }
    let hamlet = t.size == Size::Hamlet;
    // Shops wall to wall on a market town's main street, and all through a southern one; elsewhere houses apart, in front gardens.
    let terraced = t.size == Size::Market && (b.c == 0 || t.style == Style::Southern);
    let gardens = !terraced && (t.size != Size::Market || random.chance(Fx::HALF));
    // Two rows of lots back to back, each facing its street, a hamlet's only to its main street; along the block, lots 7 to 12 m wide.
    for back_row in [false, true] {
        let mut w = PAVEMENT;
        let mut n = 0;
        while w < BLOCK_D - PAVEMENT {
            let (least, more) = if hamlet { (m(10.0), m(6.0)) } else { (m(7.0), m(5.0)) };
            let mut width = least + random.below(more as u32) as i64;
            if BLOCK_D - PAVEMENT - (w + width) < m(5.0) {
                width = BLOCK_D - PAVEMENT - w;
            }
            let gap = if terraced {
                0
            } else if hamlet {
                m(1.5) + random.below(m(2.0) as u32) as i64
            } else {
                m(1.0) + random.below(m(1.0) as u32) as i64
            };
            let setback = if gardens {
                m(3.0) + random.below(m(2.0) as u32) as i64
            } else if terraced {
                0
            } else {
                random.below(m(2.0) as u32) as i64
            };
            let front = PAVEMENT + setback;
            let depth = (m(8.0) + random.below(m(3.0) as u32) as i64).min(BLOCK_W / 2 - m(1.0) - front);
            let name = |what: &str| format!("{what} {} {n}", u8::from(back_row));
            if hamlet && back_row {
                // Behind a hamlet's houses: a barn now and then, or a tree.
                if random.chance(fx(0.45)) {
                    let at = Frame::new([BLOCK_W / 2 + m(2.0), 0, w + gap], [W, V, U]);
                    let length = (width - 2 * gap).min(m(11.0));
                    let mut shapes = Vec::new();
                    barn(t.style, length, m(7.0), &mut Part::new(frame.then(at), &mut shapes), &mut random);
                    children.push(child(name("barn"), ANY_SIZE, thing(shapes)));
                } else if random.chance(fx(0.5)) {
                    let oak = random.chance(fx(0.6));
                    children.push(child(
                        name("tree"),
                        ANY_SIZE,
                        tree(t, &frame, BLOCK_W - m(8.0), w + width / 2, broadleaf(oak), &mut random),
                    ));
                }
                w += width;
                n += 1;
                continue;
            }
            // The house's own frame: u along the street, w back from its front.
            let at = if back_row {
                Frame::new([BLOCK_W - 1 - front, 0, w + gap], [W, V, back(U)])
            } else {
                Frame::new([front, 0, w + gap], [W, V, U])
            };
            let base = if t.size == Size::Market && b.c == 0 { 2 } else { 1 };
            let storeys = (base + random.below(2) as i64 + t.style.palette().storeys).min(3);
            let mut house = House::new(width - 2 * gap, depth, storeys, t.style);
            house.shop = t.size == Size::Market && b.c == 0 && !back_row;
            house.thatched = hamlet && matches!(t.style, Style::Northern | Style::Timbered) && random.chance(fx(0.6));
            let mut shapes = Vec::new();
            let door = house.build(&mut Part::new(frame.then(at), &mut shapes), &mut random);
            children.push(child(name("house"), ANY_SIZE, thing(shapes)));
            if gardens {
                let mut shapes = Vec::new();
                garden(t.style, &mut Part::new(frame.then(at), &mut shapes), [-gap, width - gap], setback, door, &mut random);
                children.push(child(name("garden"), ANY_SIZE, Node::of(shapes)));
            }
            // A tree in the yard behind, now and then.
            let yard = BLOCK_W / 2 - (front + depth);
            if yard >= m(4.0) && random.chance(fx(0.3)) {
                let u = BLOCK_W / 2 + if back_row { yard / 2 } else { -yard / 2 };
                let oak = !random.chance(fx(0.6));
                children.push(child(name("tree"), ANY_SIZE, tree(t, &frame, u, w + width / 2, broadleaf(oak), &mut random)));
            }
            w += width;
            n += 1;
        }
    }
    if t.avenue && b.c == 0 {
        // On the main street's pavement, every 8 m.
        let species = if matches!(t.style, Style::Northern | Style::Timbered) { Species::Birch } else { Species::Oak };
        for (i, w) in (0..5).map(|i| m(4.0) + i * m(8.0)).enumerate() {
            children.push(child(format!("avenue {i}"), ANY_SIZE, tree(t, &frame, m(1.25), w, species, &mut random)));
        }
    }
    node(children)
}

/** The square's side of block `b` of it: the church, or trees at its corners, a market's stalls between them. */
fn square(t: &Town, b: &Block, frame: &Frame, random: &mut Rng, children: &mut Vec<Child>) {
    if b.side == t.church {
        let mut shapes = Vec::new();
        church(&mut Part::new(frame.then(Frame::at([m(4.0), 0, BLOCK_D / 2 - m(6.0)])), &mut shapes), t.style);
        children.push(child("church".into(), ANY_SIZE, thing(shapes)));
        if t.green {
            // On a green, its churchyard: a low wall open to the main street, graves in rows, spruces at its gate.
            let mut shapes = Vec::new();
            let mut yard = Part::new(*frame, &mut shapes);
            let stone = k(K::churchStone);
            let (a, b) = (m(1.0), [BLOCK_W - m(1.0), BLOCK_D - m(1.0)]);
            yard.add([a, 0, a], [b[0], 7, a + 2], stone);
            yard.add([a, 0, b[1] - 2], [b[0], 7, b[1]], stone);
            yard.add([b[0] - 2, 0, a], [b[0], 7, b[1]], stone);
            yard.add([a, 0, a], [a + 2, 7, BLOCK_D / 2 - m(2.5)], stone);
            yard.add([a, 0, BLOCK_D / 2 + m(2.5)], [a + 2, 7, b[1]], stone);
            for w in [m(4.0), m(6.5), m(9.0), BLOCK_D - m(10.0), BLOCK_D - m(7.5), BLOCK_D - m(5.0)] {
                let mut u = m(12.0);
                while u < BLOCK_W - m(4.0) {
                    if random.chance(fx(0.6)) {
                        yard.add([u, 0, w], [u + 5, 7, w + 2], stone);
                    }
                    u += m(2.0);
                }
            }
            children.push(child("churchyard".into(), ANY_SIZE, Node::of(shapes)));
            for (i, w) in [BLOCK_D / 2 - m(5.0), BLOCK_D / 2 + m(5.0)].into_iter().enumerate() {
                children.push(child(format!("spruce {i}"), ANY_SIZE, tree(t, frame, m(4.0), w, Species::Spruce, random)));
            }
        }
        return;
    }
    // Trees at the square's corners, the forest's own; round a green, down its sides too.
    let mut spots = vec![(m(6.0), m(6.0)), (m(6.0), BLOCK_D - m(6.0)), (BLOCK_W - m(8.0), m(6.0)), (BLOCK_W - m(8.0), BLOCK_D - m(6.0))];
    if t.green {
        spots.extend([(BLOCK_W - m(6.0), BLOCK_D / 2), (m(18.0), m(5.0)), (m(18.0), BLOCK_D - m(5.0))]);
    }
    for (i, (u, w)) in spots.into_iter().enumerate() {
        let oak = random.chance(Fx::HALF);
        children.push(child(format!("tree {i}"), ANY_SIZE, tree(t, frame, u, w, broadleaf(oak), random)));
    }
    if t.size == Size::Market {
        for (i, (u, w)) in [m(10.0), m(19.0)].into_iter().flat_map(|u| [m(12.0), m(19.0), m(26.0)].map(|w| (u, w))).enumerate() {
            if random.chance(fx(0.8)) {
                let mut shapes = Vec::new();
                stall(&mut Part::new(frame.then(Frame::at([u, 0, w])), &mut shapes), random);
                children.push(child(format!("stall {i}"), ANY_SIZE, thing(shapes)));
            }
        }
    }
}

/** A thing of `shapes`, its far form measured from them (see "Levels" in docs/generators.md). */
pub fn thing(shapes: Vec<Shape>) -> Node {
    Node { far: Some(Far::measured(fx(2.0))), ..Node::of(shapes) }
}

/**
 * A house in its own frame: u along its front, v up from its floor, w back
 * from its front, built in `style`; a shop opens its ground floor, a
 * thatched one has a roof of thatch.
 */
pub struct House {
    pub length: i64,
    pub depth: i64,
    pub storeys: i64,
    pub style: Style,
    pub shop: bool,
    pub thatched: bool,
}

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

/** A gable roof's height over its eaves' line at `e` in from its walls, for a roof of `rise` over `run`, eaves `eaves` out. */
fn roof_at(e: i64, rise: f64, run: f64, eaves: i64) -> f64 {
    rise * (e + eaves) as f64 / run
}

/**
 * The ends of a gable over [min, max), its ridge along `ridge`, a unit
 * thick, painted over the roof drawn before them. A roof is solid down to
 * its eaves, so measured (`thing`) it is roof from above, not the walls
 * under it; its gables are walls only at its ends, where they show.
 */
pub fn gable_ends(part: &mut Part, mut min: [f64; 3], mut max: [f64; 3], ridge: usize, kind: Kind) {
    let (from, to) = (min[ridge], max[ridge]);
    for end in [from, to - 1.0] {
        (min[ridge], max[ridge]) = (end, end + 1.0);
        part.gable(min, max, ridge, kind, Op::Paint);
    }
}

impl House {
    /** A plain house of `style`: no shop, no thatch. */
    pub fn new(length: i64, depth: i64, storeys: i64, style: Style) -> House {
        House { length, depth, storeys, style, shop: false, thatched: false }
    }

    /** Builds it; returns where along its front its door is. */
    pub fn build(&self, part: &mut Part, random: &mut Rng) -> i64 {
        let House { length, depth, storeys, style, .. } = *self;
        let p = style.palette();
        let plaster = k(pick(random, p.walls));
        let roof = k(if self.thatched { K::thatch } else { pick(random, p.roofs) });
        let top = storeys * STOREY;
        part.add([-1, -3, -1], [length + 1, 4, depth + 1], k(K::churchStone));
        part.add([0, 0, 0], [length, top, depth], plaster);
        // Its ridge along the street, eaves to it; or gable-fronted.
        let ridge = if random.chance(fx(p.gables)) { 2 } else { 0 };
        let across = 2 - ridge;
        let (width, eaves) = (if across == 0 { length } else { depth }, if self.thatched { 5 } else { 3 });
        let run = width as f64 / 2.0 + eaves as f64;
        let rise = run * (p.pitch[0] + random.next() * (p.pitch[1] - p.pitch[0]));
        let slots = 1.max((length - m(1.0)) / m(2.4));
        let slot = |j: i64| length * (2 * j + 1) / (2 * slots);
        // Dormers out of its roof's front, if it is tall enough, their fronts `at` behind its wall.
        let at = 3;
        let low = roof_at(at, rise, run, eaves) as i64;
        let room = ridge == 0 && low + 20 < rise as i64;
        let dormers = if room && random.chance(fx(p.dormers)) { 1 + i64::from(length >= m(9.0)) } else { 0 };
        let dormer = |j: i64| length * (2 * j + 1) / (2 * dormers);
        for j in 0..dormers {
            let (u, v) = (dormer(j), top + low + 13);
            part.add([u - 6, top, at], [u + 6, v, depth / 2], plaster);
            part.gable(
                [(u - 7) as f64, v as f64, (at - 1) as f64],
                [(u + 7) as f64, (v + 6) as f64, (depth / 2) as f64],
                2,
                roof,
                Op::Fill,
            );
        }
        // Brick gables to the street rise over the roof in steps.
        if style == Style::Brick && ridge == 2 {
            let steps = 4;
            for s in 0..steps {
                let inset = length * s / (2 * steps);
                let h = top + roof_at(length * (s + 1) / (2 * steps), rise, run, eaves).ceil() as i64 + 2;
                for (w0, w1) in [(0, 3), (depth - 3, depth)] {
                    part.add([inset, top - 2, w0], [length - inset, h, w1], plaster);
                    part.add([inset - 1, h, w0 - 1], [length - inset + 1, h + 1, w1], k(K::churchStone));
                }
            }
        }
        let roofed = |rise: f64, eaves: i64| {
            let (mut min, mut max) = ([0.0, top as f64, 0.0], [length as f64, top as f64 + rise, depth as f64]);
            (min[across], max[across]) = (-eaves as f64, (width + eaves) as f64);
            (min, max)
        };
        let (min, max) = roofed(rise, eaves);
        part.gable(min, max, ridge, roof, Op::Fill);
        let (mut min, mut max) = roofed(rise * width as f64 / (width + 2 * eaves) as f64, 0);
        (min[1], max[1]) = (min[1] - ROOF as f64, max[1] - ROOF as f64);
        gable_ends(part, min, max, ridge, plaster);
        if style == Style::Timbered {
            let floors: Vec<i64> = (0..=storeys).map(|s| s * STOREY).collect();
            timbers(part, length, depth, &floors, slots, Some((ridge, max[1] - min[1])));
        }
        // Eaves boarded along both sides: a thin wedge's edge would leave gaps.
        for (from, to) in [(-eaves, 0), (width, width + eaves)] {
            let (mut min, mut max) = ([0, top - 2, 0], [length, top + 1, depth]);
            (min[across], max[across]) = (from, to);
            part.add(min, max, roof);
        }
        if !(style == Style::Southern && random.chance(Fx::HALF)) {
            let (cu, cw) = (4 + random.below((length - 9).max(1) as u32) as i64, 4 + random.below((depth - 9).max(1) as u32) as i64);
            part.add([cu, top, cw], [cu + 5, top + rise as i64 + m(0.8), cw + 5], k(K::brick));
        }

        // Windows front and back, a door in the front's middle; a shop's open door, wide windows and room on the ground floor.
        let shutters = random.chance(fx(p.shutters)).then(|| pick(random, p.shutter_kinds));
        let shop = self.shop && random.chance(Fx::HALF);
        let balcony = style == Style::Southern && storeys > 1 && random.chance(Fx::HALF);
        let door_at = slot(slots / 2);
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
                    } else if i == 0 && storey == 1 && j == slots / 2 && balcony {
                        // A balcony over the door, a door out onto it.
                        window(&mut wall, u, floor + 1, 4, 18, shutters);
                        wall.add([u - 8, floor - 1, -4], [u + 8, floor + 1, 0], k(K::trim));
                        wall.add([u - 8, floor + 1, -4], [u + 8, floor + 8, -3], k(K::iron));
                        wall.add([u - 8, floor + 1, -3], [u - 7, floor + 8, 0], k(K::iron));
                        wall.add([u + 7, floor + 1, -3], [u + 8, floor + 8, 0], k(K::iron));
                    } else {
                        window(&mut wall, u, floor + m(0.9), 4, 11, shutters);
                    }
                }
            }
        }
        let mut front = part.within(Frame::at([0, 0, at]));
        for j in 0..dormers {
            window(&mut front, dormer(j), top + low + 2, 3, 8, None);
        }
        if shop {
            let room = ([WALL, 1, WALL], [length - WALL, STOREY - 2, depth - WALL]);
            part.cut(room.0, room.1);
            part.paint([room.0[0], 0, room.0[2]], [room.1[0], 1, room.1[2]], k(K::planks));
            // Shelves of goods along the back wall, a counter across the room.
            let shelves = ([room.0[0] + 2, 1, room.1[2] - 4], [room.1[0] - 2, 17, room.1[2]]);
            part.add(shelves.0, shelves.1, k(K::counter));
            part.cells([shelves.0[0], 3, shelves.0[2] - 1], [shelves.1[0], 16, shelves.1[2]], k(K::goods), Fill::Const(0.5), Op::Paint);
            part.add([room.0[0] + 4, 1, depth / 2], [length / 2, 8, depth / 2 + 3], k(K::counter));
        }
        door_at
    }
}

/**
 * A timber frame painted over a house's walls, before its windows: posts
 * at its corners and between its `slots` of windows, a rail at each of
 * `floors` (the first 0, the last its walls' top), braces across its ends'
 * panels; on its gables, given by their ridge (u or w) and how high they
 * rise from ROOF under its top, posts and a tie, a unit inside them.
 */
fn timbers(part: &mut Part, length: i64, depth: i64, floors: &[i64], slots: i64, gable: Option<(usize, f64)>) {
    let beam = k(K::timber);
    let top = floors[floors.len() - 1];
    for (i, (wall, len)) in walls(length, depth).into_iter().enumerate() {
        let mut wall = part.within(wall);
        let front = i % 2 == 0;
        let posts: Vec<i64> = if front { (1..slots).map(|j| len * j / slots).collect() } else { vec![len / 3, len * 2 / 3] };
        for u in [1, len - 1].into_iter().chain(posts.iter().copied()) {
            wall.paint([u - 1, 0, 0], [u + 1, top, 1], beam);
        }
        for v in floors {
            let v = (*v).clamp(1, top - 1);
            wall.paint([0, v - 1, 0], [len, v + 1, 1], beam);
        }
        // Braces across the end walls' outer panels.
        if !front {
            for pair in floors.windows(2) {
                let (v0, v1) = ((pair[0] + 1) as f64, (pair[1] - 1) as f64);
                let (a, b) = (posts[0] as f64, posts[1] as f64);
                wall.capsule([2.0, v0, 0.5], [a, v1, 0.5], 0.9, beam, Op::Paint);
                wall.capsule([(len - 2) as f64, v0, 0.5], [b, v1, 0.5], 0.9, beam, Op::Paint);
            }
        }
        let Some((_, rise)) = gable.filter(|(ridge, _)| (*ridge == 0) != front) else { continue };
        // How high the gable is a unit in from its edge at e from it.
        let half = len as f64 / 2.0;
        let under = |e: i64| top - ROOF + (rise * (e - 1) as f64 / half) as i64 - 1;
        for u in [len / 4, len / 2, len * 3 / 4] {
            let e = (u - 1).min(len - u - 1);
            if under(e) > top + 2 {
                wall.paint([u - 1, top, 0], [u + 1, under(e), 1], beam);
            }
        }
        let tie = top - ROOF + (rise / 2.0) as i64;
        let e = ((tie + 2 - top + ROOF) as f64 * half / rise).ceil() as i64 + 1;
        if tie > top + 2 && e < len / 2 {
            wall.paint([e, tie - 1, 0], [len - e, tie + 1, 1], beam);
        }
    }
}

/** A barn in its own frame (u along its front, w back): plank or painted walls, a big roof, wide doors. */
fn barn(style: Style, length: i64, depth: i64, part: &mut Part, random: &mut Rng) {
    let (walls_of, roof) = match style {
        Style::Southern => (K::plasterOchre, K::roofTile),
        Style::Northern => (K::faluRed, if random.chance(Fx::HALF) { K::thatch } else { K::slate }),
        Style::Timbered => (K::planks, if random.chance(Fx::HALF) { K::thatch } else { K::roofBrown }),
        Style::Brick => (K::brick, K::roofRed),
    };
    let top = m(3.5);
    part.add([0, -3, 0], [length, top, depth], k(walls_of));
    let (eaves, run) = (4, depth as f64 / 2.0 + 4.0);
    let rise = run * 0.9;
    part.gable([0.0, top as f64, -eaves as f64], [length as f64, top as f64 + rise, (depth + eaves) as f64], 0, k(roof), Op::Fill);
    let gable = (top - ROOF) as f64;
    gable_ends(part, [0.0, gable, 0.0], [length as f64, gable + rise * 0.85, depth as f64], 0, k(walls_of));
    for (from, to) in [(-eaves, 0), (depth, depth + eaves)] {
        part.add([0, top - 2, from], [length, top + 1, to], k(roof));
    }
    // Wide doors in the middle of its front.
    let u = length / 2;
    part.paint([u - m(1.6) - 1, 0, 0], [u + m(1.6) + 1, m(2.8) + 1, 1], k(K::timber));
    part.paint([u - m(1.6), 0, 0], [u + m(1.6), m(2.8), 1], k(K::door));
}

/**
 * A front garden in a house's frame, from `lot` along the street and the
 * street `setback` before the house's front: a fence, wall or hedge by the
 * style, open at the path to its `door`, flowers along the house.
 */
fn garden(style: Style, part: &mut Part, lot: [i64; 2], setback: i64, door: i64, random: &mut Rng) {
    let w = -setback;
    for (a, b) in [(lot[0], door - 6), (door + 6, lot[1])].into_iter().filter(|(a, b)| a < b) {
        match style {
            Style::Southern => {
                part.add([a, 0, w], [b, 6, w + 2], k(K::churchStone));
                part.add([a, 6, w], [b, 7, w + 2], k(K::plasterWhite));
            }
            Style::Northern | Style::Timbered => {
                // Pickets as a board, posts every couple of metres.
                let kind = k(if style == Style::Northern { K::trim } else { K::planks });
                part.add([a, 1, w], [b, 7, w + 1], kind);
                let mut u = a;
                while u < b {
                    part.add([u, 0, w], [u + 1, 8, w + 1], kind);
                    u += m(2.0);
                }
            }
            Style::Brick => {
                part.add([a, 0, w], [b, 9, w + 4], forest::k(F::bush));
            }
        }
    }
    let flowers = [F::flowerRed, F::flowerYellow, F::flowerViolet, F::flowerWhite][random.below(4) as usize];
    let ends = [lot[0] + 2, door - 6, door + 6, lot[1] - 2];
    for (a, b) in [(ends[0], ends[1]), (ends[2], ends[3])].into_iter().filter(|(a, b)| a < b) {
        part.cells([a, 0, -4], [b, 2, -1], forest::k(flowers), Fill::Const(0.5), Op::Fill);
    }
}

/** The church, in its own frame: its tower's front on u from 0 to TOWER, a nave behind it, CHURCH wide across w. */
fn church(part: &mut Part, style: Style) {
    const TOWER: i64 = m(5.0);
    const NAVE: i64 = m(12.0);
    let (walls_of, roof, spire, spire_tall) = style.church();
    let (nave_length, nave_top, tower_top) = (BLOCK_W - m(7.0) - TOWER, m(9.0), m(22.0));
    let stone = k(walls_of);
    // The nave: u along it (from the tower), w across.
    let mut nave = part.within(Frame::new([TOWER, 0, 0], [U, V, W]));
    nave.add([0, -3, 0], [nave_length, nave_top, NAVE], stone);
    let rise = m(5.5) as f64;
    nave.gable([0.0, nave_top as f64, -3.0], [nave_length as f64, nave_top as f64 + rise, (NAVE + 3) as f64], 0, k(roof), Op::Fill);
    let gable = (nave_top - ROOF) as f64;
    gable_ends(&mut nave, [0.0, gable, 0.0], [nave_length as f64, gable + rise * 0.9, NAVE as f64], 0, stone);
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
    // A spire, or in the south a low cap over a wider cornice.
    let (c, r) = ((TOWER / 2) as f64, (TOWER / 2 + 1) as f64);
    if style == Style::Southern {
        tower.add([-2, tower_top, -2], [TOWER + 2, tower_top + 2, TOWER + 2], stone);
    }
    tower.lathe([c, tower_top as f64, c], [c, (tower_top + spire_tall) as f64, c], move |t| r * (1.0 - t) + 1.0, k(spire), SOLID, Op::Fill);
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
    // Bounded, so that it is drawn where it reaches.
    let bounds = (!shapes.is_empty()).then(|| bounds_of_shapes(shapes.iter().map(|s| &s.bounds)));
    Node { bounds, ..Node::of(shapes) }
}

/** A fountain in the middle of the square: a basin, a column and a bowl, water in both. */
fn fountain(t: &Town) -> Node {
    let mut shapes = Vec::new();
    let mut f = Part::new(Frame::at([t.x, t.level + 1, t.square_z()]), &mut shapes);
    let (stone, water) = (k(K::churchStone), k(K::water));
    let up = |y: f64| [0.0, y, 0.0];
    f.lathe(up(-1.0), up(5.0), m(3.0) as f64, stone, SOLID, Op::Fill);
    f.lathe(up(3.0), up(5.0), (m(3.0) - 3) as f64, water, SOLID, Op::Paint);
    f.lathe(up(5.0), up(15.0), 3.0, stone, SOLID, Op::Fill);
    f.lathe(up(14.0), up(18.0), |t| 4.0 + 6.0 * t, stone, SOLID, Op::Fill);
    f.lathe(up(17.0), up(18.0), 7.0, water, SOLID, Op::Paint);
    f.ball(up(20.0), 2.0, stone, Op::Fill);
    Node::of(shapes)
}

/** A well at column (x, z): a ring of stone, water in it, two posts and a little roof over a winch. */
fn well(t: &Town, [x, z]: [i64; 2]) -> Node {
    let mut shapes = Vec::new();
    let mut f = Part::new(Frame::at([x, t.level + 1, z]), &mut shapes);
    let up = |y: i64| [0.0, y as f64, 0.0];
    let roof = k(t.style.palette().roofs[0]);
    f.lathe(up(-1), up(m(0.9)), m(1.1) as f64, k(K::churchStone), SOLID, Op::Fill);
    f.lathe(up(m(0.4)), up(m(0.9) + 1), m(0.8) as f64, 0, SOLID, Op::Carve);
    f.lathe(up(m(0.4) - 1), up(m(0.4)), m(0.8) as f64, k(K::water), SOLID, Op::Paint);
    for u in [-m(1.0) - 1, m(1.0) - 1] {
        f.add([u, 0, -1], [u + 2, m(2.3), 1], k(K::timber));
    }
    f.add([-m(1.0), m(1.8), -1], [m(1.0), m(1.8) + 2, 1], k(K::timber));
    f.gable([-m(1.4) as f64, m(2.3) as f64, -m(1.0) as f64], [m(1.4) as f64, m(3.1) as f64, m(1.0) as f64], 0, roof, Op::Fill);
    Node::of(shapes)
}

/**
 * A market hall over the main street in the middle of the square: open
 * below, on posts, a hall above in the town's walls, a steep roof and a
 * bell turret on its ridge.
 */
fn hall(t: &Town) -> Node {
    const WIDE: i64 = m(9.0);
    const LONG: i64 = m(18.0);
    const FLOOR: i64 = m(3.5);
    const TOP: i64 = m(6.5);
    let mut shapes = Vec::new();
    let mut h = Part::new(Frame::at([t.x - WIDE / 2, t.level + 1, t.square_z() - LONG / 2]), &mut shapes);
    let p = t.style.palette();
    let (walls_of, roof) = (k(p.walls[0]), k(p.roofs[0]));
    let posts = if t.style == Style::Brick { k(K::brick) } else { k(K::timber) };
    // Posts down its sides and across its ends, every 3 m.
    for w in (0..=6).map(|i| (LONG - 4) * i / 6) {
        for u in [0, WIDE - 4] {
            h.add([u, -3, w], [u + 4, FLOOR, w + 4], posts);
        }
    }
    for u in [WIDE / 3 - 2, WIDE * 2 / 3 - 2] {
        for w in [0, LONG - 4] {
            h.add([u, -3, w], [u + 4, FLOOR, w + 4], posts);
        }
    }
    h.add([-2, FLOOR, -2], [WIDE + 2, FLOOR + 2, LONG + 2], k(K::timber));
    h.add([0, FLOOR + 2, 0], [WIDE, TOP, LONG], walls_of);
    if t.style == Style::Timbered {
        let mut upper = h.within(Frame::at([0, FLOOR + 2, 0]));
        timbers(&mut upper, WIDE, LONG, &[0, TOP - FLOOR - 2], (WIDE - m(1.0)) / m(2.4), None);
    }
    // Windows down its long sides.
    for (wall, length) in walls(WIDE, LONG).into_iter().skip(1).step_by(2) {
        let mut wall = h.within(wall.then(Frame::at([0, FLOOR + 2, 0])));
        for i in 0..6 {
            window(&mut wall, length * (2 * i + 1) / 12, m(0.8), 4, 12, None);
        }
    }
    let (run, eaves) = ((WIDE / 2 + 4) as f64, 4.0);
    let rise = run * 1.1;
    h.gable([-eaves, TOP as f64, 0.0], [WIDE as f64 + eaves, TOP as f64 + rise, LONG as f64], 2, roof, Op::Fill);
    let gable = (TOP - ROOF) as f64;
    gable_ends(&mut h, [0.0, gable, 0.0], [WIDE as f64, gable + rise * 0.9, LONG as f64], 2, walls_of);
    // The turret.
    let (c, top) = ((WIDE / 2) as f64, TOP + rise as i64);
    h.add([WIDE / 2 - 8, top - m(1.5), LONG / 2 - 8], [WIDE / 2 + 8, top + m(1.8), LONG / 2 + 8], k(K::timber));
    h.cut([WIDE / 2 - 8, top + 3, LONG / 2 - 5], [WIDE / 2 + 8, top + m(1.4), LONG / 2 + 5]);
    h.cut([WIDE / 2 - 5, top + 3, LONG / 2 - 8], [WIDE / 2 + 5, top + m(1.4), LONG / 2 + 8]);
    h.lathe(
        [c, (top + m(1.8)) as f64, (LONG / 2) as f64],
        [c, (top + m(4.5)) as f64, (LONG / 2) as f64],
        |t| 11.0 * (1.0 - t) + 1.0,
        roof,
        SOLID,
        Op::Fill,
    );
    thing(shapes)
}

/** A market stall in its own frame: a counter of goods, an awning on four posts. */
fn stall(part: &mut Part, random: &mut Rng) {
    let (long, deep) = (m(3.0), m(1.4));
    part.add([0, 0, 0], [long, m(0.9), deep], k(K::planks));
    part.cells([1, m(0.9), 1], [long - 1, m(0.9) + 3, deep - 1], k(K::goods), Fill::Const(0.6), Op::Fill);
    for (u, w) in [(0, -m(0.6)), (long - 1, -m(0.6)), (0, deep - 1), (long - 1, deep - 1)] {
        part.add([u, 0, w], [u + 1, m(2.2), w + 1], k(K::timber));
    }
    let cloth = [K::roofRed, K::shutterGreen, K::trim, K::shutterBlue, K::plasterOchre][random.below(5) as usize];
    part.gable([-2.0, m(2.2) as f64, -m(0.9) as f64], [(long + 2) as f64, m(2.7) as f64, (deep + 2) as f64], 0, k(cloth), Op::Fill);
}

/**
 * A walled town's wall round its blocks, RING out: the town's stone,
 * battlements along its top, round towers at its corners, and gate towers
 * where the main street goes through, each a thing of its own.
 */
fn town_wall(t: &Town) -> Vec<(String, Node)> {
    const PIECE: i64 = m(24.0);
    const TOP: i64 = m(5.0);
    const GATE: i64 = m(8.0);
    let (ox, oz) = (t.half_x() + RING, t.half_z() + RING);
    let stone = k(match t.style {
        Style::Brick => K::brick,
        Style::Northern => K::plasterGrey,
        _ => K::churchStone,
    });
    let roof = k(t.style.palette().roofs[0]);
    let foot = Frame::at([t.x - ox, t.level + 1, t.z - oz]);
    let mut pieces = Vec::new();
    for (side, (frame, length)) in walls(2 * ox, 2 * oz).into_iter().enumerate() {
        let frame = foot.then(frame);
        // Sides 0 and 2 run across the main street, a gate in their middle.
        let gate = side % 2 == 0;
        let spans: Vec<[i64; 2]> = if gate { vec![[0, length / 2 - GATE], [length / 2 + GATE, length]] } else { vec![[0, length]] };
        for [from, to] in spans {
            let mut u = from;
            while u < to {
                let end = (u + PIECE).min(to);
                let mut shapes = Vec::new();
                let mut part = Part::new(frame, &mut shapes);
                part.add([u, -m(1.5), 0], [end, TOP, TOWN_WALL], stone);
                // Merlons every other metre along its outer edge.
                let mut merlon = u + (m(2.0) - u.rem_euclid(m(2.0))) % m(2.0);
                while merlon < end {
                    part.add([merlon, TOP, 0], [(merlon + m(1.0)).min(end), TOP + m(1.0), m(0.5)], stone);
                    merlon += m(2.0);
                }
                pieces.push((format!("wall {side} {u}"), thing(shapes)));
                u = end;
            }
        }
        if gate {
            let mut shapes = Vec::new();
            let mut part = Part::new(frame.then(Frame::at([length / 2, 0, 0])), &mut shapes);
            let (deep, tall) = (m(6.0), m(11.0));
            let w0 = (TOWN_WALL - deep) / 2;
            part.add([-GATE, -m(1.5), w0], [GATE, tall, w0 + deep], stone);
            part.cut([-m(3.0), 0, w0], [m(3.0), m(4.0), w0 + deep]);
            part.ellipsoid(
                [0.0, m(4.0) as f64, (w0 + deep / 2) as f64],
                [m(3.0) as f64, m(1.5) as f64, (deep / 2 + 1) as f64],
                0,
                Op::Carve,
            );
            for v in [m(6.0), m(8.5)] {
                for u in [-m(4.5), -m(1.0), m(2.5)] {
                    part.cut([u, v, w0], [u + m(1.0), v + m(1.4), w0 + 2]);
                }
            }
            let run = (deep / 2 + 4) as f64;
            part.gable(
                [-(GATE + 4) as f64, tall as f64, (w0 - 4) as f64],
                [(GATE + 4) as f64, tall as f64 + run, (w0 + deep + 4) as f64],
                0,
                roof,
                Op::Fill,
            );
            pieces.push((format!("gate {side}"), thing(shapes)));
        }
        // A round tower at its first corner, roofed with a cone.
        let mut shapes = Vec::new();
        let mut part = Part::new(frame, &mut shapes);
        let (c, r, tall) = ((TOWN_WALL / 2) as f64, m(2.75) as f64, m(8.0) as f64);
        part.lathe([c, -m(1.5) as f64, c], [c, tall, c], r, stone, SOLID, Op::Fill);
        part.lathe([c, tall, c], [c, tall + m(4.0) as f64, c], move |t| (r + 3.0) * (1.0 - t) + 1.0, roof, SOLID, Op::Fill);
        pieces.push((format!("tower {side}"), thing(shapes)));
    }
    pieces
}
