//! World generators: a world is fields and populations of things made of
//! shapes; the framework draws any box of it alone, at any level of
//! detail. See docs/generators.md.

use std::cell::OnceCell;
use std::rc::Rc;

use crate::grid::{Box3, VoxelGrid};
use crate::js;
use crate::noise::{Fixed, ONE, Rng, fixed, float, hash, hash_text, hash64, unit};

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

/** A point in lattice units, fixed point. */
type At = [Fixed; 3];

#[derive(Clone)]
enum Solid {
    Ellipsoid { centre: At, radii: At },
    Capsule { p: At, d: At, radius: Fixed },
    Lathe { p: At, d: At, length: Fixed, widest: Fixed, radius: Rc<dyn Fn(f64) -> f64> },
    Cuboid { min: At, max: At },
    Gable { min: At, max: At, ridge: usize },
}

/** A solid, in lattice units: given in floats, kept in fixed point. */
#[derive(Clone)]
pub struct Shape {
    pub kind: Kind,
    pub op: Op,
    pub fill: Fill,
    /** Every site it draws at level 0 is inside. */
    pub bounds: Box3,
    /** Its sizes along its own three axes. */
    pub extents: [Fixed; 3],
    centre: At,
    /** Surface area of an opaque body that stops as many rays. */
    pub area: f64,
    solid: Solid,
}

/** A thing: its shapes, and finer things in it. */
pub struct Node {
    pub shapes: Shapes,
    /** Required with children: holds everything the node and its children draw. */
    pub bounds: Option<Box3>,
    pub children: Option<Children>,
    pub far: Option<Far>,
}

/** How a node is drawn while sites are further apart than `detail` (fixed point). */
pub struct Far {
    pub detail: Fixed,
    pub form: FarForm,
}

pub enum FarForm {
    Shapes(Vec<Shape>),
    /**
     * Its near form, drawn whole at the coarsest level it is drawn at
     * and coarsened: each site kept with the chance a ray crossing it is
     * stopped, of a kind in proportion to what shows on its sides.
     */
    Measured,
}

impl Far {
    pub fn of(detail: Fixed, shapes: Vec<Shape>) -> Far {
        Far { detail, form: FarForm::Shapes(shapes) }
    }

    pub fn measured(detail: Fixed) -> Far {
        assert!(detail >= ONE, "a measured far form is measured from a level its near form is drawn at; see \"Levels\" in {DOCS}");
        Far { detail, form: FarForm::Measured }
    }

    /** Its far shapes; none if measured. */
    pub fn shapes(&self) -> &[Shape] {
        match &self.form {
            FarForm::Shapes(shapes) => shapes,
            FarForm::Measured => &[],
        }
    }
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
 * random sequence.
 */
pub enum Shapes {
    Now(Vec<Shape>),
    Lazy { seed: u64, make: Box<MakeShapes>, made: OnceCell<Vec<Shape>> },
}

type ChildrenReaching = dyn Fn(&Box3) -> Vec<Child>;
type MakeShapes = dyn Fn(&mut Rng) -> Vec<Shape>;
type FieldAt = dyn Fn(i64, i64, i64, i64) -> Option<(Kind, Option<f64>)>;
type FieldSpans = dyn Fn(i64, i64, i64) -> Vec<Span>;
type Anchor = dyn Fn(&Cell) -> Option<Point>;
type Make = dyn Fn(Point, &Cell) -> Option<Node>;

impl Shapes {
    pub fn lazy(seed: u64, make: impl Fn(&mut Rng) -> Vec<Shape> + 'static) -> Shapes {
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

/** Something everywhere, given by position alone. */
pub struct Field {
    pub name: &'static str,
    pub order: f64,
    pub op: Op,
    pub values: FieldValues,
}

/** A cell of a population. */
pub struct Cell<'a> {
    pub i: i64,
    pub j: i64,
    pub k: i64,
    pub centre: [i64; 3],
    population: &'a str,
    seed: u64,
}

impl Cell<'_> {
    /** A random sequence that depends on the cell and `stream` only. */
    pub fn random(&self, stream: &str) -> Rng {
        Rng::new(hash64(hash_text(&format!("{}:{stream}", self.population), self.seed), self.i, self.j, self.k))
    }
}

/** Things placed in the cells of N·FCC, at most one per cell. */
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

#[derive(Clone)]
pub struct KindDef {
    pub kind_type: u8,
    pub colors: &'static [&'static str],
}

/**
 * A domain's kinds, after `base` others (see "Composing" in
 * docs/generators.md): `enum K` of their names, `KINDS` to append to a
 * generator's, and `k` for a name's Kind.
 */
