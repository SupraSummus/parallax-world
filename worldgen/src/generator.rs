//! World generators (generator.ts): a world is fields and populations of
//! things made of shapes; the framework draws any box of it alone, at any
//! level of detail. See docs/generators.md. Numbers follow the TS step for
//! step (js.rs), so a world comes out the same site for site.

use std::cell::OnceCell;
use std::rc::Rc;

use crate::grid::{Box3, VoxelGrid};
use crate::js;
use crate::noise::{Rng, hash};

/** A material kind: its index in Generator::kinds. */
pub type Kind = u8;
/** A point in lattice units. */
pub type Point = [f64; 3];

/** How something meets what is drawn before it: fill empty sites only, paint over them, or carve them empty. */
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Op {
    Fill,
    Paint,
    Carve,
}

/** The chance to keep each site of a shape at level 0: a number, or one per position. */
#[derive(Clone)]
pub enum Fill {
    Const(f64),
    At(Rc<dyn Fn(f64, f64, f64) -> f64>),
}

impl Fill {
    fn at(&self, x: f64, y: f64, z: f64) -> f64 {
        match self {
            Fill::Const(f) => *f,
            Fill::At(f) => f(x, y, z),
        }
    }
}

pub const SOLID: Fill = Fill::Const(1.0);

#[derive(Clone)]
enum Solid {
    Ellipsoid { centre: Point, radii: Point },
    Capsule { p: Point, d: [f64; 3], length2: f64, radius: f64 },
    Lathe { p: Point, d: [f64; 3], length2: f64, length: f64, widest: f64, radius: Rc<dyn Fn(f64) -> f64> },
    Cuboid { min: Point, max: Point, size: [f64; 3] },
}

/** A solid, in lattice units (Shape in generator.ts). */
#[derive(Clone)]
pub struct Shape {
    pub kind: Kind,
    pub op: Op,
    pub fill: Fill,
    /** Every site it draws at level 0 is inside. */
    pub bounds: Box3,
    /** Its sizes along its own three axes. */
    pub extents: [f64; 3],
    pub centre: Point,
    /** Surface area of an opaque body that stops as many rays. */
    pub area: f64,
    solid: Solid,
}

/** A thing: its shapes, and finer things in it (Node in generator.ts). */
pub struct Node {
    pub shapes: Shapes,
    /** Required with children: holds everything the node and its children draw. */
    pub bounds: Option<Box3>,
    pub children: Option<Children>,
    pub far: Option<Far>,
}

pub struct Far {
    pub detail: f64,
    pub shapes: Vec<Shape>,
}

/** A node's finer things: the same for any box, or those reaching the box asked for (lattice units). */
pub enum Children {
    Fixed(Vec<Child>),
    Reaching(Box<ChildrenReaching>),
}

/** A node's finer thing: drawn while sites are at most `size` apart. */
pub struct Child {
    pub name: String,
    /** In lattice units; ANY_SIZE for any spacing. */
    pub size: i64,
    pub node: Node,
}

/**
 * A node's shapes: given, or made only once asked for, from their own
 * random sequence (`lazily` in forest.ts).
 */
pub enum Shapes {
    Now(Vec<Shape>),
    Lazy { seed: f64, make: Box<MakeShapes>, made: OnceCell<Vec<Shape>> },
}

type ChildrenReaching = dyn Fn(&Box3) -> Vec<Child>;
type MakeShapes = dyn Fn(&mut Rng) -> Vec<Shape>;
type FieldAt = dyn Fn(i64, i64, i64, i64) -> Option<(Kind, Option<f64>)>;
type FieldSpans = dyn Fn(i64, i64, i64) -> Vec<Span>;
type Anchor = dyn Fn(&Cell) -> Option<Point>;
type Make = dyn Fn(Point, &Cell) -> Option<Node>;

impl Shapes {
    pub fn lazy(seed: f64, make: impl Fn(&mut Rng) -> Vec<Shape> + 'static) -> Shapes {
        Shapes::Lazy { seed, make: Box::new(make), made: OnceCell::new() }
    }

    pub fn get(&self) -> &[Shape] {
        match self {
            Shapes::Now(shapes) => shapes,
            Shapes::Lazy { seed, make, made } => made.get_or_init(|| make(&mut Rng::new(*seed))),
        }
    }
}

impl Node {
    pub fn of(shapes: Vec<Shape>) -> Node {
        Node { shapes: Shapes::Now(shapes), bounds: None, children: None, far: None }
    }
}

