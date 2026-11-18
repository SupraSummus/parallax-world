//! The forest: an autumn forest valley, endless, on the generator
//! framework. Its numbers are moving to integers (docs/generators.md).
//! Here: its kinds, its ground, the domain other worlds grow it as, and
//! its layers; its trees are in forest/trees.rs, what lies on its floor in
//! forest/floor.rs.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::OnceLock;

use worldgen::World;
use worldgen::column::{self, lowest_around};
use worldgen::domain::{Domain, Lie, Scope, compose};
use worldgen::generator::{Field, Generator, Holds, Layer, Line, Node, Op, Point, Population, Shape, Span};
use worldgen::js::{self, round};
use worldgen::kinds;
use worldgen::noise::{Fbm, Fx, Rng, ValueNoise, fx, hash, hash64, smoothstep, unit};
use worldgen::plan::{self, Holding, Plan, Sloped};
use worldgen::points::{Blur, MARKS, Points, Rooms};
use worldgen::units::{UNIT_SIZE, Units, m};

use crate::path::{Path, Spine};

mod floor;
mod trees;

use floor::{BOULDER_RADIUS, boulder, bush, fallen_log, fern, flower, mushrooms, tuft};
use trees::{SPECIES, bark_of, foliage, grow, height};
pub use trees::{Species, make_tree};

/** Metres in units, rounded. */
fn vox(metres: f64) -> f64 {
    round(metres / UNIT_SIZE)
}
const PI: f64 = std::f64::consts::PI;

/** The heights it has anything at, in units. */
pub const HEIGHTS: (i64, i64) = (m(-36.0), m(80.0));

// The forest's kinds are first in any world it grows in.
kinds! {
    base: 0;
    moss: 1, ["#4f7a2a", "#5a8a30", "#466e26", "#628f37"];
    grass: 1, ["#74a83c", "#7fb544", "#6a9c36"];
    tallGrass: 1, ["#8fbf4d", "#9cc95a", "#a3b84a"], walk_through;
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
    spruce: 5, ["#1f4a2c", "#24553a", "#1a3f26", "#2b5e3c"], walk_through;
    oakGreen: 5, ["#4c7f2a", "#5a8f30", "#447324", "#679a38"], walk_through;
    oakOrange: 5, ["#c46a1c", "#d68526", "#b0521a", "#9c4a1c"], walk_through;
    oakGold: 5, ["#d9a521", "#c8941c", "#e3b53a", "#b88a1e"], walk_through;
    birchGreen: 5, ["#7fae3c", "#8cbc45", "#9ac24f"], walk_through;
    birchGold: 5, ["#d8cf4a", "#e6d65a", "#c9be36", "#efe07a"], walk_through;
    fern: 5, ["#3f7a2c", "#4a8a34", "#56953a"], walk_through;
    bracken: 5, ["#a8642a", "#b97a35", "#8f5524"], walk_through;
    bush: 5, ["#3d6b28", "#477a2e", "#355f22"], walk_through;
    flowerYellow: 6, ["#e8d34a", "#f0dc58"], walk_through;
    flowerViolet: 6, ["#c9b1ff", "#b89cf0"], walk_through;
    flowerWhite: 6, ["#f2f2f2", "#e8e8e0"], walk_through;
    flowerRed: 6, ["#e86a6a", "#d95858"], walk_through;
    barkFurrow: 4, ["#2e2115", "#352717", "#281c12"];
    spruceScale: 4, ["#33241a", "#2b1e15", "#3d2b1f"];
    twig: 4, ["#5a4430", "#4e3a28", "#634b35"], walk_through;
    hay: 1, ["#b9a15a", "#c8b06a", "#a89050"];
    fallenLeaf: 2, ["#c4581c", "#d9861f", "#b8401a", "#e0b030", "#9a3a16", "#8c5a22"];
    cone: 4, ["#6b4423", "#7a5030", "#5c3a1e"], walk_through;
    amanita: 6, ["#c8281e", "#d63a2a", "#b52018"], walk_through;
    amanitaSpot: 6, ["#f4efe6", "#ebe4d6"], walk_through;
    mushroomStem: 6, ["#e9e2d0", "#ddd4bf"], walk_through;
    bolete: 6, ["#7a4a26", "#8a5530", "#6c4020"], walk_through;
    berry: 6, ["#c01f2a", "#a81824", "#d43a30"], walk_through;
}

#[derive(Clone, Copy, Default)]
struct Column {
    height: i64,
    /** Its surface, and where the ground is steep, its rock (Woods::column). */
    surface: K,
    rock: K,
    /** How much the trees around are conifers, and how dense they are: fractions. */
    conifer: Fx,
    density: Fx,
    on_path: bool,
}