#[macro_export]
macro_rules! kinds {
    (base: $base:expr; $($name:ident: $type:expr, [$($color:expr),*];)*) => {
        #[allow(non_camel_case_types, clippy::upper_case_acronyms)]
        #[derive(Clone, Copy, PartialEq, Eq, Default)]
        #[repr(u8)]
        pub enum K { #[default] $($name),* }
        pub const KINDS: &[$crate::generator::KindDef] = &[$($crate::generator::KindDef { kind_type: $type, colors: &[$($color),*] }),*];
        /** Kinds of the generator's before these. */
        pub const BASE: $crate::generator::Kind = $base;
        pub fn k(kind: K) -> $crate::generator::Kind {
            BASE + kind as $crate::generator::Kind
        }
    };
}

pub struct Generator {
    /** A world of domains has each's kinds, those it builds on first (see "Composing" in docs/generators.md). */
    pub kinds: Vec<KindDef>,
    pub seed: u64,
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

fn at(p: Point) -> At {
    p.map(fixed)
}

fn mid(p: Point, q: Point) -> Point {
    [0, 1, 2].map(|i| (p[i] + q[i]) / 2.0)
}

/** The first and last site of a level-0 box from min to max (fixed point), inclusive. */
fn sites_between(min: At, max: At) -> Box3 {
    Box3 { min: min.map(|v| -((-v) >> 16)), max: max.map(|v| (v >> 16) + 1) }
}

fn dot(a: At, b: At) -> i64 {
    (0..3).map(|i| a[i] * b[i]).sum()
}

pub fn ellipsoid(centre: Point, radii: Point, kind: Kind, fill: Fill, op: Op) -> Shape {
    let [a, b, c] = radii;
    let p = 1.6075; // Knud Thomsen's approximation of the surface area
    let area = 4.0 * PI * js::pow((js::pow(a * b, p) + js::pow(a * c, p) + js::pow(b * c, p)) / 3.0, 1.0 / p);
    let (centre_at, radii) = (at(centre), at(radii));
    Shape {
        kind,
        op,
        centre: centre_at,
        bounds: sites_between([0, 1, 2].map(|i| centre_at[i] - radii[i]), [0, 1, 2].map(|i| centre_at[i] + radii[i])),
        extents: radii.map(|r| 2 * r),
        area: effective_area(area, ((16.0 / 3.0) * PI * a * b * c) / area, &fill, centre),
        fill,
        solid: Solid::Ellipsoid { centre: centre_at, radii },
    }
}

pub fn ball(centre: Point, radius: f64, kind: Kind, fill: Fill, op: Op) -> Shape {
    ellipsoid(centre, [radius, radius, radius], kind, fill, op)
}

/** The segment from p to q in fixed point: its start, direction and length. */
fn segment(p: Point, q: Point) -> (At, At, Fixed) {
    let (p, q) = (at(p), at(q));
    let d = [0, 1, 2].map(|i| q[i] - p[i]);
    (p, d, (dot(d, d) as u64).isqrt() as Fixed)
}

/** The points within `radius` of the segment from p to q: branches, logs. */
pub fn capsule(p: Point, q: Point, radius: f64, kind: Kind, fill: Fill, op: Op) -> Shape {
    let (p_at, d, length) = segment(p, q);
    let r = fixed(radius);
    let (lengthf, rf) = (float(length), radius);
    let area = 2.0 * PI * rf * lengthf + 4.0 * PI * (rf * rf);
    let volume = PI * (rf * rf) * lengthf + (4.0 / 3.0) * PI * js::pow(rf, 3.0);
    let q_at = [0, 1, 2].map(|i| p_at[i] + d[i]);
    Shape {
        kind,
        op,
        centre: at(mid(p, q)),
        bounds: sites_between([0, 1, 2].map(|i| p_at[i].min(q_at[i]) - r), [0, 1, 2].map(|i| p_at[i].max(q_at[i]) + r)),
        extents: [length + 2 * r, 2 * r, 2 * r],
        area: effective_area(area, (4.0 * volume) / area, &fill, mid(p, q)),
        fill,
        solid: Solid::Capsule { p: p_at, d, radius: r },
    }
}

/** A solid of revolution: the points within `radius(t)` of the segment from p to q, t from 0 at p to 1 at q. */
pub fn lathe(p: Point, q: Point, radius: impl Fn(f64) -> f64 + 'static, kind: Kind, fill: Fill, op: Op) -> Shape {
    let (p_at, d, length_at) = segment(p, q);
    let length = float(length_at);
    // Area and volume by slices. The shadow it casts is its convex hull's, so the area is the hull's:
    // a crown's whorls have undersides that cast no more shadow (Cauchy holds for convex bodies).
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
    let widest = fixed(js::max(r.iter().copied()));
    // How far the widest circle reaches along each axis: widest · sin of the
    // angle to it, sin² with 22 bits after the point (l2 < 2^40), rounded up.
    let (d8, l2) = (d.map(|v| v >> 8), dot(d.map(|v| v >> 8), d.map(|v| v >> 8)));
    let out = d8.map(|v| {
        if l2 == 0 {
            return widest;
        }
        let sin2 = (((l2 - v * v) << 22) + l2 - 1) / l2;
        let sin = sin2.isqrt() + i64::from(sin2.isqrt().pow(2) < sin2);
        (widest * sin + 2047) >> 11
    });
    let q_at = [0, 1, 2].map(|i| p_at[i] + d[i]);
    Shape {
        kind,
        op,
        centre: at(mid(p, q)),
        bounds: sites_between([0, 1, 2].map(|i| p_at[i].min(q_at[i]) - out[i]), [0, 1, 2].map(|i| p_at[i].max(q_at[i]) + out[i])),
        extents: [length_at, 2 * widest, 2 * widest],
        area: effective_area(area, (4.0 * volume) / area, &fill, mid(p, q)),
        fill,
        solid: Solid::Lathe { p: p_at, d, length: length_at, widest, radius: Rc::new(radius) },
    }
}

/** The points of [min, max): at level 0, the sites with min ≤ site < max. */
pub fn cuboid(min: Point, max: Point, kind: Kind, fill: Fill, op: Op) -> Shape {
    let size = [max[0] - min[0], max[1] - min[1], max[2] - min[2]];
    let area = 2.0 * (size[0] * size[1] + size[1] * size[2] + size[0] * size[2]);
    let (lo, hi) = (at(min), at(max));
    Shape {
        kind,
        op,
        centre: at(mid(min, max)),
        bounds: Box3 { min: lo.map(|v| -((-v) >> 16)), max: hi.map(|v| -((-v) >> 16)) },
        extents: [0, 1, 2].map(|i| hi[i] - lo[i]),
        area: effective_area(area, (4.0 * size[0] * size[1] * size[2]) / area, &fill, mid(min, max)),
        fill,
        solid: Solid::Cuboid { min: lo, max: hi },
    }
}

/**
 * The points of a gable roof over [min, max): along `ridge` (x or z, 0 or
 * 2) from min to max, across the other from min to max at the bottom
 * (y = min) and narrowing to the ridge line at the top (y = max). Up is +y.
 */
pub fn gable(min: Point, max: Point, ridge: usize, kind: Kind, fill: Fill, op: Op) -> Shape {
    assert!(ridge == 0 || ridge == 2, "a gable's ridge runs along x or z");
    let across = 2 - ridge;
    let (length, width, height) = (max[ridge] - min[ridge], max[across] - min[across], max[1] - min[1]);
    let slant = js::hypot(&[width / 2.0, height]);
    let area = 2.0 * length * slant + width * height + length * width;
    let (lo, hi) = (at(min), at(max));
    Shape {
        kind,
        op,
        centre: at(mid(min, max)),
        bounds: Box3 { min: lo.map(|v| -((-v) >> 16)), max: hi.map(|v| -((-v) >> 16)) },
        extents: [hi[ridge] - lo[ridge], hi[across] - lo[across], hi[1] - lo[1]],
        area: effective_area(area, (2.0 * length * width * height) / area, &fill, mid(min, max)),
        fill,
        solid: Solid::Gable { min: lo, max: hi, ridge },
    }
}

/*
 * Containment is tested on offsets from the shape with 8 bits after the
 * point: a shape and its widening span at most 2^11 units, so squares stay
 * under 2^40 and every product fits in an i64.
 */

/** `x − p`, with 8 bits after the point. */
fn offset(x: At, p: At) -> At {
    [0, 1, 2].map(|i| (x[i] - p[i]) >> 8)
}

/** A segment from 0 to d (8 bits after the point), and 2^62 / (d · d) for finding how far along it a point is without dividing. */
#[derive(Clone, Copy)]
struct Segment {
    d: At,
    l2: i64,
    inverse: i64,
}

impl Segment {
    fn new(d: At) -> Segment {
        let d = d.map(|v| v >> 8);
        let l2 = dot(d, d);
        Segment { d, l2, inverse: if l2 == 0 { 0 } else { (1 << 62) / l2 } }
    }

    /** How far along the segment v's nearest point is, a fraction of ONE, given v · d. */
    fn along(&self, vd: i64) -> Fixed {
        // vd ≤ l2, so the product stays under 2^62.
        (vd.clamp(0, self.l2) * self.inverse) >> 46
    }

    /** The squared distance of v from the segment, given v · d. */
    fn away(&self, v: At, vd: i64) -> i64 {
        let t = self.along(vd);
        let a = [0, 1, 2].map(|i| v[i] - ((t * self.d[i]) >> 16));
        dot(a, a)
    }
}

/** A shape's test for one `min`, with what does not depend on the point worked out. */
enum Inside<'a> {
    Nothing,
    Ellipsoid {
        centre: At,
        inverse: [i64; 3],
    },
    Capsule {
        p: At,
        segment: Segment,
        r2: i64,
    },
    Lathe {
        p: At,
        segment: Segment,
        reach: i64,
        widest2: i64,
        half: Fixed,
        radius: &'a dyn Fn(f64) -> f64,
    },
    Cuboid {
        lo: At,
        hi: At,
        min: Fixed,
    },
    /** Under the slopes: |2u − lo − hi| · height ≤ width · (top − y), with 8 bits after the point. */
    Gable {
        lo: At,
        hi: At,
        min: Fixed,
        across: usize,
    },
}

impl Inside<'_> {
    fn holds(&self, x: At) -> bool {
        match self {
            Inside::Nothing => false,
            Inside::Ellipsoid { centre, inverse } => {
                let u = offset(x, *centre);
                (0..3).map(|i| ((u[i] * inverse[i]) >> 16).pow(2)).sum::<i64>() <= ONE * ONE
            }
            Inside::Capsule { p, segment, r2 } => {
                let v = offset(x, *p);
                segment.away(v, dot(v, segment.d)) <= *r2
            }
            Inside::Lathe { p, segment, reach, widest2, half, radius } => {
                let v = offset(x, *p);
                let vd = dot(v, segment.d);
                if vd < -reach || vd > segment.l2 + reach {
                    return false;
                }
                let away = segment.away(v, vd);
                away <= *widest2 && away <= (fixed(radius(float(segment.along(vd)))).max(*half) >> 8).pow(2)
            }
            Inside::Cuboid { lo, hi, min } => {
                (0..3).all(|i| if hi[i] - lo[i] >= *min { lo[i] <= x[i] && x[i] < hi[i] } else { (2 * x[i] - lo[i] - hi[i]).abs() <= *min })
            }
            Inside::Gable { lo, hi, min, across } => {
                let a = *across;
                let ridge = 2 - a;
                let side =
                    |i: usize| if hi[i] - lo[i] >= *min { lo[i] <= x[i] && x[i] < hi[i] } else { (2 * x[i] - lo[i] - hi[i]).abs() <= *min };
                if !side(ridge) || !side(1) || !side(a) {
                    return false;
                }
                // Too low or narrow for its slopes to show: its box.
                if hi[1] - lo[1] < *min || hi[a] - lo[a] < *min {
                    return true;
                }
                let u = (2 * x[a] - lo[a] - hi[a]).abs() >> 8;
                u * ((hi[1] - lo[1]) >> 8) <= ((hi[a] - lo[a]) >> 8) * ((hi[1] - x[1]) >> 8)
            }
        }
    }
}