/** Part of a line of sites: [from, to) along the line, solid, or a medium if it has `extinction`. */
pub struct Span {
    pub from: i64,
    pub to: i64,
    pub kind: Kind,
    pub extinction: Option<f64>,
}

pub enum FieldValues {
    /** A kind, with a medium's extinction if it is one, at each site; given the level's spacing. */
    At(Box<FieldAt>),
    /** Spans along each line of sites parallel to `axis`, at (u, v) the other two coordinates, in order. */
    Spans { axis: usize, spans: Box<FieldSpans> },
}

/** Something everywhere, given by position alone (Field in generator.ts). */
pub struct Field {
    pub name: &'static str,
    pub order: f64,
    pub op: Op,
    pub values: FieldValues,
}

/** A cell of a population (Cell in generator.ts). */
pub struct Cell<'a> {
    pub i: i64,
    pub j: i64,
    pub k: i64,
    pub centre: [i64; 3],
    population: &'a str,
    seed: f64,
}

impl Cell<'_> {
    /** A random sequence that depends on the cell and `stream` only. */
    pub fn random(&self, stream: &str) -> Rng {
        Rng::new(
            (hash(hash_text(&format!("{}:{stream}", self.population), self.seed) as f64, self.i as f64, self.j as f64, self.k as f64)
                * 4294967296.0)
                .floor(),
        )
    }
}

/** Things placed in the cells of N·FCC, at most one per cell (Population in generator.ts). */
pub struct Population {
    pub name: &'static str,
    pub order: f64,
    pub cell: i64,
    /** How far a thing reaches past N from its cell's centre, per axis. */
    pub reach: [i64; 3],
    /** Things are made while sites are at most this far apart (ANY_SIZE: always). */
    pub size: i64,
    pub anchor: Box<Anchor>,
    pub make: Box<Make>,
    pub far: Option<Field>,
}

pub enum Layer {
    Field(Field),
    Population(Population),
}

pub struct KindDef {
    pub kind_type: u8,
    pub colors: &'static [&'static str],
}

pub struct Generator {
    pub kinds: &'static [KindDef],
    pub seed: f64,
    pub layers: Vec<Layer>,
}

const DOCS: &str = "docs/generators.md";
/** A size no spacing exceeds. */
pub const ANY_SIZE: i64 = i64::MAX;
/** A voxel's surface area at level 0, and its mean chord (Cauchy: 4 · volume / area). */
const VOXEL_AREA: f64 = 6.0 * std::f64::consts::SQRT_2;
const CHORD: f64 = (4.0 * 2.0) / VOXEL_AREA;
const PI: f64 = std::f64::consts::PI;

// ---------------------------------------------------------------------------
// Shapes

/** The area of an opaque body stopping as many rays as one of `area` and mean chord `chord`, filled with chance `fill`. */
fn effective_area(area: f64, chord: f64, fill: &Fill, centre: Point) -> f64 {
    let f = fill.at(centre[0], centre[1], centre[2]);
    area * (1.0 - js::pow(1.0 - f, chord / CHORD))
}

pub fn ellipsoid(centre: Point, radii: Point, kind: Kind, fill: Fill, op: Op) -> Shape {
    let [cx, cy, cz] = centre;
    let [a, b, c] = radii;
    let p = 1.6075; // Knud Thomsen's approximation of the surface area
    let area = 4.0 * PI * js::pow((js::pow(a * b, p) + js::pow(a * c, p) + js::pow(b * c, p)) / 3.0, 1.0 / p);
    Shape {
        kind,
        op,
        centre,
        bounds: Box3 {
            min: [(cx - a).ceil(), (cy - b).ceil(), (cz - c).ceil()].map(|v| v as i64),
            max: [(cx + a).floor(), (cy + b).floor(), (cz + c).floor()].map(|v| v as i64 + 1),
        },
        extents: [2.0 * a, 2.0 * b, 2.0 * c],
        area: effective_area(area, ((16.0 / 3.0) * PI * a * b * c) / area, &fill, centre),
        fill,
        solid: Solid::Ellipsoid { centre, radii },
    }
}

pub fn ball(centre: Point, radius: f64, kind: Kind, fill: Fill, op: Op) -> Shape {
    ellipsoid(centre, [radius, radius, radius], kind, fill, op)
}