/** Plan scales (see "Bands" in docs/generators.md): the ground's bumps, a column; the path's bends are path.rs's. */
pub const GROUND_SCALE: i64 = 8;
/**
 * The most the forest's ground rises or falls from a column to the next,
 * in units (Sloped): its hills and the path's dip are gentle; the
 * learned town's terraces ease into them.
 */
pub const SLOPE: i64 = 2;
const COLUMN_SCALE: i64 = 1;
/** The furthest a tree reaches from its foot across, in units: its population's reach. */
const TREE_REACH: i64 = m(9.0);
/** A leaf of the trees' points: a square that holds a tree by its chance. */
const TREE_LEAF: i64 = m(2.0);
/** The most a tree weighs in its rooms (Rooms, units²): its rooms' walls move by up to that over twice their distance. */
const MOST: i64 = m(2.5) * m(2.5);
/** The gaps between neighbours' crowns: from half a metre of overlap to a metre and a half apart, a quarter of pairs meeting. */
const GAPS: [i64; 2] = [m(-0.5), m(1.5)];

/** A tree's weight in its rooms: none for the shortest, MOST for the tallest, as light favours it. */
fn weight(height: f64) -> i64 {
    (MOST as f64 * ((height - vox(12.0)) / vox(12.0)).clamp(0.0, 1.0)) as i64
}

/**
 * The relief: hills kilometres wide, and a winding path along z that the
 * ground dips along: the forest's; the valley finds its own on the floor
 * (course.rs).
 */
pub struct Relief {
    phase: f64,
    hills: Fbm<360, 4>,
    bumps: ValueNoise<48>,
    far: Fbm<7200, 4>,
}

impl Relief {
    pub fn new(scope: &Scope) -> Relief {
        Relief {
            phase: unit(hash(scope.seed("phase"), 0, 0, 0)) * 100.0,
            hills: Fbm::new(scope.seed("hills")),
            bumps: ValueNoise::new(scope.seed("bumps")),
            far: Fbm::new(scope.seed("far")),
        }
    }

    /** The path's centre line at z, with its fraction. */
    pub fn path_x(&self, z: i64) -> Fx {
        let zm = z as f64 * UNIT_SIZE;
        Fx::m(9.0 * js::sin(zm / 31.0 + self.phase) + 5.0 * js::sin(zm / 13.0 + self.phase * 2.0))
    }

    /** Ground height at (x, z), `from_path` from the path's centre line; with its fraction. */
    pub fn ground(&self, x: i64, z: i64, from_path: Fx) -> Fx {
        let valley = smoothstep(Fx::m(8.0), Fx::m(42.0), from_path) * Units::m(18.0);
        let path_dip = (Fx::ONE - smoothstep(Fx::m(1.0), Fx::m(2.2), from_path)) * Fx::m(0.3);
        self.floor(x, z) + valley - path_dip
    }

    /** The ground at (x, z) without the dip along the path: its hills, near and far; with its fraction. */
    pub fn floor(&self, x: i64, z: i64) -> Fx {
        // The bumps' product truncated towards zero, not floored, as it always was.
        let bumps = self.bumps.at(x, z) - Fx::HALF;
        let hills = (self.hills.at(x, z) - Fx::HALF) * Units::m(10.0) + bumps.abs() * Fx::m(0.8) * bumps.signum();
        hills + (self.far.at(x, z) - Fx::HALF) * Units::m(50.0)
    }
}

/**
 * The forest, a domain on ground decided outside it: it grows along
 * `path`, kept clear around `spawn`, thinned by its share of each column
 * and by `thinning` (a fraction, none by default).
 */
pub struct Forest {
    scope: Scope,
    path: Rc<Path>,
    spawn: [i64; 3],
    thinning: Rc<dyn Fn(i64, i64) -> Fx>,
}

impl Forest {
    pub fn new(scope: Scope, path: Rc<Path>, spawn: [i64; 3]) -> Forest {
        Forest { scope, path, spawn, thinning: Rc::new(|_, _| Fx::ONE) }
    }

    /** Thinned by `thinning` too, a fraction kept: at the treeline, by the sea. */
    pub fn thinned(self, thinning: impl Fn(i64, i64) -> Fx + 'static) -> Forest {
        Forest { thinning: Rc::new(thinning), ..self }
    }
}

impl Domain for Forest {
    fn layers(self: Rc<Self>, ground: &Rc<Sloped>, hold: Holding) -> Vec<Layer> {
        let thinning = self.thinning.clone();
        let share = Rc::new(move |x, z| hold.share(x, z) * thinning(x, z));
        layers(&self.scope, Ground { ground: ground.clone(), path: self.path.clone(), share }, self.spawn)
    }
}