impl Shape {
    /** Whether a point is inside the shape with each of its sizes made at least `min`. */
    fn inside(&self, min: Fixed) -> Inside<'_> {
        let half = min / 2;
        let q8 = |v: Fixed| v >> 8;
        match &self.solid {
            Solid::Ellipsoid { centre, radii } => {
                // Σ (u / a)² ≤ 1, u / a as u · (2^32 / a) with 16 bits after the point.
                let a = radii.map(|r| q8(r.max(half)));
                if a.contains(&0) {
                    return Inside::Nothing;
                }
                Inside::Ellipsoid { centre: *centre, inverse: a.map(|a| (1 << 32) / a) }
            }
            Solid::Capsule { p, d, radius } => Inside::Capsule { p: *p, segment: Segment::new(*d), r2: q8((*radius).max(half)).pow(2) },
            Solid::Lathe { p, d, length, widest, radius } => Inside::Lathe {
                p: *p,
                segment: Segment::new(*d),
                // Ends reach on by up to half of min where the lathe is shorter than it, in units of v · d.
                reach: q8(*length) * q8(0.max(min - length)) / 2,
                widest2: q8((*widest).max(half)).pow(2),
                half,
                radius: radius.as_ref(),
            },
            Solid::Cuboid { min: lo, max: hi } => Inside::Cuboid { lo: *lo, hi: *hi, min },
            Solid::Gable { min: lo, max: hi, ridge } => Inside::Gable { lo: *lo, hi: *hi, min, across: 2 - ridge },
        }
    }
}