/** The points within `radius` of the segment from p to q: branches, logs. */
pub fn capsule(p: Point, q: Point, radius: f64, kind: Kind, fill: Fill, op: Op) -> Shape {
    let d = [q[0] - p[0], q[1] - p[1], q[2] - p[2]];
    let length2 = d[0] * d[0] + d[1] * d[1] + d[2] * d[2];
    let length = length2.sqrt();
    let area = 2.0 * PI * radius * length + 4.0 * PI * (radius * radius);
    let volume = PI * (radius * radius) * length + (4.0 / 3.0) * PI * js::pow(radius, 3.0);
    let centre = [(p[0] + q[0]) / 2.0, (p[1] + q[1]) / 2.0, (p[2] + q[2]) / 2.0];
    let lo: [i64; 3] = std::array::from_fn(|i| (p[i].min(q[i]) - radius).ceil() as i64);
    let hi: [i64; 3] = std::array::from_fn(|i| (p[i].max(q[i]) + radius).floor() as i64 + 1);
    Shape {
        kind,
        op,
        centre,
        bounds: Box3 { min: lo, max: hi },
        extents: [length + 2.0 * radius, 2.0 * radius, 2.0 * radius],
        area: effective_area(area, (4.0 * volume) / area, &fill, centre),
        fill,
        solid: Solid::Capsule { p, d, length2, radius },
    }
}

/** A solid of revolution: the points within `radius(t)` of the segment from p to q, t from 0 at p to 1 at q. */
pub fn lathe(p: Point, q: Point, radius: impl Fn(f64) -> f64 + 'static, kind: Kind, fill: Fill, op: Op) -> Shape {
    let d = [q[0] - p[0], q[1] - p[1], q[2] - p[2]];
    let length2 = d[0] * d[0] + d[1] * d[1] + d[2] * d[2];
    let length = length2.sqrt();
    // Area and volume by slices; the area is the convex hull's (see generator.ts).
    let n = 8f64.max(length.ceil()) as usize;
    let r: Vec<f64> = (0..=n).map(|i| radius(i as f64 / n as f64)).collect();
    let mut hull: Vec<usize> = Vec::new();
    for i in 0..=n {
        let cross = |a: usize, b: usize| (b as f64 - a as f64) * (r[i] - r[a]) - (i as f64 - a as f64) * (r[b] - r[a]);
        while hull.len() >= 2 && cross(hull[hull.len() - 2], hull[hull.len() - 1]) >= 0.0 {
            hull.pop();
        }
        hull.push(i);
    }
    let mut area = PI * (r[0] * r[0] + r[n] * r[n]);
    let mut volume = 0.0;
    for h in 0..hull.len().saturating_sub(1) {
        let (a, b) = (hull[h], hull[h + 1]);
        area += PI * (r[a] + r[b]) * js::hypot(&[((b - a) as f64 * length) / n as f64, r[b] - r[a]]);
    }
    for i in 0..n {
        volume += (PI * (r[i] * r[i] + r[i + 1] * r[i + 1]) / 2.0) * (length / n as f64);
    }
    let widest = js::max(r.iter().copied());
    let centre = [(p[0] + q[0]) / 2.0, (p[1] + q[1]) / 2.0, (p[2] + q[2]) / 2.0];
    let out = d.map(|v| widest * 0f64.max(1.0 - (v * v) / length2).sqrt());
    let lo: [i64; 3] = std::array::from_fn(|i| (p[i].min(q[i]) - out[i]).ceil() as i64);
    let hi: [i64; 3] = std::array::from_fn(|i| (p[i].max(q[i]) + out[i]).floor() as i64 + 1);
    Shape {
        kind,
        op,
        centre,
        bounds: Box3 { min: lo, max: hi },
        extents: [length, 2.0 * widest, 2.0 * widest],
        area: effective_area(area, (4.0 * volume) / area, &fill, centre),
        fill,
        solid: Solid::Lathe { p, d, length2, length, widest, radius: Rc::new(radius) },
    }
}