/** On `path` at z, `above` units over `ground`, on a site: where a camera starts. */
pub fn on_path(ground: &Sloped, path: &Path, z: i64, above: i64) -> [i64; 3] {
    let x = path.x(z).round_units().0;
    let y = ground.at(x, z) + above;
    [x, y + ((x + y + z) & 1), z]
}

/** What the forest grows on: the ground's height, the path, and its share of each column (a fraction), which thins whatever it grows. */
struct Ground {
    ground: Rc<Sloped>,
    path: Rc<Path>,
    share: Rc<dyn Fn(i64, i64) -> Fx>,
}

/** The forest's state shared by its layers: what it knows about any column, from the seed and its ground alone. */
struct Woods {
    scope: Scope,
    spawn: [i64; 3],
    ground: Ground,
    columns: Plan<Column>,
    /** `steep` over a run of one, kept: plans and points read it, at every level. */
    steep: Plan<bool>,
}

/** The forest's own noise, by what it decides. */
struct Noise {
    conifer: Fbm<320, 4>,
    density: Fbm<176, 4>,
    path_edge: ValueNoise<24>,
    floor: ValueNoise<20>,
}

/** The rise over a run of one from which ground is stone: of a column to the next along x or z, of a slope's side to the spacing. */
const STEEP: i64 = 3;

/** Whether `ground` rises or falls STEEP a run from column (x, z) to the next `s` along x or z. */
fn steep(ground: &Sloped, x: i64, z: i64, s: i64) -> bool {
    let h = ground.at(x, z);
    let rises = |x, z| (ground.at(x, z) - h).abs() >= STEEP * s;
    rises(x + s, z) || rises(x, z + s)
}

/** How far the path's edge lies from its centre line, in metres: the least and, where it frays, the most. */
const PATH_EDGE: [f64; 2] = [1.1, 1.6];

/** Column (x, z), given the ground's height there. */
fn column_of(n: &Noise, path: &Path, x: i64, z: i64, h: i64) -> Column {
    let conifer = smoothstep(fx(0.4), fx(0.6), n.conifer.at(x, z)) * 3 / 4 + smoothstep(Fx::m(6.0), Fx::m(16.0), Units(h).to_fx()) / 4;
    let density = smoothstep(fx(0.3), fx(0.48), n.density.at(x, z));
    let on_path = path.by(x, z, Fx::m(PATH_EDGE[0]) + n.path_edge.at(z, 0) * Units::m(PATH_EDGE[1] - PATH_EDGE[0]));
    let floor = n.floor.at(x, z);
    let rock = if on_path {
        K::path
    } else if floor < fx(0.5) {
        K::stone
    } else {
        K::mossyStone
    };
    let surface = if on_path {
        K::path
    } else if density < fx(0.2) {
        if floor < fx(0.75) { K::grass } else { K::moss }
    } else if conifer > fx(0.5) {
        if floor < fx(0.55) { K::needles } else { K::moss }
    } else if floor < fx(0.6) {
        K::litter
    } else {
        K::moss
    };
    Column { height: h, surface, rock, conifer, density, on_path }
}

impl Woods {
    fn s(&self, what: &str) -> u64 {
        self.scope.seed(what)
    }

    /** Ground height in units. */
    fn ground(&self, x: i64, z: i64) -> i64 {
        self.ground.ground.at(x, z)
    }

    /** Column (x, z): rock where the ground is `steep` over the spacing read for (plan::spacing), as the terrain's cliffs. */
    fn column(&self, x: i64, z: i64) -> Column {
        self.column_over(x, z, plan::spacing())
    }

    /** Column (x, z): rock where the ground is `steep` over `s`. */
    fn column_over(&self, x: i64, z: i64, s: i64) -> Column {
        let column = self.columns.at(x, z);
        if self.ground.ground.slope().is_some_and(|slope| slope < STEEP) {
            return column;
        }
        let steep = match s {
            1 => self.steep.at(x, z),
            s => steep(&self.ground.ground, x, z, s),
        };
        Column { surface: if steep { column.rock } else { column.surface }, ..column }
    }

    /** `chance` times the forest's share of column (x, z). */
    fn thinned(&self, chance: Fx, x: i64, z: i64) -> Fx {
        chance * (self.ground.share)(x, z)
    }

    /** A spot on the ground picked evenly under a column of cells, the same for every population of its N. */
    fn spot(&self, line: &Line) -> Point {
        let [i, k] = line.across;
        line.pick(&mut Rng::new(hash64(self.s("spots"), line.n, i, k)), |x, z| self.ground(x, z))
    }