/**
 * Draws a shape at sites `s` apart, by how many of its sizes are under s.
 */
fn draw_shape(shape: &Shape, seed: u64, si: i64, clip: &Box3, put: &mut dyn FnMut(i64, i64, i64)) {
    let s = si as f64;
    let spacing = si * ONE;
    let small = shape.extents.iter().filter(|e| **e < spacing).count();
    let kept = |x: i64, y: i64, z: i64, chance: f64| chance >= 1.0 || unit(hash(seed, x * si, y * si, z * si)) < chance;
    if small == 3 {
        let [x, y, z] = nearest_site(shape.centre, si);
        if clip.holds([x, y, z]) && kept(x, y, z, shape.area / (VOXEL_AREA * s * s)) {
            put(x, y, z);
        }
        return;
    }
    let min = [0, spacing, spacing * 8 / 5][small];
    let b = &shape.bounds;
    let [x0, y0, z0] = b.min.map(|v| -(-(v * ONE - min / 2)).div_euclid(spacing));
    let [x1, y1, z1] = b.max.map(|v| (v * ONE - ONE + min / 2).div_euclid(spacing));
    let inside = shape.inside(min);
    let sites = |within: &Box3, visit: &mut dyn FnMut(i64, i64, i64)| {
        let (z_from, z_to) = (z0.max(within.min[2]), z1.min(within.max[2] - 1));
        let (y_from, y_to) = (y0.max(within.min[1]), y1.min(within.max[1] - 1));
        let (from, x_to) = (x0.max(within.min[0]), x1.min(within.max[0] - 1));
        for z in z_from..=z_to {
            for y in y_from..=y_to {
                let mut x = from + ((from + y + z) & 1);
                while x <= x_to {
                    if inside.holds([x, y, z].map(|v| v * spacing)) {
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
            let thickness = float(shape.extents.into_iter().min().expect("three"));
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

impl Material {
    /** The colour '#rrggbb' as 0xrrggbb; 0 for none (material 0, empty). */
    pub fn rgb(&self) -> u32 {
        self.color.strip_prefix('#').and_then(|hex| u32::from_str_radix(hex, 16).ok()).unwrap_or(0)
    }
}

/** Shades of the kinds' colours, as material ids picked by a hash of the position in lattice units. */
struct Palette {
    materials: Vec<Material>,
    shades: Vec<Vec<u8>>,
    seed: u64,
}

impl Palette {
    fn new(kinds: &[KindDef], seed: u64) -> Palette {
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
        assert!(materials.len() <= 256, "{} materials: ids are bytes, so at most 255 and none", materials.len());
        Palette { materials, shades, seed }
    }

    fn shade(&self, kind: Kind, x: i64, y: i64, z: i64) -> u8 {
        let shades = &self.shades[kind as usize];
        shades[((hash(self.seed, x, y, z) as u64 * shades.len() as u64) >> 32) as usize]
    }
}

/** The materials of the grids `generate` makes: ids are the same in all of them. */
pub fn materials_of(generator: &Generator) -> Vec<Material> {
    Palette::new(&generator.kinds, generator.seed).materials
}

/**
 * The voxels of `b` at `level`: sites 2^level apart, site (x, y, z) at
 * 2^level · (x, y, z) in lattice units, the box in the level's units. The
 * grid may reach one site further in x (its rows start and end on an even x).
 */
pub fn generate(generator: &Generator, b: &Box3, level: u32) -> VoxelGrid {
    let s = 1i64 << level;
    let palette = Palette::new(&generator.kinds, generator.seed);
    let mut grid = VoxelGrid::new(b);
    let clip = grid.clip();
    let mut put = |kind: Kind, op: Op, x: i64, y: i64, z: i64| {
        if op == Op::Fill && grid.get(x, y, z) != 0 {
            return;
        }
        let id = if op == Op::Carve { 0 } else { palette.shade(kind, x * s, y * s, z * s) };
        grid.set(x, y, z, id);
    };
    let drawables = drawables_in(generator, &clip, s);
    let mut at = 0;
    while at < drawables.len() {
        // A thing's drawables, together: those of fields one by one.
        let thing = drawables[at].thing;
        let end = if thing == 0 { at + 1 } else { (at..drawables.len()).find(|i| drawables[*i].thing != thing).unwrap_or(drawables.len()) };
        let group = &drawables[at..end];
        at = end;
        let carves = group.iter().any(|d| matches!(&d.draw, Draw::Shapes(shapes, _) if shapes.iter().any(|s| s.op == Op::Carve)));
        if !carves {
            for drawable in group {
                draw(drawable, generator.seed, s, &clip, &mut put);
            }
            continue;
        }
        // It cuts only itself: drawn aside, then over the world.
        let mut own = Own::new(&thing_box(group, s).intersect(&clip));
        for drawable in group {
            draw(drawable, generator.seed, s, &clip, &mut |kind, op, x, y, z| own.put(kind, op, x, y, z));
        }
        own.each(&mut put);
    }
    grid
}

/**
 * A thing's sites, drawn aside from the world: in a box of a level's
 * sites, each its kind and how it meets the world, or carved.
 */
struct Own {
    b: Box3,
    n: [i64; 3],
    /** 0: nothing; else kind + 1, and the op in bits 8 and 9. */
    cells: Vec<u16>,
}

impl Own {
    fn new(b: &Box3) -> Own {
        let n = [0, 1, 2].map(|i| (b.max[i] - b.min[i]).max(0));
        Own { b: *b, n, cells: vec![0; (n[0] * n[1] * n[2]) as usize] }
    }

    fn index(&self, x: i64, y: i64, z: i64) -> usize {
        assert!(self.b.holds([x, y, z]), "a thing drew at ({x}, {y}, {z}), outside its bounds {:?}", self.b);
        let [x, y, z] = [x - self.b.min[0], y - self.b.min[1], z - self.b.min[2]];
        (x + self.n[0] * (y + self.n[1] * z)) as usize
    }

    /** As the world takes them, but within the thing: Fill where it has nothing, Paint over, Carve out. */
    fn put(&mut self, kind: Kind, op: Op, x: i64, y: i64, z: i64) {
        let i = self.index(x, y, z);
        let carved = 3 << 8;
        if op == Op::Fill && self.cells[i] != 0 && self.cells[i] != carved {
            return;
        }
        self.cells[i] = match op {
            Op::Carve => carved,
            Op::Fill => (kind as u16 + 1) | 1 << 8,
            Op::Paint => (kind as u16 + 1) | 2 << 8,
        };
    }

    /** What the thing has at (x, y, z): its kind and how it meets the world. */
    fn at(&self, x: i64, y: i64, z: i64) -> Option<(Kind, Op)> {
        let cell = self.cells[self.index(x, y, z)];
        let op = match cell >> 8 {
            1 => Op::Fill,
            2 => Op::Paint,
            _ => return None,
        };
        Some((((cell & 255) - 1) as Kind, op))
    }

    /** Each site the thing has, with its kind and how it meets the world. */
    fn each(&self, mut visit: impl FnMut(Kind, Op, i64, i64, i64)) {
        for i in 0..self.cells.len() as i64 {
            let (x, y, z) =
                (i % self.n[0] + self.b.min[0], (i / self.n[0]) % self.n[1] + self.b.min[1], i / (self.n[0] * self.n[1]) + self.b.min[2]);
            if let Some((kind, op)) = self.at(x, y, z) {
                visit(kind, op, x, y, z);
            }
        }
    }
}

/** The sites of level spacing `s` a thing's drawables may draw at: its shapes' bounds and two sites more, and its measured sites. */
fn thing_box(group: &[Drawable], s: i64) -> Box3 {
    let boxes: Vec<Box3> = group
        .iter()
        .flat_map(|d| match &d.draw {
            Draw::Shapes(shapes, _) => shapes
                .iter()
                .map(|shape| Box3 {
                    min: shape.bounds.min.map(|v| v.div_euclid(s) - 2),
                    max: shape.bounds.max.map(|v| -(-v).div_euclid(s) + 2),
                })
                .collect(),
            Draw::Sites(sites) => sites.iter().map(|(p, ..)| Box3 { min: *p, max: p.map(|v| v + 1) }).collect(),
            Draw::Field(_) => Vec::new(),
        })
        .collect();
    bounds_of_shapes(&boxes)
}

/** Draws one drawable at sites `s` apart into `clip`. */
fn draw(drawable: &Drawable, seed: u64, s: i64, clip: &Box3, put: &mut dyn FnMut(Kind, Op, i64, i64, i64)) {
    match &drawable.draw {
        Draw::Field(field) => draw_field(field, seed, s, clip, put),
        Draw::Shapes(shapes, far) => {
            for (i, shape) in shapes.iter().enumerate() {
                let seed = hash_text(&format!("{}#{}{i}", drawable.key, if *far { "far" } else { "" }), seed);
                draw_shape(shape, seed, s, clip, &mut |x, y, z| put(shape.kind, shape.op, x, y, z));
            }
        }
        Draw::Sites(sites) => {
            for &([x, y, z], kind, op) in sites {
                if clip.holds([x, y, z]) {
                    put(kind, op, x, y, z);
                }
            }
        }
    }
}

enum Draw<'a> {
    Field(&'a Field),
    /** A node's shapes, and whether they are its far ones. */
    Shapes(Vec<Shape>, bool),
    /** A thing measured: its sites. */
    Sites(Sites),
}

struct Drawable<'a> {
    order: f64,
    key: String,
    /** The thing it is part of, numbered from 1; 0 for fields. */
    thing: u32,
    draw: Draw<'a>,
}

/** What draws into `clip` (sites s apart), in drawing order: by layer, then key. */
fn drawables_in<'a>(generator: &'a Generator, clip: &Box3, s: i64) -> Vec<Drawable<'a>> {
    // The lattice units the sites of `clip` cover, with what a shape may grow by at this level.
    let units = Box3 { min: clip.min.map(|v| v * s), max: clip.max.map(|v| (v - 1) * s + 1) }.widen(s);
    let mut drawables = Vec::new();
    let mut things = 0;
    for layer in &generator.layers {
        let population = match layer {
            Layer::Field(field) => {
                drawables.push(Drawable { order: field.order, key: field.name.to_string(), thing: 0, draw: Draw::Field(field) });
                continue;
            }
            Layer::Population(p) => p,
        };
        if s > population.size {
            if let Some(far) = &population.far {
                drawables.push(Drawable { order: population.order, key: population.name.to_string(), thing: 0, draw: Draw::Field(far) });
            }
            continue;
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
                    let [oi, oj, ok] = nearest_site(at(anchor), n);
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
                    things += 1;
                    let at = Visit { s, units: &units, order: population.order, thing: things, seed: generator.seed };
                    visit(&node, key, bounds, &at, &mut drawables);
                }
            }
        }
    }
    drawables.sort_by(|a, b| a.order.total_cmp(&b.order).then_with(|| a.key.cmp(&b.key)));
    drawables
}

/** A thing's sites of a level: each its kind and how it meets the world. */
type Sites = Vec<([i64; 3], Kind, Op)>;

/** Where a node is drawn: at sites `s` apart, for the lattice units `units`, in its layer's `order`, as part of `thing`. */
struct Visit<'u> {
    s: i64,
    units: &'u Box3,
    order: f64,
    thing: u32,
    seed: u64,
}