/** The points of [min, max): at level 0, the sites with min ≤ site < max. */
pub fn cuboid(min: Point, max: Point, kind: Kind, fill: Fill, op: Op) -> Shape {
    let size = [max[0] - min[0], max[1] - min[1], max[2] - min[2]];
    let area = 2.0 * (size[0] * size[1] + size[1] * size[2] + size[0] * size[2]);
    let centre = [(min[0] + max[0]) / 2.0, (min[1] + max[1]) / 2.0, (min[2] + max[2]) / 2.0];
    Shape {
        kind,
        op,
        centre,
        bounds: Box3 { min: min.map(|v| v.ceil() as i64), max: max.map(|v| v.ceil() as i64) },
        extents: size,
        area: effective_area(area, (4.0 * size[0] * size[1] * size[2]) / area, &fill, centre),
        fill,
        solid: Solid::Cuboid { min, max, size },
    }
}

impl Shape {
    /** Whether a point is inside the shape with each of its sizes made at least `min`. */
    pub fn contains(&self, x: f64, y: f64, z: f64, min: f64) -> bool {
        match &self.solid {
            Solid::Ellipsoid { centre: [cx, cy, cz], radii: [a, b, c] } => {
                let r = min / 2.0;
                let (u, v, w) = ((x - cx) / a.max(r), (y - cy) / b.max(r), (z - cz) / c.max(r));
                u * u + v * v + w * w <= 1.0
            }
            Solid::Capsule { p, d, length2, radius } => {
                let v = [x - p[0], y - p[1], z - p[2]];
                let t = if *length2 == 0.0 { 0.0 } else { 0f64.max(1f64.min((v[0] * d[0] + v[1] * d[1] + v[2] * d[2]) / length2)) };
                let (a, b, c) = (v[0] - t * d[0], v[1] - t * d[1], v[2] - t * d[2]);
                let r = radius.max(min / 2.0);
                a * a + b * b + c * c <= r * r
            }
            Solid::Lathe { p, d, length2, length, widest, radius } => {
                let (vx, vy, vz) = (x - p[0], y - p[1], z - p[2]);
                let t = (vx * d[0] + vy * d[1] + vz * d[2]) / length2;
                let reach = (length * 0f64.max(min - length)) / 2.0 / length2;
                if t < -reach || t > 1.0 + reach {
                    return false;
                }
                let c = 0f64.max(1f64.min(t));
                let (a, b, e) = (vx - c * d[0], vy - c * d[1], vz - c * d[2]);
                let away = a * a + b * b + e * e;
                let limit = widest.max(min / 2.0);
                if away > limit * limit {
                    return false;
                }
                let r = radius(c).max(min / 2.0);
                away <= r * r
            }
            Solid::Cuboid { min: lo, max: hi, size } => {
                let centre = self.centre;
                [x, y, z]
                    .iter()
                    .enumerate()
                    .all(|(i, v)| if size[i] >= min { lo[i] <= *v && *v < hi[i] } else { (v - centre[i]).abs() <= min / 2.0 })
            }
        }
    }
}

/**
 * Draws a shape at sites `s` apart, by how many of its sizes are under s
 * (drawShape in generator.ts).
 */
fn draw_shape(shape: &Shape, seed: f64, si: i64, clip: &Box3, put: &mut dyn FnMut(i64, i64, i64)) {
    let s = si as f64;
    let small = shape.extents.iter().filter(|e| **e < s).count();
    let kept =
        |x: i64, y: i64, z: i64, chance: f64| chance >= 1.0 || hash(seed, (x * si) as f64, (y * si) as f64, (z * si) as f64) < chance;
    if small == 3 {
        let [x, y, z] = nearest_site(shape.centre.map(|v| v / s));
        if clip.holds([x, y, z]) && kept(x, y, z, shape.area / (VOXEL_AREA * s * s)) {
            put(x, y, z);
        }
        return;
    }
    let min = [0.0, s, 1.6 * s][small];
    let b = &shape.bounds;
    let [x0, y0, z0] = b.min.map(|v| ((v as f64 - min / 2.0) / s).ceil() as i64);
    let [x1, y1, z1] = b.max.map(|v| ((v as f64 - 1.0 + min / 2.0) / s).floor() as i64);
    let sites = |within: &Box3, visit: &mut dyn FnMut(i64, i64, i64)| {
        let (z_from, z_to) = (z0.max(within.min[2]), z1.min(within.max[2] - 1));
        let (y_from, y_to) = (y0.max(within.min[1]), y1.min(within.max[1] - 1));
        let (from, x_to) = (x0.max(within.min[0]), x1.min(within.max[0] - 1));
        for z in z_from..=z_to {
            for y in y_from..=y_to {
                let mut x = from + ((from + y + z) & 1);
                while x <= x_to {
                    if shape.contains((x * si) as f64, (y * si) as f64, (z * si) as f64, min) {
                        visit(x, y, z);
                    }
                    x += 2;
                }
            }
        }
    };
    let fill = |x: i64, y: i64, z: i64| shape.fill.at((x * si) as f64, (y * si) as f64, (z * si) as f64);
    match small {
        0 => sites(clip, &mut |x, y, z| {
            if kept(x, y, z, 1.0 - js::pow(1.0 - fill(x, y, z), s)) {
                put(x, y, z)
            }
        }),
        1 => {
            let thickness = js::min(shape.extents);
            sites(clip, &mut |x, y, z| {
                if kept(x, y, z, 1.0 - js::pow(1.0 - fill(x, y, z), thickness / CHORD)) {
                    put(x, y, z)
                }
            })
        }
        _ => {
            // Counted whole, so every box gets the same chance.
            let mut count = 0;
            sites(&EVERYWHERE, &mut |_, _, _| count += 1);
            let chance = shape.area / (count as f64 * VOXEL_AREA * s * s);
            sites(clip, &mut |x, y, z| {
                if kept(x, y, z, chance) {
                    put(x, y, z)
                }
            })
        }
    }
}