    /** The chance a square 4 m wide around column (x, z) holds a tree. */
    fn tree_chance(&self, x: i64, z: i64, c: &Column) -> Fx {
        if c.surface == K::stone || self.ground.path.by(x, z, Fx::m(2.8)) {
            return Fx::ZERO;
        }
        // Keep the view from the spawn point clear of low branches: within 7 m of the 40 m ahead along +z, where the
        // camera starts looking.
        let (dx, dz) = (x - self.spawn[0], z - self.spawn[2]);
        let beyond = dz - dz.clamp(0, vox(40.0) as i64);
        if dx * dx + beyond * beyond < (vox(7.0) as i64).pow(2) {
            return Fx::ZERO;
        }
        self.thinned(fx(0.12) + c.density / 2, x, z)
    }

    /** How tall the tree at a point grows (units): of the point alone, so its neighbours know it too. */
    fn height(&self, x: i64, z: i64, mark: usize) -> f64 {
        height(SPECIES[mark], unit(hash(self.s("heights"), x, z, mark as i64)))
    }

    /** The rooms of the trees at `points`: by their heights, as light favours the taller (`weight`). */
    fn rooms(self: &Rc<Woods>, points: Rc<Points>) -> Rooms {
        let f = self.clone();
        Rooms::new(points, TREE_REACH, MOST, GAPS, move |s| weight(f.height(s.at[0], s.at[2], s.mark)))
    }

    /** The share of each species, fractions that add up to one. */
    fn species_chances(c: &Column) -> [Fx; 3] {
        let broad = Fx::ONE - c.conifer;
        [c.conifer, broad * 45 / 100, broad - broad * 45 / 100]
    }