/** Adds a node's drawables and its children's, as far as they reach `at.units` and are drawn at its spacing. */
fn visit(node: &Node, key: String, bounds: Box3, at: &Visit, drawables: &mut Vec<Drawable<'_>>) {
    let Visit { s, units, order, thing, seed } = *at;
    if !bounds.overlaps(units) {
        return;
    }
    let far = node.far.as_ref().filter(|far| s * ONE > far.detail);
    let draw = match far {
        Some(Far { detail, form: FarForm::Measured }) => Draw::Sites(measure(node, &key, bounds, *detail, s, seed)),
        Some(Far { form: FarForm::Shapes(shapes), .. }) => Draw::Shapes(shapes.to_vec(), true),
        None => Draw::Shapes(node.shapes.get().to_vec(), false),
    };
    drawables.push(Drawable { order, key: key.clone(), thing, draw });
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
            visit(&child.node, format!("{key}/{}", child.name), child_bounds, at, drawables);
        }
    }
}

/** A node drawn alone, whole, at sites `f` apart. */
fn alone(node: &Node, key: &str, bounds: Box3, f: i64, seed: u64) -> Own {
    let units = bounds.widen(f);
    let mut near = Vec::new();
    visit(node, key.to_string(), bounds, &Visit { s: f, units: &units, order: 0.0, thing: 1, seed }, &mut near);
    let fine = Box3 { min: bounds.min.map(|v| v.div_euclid(f) - 2), max: bounds.max.map(|v| -(-v).div_euclid(f) + 2) };
    let mut own = Own::new(&fine);
    for drawable in &near {
        draw(drawable, seed, f, &fine, &mut |kind, op, x, y, z| own.put(kind, op, x, y, z));
    }
    own
}