// ---------------------------------------------------------------------------
// Generating

#[derive(Clone, Debug, PartialEq)]
pub struct Material {
    pub color: &'static str,
    pub material_type: u8,
}

/** Shades of the kinds' colours, as material ids picked by a hash of the position in lattice units. */
struct Palette {
    materials: Vec<Material>,
    shades: Vec<Vec<u8>>,
    seed: f64,
}

impl Palette {
    fn new(kinds: &[KindDef], seed: f64) -> Palette {
        let mut materials = vec![Material { color: "", material_type: 0 }];
        let shades = kinds
            .iter()
            .map(|k| {
                k.colors
                    .iter()
                    .map(|color| {
                        materials.push(Material { color, material_type: k.kind_type });
                        (materials.len() - 1) as u8
                    })
                    .collect()
            })
            .collect();
        Palette { materials, shades, seed }
    }

    fn shade(&self, kind: Kind, x: i64, y: i64, z: i64) -> u8 {
        let shades = &self.shades[kind as usize];
        shades[(hash(self.seed, x as f64, y as f64, z as f64) * shades.len() as f64).floor() as usize]
    }
}

/** The materials of the grids `generate` makes: ids are the same in all of them. */
pub fn materials_of(generator: &Generator) -> Vec<Material> {
    Palette::new(generator.kinds, generator.seed).materials
}

/**
 * The voxels of `b` at `level`: sites 2^level apart, site (x, y, z) at
 * 2^level · (x, y, z) in lattice units, the box in the level's units. The
 * grid may reach one site further in x (its rows start and end on an even x).
 */
pub fn generate(generator: &Generator, b: &Box3, level: u32) -> VoxelGrid {
    let s = 1i64 << level;
    let palette = Palette::new(generator.kinds, generator.seed);
    let mut grid = VoxelGrid::new(b);
    let clip = grid.clip();
    let mut put = |kind: Kind, op: Op, x: i64, y: i64, z: i64| {
        if op == Op::Fill && grid.get(x, y, z) != 0 {
            return;
        }
        let id = if op == Op::Carve { 0 } else { palette.shade(kind, x * s, y * s, z * s) };
        grid.set(x, y, z, id);
    };
    for drawable in drawables_in(generator, &clip, s) {
        match drawable.draw {
            Draw::Field(field) => draw_field(field, generator.seed, s, &clip, &mut put),
            Draw::Shapes(shapes, far) => {
                for (i, shape) in shapes.iter().enumerate() {
                    let seed = hash_text(&format!("{}#{}{i}", drawable.key, if far { "far" } else { "" }), generator.seed) as f64;
                    draw_shape(shape, seed, s, &clip, &mut |x, y, z| put(shape.kind, shape.op, x, y, z));
                }
            }
        }
    }
    grid
}

enum Draw<'a> {
    Field(&'a Field),
    /** A node's shapes, and whether they are its far ones. */
    Shapes(Vec<Shape>, bool),
}

struct Drawable<'a> {
    order: f64,
    key: String,
    draw: Draw<'a>,
}