    /**
     * By species, the chance a square 2 m wide around column (x, z) holds a tree of it: none on rock, steep
     * over the square at least, so the trees' points read the ground only at their leaves' corners.
     */
    fn leaf_chances(&self, x: i64, z: i64) -> [Fx; 3] {
        let c = self.column_over(x, z, plan::spacing().max(TREE_LEAF));
        let chance = self.tree_chance(x, z, &c) / 4;
        Woods::species_chances(&c).map(|sp| chance * sp)
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

/** The trees measured for the canopy at a spacing, by species. */
struct Measured {
    /**
     * What each slab of a tree adds to its shadow over directions (lattice units²), from its foot up, a
     * spacing each, by STANDS: in a room among trees as dense as that.
     */
    slabs: Vec<[Vec<f64>; 3]>,
    /** The share of a tree's shadow from above each slab adds. */
    above: [Vec<f64>; 3],
    /** The share of each slab's sites that are wood. */
    wood: [Vec<f64>; 3],
    /** A tree's shadow from above (lattice units²). */
    shadow: [f64; 3],
    /** The trees standing in a column, by species: a quarter of those of each of the four cells it meets. */
    own: Blur,
    /**
     * By the offset of a crown's cell from a column's, how densely the crown alone hides it from above, as an
     * area: it covers a share f of it (Shadows::spread), and leaves 1 − f open, as dense as −ln(1 − f).
     */
    over: HashMap<[i64; 2], [f64; 3]>,
}

/** The stands trees are measured in for the canopy: as dense as the chance of a tree in a square 4 m wide. */
const STANDS: [f64; 4] = [0.0, 0.2, 0.4, 0.6];

/** How tall a tree grows at most (units): the canopy's top over the ground. */
const TREE_TOP: i64 = m(26.0);

/**
 * The sample stands at a spacing, by species, slab by slab: what each
 * adds to a tree's shadow (by STANDS), and in the sparsest, its share of
 * wood and what it adds from above; and where that falls around a cell.
 */
#[derive(Clone, Debug, PartialEq)]
struct Sampled {
    slabs: Vec<[Vec<f64>; 3]>,
    wood: [Vec<f64>; 3],
    across: [Vec<f64>; 3],
    spread: [Vec<([i64; 2], f64)>; 3],
}

/** The sample stands at each spacing the canopy is drawn at, as the test `keep_samples` wrote them. */
const SAMPLES: &str = include_str!("../data/forest-stands.txt");

/** The stands at `spacing`, as kept (SAMPLES). */
fn sampled_at(spacing: i64) -> Sampled {
    static KEPT: OnceLock<HashMap<i64, Sampled>> = OnceLock::new();
    let kept = KEPT.get_or_init(|| read_samples(SAMPLES));
    kept.get(&spacing).cloned().unwrap_or_else(|| panic!("no sample stands kept at spacing {spacing}: see keep_samples"))
}

/** What `write_samples` wrote, by spacing. */
fn read_samples(text: &str) -> HashMap<i64, Sampled> {
    let mut lines = text.lines();
    let mut samples = HashMap::new();
    while let Some(spacing) = lines.next() {
        let mut series =
            || -> Vec<f64> { lines.next().expect("a series").split_whitespace().map(|n| n.parse().expect("a number")).collect() };
        let slabs = STANDS.map(|_| [(); 3].map(|_| series())).to_vec();
        let [wood, across] = [(); 2].map(|_| [(); 3].map(|_| series()));
        let spread = [(); 3].map(|_| series().chunks(3).map(|c| ([c[0] as i64, c[1] as i64], c[2])).collect());
        samples.insert(spacing.parse().expect("a spacing"), Sampled { slabs, wood, across, spread });
    }
    samples
}

/** The forest of `seed`, alone on the relief, all of it its own. */
pub fn forest(seed: u64) -> World {
    let world = Scope::new(seed);
    let relief = Rc::new(Relief::new(&world.of("relief")));
    let spine = {
        let relief = relief.clone();
        Spine::new(move |z| relief.path_x(z))
    };
    let path = Rc::new(Path::new(spine.clone()));
    let ground = Lie::new(move |x, z| relief.ground(x, z, spine.across(x, z).abs())).ground(GROUND_SCALE, SLOPE);
    // Start on the path at the near end, at about the height of a tall shrub.
    let spawn = on_path(&ground, &path, vox(-31.0) as i64, vox(3.0) as i64);
    let forest: Rc<dyn Domain> = Rc::new(Forest::new(world.of("forest"), path, spawn));
    let layers = compose(&[forest], &ground, GROUND_SCALE, Fx::ONE);
    World {
        generator: Generator { kinds: KINDS.to_vec(), seed: world.seed("layers"), layers },
        spawn,
        heights: Some(HEIGHTS),
        points: Vec::new(),
    }
}

/** The forest's layers on `ground`, kept clear around `spawn`; their kinds are KINDS, first in the generator's. */
fn layers(scope: &Scope, ground: Ground, spawn: [i64; 3]) -> Vec<Layer> {
    let noise = Noise {
        conifer: Fbm::new(scope.seed("conifer")),
        density: Fbm::new(scope.seed("density")),
        path_edge: ValueNoise::new(scope.seed("path's edge")),
        floor: ValueNoise::new(scope.seed("floor")),
    };
    let columns = {
        let (heights, path) = (ground.ground.clone(), ground.path.clone());
        Plan::new("forest columns", COLUMN_SCALE, move |x, z| column_of(&noise, &path, x, z, heights.at(x, z)))
    };
    let on = ground.ground.clone();
    let steep = {
        let heights = ground.ground.clone();
        Plan::new("forest steep", COLUMN_SCALE, move |x, z| steep(&heights, x, z, 1))
    };
    let f = Rc::new(Woods { scope: scope.clone(), spawn, ground, columns, steep });

    // The ground: along each column, soil or stone under a layer of its surface, and stone down to the bottom.
    let terrain = {
        let f = f.clone();
        Field::columns("terrain", 0.0, Op::Paint, move |x, z| {
            let h = f.ground(x, z);
            let lowest = lowest_around(|x, z| f.ground(x, z), x, z);
            let surface = f.column(x, z).surface;
            // Stone under stone, and down a side as steep over the spacing as stone ground: a cliff's at any level,
            // a corner's where only a diagonal falls away.
            let cliff = h - lowest >= STEEP * plan::spacing();
            let below = if cliff || surface == K::stone || surface == K::mossyStone { K::stone } else { K::soil };
            column::Column::new(h).surface(surface, vox(1.0) as i64).sides(below, lowest, vox(1.5) as i64).under(K::stone)
        })
        .ground(on.clone(), vox(1.5) as i64, 1, K::stone)
    };

    // Trees, at the trees' points (worldgen's Points), of their marks' species: leaves 2 m wide, each holding one
    // by its share of tree_chance, a square 4 m wide's; roots 8 m wide, as wide as the chances follow.
    let points = {
        let f = f.clone();
        Rc::new(Points::new("trees", f.s("trees"), &[0, 2], TREE_LEAF, vox(8.0) as i64, 3, move |[x, _, z]| {
            let [spruce, birch, oak] = f.leaf_chances(x, z);
            [spruce, birch, oak, Fx::ZERO]
        }))
    };
    // What each slab of each species' trees adds to their shadow, and where around their cells they hide what
    // from above, from the sample stands (`stands`), by spacing.
    let measured = Rc::new(RefCell::new(HashMap::<i64, Rc<Measured>>::new()));
    let blurred = points.clone();
    let measured_at = move |spacing: i64| {
        measured
            .borrow_mut()
            .entry(spacing)
            .or_insert_with(|| {
                let Sampled { slabs, wood, across, spread } = sampled_at(spacing);
                let above = across.map(|slabs| {
                    let sum: f64 = slabs.iter().sum();
                    slabs.iter().map(|s| s / sum).collect()
                });
                let (mut shadow, mut over) = ([0.0; 3], HashMap::<[i64; 2], [f64; 3]>::new());
                let column = (spacing * spacing) as f64;
                for (sp, spread) in spread.into_iter().enumerate() {
                    for &([x, z], area) in &spread {
                        shadow[sp] += area;
                        over.entry([x, z]).or_default()[sp] = -js::log(1.0 - (area / column).min(0.99)) * column;
                    }
                }
                let feet = [[-1, -1], [-1, 0], [0, -1], [0, 0]].map(|o| (o, [0.25; MARKS])).to_vec();
                let own = Blur::new(blurred.clone(), spacing, feet);
                Rc::new(Measured { slabs, above, wood, shadow, own, over })
            })
            .clone()
    };
    // A medium as dense as the trees standing in each column, their area per volume. Where the crown over it
    // (its owner's: Rooms) hides more from above than that (over a clearing, a path, a street), it hides the
    // rest too, slab by slab as they do. Not their whole medium spread around: denser from above
    // than the trees ("Media are isotropic" in docs/generators.md), it filled the gaps among them (from above
    // at level 4, 0.14 apart in the forest).
    let rooms = Rc::new(f.rooms(points.clone()));
    let canopy = {
        let (f, rooms) = (f.clone(), rooms.clone());
        Field::columns("canopy", 1.0, Op::Fill, move |x, z| {
            // Its own far form, measured by the sites' spacing.
            let spacing = plan::spacing();
            let measured = measured_at(spacing);
            let Measured { slabs, above, wood, shadow, own, over } = &*measured;
            // Its trees as measured in stands as dense as they are there, between the two nearest.
            let chances = f.leaf_chances(x, z);
            let at = chances.iter().fold(0.0, |sum, c| sum + 4.0 * c.to_f64()) / STANDS[1];
            let below = (at as usize).min(STANDS.len() - 2);
            let t = (at - below as f64).min(1.0);
            let slabs: [Vec<f64>; 3] = [0, 1, 2].map(|sp| {
                let (a, b) = (&slabs[below][sp], &slabs[below + 1][sp]);
                a.iter().zip(b).map(|(a, b)| a + t * (b - a)).collect()
            });
            // The trees standing in the column, by species, and how densely the crown over it hides it from above:
            // its owner's, the tree's whose room it is in, as it would alone, as rooms cut crowns at their walls.
            // Finding the owner makes every tree near the column, so only while a column is no wider than a leaf: a
            // tile then makes no more trees than it has columns (two leaves wide, level 5 took 4 times level 4).
            let (own, over): ([f64; 3], [f64; 3]) = if spacing <= TREE_LEAF {
                let own = own.at([x, z]);
                let mut hides = [0.0; 3];
                if let Some(owner) = rooms.owner([x, 0, z]) {
                    let offset =
                        [owner.at[0].div_euclid(spacing) - x.div_euclid(spacing), owner.at[2].div_euclid(spacing) - z.div_euclid(spacing)];
                    hides[owner.mark] = over.get(&offset).map_or(0.0, |o| o[owner.mark]);
                }
                ([own[0], own[1], own[2]], hides)
            } else {
                // The trees expected over the column, by the chances at its site.
                let leaves = ((spacing / TREE_LEAF) as f64).powi(2);
                let own = [0, 1, 2].map(|sp| chances[sp].to_f64() * leaves);
                (own, [0, 1, 2].map(|sp| own[sp] * shadow[sp]))
            };
            let hidden: f64 = over.iter().sum();
            if hidden == 0.0 {
                return Vec::new();
            }
            let rest = (hidden - (0..3).map(|sp| own[sp] * slabs[sp].iter().sum::<f64>()).sum::<f64>()).max(0.0);
            let per_area = 1.0 / (spacing * spacing) as f64;
            // Of a species picked by how much their crowns hide from above, each slab its bark as often as the
            // sites of its trees' slab are wood, and else one of its foliage's colours, as often as its trees have
            // it: of one colour, all oaks were orange from afar.
            let species = pick_species(Fx::fraction(hash(f.s("canopy species"), x, z, 0)), &over.map(|o| fx(o / hidden)));
            let sp = SPECIES.iter().position(|s| *s == species).expect("a species");
            let height = f.column(x, z).height;
            let mut spans = Vec::new();
            for (i, from) in (0..TREE_TOP).step_by(spacing as usize).enumerate() {
                let shadow = (0..3).fold(0.0, |sum, sp| sum + own[sp] * slabs[sp][i] + rest * over[sp] / hidden * above[sp][i]);
                if shadow > 0.0 {
                    let (from, to) = (height + from, height + from + spacing);
                    let bark = unit(hash(f.s("canopy bark"), x, i as i64, z)) < wood[sp][i];
                    let kind = k(if bark { bark_of(species) } else { foliage(species, unit(hash(f.s("canopy foliage"), x, i as i64, z))) });
                    spans.push(Span { from, to, kind, holds: Holds::Medium(per_area * shadow / spacing as f64) });
                }
            }
            spans
        })
    };
    let trees = {
        let (f, ground) = (f.clone(), f.clone());
        Population::new("trees", 1.0, TREE_LEAF)
            .reach([TREE_REACH, TREE_TOP, TREE_REACH])
            .size(vox(1.0) as i64)
            .anchor_points(points, move |spot| {
                let [x, _, z] = spot.at;
                Some([x as f64, ground.ground(x, z) as f64, z as f64])
            })
            .make(move |[x, y, z], cell| {
                let (at, mark) = ([x as i64, 0, z as i64], cell.mark);
                let (height, room) = (f.height(at[0], at[2], mark), Rc::new(rooms.room(at, mark)));
                Some(grow(SPECIES[cell.mark], x, y, z, height, room, &mut cell.random("tree"), &|x, z| f.ground(x, z)))
            })
            .on(on.clone())
            .far(canopy.about(on.clone(), 0, TREE_TOP))
    };

    // Boulders and fallen logs.
    let rock_cell = vox(4.0) as i64;
    let on_ground = |name, reach| {
        let anchor = f.clone();
        Population::new(name, 2.0, rock_cell)
            .reach(reach)
            .size(vox(2.0) as i64)
            .on(on.clone())
            .anchor_along(1, move |line| Some(anchor.spot(line)))
    };
    let boulders = {
        let f = f.clone();
        on_ground("boulders", vox(3.0) as i64).make(move |[x, y, z], cell| {
            let mut random = cell.random("boulder");
            let (xi, zi) = (x as i64, z as i64);
            let clear = !f.ground.path.by(xi, zi, Fx::m(PATH_EDGE[1] + BOULDER_RADIUS[1]));
            if clear && random.chance(f.thinned(fx(0.15), xi, zi)) { Some(Node::of(boulder(x, y, z, &mut random))) } else { None }
        })
    };
    let logs = {
        let f = f.clone();
        on_ground("logs", vox(10.0) as i64).make(move |[x, _, z], cell| {
            let mut random = cell.random("log");
            if random.chance(f.thinned(fx(0.077), x as i64, z as i64)) && !f.column(x as i64, z as i64).on_path {
                fallen_log(x, z, &|x, z| f.ground(x, z), &mut random).map(Node::of)
            } else {
                None
            }
        })
    };

    // Undergrowth, per column of (0.25 m)², in square patches N wide around each cell's centre.
    const PLANTS: i64 = 16;
    // The most of a column's roll any plant takes, fallen leaves': a column rolling more draws nothing, whatever the share.
    const LITTER: Fx = fx(0.4);
    let column_width = vox(0.25) as i64;
    let anchor = f.clone();
    let plants = Population::new("plants", 3.0, PLANTS)
        .reach([0, vox(10.0) as i64, 0])
        .size(column_width)
        .on(on.clone())
        .anchor_along(1, move |line| {
            let [cx, cz] = line.centre;
            Some([cx as f64, anchor.ground(cx, cz) as f64 + 0.5, cz as f64])
        })
        .make(move |_, cell| {
            let [cx, _, cz] = cell.centre;
            let mut shapes = Vec::new();
            let span = |c: i64| (c - PLANTS / 2..c + PLANTS / 2).step_by(column_width as usize);
            for (xi, zi) in span(cx).flat_map(|xi| span(cz).map(move |zi| (xi, zi))) {
                let r = Fx::fraction(hash(f.s("plants"), xi, zi, 0));
                if r >= LITTER {
                    continue;
                }
                // Rarer as the forest's share falls: as dense as it is.
                let share = (f.ground.share)(xi, zi);
                let r = if share >= Fx::ONE { r } else { r / share.max(Fx::EPSILON) };
                let c = f.column(xi, zi);
                let (x, y, z) = (xi as f64, (c.height + 1) as f64, zi as f64);
                let random = &mut Rng::new(hash64(f.s("plants, randomly"), xi, zi, 0));
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
                    } else if c.surface == K::litter && r < LITTER {
                        shapes.push(Shape::cuboid([x - 1.0, y - 1.0, z - 1.0], [x + 1.0, y, z + 1.0], K::fallenLeaf).paint());
                    }
                }
            }
            if shapes.is_empty() { None } else { Some(Node::of(shapes)) }
        });