/** A layer of a thing across an axis: what it hides seen along the axis (a bitset of the plane), and the columns along the other two it holds. */
type Slice = (Vec<u64>, u32, u32);

/**
 * What slabs of things hide, measured: things drawn alone at a level,
 * and for each slab of them across an axis the area it hides seen along
 * each axis, on average over the three and the things. A population's
 * far form, from things made at the origin, is as dense as that
 * (extinction = things per volume · their mean shadow).
 */
pub struct Shadows {
    f: i64,
    /** By thing, its first layer across the axis and its layers from it. */
    things: Vec<(i64, Vec<Slice>)>,
}

impl Shadows {
    pub fn new(nodes: &[Node], level: u32, axis: usize) -> Shadows {
        let f = 1i64 << level;
        let (a, b) = ((axis + 1) % 3, (axis + 2) % 3);
        let things = nodes
            .iter()
            .enumerate()
            .map(|(i, node)| {
                let own = alone(node, &format!("sample {i}"), bounds_of(node), f, 0);
                let (n, min) = (own.n, own.b.min);
                let words = ((n[a] * n[b]) as usize).div_ceil(64);
                let layers = (0..n[axis])
                    .map(|l| {
                        let mut plane = vec![0u64; words];
                        let (mut along_a, mut along_b) = (vec![false; n[b] as usize], vec![false; n[a] as usize]);
                        for i in 0..n[a] {
                            for j in 0..n[b] {
                                let mut p = [0; 3];
                                (p[axis], p[a], p[b]) = (l + min[axis], i + min[a], j + min[b]);
                                if own.at(p[0], p[1], p[2]).is_some() {
                                    let bit = (i * n[b] + j) as usize;
                                    plane[bit / 64] |= 1 << (bit % 64);
                                    (along_a[j as usize], along_b[i as usize]) = (true, true);
                                }
                            }
                        }
                        let count = |v: Vec<bool>| v.into_iter().filter(|b| *b).count() as u32;
                        (plane, count(along_a), count(along_b))
                    })
                    .collect();
                (min[axis], layers)
            })
            .collect();
        Shadows { f, things }
    }