/** What draws into `clip` (sites s apart), in drawing order: by layer, then key. */
fn drawables_in<'a>(generator: &'a Generator, clip: &Box3, s: i64) -> Vec<Drawable<'a>> {
    // The lattice units the sites of `clip` cover, with what a shape may grow by at this level.
    let units = Box3 { min: clip.min.map(|v| v * s), max: clip.max.map(|v| (v - 1) * s + 1) }.widen(s);
    let mut drawables = Vec::new();
    for layer in &generator.layers {
        let population = match layer {
            Layer::Field(field) => {
                drawables.push(Drawable { order: field.order, key: field.name.to_string(), draw: Draw::Field(field) });
                continue;
            }
            Layer::Population(p) => p,
        };
        if s > population.size {
            if let Some(far) = &population.far {
                drawables.push(Drawable { order: population.order, key: population.name.to_string(), draw: Draw::Field(far) });
            }
            continue;
        }
        fn visit<'a>(node: &Node, key: String, bounds: Box3, s: i64, units: &Box3, order: f64, drawables: &mut Vec<Drawable<'a>>) {
            if !bounds.overlaps(units) {
                return;
            }
            let far = node.far.as_ref().filter(|far| s as f64 > far.detail);
            let shapes = far.map_or_else(|| node.shapes.get(), |far| &far.shapes);
            drawables.push(Drawable { order, key: key.clone(), draw: Draw::Shapes(shapes.to_vec(), far.is_some()) });
            if far.is_some() {
                return;
            }
            let reaching;
            let children: &[Child] = match &node.children {
                None => &[],
                Some(Children::Fixed(children)) => children,
                Some(Children::Reaching(children)) => {
                    reaching = children(units);
                    &reaching
                }
            };
            let mut names = std::collections::HashSet::new();
            for child in children {
                assert!(names.insert(&child.name), "{key} has two children named {}; see \"Things\" in {DOCS}", child.name);
                let child_bounds = bounds_of(&child.node);
                assert!(child_bounds.inside(&bounds), "{key}/{} reaches past its parent; see \"Things\" in {DOCS}", child.name);
                if s <= child.size {
                    visit(&child.node, format!("{key}/{}", child.name), child_bounds, s, units, order, drawables);
                }
            }
        }
        let n = population.cell;
        let [rx, ry, rz] = population.reach;
        // Cells whose reach meets `units`: centres within N + reach of it.
        let range =
            |axis: usize, reach: i64| (-(-(units.min[axis] - n - reach)).div_euclid(n), (units.max[axis] - 1 + n + reach).div_euclid(n));
        let ((i0, i1), (j0, j1), (k0, k1)) = (range(0, rx), range(1, ry), range(2, rz));
        for i in i0..=i1 {
            for j in j0..=j1 {
                let mut k = k0 + ((k0 + i + j) & 1);
                while k <= k1 {
                    let centre = [i * n, j * n, k * n];
                    let cell = Cell { i, j, k, centre, population: population.name, seed: generator.seed };
                    k += 2;
                    let Some(anchor) = (population.anchor)(&cell) else { continue };
                    let nf = n as f64;
                    let [oi, oj, ok] = nearest_site([anchor[0] / nf, anchor[1] / nf, anchor[2] / nf]);
                    if oi != i || oj != j || ok != k - 2 {
                        continue;
                    }
                    let Some(node) = (population.make)(anchor, &cell) else { continue };
                    let key = format!("{} {i},{j},{}", population.name, k - 2);
                    let bounds = bounds_of(&node);
                    let reach = Box3 {
                        min: [centre[0] - n - rx, centre[1] - n - ry, centre[2] - n - rz],
                        max: [centre[0] + n + rx, centre[1] + n + ry, centre[2] + n + rz],
                    };
                    assert!(bounds.inside(&reach), "{key} reaches past its population's reach; see \"Populations\" in {DOCS}");
                    visit(&node, key, bounds, s, &units, population.order, &mut drawables);
                }
            }
        }
    }
    drawables.sort_by(|a, b| a.order.total_cmp(&b.order).then_with(|| a.key.cmp(&b.key)));
    drawables
}