    vec![Layer::Field(terrain), Layer::Population(trees), Layer::Population(boulders), Layer::Population(logs), Layer::Population(plants)]
}

#[cfg(test)]
mod tests {
    use super::*;
    use worldgen::cost;
    use worldgen::generator::Shadows;
    use worldgen::points::Spot;

    /** The spacings SAMPLES holds: those of levels 4 to 12, past any a host draws. */
    const SAMPLED: std::ops::RangeInclusive<u32> = 4..=12;

    /**
     * 64 trees of `species` in a stand of them as dense as `chance` (of one in
     * a square 4 m wide), each in its room, the nearest the origin; alone, if
     * the chance is none.
     */
    fn stand_of(species: Species, chance: f64, seed: u64) -> Vec<Node> {
        if chance == 0.0 {
            return (0..64).map(|i| make_tree(species, 0.0, 0.0, 0.0, &mut Rng::new(hash64(seed, i, 0, 0)), &|_, _| 0)).collect();
        }
        let tall = move |s: &Spot| height(species, unit(hash(seed, s.at[0], s.at[2], 0)));
        let points =
            Rc::new(Points::new("stand", seed, &[0, 2], m(2.0), m(8.0), 1, move |_| [fx(chance / 4.0), Fx::ZERO, Fx::ZERO, Fx::ZERO]));
        let rooms = Rooms::new(points.clone(), TREE_REACH, MOST, GAPS, move |s| weight(tall(s)));
        let half = m(64.0);
        let mut spots = points.points_in([-half, 0, -half], [half, 1, half]);
        spots.sort_by_key(|s| (s.at[0] * s.at[0] + s.at[2] * s.at[2], s.at, s.nth));
        let grown = |s: &Spot| {
            let ([x, _, z], random) = (s.at, &mut Rng::new(hash64(seed, s.at[0], s.at[2], 1)));
            grow(species, x as f64, 0.0, z as f64, tall(s), Rc::new(rooms.room(s.at, 0)), random, &|_, _| 0)
        };
        spots.iter().take(64).map(grown).collect()
    }