    /** The mean shadow (lattice units²) of the slab from `from` to `to` (exclusive) along the axis. */
    pub fn slab(&self, from: i64, to: i64) -> f64 {
        let (f, mut sum) = (self.f, 0.0);
        for (first, layers) in &self.things {
            let ceil = |v: i64| -(-v).div_euclid(f) - first;
            let (lo, hi) = (ceil(from).max(0) as usize, ceil(to).clamp(0, layers.len() as i64) as usize);
            if lo >= hi {
                continue;
            }
            let mut plane = vec![0u64; layers[0].0.len()];
            let (mut along_a, mut along_b) = (0, 0);
            for (bits, a, b) in &layers[lo..hi] {
                for (p, w) in plane.iter_mut().zip(bits) {
                    *p |= w;
                }
                (along_a, along_b) = (along_a + a, along_b + b);
            }
            let across = plane.iter().map(|w| w.count_ones()).sum::<u32>();
            sum += (across + along_a + along_b) as f64 * (f * f) as f64 / 3.0;
        }
        sum / self.things.len() as f64
    }
}

/**
 * A node's far form at sites `s` apart, measured from its near form:
 * drawn whole at the coarsest level it is drawn at (spacing f, at most
 * `detail`), then coarsened. A coarse site stands for the fine sites in
 * its cell (those nearest it); it is kept with the chance a ray crossing
 * the cell is stopped, the share of its columns along each axis holding
 * a fine site (2 (s / f)² of them), on average over the axes, against
 * one threshold for the whole thing; and is of a kind of the fine sites
 * first along those columns from either end, in proportion.
 */
fn measure(node: &Node, key: &str, bounds: Box3, detail: Fixed, s: i64, seed: u64) -> Sites {
    let f = 1i64 << ((detail / ONE).max(1).ilog2());
    // Its far form not drawn at f: f is at most detail.
    let own = alone(node, key, bounds, f, seed);
    // By coarse site: per axis, how many of its columns hold a fine site; what shows at their ends, counted by kind.
    #[derive(Default)]
    struct Stats {
        columns: [u32; 3],
        shown: Vec<(Kind, Op, u32)>,
    }
    let show = |stats: &mut Stats, kind: Kind, op: Op| match stats.shown.iter_mut().find(|e| e.0 == kind) {
        Some(e) => e.2 += 1,
        None => stats.shown.push((kind, op, 1)),
    };
    let mut cells = std::collections::HashMap::<[i64; 3], Stats>::new();
    let n = own.n;
    for axis in 0..3 {
        let (a, b) = ((axis + 1) % 3, (axis + 2) % 3);
        let mut p = [0i64; 3];
        for i in 0..n[a] {
            for j in 0..n[b] {
                (p[a], p[b]) = (i, j);
                // A column crosses each cell once (they are convex): the run of it in a cell is one column of the cell's.
                let mut run: Option<([i64; 3], Kind, Op)> = None;
                for k in 0..n[axis] {
                    p[axis] = k;
                    let at = [0, 1, 2].map(|c| p[c] + own.b.min[c]);
                    let Some((kind, op)) = own.at(at[0], at[1], at[2]) else { continue };
                    let site = nearest_site(at.map(|v| v * f * ONE), s);
                    match run {
                        Some((cell, ..)) if cell == site => run = Some((site, kind, op)),
                        _ => {
                            if let Some((cell, last, last_op)) = run {
                                show(cells.get_mut(&cell).expect("counted"), last, last_op);
                            }
                            let stats = cells.entry(site).or_default();
                            stats.columns[axis] += 1;
                            show(stats, kind, op);
                            run = Some((site, kind, op));
                        }
                    }
                }
                if let Some((cell, last, last_op)) = run {
                    show(cells.get_mut(&cell).expect("counted"), last, last_op);
                }
            }
        }
    }
    let all = 2.0 * ((s / f) as f64).powi(2);
    let kept = hash_text(&format!("{key}#measured"), seed);
    let mut sites = Vec::new();
    for (site, mut stats) in cells {
        let chance = stats.columns.iter().map(|c| *c as f64 / all).sum::<f64>() / 3.0;
        let [x, y, z] = site;
        // One threshold for the whole thing, not one per site: a solid thing keeps a whole outline,
        // not a ragged one; a porous one is kept or not as a whole, in proportion over many.
        if unit(hash(kept, 0, 0, 0)) >= chance {
            continue;
        }
        // In an order of their own: the columns' isn't one.
        stats.shown.sort_by_key(|e| e.0);
        let total: u32 = stats.shown.iter().map(|e| e.2).sum();
        let mut pick = ((hash(kept ^ 1, x * s, y * s, z * s) as u64 * total as u64) >> 32) as u32;
        let &(kind, op, _) = stats
            .shown
            .iter()
            .find(|e| {
                pick < e.2 || {
                    pick -= e.2;
                    false
                }
            })
            .expect("a kind");
        sites.push((site, kind, op));
    }
    // In an order of their own, as sites are put in it.
    sites.sort_by_key(|e| e.0);
    sites
}