/** Draws a field into `clip`; a medium keeps a site with the chance a ray crossing it is stopped. */
fn draw_field(field: &Field, seed: f64, s: i64, clip: &Box3, put: &mut dyn FnMut(Kind, Op, i64, i64, i64)) {
    let op = field.op;
    let field_seed = hash_text(field.name, seed) as f64;
    let kept = |x: i64, y: i64, z: i64, extinction: Option<f64>| {
        extinction.is_none_or(|e| hash(field_seed, (x * s) as f64, (y * s) as f64, (z * s) as f64) < 1.0 - js::exp(-e * CHORD * s as f64))
    };
    match &field.values {
        FieldValues::At(at) => {
            for z in clip.min[2]..clip.max[2] {
                for y in clip.min[1]..clip.max[1] {
                    let mut x = clip.min[0] + ((clip.min[0] + y + z) & 1);
                    while x < clip.max[0] {
                        if let Some((kind, extinction)) = at(x * s, y * s, z * s, s)
                            && kept(x, y, z, extinction)
                        {
                            put(kind, op, x, y, z);
                        }
                        x += 2;
                    }
                }
            }
        }
        FieldValues::Spans { axis, spans } => {
            let axis = *axis;
            let (lo, hi) = (clip.min, clip.max);
            let others: Vec<usize> = (0..3).filter(|i| *i != axis).collect();
            let (a, b) = (others[0], others[1]);
            let mut site = [0i64; 3];
            for u in lo[a]..hi[a] {
                for v in lo[b]..hi[b] {
                    site[a] = u;
                    site[b] = v;
                    for span in spans(u * s, v * s, s) {
                        let first = (-(-span.from).div_euclid(s)).max(lo[axis]);
                        let mut w = first + ((first + u + v) & 1);
                        while w * s < span.to && w < hi[axis] {
                            site[axis] = w;
                            if kept(site[0], site[1], site[2], span.extinction) {
                                put(span.kind, op, site[0], site[1], site[2]);
                            }
                            w += 2;
                        }
                    }
                }
            }
        }
    }
}

/**
 * The mean area per unit length of `nodes` along `axis`, as a function of
 * the coordinate: for a population's far form, from things made at the origin.
 */
pub fn area_profile(nodes: &[Node], axis: usize) -> impl Fn(i64) -> f64 + use<> {
    let mut profile = std::collections::BTreeMap::<i64, f64>::new();
    for node in nodes {
        let shapes = node.far.as_ref().map_or_else(|| node.shapes.get(), |far| &far.shapes);
        for shape in shapes {
            let (lo, hi) = (shape.bounds.min[axis], shape.bounds.max[axis]);
            for at in lo..hi {
                *profile.entry(at).or_insert(0.0) += shape.area / (hi - lo) as f64 / nodes.len() as f64;
            }
        }
    }
    move |at: i64| profile.get(&at).copied().unwrap_or(0.0)
}

// ---------------------------------------------------------------------------
// Helpers

const EVERYWHERE: Box3 = Box3 { min: [i64::MIN / 4; 3], max: [i64::MAX / 4; 3] };

/** The site nearest to a point: the one whose voxel holds it. */
fn nearest_site(p: [f64; 3]) -> [i64; 3] {
    let mut r = p.map(js::round);
    if (r[0] as i64 + r[1] as i64 + r[2] as i64) & 1 != 0 {
        // Round the coordinate furthest from its integer the other way.
        let [e0, e1, e2] = [(p[0] - r[0]).abs(), (p[1] - r[1]).abs(), (p[2] - r[2]).abs()];
        let i = if e0 >= e1 && e0 >= e2 {
            0
        } else if e1 >= e2 {
            1
        } else {
            2
        };
        r[i] += if p[i] > r[i] { 1.0 } else { -1.0 };
    }
    r.map(|v| v as i64)
}

fn bounds_of(node: &Node) -> Box3 {
    if let Some(bounds) = node.bounds {
        return bounds;
    }
    assert!(node.children.is_none(), "a node with children needs bounds; see \"Things\" in {DOCS}");
    bounds_of_shapes(node.shapes.get().iter().map(|s| &s.bounds))
}

/** The box holding the boxes. */
pub fn bounds_of_shapes<'a>(boxes: impl IntoIterator<Item = &'a Box3>) -> Box3 {
    let mut b = Box3 { min: EVERYWHERE.max, max: EVERYWHERE.min };
    for s in boxes {
        for i in 0..3 {
            b.min[i] = b.min[i].min(s.min[i]);
            b.max[i] = b.max[i].max(s.max[i]);
        }
    }
    b
}

/** A seed from a text and a seed. */
pub fn hash_text(text: &str, seed: f64) -> i32 {
    let mut h = js::int32(seed);
    for c in text.encode_utf16() {
        h = (h ^ c as i32).wrapping_mul(16777619);
    }
    h
}