    /**
     * What the trees of each of STANDS hide, by species, measured from
     * samples drawn at level 2 (the canopy's from level 4 on): the species'
     * own, the same in every world.
     */
    fn stands() -> Vec<[Shadows; 3]> {
        let samples = Scope::new(0).of("forest").seed("stands");
        cost::aside(|| {
            (0..STANDS.len())
                .map(|stand| {
                    SPECIES.map(|species| {
                        let seed = hash64(samples, stand as i64, species as i64, 0);
                        Shadows::new(&stand_of(species, STANDS[stand], seed), 2, 1)
                    })
                })
                .collect()
        })
    }

    /** The stands measured at each spacing SAMPLED holds. */
    fn measured() -> Vec<(i64, Sampled)> {
        let stands = stands();
        SAMPLED
            .map(|level| {
                let spacing = 1i64 << level;
                let slabs_of = |shadows: &[Shadows; 3], of: &dyn Fn(&Shadows, i64, i64) -> f64| {
                    [0, 1, 2].map(|sp| {
                        (0..TREE_TOP).step_by(spacing as usize).map(|from| of(&shadows[sp], from, (from + spacing).min(TREE_TOP))).collect()
                    })
                };
                let sampled = Sampled {
                    slabs: stands.iter().map(|shadows| slabs_of(shadows, &|s, from, to| s.slab(from, to))).collect(),
                    wood: slabs_of(&stands[0], &|s, from, to| s.share(from, to, |kind| KINDS[kind as usize].kind_type == 4)),
                    across: slabs_of(&stands[0], &|s, from, to| s.across(from, to)),
                    spread: [0, 1, 2].map(|sp| stands[0][sp].spread(spacing).into_iter().map(|([x, _, z], area)| ([x, z], area)).collect()),
                };
                (spacing, sampled)
            })
            .collect()
    }