/** Draws a field into `clip`; a medium keeps a site with the chance a ray crossing it is stopped. */
fn draw_field(field: &Field, seed: u64, s: i64, clip: &Box3, put: &mut dyn FnMut(Kind, Op, i64, i64, i64)) {
    let op = field.op;
    let field_seed = hash_text(field.name, seed);
    // The chance a medium's site is kept, from its extinction.
    let chance_of = |extinction: Option<f64>| extinction.map(|e| 1.0 - js::exp(-e * CHORD * s as f64));
    let kept = |x: i64, y: i64, z: i64, chance: Option<f64>| chance.is_none_or(|p| unit(hash(field_seed, x * s, y * s, z * s)) < p);
    match &field.values {
        FieldValues::At(at) => {
            for z in clip.min[2]..clip.max[2] {
                for y in clip.min[1]..clip.max[1] {
                    let mut x = clip.min[0] + ((clip.min[0] + y + z) & 1);
                    while x < clip.max[0] {
                        if let Some((kind, extinction)) = at(x * s, y * s, z * s, s)
                            && kept(x, y, z, chance_of(extinction))
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
                        // s is a power of two: shifting is dividing, rounded down.
                        let first = (-((-span.from) >> s.trailing_zeros())).max(lo[axis]);
                        let mut w = first + ((first + u + v) & 1);
                        let chance = chance_of(span.extinction);
                        while w * s < span.to && w < hi[axis] {
                            site[axis] = w;
                            if kept(site[0], site[1], site[2], chance) {
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

// ---------------------------------------------------------------------------
// Helpers

/**
 * Whether coordinate `at` is in the first half of a stripe `period` wide,
 * at sites `spacing` apart. Stripes finer than the sites would alias into
 * wider ones: they draw each half as often as it is, by `dither` (a hash
 * of the site).
 */
pub fn stripe(at: i64, period: i64, spacing: i64, dither: u32) -> bool {
    if 2 * spacing <= period { at.rem_euclid(period) < period / 2 } else { dither < 1 << 31 }
}

const EVERYWHERE: Box3 = Box3 { min: [i64::MIN / 4; 3], max: [i64::MAX / 4; 3] };

/** The site of sites `s` apart nearest to a point: the one whose voxel holds it. */
fn nearest_site(p: At, s: i64) -> [i64; 3] {
    let v = p.map(|c| c.div_euclid(s));
    let mut r = v.map(|c| (c + ONE / 2) >> 16);
    if (r[0] + r[1] + r[2]) & 1 != 0 {
        // Round the coordinate furthest from its integer the other way.
        let e = [0, 1, 2].map(|i| (v[i] - r[i] * ONE).abs());
        let i = if e[0] >= e[1] && e[0] >= e[2] {
            0
        } else if e[1] >= e[2] {
            1
        } else {
            2
        };
        r[i] += if v[i] > r[i] * ONE { 1 } else { -1 };
    }
    r
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_long_lathe_reaches_its_widest_circle() {
        // 1000 units long, at 3 : 4 : 0, so its widest circle reaches 4, 3 and 5 units along x, y and z.
        let shape = lathe([0.0; 3], [600.0, 800.0, 0.0], |_| 5.0, 1, Fill::Const(1.0), Op::Fill);
        assert_eq!((shape.bounds.min, shape.bounds.max), ([-4, -3, -5], [605, 804, 6]));
    }

    #[test]
    fn a_measured_far_form_keeps_what_rays_see() {
        // A solid block 64 units wide, painted with a sheet of glass on top, and a sparse cloud: measured at sites 16 apart from sites 2 apart.
        let block = Node {
            far: Some(Far::measured(fixed(2.0))),
            ..Node::of(vec![
                cuboid([0.0; 3], [64.0; 3], 1, SOLID, Op::Fill),
                cuboid([0.0, 60.0, 0.0], [64.0, 64.0, 64.0], 2, SOLID, Op::Paint),
            ])
        };
        let sites = measure(&block, "block", bounds_of(&block), fixed(2.0), 16, 1);
        let inside = |p: [i64; 3]| p.iter().all(|v| (1..4).contains(v));
        assert!(
            (1..4).all(|y| (1..4).all(|x| (1..4).all(|z| (x + y + z) % 2 == 1 || sites.iter().any(|s| s.0 == [x, y, z])))),
            "its inside is all kept"
        );
        assert!(sites.iter().filter(|s| inside(s.0)).all(|s| s.1 == 1), "what shows inside is the block's");
        // Its top row of cells is half covered, kept by some things' thresholds: there, the glass shows.
        let tops =
            (0..16).flat_map(|i| measure(&block, &format!("block {i}"), bounds_of(&block), fixed(2.0), 16, 1)).filter(|s| s.0[1] == 4);
        assert!(tops.filter(|s| s.1 == 2).count() > 0, "the glass on top shows");
        let cloud = Node { far: Some(Far::measured(fixed(2.0))), ..Node::of(vec![ball([0.0; 3], 64.0, 1, Fill::Const(0.02), Op::Fill)]) };
        // Of its about 134 cells (a ball's volume over a site's, 2 · 16³), those a ray crossing is stopped in:
        // a column crosses a cell over 16 units on average (its volume over its shadow, 2 · 16³ / 2 · 16²), so 4 sites 4 apart,
        // each kept with 1 − 0.98² ≈ 0.04: 1 − 0.96⁴ ≈ 0.15 of them, a little less with the cells on its edge.
        // Each cloud is kept or not as a whole, against its own threshold: on average over many.
        let kept = (0..64).map(|i| measure(&cloud, &format!("cloud {i}"), bounds_of(&cloud), fixed(2.0), 16, 1).len()).sum::<usize>()
            as f64
            / (64.0 * 134.0);
        assert!((0.1..0.2).contains(&kept), "sparse clouds keep {kept} of their sites");
    }

    #[test]
    fn a_gable_narrows_to_its_ridge() {
        // 8 wide across x, 4 high: at height h, the sites within 4 · (1 − h / 4) of x = 4.
        let shape = gable([0.0; 3], [8.0, 4.0, 6.0], 2, 1, SOLID, Op::Fill);
        let inside = shape.inside(0);
        let holds = |x: i64, y: i64| inside.holds([x, y, 2].map(|v| v * ONE));
        assert!(holds(0, 0) && holds(7, 0) && holds(4, 3) && holds(3, 2) && holds(6, 1));
        assert!(!holds(1, 3) && !holds(8, 0) && !holds(4, 4) && !holds(7, 2));
    }
}
