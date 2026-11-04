//! The forest: an autumn forest valley, endless, on the generator
//! framework. Its numbers are moving to integers (docs/generators.md).

use std::cell::{OnceCell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use worldgen::World;
use worldgen::column;
use worldgen::generator::{
    ANY_SIZE, Cell, Child, Far, Field, Fill, Generator, Layer, Node, Op, Point, Population, SOLID, Shadows, Shape, Span, ball,
    bounds_of_shapes, capsule, ellipsoid, lathe,
};
use worldgen::grid::Box3;
use worldgen::js::{self, round};
use worldgen::kinds;
use worldgen::noise::{Fbm, Fx, Rng, ValueNoise, fx, hash, hash64, smoothstep, stream, unit};
use worldgen::plan::Plan;
use worldgen::units::{UNIT_SIZE, Units, m};

/** Metres in units, rounded. */
fn vox(metres: f64) -> f64 {
    round(metres / UNIT_SIZE)
}
const BRANCH_RADIUS: f64 = 1.2;
fn log_radius() -> f64 {
    vox(0.45)
}
const FINE: f64 = 1.0;
const PI: f64 = std::f64::consts::PI;

/** How far around the camera the grid of level 0 must hold the forest, in units. */
pub const REACH: f64 = 352.0;
/** The heights it has anything at, in units. */
pub const HEIGHTS: (i64, i64) = (m(-36.0), m(80.0));

// A kind's materials follow it, in this order. The forest's are first in any world it grows in.
kinds! {
    base: 0;
    moss: 1, ["#4f7a2a", "#5a8a30", "#466e26", "#628f37"];
    grass: 1, ["#74a83c", "#7fb544", "#6a9c36"];
    tallGrass: 1, ["#8fbf4d", "#9cc95a", "#a3b84a"];
    litter: 2, ["#8a5a2b", "#9c6b30", "#7a4e24", "#a8763a"];
    needles: 2, ["#6b4a2e", "#5e4029", "#76533a"];
    path: 2, ["#a08262", "#957656", "#ab8c6b"];
    soil: 2, ["#4a3423", "#3f2c1e", "#54392a"];
    stone: 3, ["#7d7d78", "#6e6e6a", "#8a8a84"];
    mossyStone: 3, ["#5f7348", "#687c4f", "#56693f"];
    bark: 4, ["#4b3621", "#3f2d1c", "#57402a"];
    spruceBark: 4, ["#4a3426", "#5a4030"];
    birchBark: 4, ["#e6e2d6", "#dcd7c9", "#efece4"];
    birchMark: 4, ["#2e2b28", "#3b3733"];
    deadwood: 4, ["#6a5640", "#5c4a36", "#776149"];
    spruce: 5, ["#1f4a2c", "#24553a", "#1a3f26", "#2b5e3c"];
    oakGreen: 5, ["#4c7f2a", "#5a8f30", "#447324", "#679a38"];
    oakOrange: 5, ["#c46a1c", "#d68526", "#b0521a", "#9c4a1c"];
    oakGold: 5, ["#d9a521", "#c8941c", "#e3b53a", "#b88a1e"];
    birchGreen: 5, ["#7fae3c", "#8cbc45", "#9ac24f"];
    birchGold: 5, ["#d8cf4a", "#e6d65a", "#c9be36", "#efe07a"];
    fern: 5, ["#3f7a2c", "#4a8a34", "#56953a"];
    bracken: 5, ["#a8642a", "#b97a35", "#8f5524"];
    bush: 5, ["#3d6b28", "#477a2e", "#355f22"];
    flowerYellow: 6, ["#e8d34a", "#f0dc58"];
    flowerViolet: 6, ["#c9b1ff", "#b89cf0"];
    flowerWhite: 6, ["#f2f2f2", "#e8e8e0"];
    flowerRed: 6, ["#e86a6a", "#d95858"];
    barkFurrow: 4, ["#2e2115", "#352717", "#281c12"];
    spruceScale: 4, ["#33241a", "#2b1e15", "#3d2b1f"];
    twig: 4, ["#5a4430", "#4e3a28", "#634b35"];
    hay: 1, ["#b9a15a", "#c8b06a", "#a89050"];
    fallenLeaf: 2, ["#c4581c", "#d9861f", "#b8401a", "#e0b030", "#9a3a16", "#8c5a22"];
    cone: 4, ["#6b4423", "#7a5030", "#5c3a1e"];
    amanita: 6, ["#c8281e", "#d63a2a", "#b52018"];
    amanitaSpot: 6, ["#f4efe6", "#ebe4d6"];
    mushroomStem: 6, ["#e9e2d0", "#ddd4bf"];
    bolete: 6, ["#7a4a26", "#8a5530", "#6c4020"];
    berry: 6, ["#c01f2a", "#a81824", "#d43a30"];
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Species {
    Spruce,
    Birch,
    Oak,
}
const SPECIES: [Species; 3] = [Species::Spruce, Species::Birch, Species::Oak];

pub type GroundAt<'a> = &'a dyn Fn(i64, i64) -> i64;

pub fn make_tree(species: Species, x: f64, ground: f64, z: f64, random: &mut Rng, ground_at: GroundAt) -> Node {
    match species {
        Species::Spruce => spruce_tree(x, ground, z, random, ground_at),
        Species::Birch => birch_tree(x, ground, z, random),
        Species::Oak => oak_tree(x, ground, z, random, ground_at),
    }
}

/** The colour of each species' crowns from far away. */
fn foliage(species: Species) -> K {
    match species {
        Species::Spruce => K::spruce,
        Species::Birch => K::birchGold,
        Species::Oak => K::oakOrange,
    }
}

#[derive(Clone, Copy, Default)]
struct Column {
    height: i64,
    surface: K,
    /** How much the trees around are conifers, and how dense they are: fractions. */
    conifer: Fx,
    density: Fx,
    on_path: bool,
}

/** Plan scales (see "Bands" in docs/generators.md): the path's bends, the ground's bumps, a column. */
pub const PATH_SCALE: i64 = 256;
pub const GROUND_SCALE: i64 = 8;
const COLUMN_SCALE: i64 = 1;

/** The valley's relief: hills kilometres wide, and a winding path along z that the ground dips along. */
pub struct Relief {
    phase: f64,
    hills: Fbm<360, 4>,
    bumps: ValueNoise<48>,
    far: Fbm<7200, 4>,
}

impl Relief {
    pub fn new(seed: u64) -> Relief {
        Relief {
            phase: unit(hash(stream(seed, 1), 0, 0, 0)) * 100.0,
            hills: Fbm::new(stream(seed, 2)),
            bumps: ValueNoise::new(stream(seed, 3)),
            far: Fbm::new(stream(seed, 9)),
        }
    }

    /** The path's centre line at z, with its fraction. */
    pub fn path_x(&self, z: i64) -> Fx {
        let zm = z as f64 * UNIT_SIZE;
        Fx::m(9.0 * js::sin(zm / 31.0 + self.phase) + 5.0 * js::sin(zm / 13.0 + self.phase * 2.0))
    }

    /** Ground height at (x, z), `from_path` from the path's centre line; with its fraction. */
    pub fn ground(&self, x: i64, z: i64, from_path: Fx) -> Fx {
        // The bumps' product truncated towards zero, not floored, as it always was.
        let bumps = self.bumps.at(x, z) - Fx::HALF;
        let hills = (self.hills.at(x, z) - Fx::HALF) * Units::m(10.0) + bumps.abs() * Fx::m(0.8) * bumps.signum();
        let valley = smoothstep(Fx::m(8.0), Fx::m(42.0), from_path) * Units::m(18.0);
        let far = (self.far.at(x, z) - Fx::HALF) * Units::m(50.0);
        let path_dip = (Fx::ONE - smoothstep(Fx::m(1.0), Fx::m(2.2), from_path)) * Fx::m(0.3);
        hills + valley + far - path_dip
    }
}

/**
 * What the forest grows on, decided outside it when it shares its land:
 * the ground's height, the path's centre line (by z, at x = 0), and its
 * share of each column (a fraction), which thins whatever it grows.
 */
pub struct Ground {
    pub ground: Rc<Plan<i64>>,
    pub path: Rc<Plan<Fx>>,
    pub share: Rc<dyn Fn(i64, i64) -> Fx>,
}

impl Ground {
    /** The valley alone: the relief along its own path, all of it the forest's. */
    pub fn alone(relief: Rc<Relief>) -> Ground {
        let path = {
            let relief = relief.clone();
            Rc::new(Plan::new("path", PATH_SCALE, move |_, z| relief.path_x(z)))
        };
        let ground = {
            let path = path.clone();
            Rc::new(Plan::new("ground", GROUND_SCALE, move |x, z| relief.ground(x, z, path_distance(&path, x, z)).round_units().0))
        };
        Ground { ground, path, share: Rc::new(|_, _| Fx::ONE) }
    }

    /** On the path at z, `above` units over the ground, on a site. */
    pub fn on_path(&self, z: i64, above: i64) -> [i64; 3] {
        let x = self.path.at(0, z).round_units().0;
        let y = self.ground.at(x, z) + above;
        [x, y + ((x + y + z) & 1), z]
    }
}

/** The forest's state shared by its layers: what it knows about any column, from the seed and its ground alone. */
struct Forest {
    seed: u64,
    spawn: [i64; 3],
    ground: Ground,
    columns: Plan<Column>,
}

/** The forest's own noise, by what it decides. */
struct Noise {
    conifer: Fbm<320, 4>,
    density: Fbm<176, 4>,
    path_edge: ValueNoise<24>,
    floor: ValueNoise<20>,
}

/** How far column (x, z) is from the path's centre line, with its fraction. */
pub fn path_distance(path: &Plan<Fx>, x: i64, z: i64) -> Fx {
    (Units(x).to_fx() - path.at(0, z)).abs()
}

/** Column (x, z), given the ground's height there and at its four neighbours. */
fn column_of(n: &Noise, path: &Plan<Fx>, x: i64, z: i64, h: i64, neighbours: [i64; 4]) -> Column {
    let slope = neighbours.map(|n| (n - h).abs()).into_iter().max().expect("four");
    let conifer = smoothstep(fx(0.4), fx(0.6), n.conifer.at(x, z)) * 3 / 4 + smoothstep(Fx::m(6.0), Fx::m(16.0), Units(h).to_fx()) / 4;
    let density = smoothstep(fx(0.3), fx(0.48), n.density.at(x, z));
    let on_path = path_distance(path, x, z) < Fx::m(1.1) + n.path_edge.at(z, 0) * Units::m(0.5);
    let floor = n.floor.at(x, z);
    let surface = if on_path {
        K::path
    } else if slope >= 3 {
        if floor < fx(0.5) { K::stone } else { K::mossyStone }
    } else if density < fx(0.2) {
        if floor < fx(0.75) { K::grass } else { K::moss }
    } else if conifer > fx(0.5) {
        if floor < fx(0.55) { K::needles } else { K::moss }
    } else if floor < fx(0.6) {
        K::litter
    } else {
        K::moss
    };
    Column { height: h, surface, conifer, density, on_path }
}

impl Forest {
    fn s(&self, k: u64) -> u64 {
        stream(self.seed, k)
    }

    fn path_distance(&self, x: i64, z: i64) -> Fx {
        path_distance(&self.ground.path, x, z)
    }

    /** Ground height in units. */
    fn ground(&self, x: i64, z: i64) -> i64 {
        self.ground.ground.at(x, z)
    }

    fn column(&self, x: i64, z: i64) -> Column {
        self.columns.at(x, z)
    }

    /** `chance` times the forest's share of column (x, z). */
    fn thinned(&self, chance: Fx, x: i64, z: i64) -> Fx {
        chance * (self.ground.share)(x, z)
    }

    /** A random spot on the ground under the square a cell reaches over (2N wide). */
    fn spot(&self, cell: &Cell) -> Point {
        // The same for every cell over a column: the one the ground is in owns it.
        cell.on_ground_by(&mut Rng::new(hash64(self.s(19), cell.n, cell.i, cell.k)), |x, z| self.ground(x, z))
    }

    /** The chance a cell of trees around column (x, z) has one. */
    fn tree_chance(&self, x: i64, z: i64, c: &Column) -> Fx {
        if c.surface == K::stone || self.path_distance(x, z) < Fx::m(2.8) {
            return Fx::ZERO;
        }
        // Keep the view from the spawn point clear of low branches.
        let (dx, dz) = (x - self.spawn[0], z - self.spawn[2]);
        if dx * dx + dz * dz < (vox(7.0) as i64).pow(2) {
            return Fx::ZERO;
        }
        self.thinned(fx(0.15) + c.density * 3 / 4, x, z)
    }

    /** The share of each species, fractions that add up to one. */
    fn species_chances(c: &Column) -> [Fx; 3] {
        let broad = Fx::ONE - c.conifer;
        [c.conifer, broad * 45 / 100, broad - broad * 45 / 100]
    }
}

/** The species `pick` (a fraction) falls on, the chances laid end to end. */
fn pick_species(mut pick: Fx, chances: &[Fx; 3]) -> Species {
    for (i, species) in SPECIES.iter().enumerate() {
        pick -= chances[i];
        if pick < Fx::ZERO {
            return *species;
        }
    }
    Species::Oak
}

/** The forest of `seed`. */
pub fn forest(seed: u64) -> World {
    let ground = Ground::alone(Rc::new(Relief::new(seed)));
    // Start on the path at the near end, at about the height of a tall shrub.
    let spawn = ground.on_path(vox(-31.0) as i64, vox(3.0) as i64);
    let generator = Generator { kinds: KINDS.to_vec(), seed: stream(seed, 4), layers: layers(seed, ground, spawn) };
    World { generator, spawn, reach: REACH, heights: HEIGHTS }
}

/** The forest's layers on `ground`, kept clear around `spawn`; their kinds are KINDS, first in the generator's. */
pub fn layers(seed: u64, ground: Ground, spawn: [i64; 3]) -> Vec<Layer> {
    let noise = Noise {
        conifer: Fbm::new(stream(seed, 5)),
        density: Fbm::new(stream(seed, 6)),
        path_edge: ValueNoise::new(stream(seed, 7)),
        floor: ValueNoise::new(stream(seed, 8)),
    };
    let columns = {
        let (heights, path) = (ground.ground.clone(), ground.path.clone());
        Plan::new("forest columns", COLUMN_SCALE, move |x, z| {
            let h = |x, z| heights.at(x, z);
            column_of(&noise, &path, x, z, h(x, z), [h(x - 1, z), h(x + 1, z), h(x, z - 1), h(x, z + 1)])
        })
    };
    let f = Rc::new(Forest { seed, spawn, ground, columns });

    // The ground: along each column, soil or stone under a layer of its surface.
    let terrain = {
        let f = f.clone();
        Field::columns("terrain", 0.0, Op::Paint, move |x, z, spacing| {
            let h = f.ground(x, z);
            let around = [f.ground(x - spacing, z), f.ground(x + spacing, z), f.ground(x, z - spacing), f.ground(x, z + spacing)];
            let surface = f.column(x, z).surface;
            let below = if surface == K::stone || surface == K::mossyStone { K::stone } else { K::soil };
            // Down to the lowest neighbour, so a slope's sides are solid.
            let bottom = around.into_iter().fold(h - vox(1.5) as i64, i64::min);
            column::Column::new(h, spacing).surface(surface, vox(1.0) as i64).fill_below_exactly(below, bottom)
        })
    };

    // Trees, each in the cell its foot is in.
    let tree_cell = vox(2.0) as i64;
    let tree_top = vox(26.0) as i64;
    // What slabs of each species' trees hide, measured from samples drawn at level 2 (the canopy's from level 4 on), by spacing.
    let sample_seed = f.s(17);
    let shadows = Rc::new(OnceCell::new());
    let slabs = Rc::new(RefCell::new(HashMap::<i64, Rc<[Vec<f64>; 3]>>::new()));
    let slabs_at = move |spacing: i64| {
        slabs
            .borrow_mut()
            .entry(spacing)
            .or_insert_with(|| {
                let shadows: &[Shadows; 3] = shadows.get_or_init(|| {
                    SPECIES.map(|species| {
                        let samples: Vec<Node> = (0..64)
                            .map(|i| make_tree(species, 0.0, 0.0, 0.0, &mut Rng::new(hash64(sample_seed, i, 0, 0)), &|_, _| 0))
                            .collect();
                        Shadows::new(&samples, 2, 1)
                    })
                });
                Rc::new([0, 1, 2].map(|sp| {
                    (0..tree_top).step_by(spacing as usize).map(|from| shadows[sp].slab(from, (from + spacing).min(tree_top))).collect()
                }))
            })
            .clone()
    };
    // A medium as dense as the trees' area per volume over each column.
    let canopy = {
        let f = f.clone();
        Field::columns("canopy", 1.0, Op::Fill, move |x, z, spacing| {
            let c = f.column(x, z);
            let per_area = f.tree_chance(x, z, &c).to_f64() / ((2 * tree_cell) * (2 * tree_cell)) as f64;
            if per_area == 0.0 {
                return Vec::new();
            }
            let chances = Forest::species_chances(&c);
            let kind = k(foliage(pick_species(Fx::fraction(hash(f.s(18), x, z, 0)), &chances)));
            let slabs = slabs_at(spacing);
            let mut spans = Vec::new();
            for (i, from) in (0..tree_top).step_by(spacing as usize).enumerate() {
                let shadow = (0..3).fold(0.0, |sum, sp| sum + chances[sp].to_f64() * slabs[sp][i]);
                if shadow > 0.0 {
                    let (from, to) = (c.height + from, c.height + from + spacing);
                    spans.push(Span { from, to, kind, extinction: Some(per_area * shadow / spacing as f64) });
                }
            }
            spans
        })
    };
    let trees = {
        let (f, anchor) = (f.clone(), f.clone());
        Population::new("trees", 1.0, tree_cell)
            .reach([vox(9.0) as i64, tree_top, vox(9.0) as i64])
            .size(vox(1.0) as i64)
            .anchor(move |cell| Some(anchor.spot(cell)))
            .make(move |[x, y, z], cell| {
                let mut random = cell.random("tree");
                let (xi, zi) = (x as i64, z as i64);
                let c = f.column(xi, zi);
                if !random.chance(f.tree_chance(xi, zi, &c)) {
                    return None;
                }
                let species = pick_species(Fx::fraction(random.next_u32()), &Forest::species_chances(&c));
                Some(make_tree(species, x, y, z, &mut random, &|x, z| f.ground(x, z)))
            })
            .far(canopy)
    };

    // Boulders and fallen logs.
    let rock_cell = vox(4.0) as i64;
    let on_ground = |name, reach| {
        let anchor = f.clone();
        Population::new(name, 2.0, rock_cell).reach(reach).size(vox(2.0) as i64).anchor(move |cell| Some(anchor.spot(cell)))
    };
    let boulders = {
        let f = f.clone();
        on_ground("boulders", vox(3.0) as i64).make(move |[x, y, z], cell| {
            let mut random = cell.random("boulder");
            let chance = f.thinned(fx(0.15), x as i64, z as i64);
            if random.chance(chance) { Some(Node::of(boulder(x, y, z, &mut random))) } else { None }
        })
    };
    let logs = {
        let f = f.clone();
        on_ground("logs", vox(10.0) as i64).make(move |[x, _, z], cell| {
            let mut random = cell.random("log");
            if random.chance(f.thinned(fx(0.077), x as i64, z as i64)) && !f.column(x as i64, z as i64).on_path {
                Some(Node::of(fallen_log(x, z, &|x, z| f.ground(x, z), &mut random)))
            } else {
                None
            }
        })
    };

    // Undergrowth, per column of (0.25 m)², in square patches N wide around each cell's centre.
    const PLANTS: i64 = 16;
    let column_width = vox(0.25) as i64;
    let anchor = f.clone();
    let plants = Population::new("plants", 3.0, PLANTS)
        .reach([0, vox(10.0) as i64, 0])
        .size(column_width)
        .anchor(move |cell| {
            let [cx, _, cz] = cell.centre;
            Some([cx as f64, anchor.ground(cx, cz) as f64 + 0.5, cz as f64])
        })
        .make(move |_, cell| {
            let [cx, _, cz] = cell.centre;
            let mut shapes = Vec::new();
            let mut xi = cx - PLANTS / 2;
            while xi < cx + PLANTS / 2 {
                let mut zi = cz - PLANTS / 2;
                while zi < cz + PLANTS / 2 {
                    let share = (f.ground.share)(xi, zi);
                    let c = f.column(xi, zi);
                    let (x, y, z) = (xi as f64, (c.height + 1) as f64, zi as f64);
                    // Rarer as the forest's share falls: as dense as it is.
                    let r = Fx::fraction(hash(f.s(15), xi, zi, 0));
                    let r = if share >= Fx::ONE { r } else { r / share.max(Fx::EPSILON) };
                    let random = &mut Rng::new(hash64(f.s(16), xi, zi, 0));
                    if c.on_path {
                        if r < fx(0.05) {
                            shapes.push(Shape::cuboid([x, y, z], [x + 2.0, y + 1.0, z + 1.0], K::stone));
                        }
                    } else if c.surface == K::grass {
                        if r < fx(0.02) {
                            let count = 3.0 + random.below(3) as f64;
                            let kind = if random.chance(fx(0.3)) { K::hay } else { K::tallGrass };
                            shapes.extend(tuft(x, y, z, count, vox(0.3), kind, random));
                        } else if r < fx(0.0275) {
                            shapes.extend(flower(x, y, z, random));
                        } else if r < fx(0.2) {
                            let count = 1.0 + random.below(2) as f64;
                            shapes.extend(tuft(x, y, z, count, vox(0.12), K::grass, random));
                        }
                    } else if c.surface != K::stone && c.surface != K::mossyStone {
                        if r < fx(0.00625) {
                            shapes.extend(fern(x, y, z, if c.conifer > Fx::HALF { K::fern } else { K::bracken }, random));
                        } else if r < fx(0.008) {
                            shapes.extend(bush(x, y, z, random));
                        } else if r < fx(0.0085) {
                            shapes.extend(flower(x, y, z, random));
                        } else if r < fx(0.011) && c.density > fx(0.3) {
                            shapes.extend(mushrooms(x, y, z, if c.conifer > Fx::HALF { K::amanita } else { K::bolete }, random));
                        } else if c.surface == K::needles && r < fx(0.04) {
                            shapes.push(Shape::cuboid([x, y, z], [x + 2.0, y + 2.0, z + 1.0], K::cone));
                        } else if c.surface == K::litter && r < fx(0.4) {
                            shapes.push(Shape::cuboid([x - 1.0, y - 1.0, z - 1.0], [x + 1.0, y, z + 1.0], K::fallenLeaf).paint());
                        }
                    }
                    zi += column_width;
                }
                xi += column_width;
            }
            if shapes.is_empty() { None } else { Some(Node::of(shapes)) }
        });

    vec![Layer::Field(terrain), Layer::Population(trees), Layer::Population(boulders), Layer::Population(logs), Layer::Population(plants)]
}

// ---------------------------------------------------------------------------
// Things. `random` picks their shapes.

struct Crown {
    trunk: Shape,
    clusters: Vec<Child>,
    fill: f64,
}

/** A tree of `shapes`, with the parts its crown hides as a child. */
fn tree(shapes: Vec<Shape>, hidden: Vec<Shape>, fine: Vec<Child>, crown: Option<Crown>) -> Node {
    let mut children = vec![Child::new("hidden", vox(0.5) as i64, Node::of(hidden))];
    children.extend(fine);
    let far = crown.map(|Crown { trunk, clusters, fill }| {
        let far_shapes: Vec<&Shape> = clusters.iter().flat_map(|c| c.node.far.iter().flat_map(|f| f.shapes())).collect();
        // An ellipsoid of the clusters' outline, shrunk to their volume and solid.
        let b = bounds_of_shapes(far_shapes.iter().map(|s| &s.bounds));
        let half = [0, 1, 2].map(|i| (b.max[i] - b.min[i]) as f64 / 2.0);
        let volume = (4.0 / 3.0) * PI * half[0] * half[1] * half[2];
        let filled = far_shapes.iter().fold(0.0, |sum, s| {
            let [a, c, d] = s.extents.map(Fx::to_f64);
            sum + (PI / 6.0) * a * c * d * fill
        });
        let shrink = js::pow(1.0 - js::exp(-filled / volume), 1.0 / 3.0);
        let outline = ellipsoid(
            [0, 1, 2].map(|i| (b.min[i] + b.max[i]) as f64 / 2.0),
            [half[0] * shrink, half[1] * shrink, half[2] * shrink],
            far_shapes[0].kind,
            SOLID,
            Op::Fill,
        );
        let detail = far_shapes.iter().flat_map(|s| s.extents).min().expect("clusters");
        let far = Far::of(detail, vec![trunk, outline]);
        children.extend(clusters);
        far
    });
    Node::of(shapes).with_children(children).with_far(far)
}

/** A vertical trunk, `width` units square, from just below `ground` up to `top`. */
fn trunk(x: f64, ground: f64, z: f64, top: f64, width: f64, kind: K) -> Shape {
    Shape::cuboid([x, ground - 1.0, z], [x + width, top + 1.0, z + width], kind).paint()
}

/** A random direction, evenly over the sphere. */
fn direction(random: &mut Rng) -> Point {
    let y = 2.0 * random.next() - 1.0;
    let a = 2.0 * PI * random.next();
    let r = (1.0 - y * y).sqrt();
    [r * js::cos(a), y, r * js::sin(a)]
}

/** A cluster of leaves around `centre`: sprays of leaves on twigs at level 0, further the ball they fill. */
#[allow(clippy::too_many_arguments)]
fn leaf_cluster(name: String, centre: Point, radius: f64, kind: K, fill: f64, random: &mut Rng, droop: f64, turning: K) -> Child {
    let seed = random.next_u64();
    let sprays = move |random: &mut Rng| {
        let spray = vox(0.45).min(radius * 0.4);
        let count = 4f64.max(round(0.95 * js::pow(radius / spray, 2.0)));
        let (mut twigs, mut leaves) = (Vec::new(), Vec::new());
        let at = |d: Point, r: f64| [centre[0] + d[0] * r, centre[1] + d[1] * r, centre[2] + d[2] * r];
        for _ in 0..count as usize {
            let [dx, dy, dz] = direction(random);
            let length = js::hypot(&[dx, dy - droop, dz]);
            let d = [dx / length, (dy - droop) / length, dz / length];
            let tip = radius - spray;
            twigs.push(Shape::capsule(at(d, radius * 0.1), at(d, tip - spray / 2.0), FINE, K::twig));
            let leaf = if random.chance(fx(0.25)) { turning } else { kind };
            leaves.push(capsule(at(d, radius * 0.3), at(d, tip), spray, k(leaf), Fill::Const(0.75), Op::Fill));
        }
        twigs.push(ball(centre, radius * 0.55, k(kind), Fill::Const(fill), Op::Fill));
        twigs.extend(leaves);
        twigs
    };
    let far = ball(centre, radius, k(kind), Fill::Const(fill), Op::Fill);
    Child::new(name, ANY_SIZE, Node::lazy(far.bounds, seed, sprays).with_far(Far::of(fx(FINE), vec![far])))
}

/** A child of a thing within `bounds`, drawn only at level 0, its shapes made only then. */
fn fine_child(name: &str, bounds: Box3, random: &mut Rng, make: impl Fn(&mut Rng) -> Vec<Shape> + 'static) -> Child {
    Child::new(name, FINE as i64, Node::lazy(bounds, random.next_u64(), make))
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum BarkStyle {
    Furrows,
    Dashes,
}

/** Bark on the four faces of a trunk [x, x + width) × [from, to) × [z, z + width). */
#[allow(clippy::too_many_arguments)]
fn bark(x: f64, z: f64, width: f64, from: f64, to: f64, style: BarkStyle, kind: K, random: &mut Rng) -> Vec<Shape> {
    let mut shapes = Vec::new();
    let mark = |f: usize, u: f64, length: f64, y0: f64, y1: f64| {
        let layer = if f.is_multiple_of(2) {
            if f < 2 { z } else { x }
        } else if f < 2 {
            z + width - 1.0
        } else {
            x + width - 1.0
        };
        if f < 2 {
            Shape::cuboid([u, y0, layer], [u + length, y1, layer + 1.0], kind).paint()
        } else {
            Shape::cuboid([layer, y0, u], [layer + 1.0, y1, u + length], kind).paint()
        }
    };
    let start = |f: usize| if f < 2 { x } else { z };
    for f in 0..4 {
        if style == BarkStyle::Furrows {
            let mut n = 1f64.max(round(width / 3.0));
            while n > 0.0 {
                let mut u = start(f) + random.below(width as u32 - 1) as f64;
                let mut y = from;
                while y < to {
                    let length = vox(0.4 + random.next() * 1.2);
                    shapes.push(mark(f, u, 2.0, y, (y + length).min(to)));
                    y += length + (random.next() * vox(0.3)).floor();
                    u = start(f).max((start(f) + width - 2.0).min(u + random.below(3) as f64 - 1.0));
                }
                n -= 1.0;
            }
        } else {
            let mut y = from;
            while y < to {
                if random.chance(fx(0.5)) {
                    let u = start(f) + random.below(width as u32 - 1) as f64;
                    let length = 2.0 + random.below(2) as f64;
                    shapes.push(mark(f, u, length, y, y + 1.0));
                }
                y += vox(0.25);
            }
        }
    }
    shapes
}

/** Roots spreading from a trunk's foot around (x, z) into the ground, `half` its half-width. */
fn roots(x: f64, ground: f64, z: f64, half: f64, kind: K, random: &mut Rng, ground_at: GroundAt) -> Vec<Shape> {
    let count = 4.0 + random.below(3) as f64;
    (0..count as usize)
        .map(|i| {
            let angle = (i as f64 / count) * PI * 2.0 + random.next() * 0.6;
            let (c, s) = (js::cos(angle), js::sin(angle));
            let out = half + vox(0.5 + random.next() * 0.7);
            let (ex, ez) = (x + c * out, z + s * out);
            let p = [x + c * half * 0.6, ground + vox(0.35), z + s * half * 0.6];
            let q = [ex, (ground_at(round(ex) as i64, round(ez) as i64) - 1) as f64, ez];
            Shape::capsule(p, q, vox(0.15 + random.next() * 0.08), kind)
        })
        .collect()
}

/** Conical spruce with drooping whorls of branches. */
fn spruce_tree(x: f64, ground: f64, z: f64, random: &mut Rng, ground_at: GroundAt) -> Node {
    let height = vox(14.0 + random.next() * 10.0);
    let width = if height >= vox(20.0) { vox(1.0) } else { vox(0.5) };
    let top = ground + height;
    let base = ground + round(height * (0.15 + random.next() * 0.15));
    let bole = trunk(x, ground, z, base, width, K::spruceBark);
    let hidden = vec![Shape::cuboid([x, base + 1.0, z], [x + width, top, z + width], K::spruceBark).paint()];
    let radius = height * (0.17 + random.next() * 0.05);
    let whorl = vox(1.5 + random.next() * 0.6);
    let (cx, cz) = (x + (width - 1.0) / 2.0, z + (width - 1.0) / 2.0);
    let crown_top = top + vox(0.5);
    let r = move |y: f64| {
        // Each whorl is widest at its bottom, giving the layered silhouette.
        let phase = js::wrap(top - y, whorl) / whorl;
        radius * (1.0 - (y - base) / (crown_top - base)) * (0.5 + 0.5 * phase) + vox(0.35)
    };
    let (from, to) = (base - 0.5, crown_top + 0.5);
    let density = move |distance: f64, py: f64| 0.95 - 0.35 * js::pow(distance / r(py), 3.0);
    let spokes = 6.0 + random.below(3) as f64;
    let sector = (2.0 * PI) / spokes;
    let twist = random.next() * sector;
    let half_width = |distance: f64| vox(0.15) + 0.07 * distance;
    let covered = move |distance: f64| 1f64.min((2.0 * half_width(distance)) / (distance * sector));
    let fans = move |px: f64, py: f64, pz: f64| {
        let distance = js::hypot(&[px - cx, pz - cz]);
        if distance < vox(0.5) {
            return density(distance, py);
        }
        let a = js::atan2(pz - cz, px - cx) - twist - ((top - py) / whorl).floor() * 2.39996;
        let off = js::wrap(a, sector);
        if off.min(sector - off) * distance < half_width(distance) { density(distance, py) } else { 0.0 }
    };
    let crown = |fill: Fill| lathe([cx, from, cz], [cx, to, cz], move |t| r(round(from + t * (to - from))), k(K::spruce), fill, Op::Fill);
    let foot = roots(cx, ground, cz, width / 2.0, K::spruceBark, random, ground_at);
    let fine = vec![fine_child("bark", bole.bounds, random, move |random| {
        bark(x, z, width, ground, base, BarkStyle::Furrows, K::spruceScale, random)
    })];
    let mut shapes = vec![bole.clone()];
    shapes.extend(foot.iter().cloned());
    shapes.push(crown(Fill::At(Rc::new(fans))));
    let mut near = tree(shapes, hidden, fine, None);
    // Its hidden parts are not drawn beyond level 0: inside a crown this dense they don't show.
    let mut far = vec![bole];
    far.extend(foot);
    far.push(crown(Fill::At(Rc::new(move |px, py, pz| {
        let distance = js::hypot(&[px - cx, pz - cz]);
        density(distance, py) * covered(vox(0.5).max(distance))
    }))));
    near.far = Some(Far::of(fx(FINE), far));
    near
}

/** Slender white birch with a wavy trunk and a loose, clumpy crown of drooping twigs. */
fn birch_tree(x: f64, ground: f64, z: f64, random: &mut Rng) -> Node {
    let height = vox(12.0 + random.next() * 7.0);
    let width = vox(0.375);
    let (mut shapes, mut marks) = (Vec::new(), Vec::new());
    let mut segments: Vec<[f64; 4]> = Vec::new();
    // The trunk shifts by a unit now and then.
    let (mut tx, mut tz, mut from) = (x, z, ground - 1.0);
    let mut y = ground - 1.0;
    while y <= ground + height + 1.0 {
        let end = y == ground + height + 1.0;
        if end || ((y - ground) % vox(3.0) == 0.0 && y - ground > vox(4.0) && random.chance(fx(0.35))) {
            shapes.push(Shape::cuboid([tx, from, tz], [tx + width, y, tz + width], K::birchBark).paint());
            segments.push([tx, tz, from.max(ground), y]);
            if end {
                break;
            }
            if random.chance(fx(0.5)) {
                tx += if random.chance(fx(0.5)) { -1.0 } else { 1.0 };
            } else {
                tz += if random.chance(fx(0.5)) { -1.0 } else { 1.0 };
            }
            from = y;
        }
        // Black marks in bands half a metre tall.
        if (y - ground) % vox(0.5) == 0.0 && random.chance(fx(0.18)) {
            marks.push(Shape::cuboid([tx, y, tz], [tx + width, y + vox(0.5), tz + width], K::birchMark).paint());
        }
        y += 1.0;
    }
    shapes.extend(marks);
    let foliage = if random.chance(fx(0.65)) { K::birchGold } else { K::birchGreen };
    let turning = if foliage == K::birchGold { K::birchGreen } else { K::birchGold };
    let crown_bottom = ground + height * 0.45;
    let bole = Shape::cuboid([x, ground - 1.0, z], [x + width, crown_bottom, z + width], K::birchBark).paint();
    let crown_top = ground + height + vox(1.0);
    let cy = (crown_bottom + crown_top) / 2.0;
    let ry = (crown_top - crown_bottom) / 2.0;
    let rx = vox(2.0 + random.next() * 1.2);
    let count = 12.0 + random.below(6) as f64;
    let mid = (width - 1.0) / 2.0;
    let (mut branches, mut clusters) = (Vec::new(), Vec::new());
    for i in 0..count as usize {
        // Uniform point in the crown ellipsoid.
        let (mut ox, mut oy, mut oz);
        loop {
            ox = random.next() * 2.0 - 1.0;
            oy = random.next() * 2.0 - 1.0;
            oz = random.next() * 2.0 - 1.0;
            if ox * ox + oy * oy + oz * oz <= 1.0 {
                break;
            }
        }
        let p = [tx + ox * rx, cy + oy * ry, tz + oz * rx];
        branches.push(Shape::capsule([tx + mid, p[1] - vox(0.5), tz + mid], p, BRANCH_RADIUS, K::birchBark).paint());
        let radius = vox(0.8 + random.next() * 0.6);
        clusters.push(leaf_cluster(format!("leaves {i}"), p, radius, foliage, 0.7, random, 0.6, turning));
    }
    let dashes = fine_child("bark", bounds_of_shapes(shapes.iter().map(|s| &s.bounds)), random, move |random| {
        segments.iter().flat_map(|&[sx, sz, y0, y1]| bark(sx, sz, width, y0, y1, BarkStyle::Dashes, K::birchMark, random)).collect()
    });
    tree(shapes, branches, vec![dashes], Some(Crown { trunk: bole, clusters, fill: 0.7 }))
}

/** Broad oak: thick trunk, a few branches each ending in a leaf cluster. */
fn oak_tree(x: f64, ground: f64, z: f64, random: &mut Rng, ground_at: GroundAt) -> Node {
    let height = vox(12.0 + random.next() * 8.0);
    let trunk_top = ground + round(height * (0.45 + random.next() * 0.1));
    let width = vox(1.0);
    let mut shapes = vec![trunk(x, ground, z, trunk_top, width, K::bark)];
    let roll = random.next();
    let foliage = if roll < 0.45 {
        K::oakGreen
    } else if roll < 0.75 {
        K::oakOrange
    } else {
        K::oakGold
    };
    let turning = if foliage == K::oakGold { K::oakOrange } else { K::oakGold };
    let cx = x + (width - 1.0) / 2.0;
    let cz = z + (width - 1.0) / 2.0;
    shapes.extend(roots(cx, ground, cz, width / 2.0, K::bark, random, ground_at));
    let branches = 4.0 + random.below(3) as f64;
    let mut ends: Vec<Point> = vec![[cx, ground + height - vox(2.0), cz]];
    let mut hidden = Vec::new();
    for i in 0..branches as usize {
        let angle = (i as f64 / branches) * PI * 2.0 + random.next() * 0.8;
        let reach = vox(2.5 + random.next() * 2.5);
        let start_y = trunk_top - (random.next() * vox(2.0)).floor();
        let end_x = cx + js::cos(angle) * reach;
        let end_y = (start_y + vox(2.0 + random.next() * 3.0)).min(ground + height - vox(1.5));
        let end = [end_x, end_y, cz + js::sin(angle) * reach];
        hidden.push(Shape::capsule([cx, start_y, cz], end, BRANCH_RADIUS * 1.5, K::bark).paint());
        ends.push(end);
    }
    let clusters = ends
        .iter()
        .enumerate()
        .map(|(i, end)| {
            let radius = vox(1.8 + random.next() * 1.2);
            leaf_cluster(format!("leaves {i}"), *end, radius, foliage, 0.8, random, 0.0, turning)
        })
        .collect();
    let fine = vec![fine_child("bark", shapes[0].bounds, random, move |random| {
        bark(x, z, width, ground, trunk_top, BarkStyle::Furrows, K::barkFurrow, random)
    })];
    let trunk_shape = shapes[0].clone();
    tree(shapes, hidden, fine, Some(Crown { trunk: trunk_shape, clusters, fill: 0.8 }))
}

// Undergrowth on the ground at (x, y, z): blades, stems and leaves a unit or two thick.

/** A tuft of `count` blades about `tall` high. */
fn tuft(x: f64, y: f64, z: f64, count: f64, tall: f64, kind: K, random: &mut Rng) -> Vec<Shape> {
    (0..count as usize)
        .map(|_| {
            let dx = random.below(3) as f64 - 1.0;
            let dz = random.below(3) as f64 - 1.0;
            let blade = 1f64.max(round(tall * (0.6 + random.next() * 0.6)));
            Shape::cuboid([x + dx, y, z + dz], [x + dx + 2.0, y + blade, z + dz + 1.0], kind)
        })
        .collect()
}

const FLOWERS: [K; 4] = [K::flowerYellow, K::flowerViolet, K::flowerWhite, K::flowerRed];

/** A flower: a stem and a head two sites wide. */
fn flower(x: f64, y: f64, z: f64, random: &mut Rng) -> Vec<Shape> {
    let stem = 2.0 + random.below(2) as f64;
    let colour = FLOWERS[random.below(FLOWERS.len() as u32) as usize];
    vec![
        Shape::cuboid([x, y, z], [x + 2.0, y + stem, z + 1.0], K::tallGrass),
        Shape::cuboid([x - 1.0, y + stem, z - 1.0], [x + 1.0, y + stem + 1.0, z + 1.0], colour),
    ]
}

/** A fern or bracken clump about a metre across. */
fn fern(x: f64, y: f64, z: f64, kind: K, random: &mut Rng) -> Vec<Shape> {
    let fronds = 5.0 + random.below(4) as f64;
    let mut shapes = Vec::new();
    for i in 0..fronds as usize {
        let angle = (i as f64 / fronds) * PI * 2.0 + random.next() * 0.5;
        let (c, s) = (js::cos(angle), js::sin(angle));
        let length = vox(0.45 + random.next() * 0.35);
        let rise = vox(0.25 + random.next() * 0.2);
        let bend = [x + c * length * 0.5, y + rise, z + s * length * 0.5];
        shapes.push(capsule([x, y, z], bend, 1.1, k(kind), Fill::Const(0.85), Op::Fill));
        shapes.push(capsule(bend, [x + c * length, y + rise * 0.3, z + s * length], 1.0, k(kind), Fill::Const(0.7), Op::Fill));
    }
    shapes
}

/** A bush, half the time with red berries on its upper side. */
fn bush(x: f64, y: f64, z: f64, random: &mut Rng) -> Vec<Shape> {
    let radius = vox(0.5 + random.next() * 0.5);
    let mut shapes = vec![ball([x, y, z], radius, k(K::bush), Fill::Const(0.8), Op::Fill)];
    if random.chance(fx(0.5)) {
        let mut n = 4.0 + random.below(6) as f64;
        while n > 0.0 {
            let [dx, dy, dz] = direction(random);
            let (px, py, pz) = (round(x + dx * radius * 0.9), round(y + dy.abs() * radius * 0.9), round(z + dz * radius * 0.9));
            // On a site.
            let at = [px, py + ((px as i64 + py as i64 + pz as i64) & 1) as f64, pz];
            shapes.push(Shape::cuboid(at, [at[0] + 1.0, at[1] + 1.0, at[2] + 1.0], K::berry).paint());
            n -= 1.0;
        }
    }
    shapes
}

/** One to three mushrooms: fly agarics (white-spotted red caps) or boletes. */
fn mushrooms(x: f64, y: f64, z: f64, kind: K, random: &mut Rng) -> Vec<Shape> {
    let mut shapes = Vec::new();
    let mut n = 1.0 + random.below(3) as f64;
    while n > 0.0 {
        let mx = x + random.below(5) as f64 - 2.0;
        let mz = z + random.below(5) as f64 - 2.0;
        let stem = 1.0 + random.below(2) as f64;
        let cap = [mx + 0.5, y + stem, mz];
        let rx = 1.4 + random.next() * 0.4;
        let radii = [rx, 1.0, 1.4 + random.next() * 0.4];
        shapes.push(Shape::cuboid([mx, y, mz], [mx + 2.0, y + stem, mz + 1.0], K::mushroomStem));
        shapes.push(Shape::ellipsoid(cap, radii, kind));
        if kind == K::amanita {
            let top = cap[1];
            shapes.push(ellipsoid(
                cap,
                radii,
                k(K::amanitaSpot),
                Fill::At(Rc::new(move |_, py, _| if py > top { 0.35 } else { 0.0 })),
                Op::Paint,
            ));
        }
        n -= 1.0;
    }
    shapes
}

/** A half-buried rock with moss on its upper side. */
fn boulder(x: f64, ground: f64, z: f64, random: &mut Rng) -> Vec<Shape> {
    let rx = vox(0.8 + random.next() * 1.4);
    let ry = rx * (0.6 + random.next() * 0.3);
    let rz = vox(0.8 + random.next() * 1.4);
    vec![
        Shape::ellipsoid([x, ground, z], [rx, ry, rz], K::stone).paint(),
        ellipsoid(
            [x, ground, z],
            [rx, ry, rz],
            k(K::mossyStone),
            Fill::At(Rc::new(move |_, y, _| if y - ground > ry * 0.4 { 0.7 } else { 0.0 })),
            Op::Paint,
        ),
    ]
}

/** A fallen trunk lying across the ground, with moss on top. */
fn fallen_log(x: f64, z: f64, ground: GroundAt, random: &mut Rng) -> Vec<Shape> {
    let length = vox(4.0 + random.next() * 5.0);
    let angle = random.next() * PI;
    let dx = js::cos(angle);
    let dz = js::sin(angle);
    let (ex, ez) = (x + dx * length, z + dz * length);
    let lift = log_radius();
    let at = |x: f64, z: f64| ground(x as i64, z as i64) as f64;
    let mut shapes =
        vec![Shape::capsule([x, at(x, z) + lift, z], [ex, at(round(ex), round(ez)) + lift, ez], log_radius(), K::deadwood).paint()];
    // Moss in patches on top.
    let mut i = 0.0;
    while i <= length {
        let px = round(x + dx * i);
        let pz = round(z + dz * i);
        if random.chance(fx(0.5)) {
            shapes.push(Shape::ball([px, at(px, pz) + lift * 2.0 + vox(0.25), pz], vox(0.3), K::moss));
        }
        i += vox(0.5);
    }
    shapes
}