    /** As SAMPLES says (`{:?}` writes each number exactly). */
    fn write_samples(samples: &[(i64, Sampled)]) -> String {
        let line = |values: &[f64]| values.iter().map(|v| format!("{v:?}")).collect::<Vec<_>>().join(" ") + "\n";
        let mut text = String::new();
        for (spacing, Sampled { slabs, wood, across, spread }) in samples {
            text += &format!("{spacing}\n");
            for series in slabs.iter().flatten().chain(wood).chain(across) {
                text += &line(series);
            }
            for spread in spread {
                text += &line(&spread.iter().flat_map(|([x, z], area)| [*x as f64, *z as f64, *area]).collect::<Vec<_>>());
            }
        }
        text
    }

    /** If not, `cargo test --release keep_samples -- --ignored` in worlds/ writes them anew. */
    #[test]
    fn kept_samples_are_measured() {
        let measured = measured();
        assert!(read_samples(SAMPLES) == measured.into_iter().collect(), "the sample stands aren't as kept; see keep_samples");
    }

    /** Writes SAMPLES' file. */
    #[test]
    #[ignore]
    fn keep_samples() {
        std::fs::write(concat!(env!("CARGO_MANIFEST_DIR"), "/data/forest-stands.txt"), write_samples(&measured())).expect("written");
    }
}
