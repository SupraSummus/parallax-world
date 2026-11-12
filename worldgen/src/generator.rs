//! World generators: a world is fields and populations of things made of
//! shapes; the framework draws any box of it alone, at any level of
//! detail. See docs/generators.md.

use std::cell::{OnceCell, RefCell};
use std::rc::Rc;
use std::sync::OnceLock;

use bake::light::srgb_to_linear;
use bake::{FACE_KINDS, Faces, VOXEL_FACES};

use crate::cost::{self, Work};
use crate::curve::Curve;
use crate::grid::{ALPHA, Box3, VoxelGrid};
use crate::js;
use crate::noise::{Fixed, Fx, ONE, Rng, fixed, float, hash, hash_text, hash64};
use crate::plan::{self, Sloped};
use crate::points::{Points, Spot};

/** A material kind: its index in Generator::kinds. */
pub type Kind = u8;

/**
 * What a site is of: a kind, and the kind of each of its faces (by bake's
 * VOXEL_FACES), what is seen looking at that face, where they differ:
 * a coarse site at a house's eaves is roof seen from above, wall from the side.
 */
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Look {
    pub kind: Kind,
    pub faces: Option<[Kind; FACE_KINDS]>,
}

impl From<Kind> for Look {
    fn from(kind: Kind) -> Look {
        Look { kind, faces: None }
    }
}

/**
 * Where drawing puts a site: what it is of, how it meets the world, its
 * share of the site (of SHARE: how much of the site's rays it stops) and
 * its pick (of SHARE, the thing's own hash there), at (x, y, z) of the
 * level. The shares add up to the site's alpha; the picks choose what
 * of it the site shows (`see`).
 */
type Put<'a> = dyn FnMut(Look, Op, u32, u32, i64, i64, i64) + 'a;
/** A whole site's share. */
pub const SHARE: u32 = 1 << 16;

/** A chance as a share (of SHARE). */
fn share(chance: f64) -> u32 {
    (chance.clamp(0.0, 1.0) * SHARE as f64) as u32
}

/** A hash as a pick: uniform, under SHARE. */
fn to_pick(hash: u32) -> u32 {
    ((hash as u64 * SHARE as u64) >> 32) as u32
}
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

/**
 * A tube's radius: the same all along it, or by how far along it is, t
 * from 0 at its start to 1 at its end (a trunk narrowing, a root's foot).
 */
#[derive(Clone)]
pub enum Radius {
    Const(f64),
    Along(Rc<dyn Fn(f64) -> f64>),
}

impl Radius {
    fn at(&self, t: f64) -> f64 {
        match self {
            Radius::Const(r) => *r,
            Radius::Along(radius) => radius(t),
        }
    }
}

impl From<f64> for Radius {
    fn from(r: f64) -> Radius {
        Radius::Const(r)
    }
}

impl<F: Fn(f64) -> f64 + 'static> From<F> for Radius {
    fn from(radius: F) -> Radius {
        Radius::Along(Rc::new(radius))
    }
}

/** A point in lattice units, fixed point. */
type At = [Fixed; 3];

#[derive(Clone)]
enum Solid {
    Ellipsoid { centre: At, radii: At },
    Tube { pieces: Vec<Piece>, radius: Radius, widest: Fixed, length: Fixed },
    Cuboid { min: At, max: At },
    Gable { min: At, max: At, ridge: usize },
}

/** A solid, in lattice units: given in floats, kept in fixed point. */
#[derive(Clone)]
pub struct Shape {
    pub kind: Kind,
    pub op: Op,
    /** Given only to its constructor, which works out its `area` from it: set after, it would not count. */
    fill: Fill,
    /** Every site it draws at level 0 is inside. */
    pub bounds: Box3,
    /** Its sizes along its own three axes. */
    pub extents: [Fx; 3],
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

/** How a node is drawn while sites are further apart than `detail` (lattice units). */
pub struct Far {
    pub detail: Fx,
    pub form: FarForm,
}

pub enum FarForm {
    Shapes(Vec<Shape>),
    /**
     * Its near form, drawn whole two levels finer and coarsened: its most
     * covered sites, as many as their shares of rays stopped add up to,
     * each of a kind in proportion to what shows on its sides.
     */
    Measured,
}

impl Far {
    pub fn of(detail: Fx, shapes: Vec<Shape>) -> Far {
        Far { detail, form: FarForm::Shapes(shapes) }
    }

    pub fn measured(detail: Fx) -> Far {
        assert!(detail >= Fx::ONE, "a measured far form is measured from a level its near form is drawn at; see \"Levels\" in {DOCS}");
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
type CellAnchor = dyn Fn(&Cell) -> Option<Point>;
type LineAnchor = dyn Fn(&Line) -> Option<Point>;
type SpotAnchor = dyn Fn(&Spot) -> Option<Point>;
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

    /** Its `bounds`, or without children its shapes'. */
    pub fn bounds(&self) -> Box3 {
        bounds_of(self)
    }
}

/**
 * Part of a line of sites: [from, to) along the line, solid, or a medium
 * if it has `extinction`: per lattice unit of a ray's path through it,
 * so a site is kept with the chance 1 − exp(−extinction · c), c the mean
 * chord of its voxel (about 0.94 units at level 0, twice that a level up).
 * A line has a site every two spacings, so a span shorter than that may
 * have none: `Column` (column.rs) makes spans that keep one.
 */
pub struct Span {
    pub from: i64,
    pub to: i64,
    pub kind: Kind,
    pub extinction: Option<f64>,
}

pub enum FieldValues {
    /** A kind, with a medium's extinction if it is one, at each site; given (x, y, z, spacing), all in lattice units. */
    At(Box<FieldAt>),
    /**
     * Spans along each line of sites parallel to `axis`, drawn in order;
     * given (u, v, spacing): the line's other two coordinates in axis
     * order and the sites' spacing, all in lattice units.
     */
    Spans { axis: usize, spans: Box<FieldSpans> },
}

/** Something everywhere, given by position alone. */
pub struct Field {
    pub name: &'static str,
    pub order: f64,
    pub op: Op,
    pub values: FieldValues,
    /** Spans along y only about a plan of heights; None: anywhere. */
    pub about: Option<About>,
}

/**
 * Where a field's spans along y are: from `below` under to `above` over
 * `of` at their column, give or take what a line of sites stands for
 * (`slack`). A rule, checked wherever it draws; so it is asked to draw
 * only where that may be (Sloped::range).
 */
pub struct About {
    pub of: Rc<Sloped>,
    pub below: i64,
    pub above: i64,
}

impl About {
    /**
     * How far past `below` and `above` a line's spans may reach, at sites
     * `spacing` apart: spans are at least two sites tall, and a line
     * reaches down past the eight columns a spacing around it (column.rs's
     * `sides`), the diagonals' up to twice the slope times that off;
     * without one, any way.
     */
    fn slack(&self, spacing: i64) -> Option<i64> {
        Some(2 * (1 + self.of.slope()?) * spacing)
    }

    /** Whether spans of lines in [min, max] (x, z, lattice units, inclusive) may meet heights [y0, y1], at sites `spacing` apart. */
    fn may_meet(&self, min: [i64; 2], max: [i64; 2], [y0, y1]: [i64; 2], spacing: i64) -> bool {
        let (Some((lo, hi)), Some(slack)) = (self.of.range(min, max), self.slack(spacing)) else { return true };
        y0 <= hi + self.above + slack && y1 >= lo - self.below - slack
    }
}

/**
 * A cell of a population: of N·FCC, or the leaf of a point (`Points`),
 * with its mark and number there; along the axes the points don't span,
 * its index is 0 and its centre the anchor's.
 */
pub struct Cell<'a> {
    pub i: i64,
    pub j: i64,
    pub k: i64,
    pub centre: [i64; 3],
    /** N: the cell reaches N from its centre along each axis. */
    pub n: i64,
    /** Its point's mark; 0 for cells of N·FCC. */
    pub mark: usize,
    nth: u32,
    population: &'a str,
    seed: u64,
}

impl Cell<'_> {
    /** Cell (i, j, k) of N·FCC, i + j + k even. */
    fn fcc(population: &Population, seed: u64, [i, j, k]: [i64; 3]) -> Cell<'_> {
        let n = population.cell;
        Cell { i, j, k, centre: [i * n, j * n, k * n], n, mark: 0, nth: 0, population: population.name, seed }
    }

    /** A random sequence that depends on the cell and `stream` only. */
    pub fn random(&self, stream: &str) -> Rng {
        let point = (self.mark as u64) << 32 | self.nth as u64;
        Rng::new(hash64(hash_text(&format!("{}:{stream}", self.population), self.seed) ^ point, self.i, self.j, self.k))
    }

    /** Its thing's name, for messages and drawing order. */
    fn key(&self) -> String {
        let point = if (self.mark, self.nth) == (0, 0) { String::new() } else { format!(" {}.{}", self.mark, self.nth) };
        format!("{} {},{},{}{point}", self.population, self.i, self.j, self.k)
    }
}

/**
 * A line of cells of a population along an axis: those whose centres
 * differ only along it (every 2N, as i + j + k is even). They lie in the
 * square 2N wide across the axis around its centre; every point is in
 * one cell, so in one line along each axis.
 */
pub struct Line<'a> {
    pub axis: usize,
    /** The cells' indices across the axis, in axis order. */
    pub across: [i64; 2],
    /** Their centres across the axis, lattice units: `across` times N. */
    pub centre: [i64; 2],
    pub n: i64,
    population: &'a str,
    seed: u64,
}

impl Line<'_> {
    /** A random sequence that depends on the line and `stream` only. */
    pub fn random(&self, stream: &str) -> Rng {
        let [u, v] = self.across;
        Rng::new(hash64(hash_text(&format!("{}:{stream} along {}", self.population, self.axis), self.seed), u, v, 0))
    }

    /**
     * A point picked by `random` evenly in the line's square across its
     * axis, at `along(u, v)` along it (u and v in axis order). Each point
     * of a surface crossing the lines once is in one line's cells, which
     * picks it as often as any: anchored there, things are as even as one
     * per square 2N wide.
     */
    pub fn pick(&self, random: &mut Rng, along: impl Fn(i64, i64) -> i64) -> Point {
        let [u, v] = self.centre.map(|c| c - self.n + random.below(2 * self.n as u32) as i64);
        let ([a, b], mut point) = (across(self.axis), [0.0; 3]);
        (point[a], point[b], point[self.axis]) = (u as f64, v as f64, along(u, v) as f64);
        point
    }
}

/** The axes across `axis`, in order. */
fn across(axis: usize) -> [usize; 2] {
    [(axis == 0) as usize, 2 - (axis == 2) as usize]
}

/** Things placed in the cells of N·FCC, at most one per cell, or at points. */
pub struct Population {
    pub name: &'static str,
    pub order: f64,
    /** N; anchored at points, their leaves' width. */
    pub cell: i64,
    /** How far a thing reaches past N from its cell's centre, per axis. */
    pub reach: [i64; 3],
    /** Things are made while sites are at most this far apart (ANY_SIZE: always). */
    pub size: i64,
    pub anchor: Anchors,
    pub make: Box<Make>,
    pub far: Option<Field>,
    /**
     * Its things stand on a plan of heights: each anchored at most N over
     * or under it (a rule, checked wherever one is made); so it is asked
     * to draw only near it. None: anywhere.
     */
    pub on: Option<Rc<Sloped>>,
}

/** Where a population's things are, each owned by the cell it is in, if asked there. */
pub enum Anchors {
    /** At most one a cell, asked of each. */
    Cells(Box<CellAnchor>),
    /**
     * At most one a line of cells along `axis`, asked once however many
     * of its cells a box reaches: for things on a surface across it (a
     * ground under columns of cells), as even as one per cell's square.
     */
    Lines { axis: usize, anchor: Box<LineAnchor> },
    /**
     * A thing at each point of `points` (points.rs), anchored where
     * `anchor` lifts it to along the axes they don't span (onto a ground).
     */
    Points { points: Rc<Points>, anchor: Box<SpotAnchor> },
}

pub enum Layer {
    Field(Field),
    Population(Population),
}

#[derive(Clone)]
pub struct KindDef {
    /** For messages (worldgen::contract's margins). */
    pub name: &'static str,
    /**
     * Its class of material, shared by kinds that may stand in for each
     * other far away: the contract compares what rays hit by type, not
     * kind, and look's stats count by it. Not a colour or a behaviour.
     */
    pub kind_type: u8,
    pub colors: &'static [&'static str],
    /**
     * Whether a body walking in the world goes through it, as through
     * plants and water, rather than stands on it and is stopped by it.
     */
    pub walk_through: bool,
    /** Whether it gives light of its colour (light.rs), as a lamp does. */
    pub glows: bool,
    /**
     * Whether it stops a share of every ray, as mist does, and is drawn
     * see-through; the rest are in pieces smaller than a site, as leaves
     * are, which stop a share of the rays whole, and are drawn as grain
     * (VoxelGrid).
     */
    pub medium: bool,
}

/**
 * A domain's kinds, after `base` others (see "Composing" in
 * docs/generators.md): `enum K` of their names, `KINDS` to append to a
 * generator's, and `k` for a name's Kind. A kind followed by `walk_through`
 * is one a body walks through (KindDef::walk_through), by `glows` one that
 * gives light (KindDef::glows), by `medium` one drawn see-through
 * (KindDef::medium).
 */
#[macro_export]
macro_rules! kinds {
    // Whether the flag is among those that follow.
    (@flag $flag:ident) => { false };
    (@flag walk_through walk_through $($rest:ident)*) => { true };
    (@flag glows glows $($rest:ident)*) => { true };
    (@flag medium medium $($rest:ident)*) => { true };
    (@flag $flag:ident $other:ident $($rest:ident)*) => { $crate::kinds!(@flag $flag $($rest)*) };
    (base: $base:expr; $($name:ident: $type:expr, [$($color:expr),*] $(, $flags:ident)*;)*) => {
        #[allow(non_camel_case_types, clippy::upper_case_acronyms)]
        #[derive(Clone, Copy, PartialEq, Eq, Default)]
        #[repr(u8)]
        pub enum K { #[default] $($name),* }
        pub const KINDS: &[$crate::generator::KindDef] = &[$($crate::generator::KindDef { name: stringify!($name), kind_type: $type, colors: &[$($color),*], walk_through: $crate::kinds!(@flag walk_through $($flags)*), glows: $crate::kinds!(@flag glows $($flags)*), medium: $crate::kinds!(@flag medium $($flags)*) }),*];
        /** Kinds of the generator's before these. */
        pub const BASE: $crate::generator::Kind = $base;
        pub fn k(kind: K) -> $crate::generator::Kind {
            BASE + kind as $crate::generator::Kind
        }
        /** A K where a Kind is taken: shapes, parts and columns take either. */
        impl From<K> for $crate::generator::Kind {
            fn from(kind: K) -> $crate::generator::Kind {
                k(kind)
            }
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

/**
 * Which side of a face a site on it counts as on. A box or span keeps the
 * sites whose centres are in [from, to); on a plane of a level's sites
 * (y = 0 at every level), half its lines of sites have one on each face,
 * kept at the bottom, dropped at the top, so it sinks half a site, more
 * the coarser the level. Level 0's sink, half a unit, is the reference:
 * a site on a face counts as above it, along `axis`, with the chance
 * ½ + 1/(2s), sites s apart: always at level 0, nearly half the time far
 * away, so a face is where level 0 has it on average (docs/generators.md,
 * "Levels"). By a hash of the position and axis alone, so boxes and
 * spans meeting on a face share its sites, and a site above at one level
 * is above at the finer ones.
 */
#[derive(Clone, Copy)]
struct Tie {
    s: i64,
    /** Above when the hash is under this, of 2^32. */
    under: u64,
}

impl Tie {
    fn new(s: i64) -> Tie {
        Tie { s, under: (1 << 31) + (1 << 31) / s as u64 }
    }

    /**
     * Whether coordinate `axis` of site `p` is in [lo, hi), all in units
     * `one` to a lattice unit. A box one site thick keeps its bottom face's
     * sites, as at level 0: each line across it has a site on one face
     * only, and halves of both would let rays through.
     */
    fn within(&self, axis: usize, p: [i64; 3], lo: i64, hi: i64, one: i64) -> bool {
        let v = p[axis];
        if lo < v && v < hi {
            return true;
        }
        if v != lo && v != hi {
            return false;
        }
        if hi - lo <= self.s * one {
            return v == lo;
        }
        let [x, y, z] = p.map(|c| c.div_euclid(one));
        ((hash(TIE_SEEDS[axis], x, y, z) as u64) < self.under) == (v == lo)
    }
}

const TIE_SEEDS: [u64; 3] = [0x7a1e_0001, 0x7a1e_0002, 0x7a1e_0003];

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
        extents: radii.map(|r| Fx::from_bits(2 * r)),
        area: effective_area(area, ((16.0 / 3.0) * PI * a * b * c) / area, &fill, centre),
        fill,
        solid: Solid::Ellipsoid { centre: centre_at, radii },
    }
}

pub fn ball(centre: Point, radius: f64, kind: Kind, fill: Fill, op: Op) -> Shape {
    ellipsoid(centre, [radius, radius, radius], kind, fill, op)
}

/** The points within `radius` of the segment from p to q: twigs, logs, trunks; a straight tube, its ends round. */
pub fn capsule(p: Point, q: Point, radius: impl Into<Radius>, kind: Kind, fill: Fill, op: Op) -> Shape {
    swept(&straight(), p, q, radius.into(), false, kind, fill, op)
}

/** A solid of revolution: a crown, a tower, a window's disc; a straight tube, its ends flat. */
pub fn lathe(p: Point, q: Point, radius: impl Into<Radius>, kind: Kind, fill: Fill, op: Op) -> Shape {
    swept(&straight(), p, q, radius.into(), true, kind, fill, op)
}

/**
 * The points within `radius` of `curve` from p to q: a branch, a root.
 * At sites s apart it is drawn along the points of the coarsest level
 * the curve strays from by s / 2 or less, so within half a site of
 * where it is.
 */
pub fn tube(curve: &Curve<3>, p: Point, q: Point, radius: impl Into<Radius>, kind: Kind, fill: Fill, op: Op) -> Shape {
    swept(curve, p, q, radius.into(), false, kind, fill, op)
}

/** A curve refined no times: the segment. */
fn straight() -> Curve<3> {
    Curve::new(Vec::new(), |_, a, _| a)
}

/** A tube, its ends round, or `flat`: cut across the curve there. */
#[allow(clippy::too_many_arguments)]
fn swept(curve: &Curve<3>, p: Point, q: Point, radius: Radius, flat: bool, kind: Kind, fill: Fill, op: Op) -> Shape {
    let points: Vec<At> = curve.points([p, q].map(|e| at(e).map(Fx::from_bits))).into_iter().map(|e| e.map(Fx::to_bits)).collect();
    let n = points.len() - 1;
    let between = |i: usize, j: usize| [0, 1, 2].map(|k| points[j][k] - points[i][k]);
    // How far along each point is, from its start.
    let mut along: Vec<Fixed> = vec![0];
    for i in 0..n {
        let d = between(i, i + 1);
        along.push(along[i] + (dot(d, d) as u64).isqrt() as Fixed);
    }
    let length = along[n];
    let t = |i: usize| if length == 0 { 0 } else { ((i128::from(along[i]) << 16) / i128::from(length)) as Fixed };
    // Level by level, coarsest first, 1, 2, 4… pieces, in one allocation: capsules are many, twigs and leaves.
    let mut pieces = Vec::with_capacity(2 * n - 1);
    for (level, stray) in curve.corridors().enumerate() {
        let step = n >> level;
        pieces.extend((0..n).step_by(step).map(|i| Piece {
            p: points[i],
            segment: Segment::new(between(i, i + step)),
            stray: stray.to_bits(),
            t: [t(i), t(i + step)],
            flat: [flat && i == 0, flat && i + step == n],
        }));
    }
    let (area, volume, widest) = revolved(&radius, float(length), flat);
    let w = fixed(widest);
    // Around each of its points a ball, or at a flat end a disc across it.
    let around = |i: usize| match (flat, i) {
        (true, 0) => disc(between(0, 1), w),
        (true, i) if i == n => disc(between(n - 1, n), w),
        _ => [w; 3],
    };
    Shape {
        kind,
        op,
        centre: at(mid(p, q)),
        bounds: sites_between(
            [0, 1, 2].map(|k| (0..=n).map(|i| points[i][k] - around(i)[k]).min().expect("ends")),
            [0, 1, 2].map(|k| (0..=n).map(|i| points[i][k] + around(i)[k]).max().expect("ends")),
        ),
        extents: [length + if flat { 0 } else { 2 * w }, 2 * w, 2 * w].map(Fx::from_bits),
        area: effective_area(area, (4.0 * volume) / area, &fill, mid(p, q)),
        fill,
        solid: Solid::Tube { pieces, radius, widest: w, length },
    }
}

/**
 * How far a disc of radius w across direction d reaches along each axis:
 * w · sin of the angle to it, sin² with 22 bits after the point
 * (d · d < 2^40), rounded up.
 */
fn disc(d: At, w: Fixed) -> At {
    let (d8, l2) = (d.map(|v| v >> 8), dot(d.map(|v| v >> 8), d.map(|v| v >> 8)));
    d8.map(|v| {
        if l2 == 0 {
            return w;
        }
        let sin2 = (((l2 - v * v) << 22) + l2 - 1) / l2;
        let sin = sin2.isqrt() + i64::from(sin2.isqrt().pow(2) < sin2);
        (w * sin + 2047) >> 11
    })
}

/**
 * A solid of revolution `length` long, of `radius`, by slices: its area,
 * volume and widest radius, its ends half balls, or `flat` discs. The area
 * is its convex hull's, as is the shadow it casts: a crown's whorls have
 * undersides that cast no more (Cauchy holds for convex bodies).
 */
fn revolved(radius: &Radius, length: f64, flat: bool) -> (f64, f64, f64) {
    let n = 8f64.max(length.ceil()) as usize;
    let r: Vec<f64> = (0..=n).map(|i| radius.at(i as f64 / n as f64)).collect();
    let mut hull: Vec<usize> = Vec::new();
    for i in 0..=n {
        let cross = |a: usize, b: usize| (b as f64 - a as f64) * (r[i] - r[a]) - (i as f64 - a as f64) * (r[b] - r[a]);
        while hull.len() >= 2 && cross(hull[hull.len() - 2], hull[hull.len() - 1]) >= 0.0 {
            hull.pop();
        }
        hull.push(i);
    }
    let caps = if flat { 1.0 } else { 2.0 };
    let mut area = caps * PI * (r[0] * r[0] + r[n] * r[n]);
    let mut volume = if flat { 0.0 } else { (2.0 / 3.0) * PI * (js::pow(r[0], 3.0) + js::pow(r[n], 3.0)) };
    for h in 0..hull.len().saturating_sub(1) {
        let (a, b) = (hull[h], hull[h + 1]);
        area += PI * (r[a] + r[b]) * js::hypot(&[((b - a) as f64 * length) / n as f64, r[b] - r[a]]);
    }
    for i in 0..n {
        volume += (PI * (r[i] * r[i] + r[i + 1] * r[i + 1]) / 2.0) * (length / n as f64);
    }
    (area, volume, js::max(r.iter().copied()))
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
        extents: [0, 1, 2].map(|i| Fx::from_bits(hi[i] - lo[i])),
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
        extents: [hi[ridge] - lo[ridge], hi[across] - lo[across], hi[1] - lo[1]].map(Fx::from_bits),
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
    /** Its length, √l2. */
    l: i64,
    inverse: i64,
}

impl Segment {
    fn new(d: At) -> Segment {
        let d = d.map(|v| v >> 8);
        let l2 = dot(d, d);
        Segment { d, l2, l: (l2 as u64).isqrt() as i64, inverse: if l2 == 0 { 0 } else { (1 << 62) / l2 } }
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

/**
 * A tube's piece: from p along `segment`, of a level whose points the
 * curve strays from by `stray`, from `t[0]` to `t[1]` along the tube
 * (fractions of ONE), and whether the tube ends flat at its start or end.
 */
#[derive(Clone, Copy)]
struct Piece {
    p: At,
    segment: Segment,
    stray: Fixed,
    t: [Fixed; 2],
    flat: [bool; 2],
}

/**
 * How thick a tube is, tested at one `min`: within √r2 (its widest, at
 * least half min), and where its radius changes, within `radius` (at
 * least `half`) at the nearest point, so a piece is about a cone; `short`,
 * how much shorter than min it is (8 bits after the point).
 */
#[derive(Clone, Copy)]
struct Thick<'a> {
    r2: i64,
    radius: Option<&'a dyn Fn(f64) -> f64>,
    half: Fixed,
    short: i64,
}

impl Piece {
    fn holds(&self, x: At, thick: &Thick) -> bool {
        let v = offset(x, self.p);
        let vd = dot(v, self.segment.d);
        // Nothing past a flat end, but by half what the tube is short of min (in units of v · d).
        let reach = self.segment.l * thick.short / 2;
        if (self.flat[0] && vd < -reach) || (self.flat[1] && vd > self.segment.l2 + reach) {
            return false;
        }
        let away = self.segment.away(v, vd);
        if away > thick.r2 {
            return false;
        }
        let Some(radius) = thick.radius else { return true };
        let [t0, t1] = self.t;
        let r = fixed(radius(float(t0 + (((t1 - t0) * self.segment.along(vd)) >> 16)))).max(thick.half) >> 8;
        away <= r * r
    }
}

/** A shape's test for one `min`, with what does not depend on the point worked out. */
enum Inside<'a> {
    Nothing,
    Ellipsoid {
        centre: At,
        inverse: [i64; 3],
    },
    /** A tube drawn in one piece, as most are, by value: looping over one cost a few percent at level 0. */
    Piece {
        piece: Piece,
        thick: Thick<'a>,
    },
    /** The pieces of the level drawn. */
    Tube {
        pieces: &'a [Piece],
        thick: Thick<'a>,
    },
    Cuboid {
        lo: At,
        hi: At,
        min: Fixed,
        tie: Tie,
    },
    /** Under the slopes: |2u − lo − hi| · height ≤ width · (top − y), with 8 bits after the point. */
    Gable {
        lo: At,
        hi: At,
        min: Fixed,
        across: usize,
        tie: Tie,
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
            Inside::Piece { piece, thick } => piece.holds(x, thick),
            Inside::Tube { pieces, thick } => pieces.iter().any(|piece| piece.holds(x, thick)),
            Inside::Cuboid { lo, hi, min, tie } => (0..3).all(|i| {
                if hi[i] - lo[i] >= *min { tie.within(i, x, lo[i], hi[i], ONE) } else { (2 * x[i] - lo[i] - hi[i]).abs() <= *min }
            }),
            Inside::Gable { lo, hi, min, across, tie } => {
                let a = *across;
                let ridge = 2 - a;
                let side = |i: usize| {
                    if hi[i] - lo[i] >= *min { tie.within(i, x, lo[i], hi[i], ONE) } else { (2 * x[i] - lo[i] - hi[i]).abs() <= *min }
                };
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
    /** Whether a point is inside the shape with each of its sizes made at least `min`, at sites `s` apart. */
    fn inside(&self, min: Fixed, s: i64) -> Inside<'_> {
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
            Solid::Tube { pieces, radius, widest, length } => {
                // The coarsest level within half a site of the curve; level l is pieces 2^l − 1 to 2^(l+1) − 2.
                let level = (0..).find(|&l| pieces[(1 << l) - 1].stray <= s * ONE / 2).expect("the finest strays nowhere");
                let radius = match radius {
                    Radius::Const(_) => None,
                    Radius::Along(radius) => Some(radius.as_ref()),
                };
                let thick = Thick { r2: q8((*widest).max(half)).pow(2), radius, half, short: q8(0.max(min - length)) };
                match &pieces[(1 << level) - 1..(2 << level) - 1] {
                    [piece] => Inside::Piece { piece: *piece, thick },
                    pieces => Inside::Tube { pieces, thick },
                }
            }
            Solid::Cuboid { min: lo, max: hi } => Inside::Cuboid { lo: *lo, hi: *hi, min, tie: Tie::new(s) },
            Solid::Gable { min: lo, max: hi, ridge } => Inside::Gable { lo: *lo, hi: *hi, min, across: 2 - ridge, tie: Tie::new(s) },
        }
    }
}

/**
 * Draws a shape at sites `s` apart, by how many of its sizes are under s;
 * returns how many sites it tested.
 */
fn draw_shape(shape: &Shape, seed: u64, si: i64, clip: &Box3, put: &mut dyn FnMut(u32, u32, i64, i64, i64)) -> u64 {
    let s = si as f64;
    let spacing = si * ONE;
    let small = shape.extents.iter().filter(|e| e.to_bits() < spacing).count();
    let picked = |x: i64, y: i64, z: i64| to_pick(hash(seed, x * si, y * si, z * si));
    if small == 3 {
        let [x, y, z] = nearest_site(shape.centre, si);
        let chance = shape.area / (VOXEL_AREA * s * s);
        if clip.holds([x, y, z]) {
            put(share(chance), picked(x, y, z), x, y, z);
        }
        return 1;
    }
    let min = [0, spacing, spacing * 8 / 5][small];
    let b = &shape.bounds;
    // A box's sites on its top faces may count as under them (Tie), past its bounds but at level 0.
    let past = if matches!(shape.solid, Solid::Cuboid { .. } | Solid::Gable { .. }) { 0 } else { ONE };
    let [x0, y0, z0] = b.min.map(|v| -(-(v * ONE - min / 2)).div_euclid(spacing));
    let [x1, y1, z1] = b.max.map(|v| (v * ONE - past + min / 2).div_euclid(spacing));
    let inside = shape.inside(min, si);
    // Visits the sites within that are inside; returns about how many it tested (half of each row's points).
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
        [z_to - z_from, y_to - y_from, (x_to - from).div_euclid(2)].iter().map(|n| (n + 1).max(0) as u64).product::<u64>()
    };
    let fill = |x: i64, y: i64, z: i64| shape.fill.at((x * si) as f64, (y * si) as f64, (z * si) as f64);
    match small {
        0 => sites(clip, &mut |x, y, z| {
            let chance = 1.0 - js::pow(1.0 - fill(x, y, z), s);
            put(share(chance), picked(x, y, z), x, y, z)
        }),
        1 => {
            let thickness = shape.extents.into_iter().min().expect("three").to_f64();
            sites(clip, &mut |x, y, z| {
                let chance = 1.0 - js::pow(1.0 - fill(x, y, z), thickness / CHORD);
                put(share(chance), picked(x, y, z), x, y, z)
            })
        }
        _ => {
            // Counted whole, so every box gets the same chance.
            let mut count = 0;
            let tested = sites(&EVERYWHERE, &mut |_, _, _| count += 1);
            let chance = shape.area / (count as f64 * VOXEL_AREA * s * s);
            tested + sites(clip, &mut |x, y, z| put(share(chance), picked(x, y, z), x, y, z))
        }
    }
}

// ---------------------------------------------------------------------------
// Generating

#[derive(Clone, Debug, PartialEq)]
pub struct Material {
    pub color: &'static str,
    pub material_type: u8,
    /** Its kind's KindDef::walk_through. */
    pub walk_through: bool,
    /** Its kind's KindDef::glows. */
    pub glows: bool,
    /** Its kind's KindDef::medium. */
    pub medium: bool,
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
    /** By material, what it glows: its colour, linear, of 255; none if its kind does not glow. */
    glows: Vec<[u32; 3]>,
    seed: u64,
}

impl Palette {
    fn new(kinds: &[KindDef], seed: u64) -> Palette {
        let mut materials = vec![Material { color: "", material_type: 0, walk_through: true, glows: false, medium: false }];
        let shades = kinds
            .iter()
            .map(|k| {
                k.colors
                    .iter()
                    .map(|color| {
                        materials.push(Material {
                            color,
                            material_type: k.kind_type,
                            walk_through: k.walk_through,
                            glows: k.glows,
                            medium: k.medium,
                        });
                        (materials.len() - 1) as u8
                    })
                    .collect()
            })
            .collect();
        assert!(materials.len() <= 256, "{} materials: ids are bytes, so at most 255 and none", materials.len());
        let glows = materials
            .iter()
            .map(|m| {
                let rgb = m.rgb();
                let linear = |shift: u32| (255.0 * srgb_to_linear(((rgb >> shift) & 255) as f32 / 255.0)).round() as u32;
                if m.glows { [linear(16), linear(8), linear(0)] } else { [0; 3] }
            })
            .collect();
        Palette { materials, shades, glows, seed }
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
    let mut put = |look: Look, op: Op, share: u32, pick: u32, x: i64, y: i64, z: i64| {
        let shade = |kind: Kind| palette.shade(kind, x * s, y * s, z * s);
        let id = shade(look.kind);
        let faces = look.faces.map(|faces| faces.map(shade));
        let medium = palette.materials[id as usize].medium;
        see(&mut grid, op, share, pick, (id, faces, medium), palette.glows[id as usize], x, y, z);
    };
    let drawables = drawables_in(generator, &clip, s);
    let mut at = 0;
    while at < drawables.len() {
        // A thing's drawables, together: those of fields one by one.
        let thing = drawables[at].thing;
        let end = if thing == 0 { at + 1 } else { (at..drawables.len()).find(|i| drawables[*i].thing != thing).unwrap_or(drawables.len()) };
        let group = &drawables[at..end];
        at = end;
        if group[0].checked {
            let mut drew = None;
            for drawable in group {
                draw(drawable, generator.seed, s, &clip, &mut |_, _, share, _, x, y, z| drew = drew.or((share > 0).then_some([x, y, z])));
            }
            if let Some(site) = drew {
                broke(&group[0].key, site, s, &clip);
            }
            continue;
        }
        let carves = group.iter().any(|d| matches!(&d.draw, Draw::Shapes(shapes, _) if shapes.iter().any(|s| s.op == Op::Carve)));
        let mut work = Work::default();
        if !carves {
            for drawable in group {
                work.add(&draw(drawable, generator.seed, s, &clip, &mut put));
            }
        } else {
            // It cuts only itself: drawn aside, then over the world.
            let mut own = Own::new(&thing_box(group, s).intersect(&clip));
            for drawable in group {
                work.add(&draw(drawable, generator.seed, s, &clip, &mut |look, op, share, pick, x, y, z| {
                    own.put(look, op, share, pick, x, y, z)
                }));
            }
            own.each(&mut put);
        }
        if thing != 0 {
            work.largest = work.tests;
        }
        cost::add(group[0].layer, work);
    }
    // One coin per site: its pieces shown as often as they stop rays.
    let coins = hash_text("shown", generator.seed);
    grid.show(|x, y, z| ((hash(coins, x * s, y * s, z * s) as u64 * ALPHA as u64) >> 32) as u8);
    grid
}

/**
 * Adds a thing's `share` of site (x, y, z) to it (VoxelGrid's `pieces`,
 * `medium`, `glow`), of material `id` (its faces' if they differ) glowing
 * `glow`: as a Fill behind what is there, a Paint in front of it, a Carve
 * taking it out, of both. A `medium` joins the site's medium, the rest
 * its pieces. What either shows is one of what is in it, each as often
 * as it is seen there: the thing replaces it as often as it makes up of
 * what they now stop, by its pick.
 */
#[allow(clippy::too_many_arguments)]
fn see(
    grid: &mut VoxelGrid,
    op: Op,
    share: u32,
    pick: u32,
    (id, faces, medium): (u8, Option<Faces>, bool),
    glow: [u32; 3],
    x: i64,
    y: i64,
    z: i64,
) {
    if share == 0 {
        return;
    }
    let Some(i) = grid.index(x, y, z) else { return };
    let part = if medium { &mut grid.medium } else { &mut grid.pieces };
    let (alpha, share) = (part.alpha[i] as u64, share.min(SHARE) as u64);
    let (all, one) = (ALPHA as u64, SHARE as u64);
    let old = grid.glow.get(&i).copied().unwrap_or_default();
    // Of SHARE: how much of what was there is still seen, and how much of the site the thing is seen in.
    let (kept, visible) = match op {
        Op::Carve => (one - share, 0),
        Op::Fill => (one, (all - alpha) * share / all),
        Op::Paint => (one - share, share),
    };
    // In SHARE-ths of a step of alpha, rounded up as often as the remainder,
    // by a coin from the pick (its bits scrambled by an odd factor):
    // cut off, all of a speck's share under one step was lost far away.
    let coin = (pick as u64 * 40_503) % one;
    let rounded = |exact: u64| (exact / one + u64::from(exact % one > coin)) as u8;
    let exact = alpha * one + (all - alpha) * share;
    if op == Op::Carve {
        for part in [&mut grid.pieces, &mut grid.medium] {
            part.alpha[i] = rounded(part.alpha[i] as u64 * kept);
        }
    } else {
        part.alpha[i] = rounded(exact);
    }
    // As often as it makes up of what the site stops before that rounding: of `now`, a medium of a few
    // SHARE-ths of a step was seen as seldom as rounded up, and nine in ten of its sites that stop rays showed nothing.
    if (pick as u64) * exact < visible * all * one {
        match medium {
            true => grid.medium.seen[i] = id,
            false => grid.see(i, id, faces),
        }
    }
    let glow = [0, 1, 2].map(|c| (old[c] as u64 * kept / one + visible * glow[c] as u64) as u32);
    if glow != [0; 3] {
        grid.glow.insert(i, glow);
    } else if old != [0; 3] {
        grid.glow.remove(&i);
    }
}

/**
 * A thing's sites, drawn aside from the world: in a box of a level's
 * sites, what it puts at each since it last carved it, put on the world
 * as they came (`each`).
 */
struct Own {
    b: Box3,
    n: [i64; 3],
    /** By site, its last put in `puts`; NONE if none. */
    cells: Vec<u32>,
    /** Puts: each what it is of, how it meets the world, its share and pick (Put), and the site's put before it. */
    puts: Vec<(Look, Op, u32, u32, u32)>,
}

const NONE: u32 = u32::MAX;

impl Own {
    fn new(b: &Box3) -> Own {
        let n = [0, 1, 2].map(|i| (b.max[i] - b.min[i]).max(0));
        Own { b: *b, n, cells: vec![NONE; (n[0] * n[1] * n[2]) as usize], puts: Vec::new() }
    }

    fn index(&self, x: i64, y: i64, z: i64) -> usize {
        assert!(self.b.holds([x, y, z]), "a thing drew at ({x}, {y}, {z}), outside its bounds {:?}", self.b);
        let [x, y, z] = [x - self.b.min[0], y - self.b.min[1], z - self.b.min[2]];
        (x + self.n[0] * (y + self.n[1] * z)) as usize
    }

    /** Within the thing only: a Carve, where it keeps a voxel, takes out what the thing put there. */
    #[allow(clippy::too_many_arguments)]
    fn put(&mut self, look: Look, op: Op, share: u32, pick: u32, x: i64, y: i64, z: i64) {
        let i = self.index(x, y, z);
        if op == Op::Carve {
            if pick < share {
                self.cells[i] = NONE;
            }
            return;
        }
        self.puts.push((look, op, share, pick, self.cells[i]));
        self.cells[i] = (self.puts.len() - 1) as u32;
    }

    /** The puts at index `i` of `cells`, last first. */
    fn puts_at(&self, i: usize) -> impl Iterator<Item = &(Look, Op, u32, u32, u32)> {
        std::iter::successors(self.puts.get(self.cells[i] as usize), |p| self.puts.get(p.4 as usize))
    }

    /** What the thing has at (x, y, z), as `of`. */
    fn at(&self, x: i64, y: i64, z: i64) -> Option<(Kind, Op)> {
        self.of(self.index(x, y, z))
    }

    /**
     * What the thing has at index `i` of `cells`, its voxels kept by chance:
     * its site's kind (measured, its faces are measured anew) and how it meets the world.
     */
    fn of(&self, i: usize) -> Option<(Kind, Op)> {
        // Last first: the last Paint kept, else the first Fill kept.
        let mut fill = None;
        for &(look, op, ..) in self.puts_at(i).filter(|p| p.3 < p.2) {
            match op {
                Op::Paint => return Some((look.kind, op)),
                _ => fill = Some((look.kind, op)),
            }
        }
        fill
    }

    /** Puts what the thing put, as it came. */
    fn each(&self, visit: &mut Put) {
        for i in 0..self.cells.len() {
            let puts: Vec<_> = self.puts_at(i).collect();
            let j = i as i64;
            let (x, y, z) =
                (j % self.n[0] + self.b.min[0], (j / self.n[0]) % self.n[1] + self.b.min[1], j / (self.n[0] * self.n[1]) + self.b.min[2]);
            for &&(look, op, share, pick, _) in puts.iter().rev() {
                visit(look, op, share, pick, x, y, z);
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

/** Draws one drawable at sites `s` apart into `clip`; returns the work. */
fn draw(drawable: &Drawable, seed: u64, s: i64, clip: &Box3, put: &mut Put) -> Work {
    match &drawable.draw {
        Draw::Field(field) => draw_field(field, seed, s, clip, put),
        Draw::Shapes(shapes, far) => {
            let mut tests = 0;
            for (i, shape) in shapes.iter().enumerate() {
                let seed = hash_text(&format!("{}#{}{i}", drawable.key, if *far { "far" } else { "" }), seed);
                tests +=
                    draw_shape(shape, seed, s, clip, &mut |share, pick, x, y, z| put(shape.kind.into(), shape.op, share, pick, x, y, z));
            }
            Work { nodes: 1, tests, ..Work::default() }
        }
        Draw::Sites(sites) => {
            for &([x, y, z], look, op, share, pick) in sites {
                if clip.holds([x, y, z]) {
                    put(look, op, share, pick, x, y, z);
                }
            }
            Work { nodes: 1, tests: sites.len() as u64, ..Work::default() }
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
    /** The layer it is of, for its cost. */
    layer: &'static str,
    /** The thing it is part of, numbered from 1; 0 for fields. */
    thing: u32,
    draw: Draw<'a>,
    /** Drawn only to see it draws nothing: its population said it stands elsewhere (`checking_skipped`). */
    checked: bool,
}

/** What draws into `clip` (sites s apart), in drawing order: by layer, then key. */
fn drawables_in<'a>(generator: &'a Generator, clip: &Box3, s: i64) -> Vec<Drawable<'a>> {
    // The lattice units the sites of `clip` cover, with what a shape may grow by at this level.
    let sites = units_of(clip, s);
    let units = sites.widen(s);
    let mut drawables = Vec::new();
    let mut things = 0;
    for layer in &generator.layers {
        let population = match layer {
            Layer::Field(field) => {
                // Asked where it may draw line by line (draw_field).
                let (order, key, layer) = (field.order, field.name.to_string(), field.name);
                drawables.push(Drawable { order, key, layer, thing: 0, draw: Draw::Field(field), checked: false });
                continue;
            }
            Layer::Population(p) => p,
        };
        let checked = !stands_within(population, &sites);
        if checked && !checking() {
            continue;
        }
        if s > population.size {
            if let Some(far) = &population.far {
                let (order, layer) = (population.order, population.name);
                let (key, draw) = (population.name.to_string(), Draw::Field(far));
                drawables.push(Drawable { order, key, layer, thing: 0, draw, checked });
            }
            continue;
        }
        let n = population.cell;
        // Cells whose reach meets `units`: centres within N + reach of it.
        let cells = [0, 1, 2].map(|axis| {
            let reach = population.reach[axis];
            (-(-(units.min[axis] - n - reach)).div_euclid(n), (units.max[axis] - 1 + n + reach).div_euclid(n))
        });
        let work = each_thing(generator, population, cells, |cell, _, node| {
            let (key, bounds, reach) = (cell.key(), bounds_of(&node), reach_of(population, cell));
            assert!(bounds.inside(&reach), "{}", past_reach(&key, &bounds, &reach, n, population.reach));
            things += 1;
            let (order, layer, seed) = (population.order, population.name, generator.seed);
            let at = Visit { s, units: &units, order, layer, thing: things, seed, checked };
            visit(&node, key, bounds, &at, &mut drawables);
        });
        cost::add(population.name, work);
    }
    drawables.sort_by(|a, b| a.order.total_cmp(&b.order).then_with(|| a.key.cmp(&b.key)));
    drawables
}

/** Why a thing within `bounds` is past its cell's `reach` (N and the population's reach past its centre): along which axes, by how much, what would hold it. */
fn past_reach(key: &str, bounds: &Box3, reach: &Box3, n: i64, population: [i64; 3]) -> String {
    let centre = [0, 1, 2].map(|i| (reach.min[i] + reach.max[i]) / 2);
    let needed = [0, 1, 2].map(|i| population[i].max(centre[i] - n - bounds.min[i]).max(bounds.max[i] - centre[i] - n));
    let axes: Vec<String> = (0..3)
        .filter(|i| needed[*i] > population[*i])
        .map(|i| format!("{} by {}", ["x", "y", "z"][i], needed[i] - population[i]))
        .collect();
    format!(
        "{key} reaches past its population's reach along {}: its bounds are {bounds:?}, its cell's {reach:?} \
         (N = {n} and reach {population:?} from its centre); reach {needed:?} would hold this one, \
         but derive it from the largest possible thing; see \"Populations\" in {DOCS}",
        axes.join(", ")
    )
}

/** A thing's sites of a level: each what it is of, how it meets the world, its share and pick (Put). */
type Sites = Vec<([i64; 3], Look, Op, u32, u32)>;

/** Where a node is drawn: at sites `s` apart, for the lattice units `units`, in its layer's `order`, as part of `thing`. */
struct Visit<'u> {
    s: i64,
    units: &'u Box3,
    order: f64,
    layer: &'static str,
    thing: u32,
    seed: u64,
    checked: bool,
}

/** Adds a node's drawables and its children's, as far as they reach `at.units` and are drawn at its spacing. */
fn visit(node: &Node, key: String, bounds: Box3, at: &Visit, drawables: &mut Vec<Drawable<'_>>) {
    let Visit { s, units, order, layer, thing, seed, checked } = *at;
    if !bounds.overlaps(units) {
        return;
    }
    let draw = match node.far.as_ref().filter(|far| s * ONE > far.detail.to_bits()) {
        Some(Far { detail, form: FarForm::Measured }) => Draw::Sites(measure(node, &key, layer, bounds, detail.to_bits(), s, seed)),
        Some(Far { form: FarForm::Shapes(shapes), .. }) => Draw::Shapes(shapes.to_vec(), true),
        None => return near(node, key, bounds, at, drawables),
    };
    drawables.push(Drawable { order, key, layer, thing, draw, checked });
}

/** Adds a node's near form, whatever the spacing: its shapes, and its children's drawables (`visit`). */
fn near(node: &Node, key: String, bounds: Box3, at: &Visit, drawables: &mut Vec<Drawable<'_>>) {
    let Visit { s, units, order, layer, thing, checked, .. } = *at;
    drawables.push(Drawable { order, key: key.clone(), layer, thing, draw: Draw::Shapes(node.shapes.get().to_vec(), false), checked });
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

/** How a node's drawables are added: `visit`, or `near` for its near form. */
type Form = fn(&Node, String, Box3, &Visit, &mut Vec<Drawable>);

/** A node of `layer` drawn alone, whole, at sites `f` apart, its drawables added by `form`. */
fn alone(node: &Node, key: &str, layer: &'static str, bounds: Box3, f: i64, seed: u64, form: Form) -> Own {
    let units = bounds.widen(f);
    let mut drawables = Vec::new();
    form(node, key.to_string(), bounds, &Visit { s: f, units: &units, order: 0.0, layer, thing: 1, seed, checked: false }, &mut drawables);
    let fine = Box3 { min: bounds.min.map(|v| v.div_euclid(f) - 2), max: bounds.max.map(|v| -(-v).div_euclid(f) + 2) };
    let mut own = Own::new(&fine);
    let mut work = Work::default();
    for drawable in &drawables {
        work.add(&draw(drawable, seed, f, &fine, &mut |look, op, share, pick, x, y, z| own.put(look, op, share, pick, x, y, z)));
    }
    cost::add(layer, work);
    own
}

/**
 * The pairs of neighbouring columns along an axis that meet an n[0] × n[1]
 * box of them from (0, 0), to `pair`. Seen along an axis, a voxel is a
 * square turned 45° round its column (|u| + |v| ≤ 1, columns 1 apart),
 * twice a column's area: the voxels of the columns whose u + v is even
 * cover the plane once, those of the odd ones once more. So a ray along an
 * axis meets the voxels of two neighbouring columns, one of each, and half
 * a column's area of rays those of each pair: what a thing hides along the
 * axis is half a column's area for each pair either of which holds one of
 * its sites. Counted by columns, a sheet one site thick hid half what it
 * does, and a lone voxel half its shadow.
 */
fn pairs(n: [i64; 2], mut pair: impl FnMut([i64; 2], [i64; 2])) {
    for i in -1..n[0] {
        for j in 0..n[1] {
            pair([i, j], [i + 1, j]);
        }
    }
    for i in 0..n[0] {
        for j in -1..n[1] {
            pair([i, j], [i, j + 1]);
        }
    }
}

/**
 * Columns along an axis, an n[0] × n[1] box of them from (0, 0), each
 * holding a site or not: what a thing hides along the axis.
 */
#[derive(Clone)]
struct Columns {
    n: [i64; 2],
    held: Vec<bool>,
}

impl Columns {
    fn new(n: [i64; 2]) -> Columns {
        Columns { n, held: vec![false; (n[0] * n[1]) as usize] }
    }

    /** Whether column `p` holds a site: none outside the box. */
    fn holds(&self, p: [i64; 2]) -> bool {
        (0..2).all(|k| (0..self.n[k]).contains(&p[k])) && self.held[(p[0] * self.n[1] + p[1]) as usize]
    }

    fn set(&mut self, p: [i64; 2]) {
        self.held[(p[0] * self.n[1] + p[1]) as usize] = true;
    }

    /** Those any of `planes` holds, of the box of the first (none if there are none). */
    fn union<'a>(planes: impl IntoIterator<Item = &'a Columns>) -> Columns {
        let mut planes = planes.into_iter();
        let mut out = planes.next().cloned().unwrap_or_else(|| Columns::new([0, 0]));
        for plane in planes {
            out.held.iter_mut().zip(&plane.held).for_each(|(o, h)| *o |= h);
        }
        out
    }

    /** Rows, each a box one column deep, one after another along the box's first axis. */
    fn stacked<'a>(rows: impl ExactSizeIterator<Item = &'a Columns>) -> Columns {
        let height = rows.len() as i64;
        let held: Vec<bool> = rows.flat_map(|row| row.held.iter().copied()).collect();
        Columns { n: [height, held.len() as i64 / height.max(1)], held }
    }

    /** How many pairs of neighbouring columns (`pairs`) meeting the box either of which holds a site. */
    fn pairs_held(&self) -> u32 {
        let mut count = 0;
        pairs(self.n, |p, q| count += u32::from(self.holds(p) || self.holds(q)));
        count
    }

    /** What the sites on them hide along the axis, in columns' areas (`pairs`). */
    fn shadow(&self) -> f64 {
        self.pairs_held() as f64 / 2.0
    }
}

/**
 * A layer of a thing across an axis: what it hides seen along the axis, the columns along each of the other two
 * it holds (one deep), and how many of its sites are of each kind.
 */
struct Slice {
    across: Columns,
    along: [Columns; 2],
    kinds: Vec<(Kind, u32)>,
}

/**
 * What slabs of things hide, measured: things drawn alone at a level,
 * and for each slab of them across an axis what it adds to their shadow,
 * on average over directions and the things. A population's far form,
 * from things made at the origin, is as dense as that (extinction =
 * things per volume · their mean shadow), each slab a medium of its own.
 */
pub struct Shadows {
    f: i64,
    axis: usize,
    /** By thing, its first site along each axis, and its layers across the axis from the first. */
    things: Vec<([i64; 3], Vec<Slice>)>,
}

/**
 * How much of what a slab adds to a thing's shadow across it, and of its
 * shadow along the other axes, it shows on average over directions. A
 * convex body's mean shadow is a quarter of its area (Cauchy): for a
 * cylinder, half its shadow along its axis and π/4 of that across it,
 * for a ball half of each. Media are isotropic: denser, the canopy hides
 * more from above than its trees; thinner, less among them; these keep
 * both best ("Levels" in docs/generators.md).
 */
const ACROSS: f64 = 0.5;
const ALONG: f64 = 2.0 / 3.0;

impl Shadows {
    pub fn new(nodes: &[Node], level: u32, axis: usize) -> Shadows {
        let f = 1i64 << level;
        let (a, b) = ((axis + 1) % 3, (axis + 2) % 3);
        let things = nodes
            .iter()
            .enumerate()
            .map(|(i, node)| {
                let own = alone(node, &format!("sample {i}"), cost::SHADOWS, bounds_of(node), f, 0, visit);
                let (n, min) = (own.n, own.b.min);
                let layers = (0..n[axis])
                    .map(|l| {
                        let mut slice = Slice {
                            across: Columns::new([n[a], n[b]]),
                            along: [[1, n[b]], [1, n[a]]].map(Columns::new),
                            kinds: Vec::new(),
                        };
                        for i in 0..n[a] {
                            for j in 0..n[b] {
                                let mut p = [0; 3];
                                (p[axis], p[a], p[b]) = (l + min[axis], i + min[a], j + min[b]);
                                if let Some((kind, _)) = own.at(p[0], p[1], p[2]) {
                                    match slice.kinds.iter_mut().find(|k| k.0 == kind) {
                                        Some(k) => k.1 += 1,
                                        None => slice.kinds.push((kind, 1)),
                                    }
                                    slice.across.set([i, j]);
                                    slice.along[0].set([0, j]);
                                    slice.along[1].set([0, i]);
                                }
                            }
                        }
                        slice
                    })
                    .collect();
                (min, layers)
            })
            .collect();
        Shadows { f, axis, things }
    }

    /** The share of the things' sites in the slab from `from` to `to` (exclusive) along the axis whose kinds `of` holds. */
    pub fn share(&self, from: i64, to: i64, of: impl Fn(Kind) -> bool) -> f64 {
        let (mut sum, mut all) = (0u64, 0u64);
        for (min, layers) in &self.things {
            let ceil = |v: i64| (-(-v).div_euclid(self.f) - min[self.axis]).clamp(0, layers.len() as i64) as usize;
            for slice in &layers[ceil(from)..ceil(to)] {
                for &(kind, n) in &slice.kinds {
                    all += n as u64;
                    sum += if of(kind) { n as u64 } else { 0 };
                }
            }
        }
        if all == 0 { 0.0 } else { sum as f64 / all as f64 }
    }

    /**
     * The mean shadow (lattice units²) over directions of what the slab
     * from `from` to `to` (exclusive) along the axis adds to the things':
     * across it, what it adds seen from either side, as slabs of a thing
     * overlap (from above, 2 to 3 times the tree's own in 2 m slabs);
     * along the other axes, its own, as slabs side by side don't.
     */
    pub fn slab(&self, from: i64, to: i64) -> f64 {
        self.mean(from, to, |adds, along| ACROSS * adds + ALONG * along / 2.0)
    }

    /**
     * What the slab from `from` to `to` (exclusive) adds to the things'
     * shadow seen across it, from either side (lattice units²): what the
     * slabs add sums to the shadow.
     */
    pub fn across(&self, from: i64, to: i64) -> f64 {
        self.mean(from, to, |adds, _| adds)
    }

    /** The mean over the things of `of`(what their slab adds to their shadow across the axis, its own shadow along the other two). */
    fn mean(&self, from: i64, to: i64, of: impl Fn(f64, f64) -> f64) -> f64 {
        let (f, mut sum) = (self.f, 0.0);
        for (min, layers) in &self.things {
            let ceil = |v: i64| -(-v).div_euclid(f) - min[self.axis];
            let (lo, hi) = (ceil(from).clamp(0, layers.len() as i64) as usize, ceil(to).clamp(0, layers.len() as i64) as usize);
            if lo >= hi {
                continue;
            }
            let across = |range: std::ops::Range<usize>| Columns::union(layers[range].iter().map(|s| &s.across)).shadow();
            let n = layers.len();
            let adds = (across(lo..n) - across(hi..n) + across(0..hi) - across(0..lo)) / 2.0;
            // Along each of the other two, the slab's layers stacked: a plane of the axis and the third.
            let along: f64 = (0..2).map(|k| Columns::stacked(layers[lo..hi].iter().map(|s| &s.along[k])).shadow()).sum();
            sum += of(adds, along) * (f * f) as f64;
        }
        sum / self.things.len() as f64
    }

    /**
     * Where the things' shadow across the axis falls, for things anywhere
     * in a cell `size` wide (of `Points`, cornered at multiples of it): its
     * mean area (lattice units²) over the column of sites `size` apart at
     * (0, 0), by the cell's offset from it in cells (zero along the axis).
     * A column reaches half its width round its site, so a cell meets four,
     * and a part of the shadow covers each column as often as the thing's
     * place in the cell puts it there.
     */
    pub fn spread(&self, size: i64) -> Vec<([i64; 3], f64)> {
        let (f, axis) = (self.f, self.axis);
        let (a, b) = ((axis + 1) % 3, (axis + 2) % 3);
        // The cells a part p from the thing's foot (given as 2p) covers the column from, with how often: the overlap
        // of the column with the part's places, the cell's corner plus [0, size).
        let cells = |p2: i64| {
            let (r2, size2) = (p2 + size, 2 * size);
            let first = -(r2 + size2 - 1).div_euclid(size2);
            (first..=first + 1).filter_map(move |o| {
                let w = 1.0 - (o * size2 + r2).abs() as f64 / size2 as f64;
                (w > 0.0).then_some((o, w))
            })
        };
        let mut areas = std::collections::BTreeMap::<[i64; 3], f64>::new();
        for (min, layers) in &self.things {
            // Half a column's area at the middle of each pair of neighbouring columns holding a site (`pairs`), its
            // place in halves of the sites' spacing.
            let plane = Columns::union(layers.iter().map(|s| &s.across));
            pairs(plane.n, |p, q| {
                if plane.holds(p) || plane.holds(q) {
                    for (oa, wa) in cells((p[0] + q[0] + 2 * min[a]) * f) {
                        for (ob, wb) in cells((p[1] + q[1] + 2 * min[b]) * f) {
                            let mut offset = [0; 3];
                            (offset[a], offset[b]) = (oa, ob);
                            *areas.entry(offset).or_insert(0.0) += wa * wb * (f * f) as f64 / 2.0;
                        }
                    }
                }
            });
        }
        let things = self.things.len() as f64;
        areas.into_iter().map(|(o, area)| (o, area / things)).collect()
    }
}

/**
 * How many pairs of neighbouring columns along each axis (`pairs`) of the
 * sites `r` times finer than a cell hold one of them in the cell, either
 * of the two: by halves of a column's area, about the 2 r² columns' area
 * of its shadow (a cell's are 2 r² wide), and unevenly by axis, as
 * nearest_site breaks ties one way. Cells of a level are congruent, ties
 * and all (they differ by an even number of cells along the axes), so the
 * count depends on r only.
 */
fn pairs_of_cell(r: i64) -> [u32; 3] {
    static PAIRS: [OnceLock<[u32; 3]>; 64] = [const { OnceLock::new() }; 64];
    *PAIRS[r.trailing_zeros() as usize].get_or_init(|| {
        std::array::from_fn(|axis| {
            let (a, b) = ((axis + 1) % 3, (axis + 2) % 3);
            // The columns from (−r, −r), as the cell reaches r along the axes.
            let mut cell = Columns::new([2 * r + 1; 2]);
            for [i, j] in (-r..=r).flat_map(|i| (-r..=r).map(move |j| [i, j])) {
                // The cell is symmetric about its centre's plane across the column, and where a column runs
                // in one of its faces, ties go one way all along it: if the column has a site in the cell,
                // it has one within 2 of that plane (checked by a_cells_pairs_of_columns_are_counted).
                let holds = (-2..=2i64).filter(|k| (i + j + k) & 1 == 0).any(|k| {
                    let mut p = [0; 3];
                    (p[axis], p[a], p[b]) = (k, i, j);
                    nearest_site(p.map(|v| v * ONE), r) == [0; 3]
                });
                if holds {
                    cell.set([i + r, j + r]);
                }
            }
            cell.pairs_held()
        })
    })
}

/**
 * A node's far form at sites `s` apart, measured from its near form:
 * drawn whole two levels finer (spacing f), or at the coarsest level it
 * is drawn at (at most `detail`) if that is coarser, then coarsened.
 * A coarse site stands for the fine sites in its cell (those nearest
 * it). A ray crossing the cell along an axis meets the voxels of two
 * neighbouring columns (`pairs`): it is stopped as often as the pairs that
 * hold a fine site of the cell hold one of the thing, on average over the
 * axes: its share, with which it is drawn.
 * Each of its faces is of a kind of what rays through it see: along the
 * face's normal, a line of nearest neighbours, each such column's first
 * fine site from the face's end is credited to that face of every cell
 * the column crosses, in proportion (a ray that gets to the cell sees
 * that). The site is of a kind of all its faces see, for whatever does
 * not look at faces.
 */
fn measure(node: &Node, key: &str, layer: &'static str, bounds: Box3, detail: Fixed, s: i64, seed: u64) -> Sites {
    // Two levels finer, its cells hold enough fine sites to measure, and a tile's things cost less a level coarser.
    let f = (1i64 << ((detail / ONE).max(1).ilog2())).max(s / 4);
    let own = alone(node, key, layer, bounds, f, seed, near);
    cost::add(layer, Work { measured: own.cells.len() as u64, ..Work::default() });
    // Kinds counted, each with how it meets the world.
    type Counts = Vec<(Kind, Op, u32)>;
    // By coarse site: per axis, how many pairs of neighbouring columns hold a fine site of it; per face, what rays
    // through it see first of all, counted by kind.
    #[derive(Default)]
    struct Stats {
        pairs: [u32; 3],
        seen: [Counts; FACE_KINDS],
    }
    let count = |kinds: &mut Counts, kind: Kind, op: Op| match kinds.iter_mut().find(|e| e.0 == kind) {
        Some(e) => e.2 += 1,
        None => kinds.push((kind, op, 1)),
    };
    let n = own.n;
    // A fine point's coordinate along an axis in units of the coarse sites' spacing, then its cell.
    let scaled = |c: usize, v: i64| ((v + own.b.min[c]) * f * ONE).div_euclid(s);
    let cell_of = |p: [i64; 3]| nearest_unit_site([0, 1, 2].map(|c| scaled(c, p[c])));
    // The cells of the coarse sites from those of its box's corners and two more on each side, in an array:
    // nearest_site rounds each coordinate, then may move one by one, the corners' included.
    let lo = cell_of([0; 3]).map(|v| v - 2);
    let m = cell_of(n.map(|v| v - 1)).map(|v| v + 3);
    let m = [0, 1, 2].map(|i| m[i] - lo[i]);
    let index = |c: [i64; 3]| ((c[0] - lo[0]) + m[0] * ((c[1] - lo[1]) + m[1] * (c[2] - lo[2]))) as usize;
    let site_of = |i: usize| {
        let i = i as i64;
        [i % m[0] + lo[0], i / m[0] % m[1] + lo[1], i / (m[0] * m[1]) + lo[2]]
    };
    let mut cells: Vec<Stats> = (0..m[0] * m[1] * m[2]).map(|_| Stats::default()).collect();
    // Fine points by their index in `own` (x first): steps along each axis, and the cell of each fine site
    // (x + y + z even), found once for all the passes.
    let stride = [1, n[0], n[0] * n[1]];
    let parity = own.b.min.iter().sum::<i64>();
    let units: [Vec<Fixed>; 3] = std::array::from_fn(|c| (0..n[c]).map(|v| scaled(c, v)).collect());
    let mut cell_at = vec![u32::MAX; own.cells.len()];
    for z in 0..n[2] {
        for y in 0..n[1] {
            let row = (y * stride[1] + z * stride[2]) as usize;
            for x in ((y + z + parity) & 1..n[0]).step_by(2) {
                let v = [units[0][x as usize], units[1][y as usize], units[2][z as usize]];
                cell_at[row + x as usize] = index(nearest_unit_site(v)) as u32;
            }
        }
    }
    // Along each axis, the cells each column holds a fine site in (a column crosses each cell once: they are
    // convex), then for each pair of neighbouring columns, the cells either holds one in.
    for axis in 0..3 {
        let (a, b) = ((axis + 1) % 3, (axis + 2) % 3);
        let mut held: Vec<Vec<u32>> = vec![Vec::new(); (n[a] * n[b]) as usize];
        for i in 0..n[a] {
            for j in 0..n[b] {
                let column = &mut held[(i * n[b] + j) as usize];
                for k in 0..n[axis] {
                    let p = (i * stride[a] + j * stride[b] + k * stride[axis]) as usize;
                    if own.of(p).is_some() && column.last() != Some(&cell_at[p]) {
                        column.push(cell_at[p]);
                    }
                }
            }
        }
        let none = Vec::new();
        let at = |[i, j]: [i64; 2]| if (0..n[a]).contains(&i) && (0..n[b]).contains(&j) { &held[(i * n[b] + j) as usize] } else { &none };
        pairs([n[a], n[b]], |p, q| {
            let (p, q) = (at(p), at(q));
            for &c in p.iter().chain(q.iter().filter(|c| !p.contains(c))) {
                cells[c as usize].pairs[axis] += 1;
            }
        });
    }
    // Each pair counted against those holding one of the cell's fine sites at all.
    let all = pairs_of_cell(s / f);
    let picks = hash_text(&format!("{key}#measured"), seed);
    let shares: Vec<f64> = cells.iter().map(|stats| (0..3).map(|i| stats.pairs[i] as f64 / all[i] as f64).sum::<f64>() / 3.0).collect();
    // Columns along each line of nearest neighbours d, from its start on the side of −d to its end on the side of
    // d: rays coming from either end see the faces towards it.
    for (from_end, d) in VOXEL_FACES.iter().enumerate().filter(|(_, d)| d.iter().find(|v| **v != 0) == Some(&-1)) {
        let d = d.map(i64::from);
        let from_start = VOXEL_FACES.iter().position(|e| e.map(i64::from) == d.map(|v| -v)).expect("−d is a face");
        let step: i64 = (0..3).map(|c| d[c] * stride[c]).sum();
        // Columns of sites start where a step back leaves the box: on its side of −d along an axis d moves on.
        let edge = |c: usize, v: i64| d[c] != 0 && v == if d[c] < 0 { n[c] - 1 } else { 0 };
        // In order of x, y, z, as counts keep the first way a kind meets the world they are given.
        let starts = (0..n[0]).flat_map(|x| (0..n[1]).map(move |y| [x, y]));
        for [x, y] in starts {
            let zs = if edge(0, x) || edge(1, y) {
                0..n[2]
            } else if d[2] != 0 && n[2] > 0 {
                let z = if d[2] < 0 { n[2] - 1 } else { 0 };
                z..z + 1
            } else {
                0..0
            };
            for z in zs.filter(|z| (x + y + z + parity) & 1 == 0) {
                let start = [x, y, z];
                let origin = x + y * stride[1] + z * stride[2];
                let length =
                    (0..3).filter(|c| d[*c] != 0).map(|c| if d[c] > 0 { n[c] - start[c] } else { start[c] + 1 }).min().expect("d moves");
                // What it holds first and last, and the cells it crosses that the thing is in.
                let (mut first, mut last) = (None, None);
                let mut crossed: Vec<usize> = Vec::new();
                for p in (0..length).map(|t| (origin + t * step) as usize) {
                    let cell = cell_at[p] as usize;
                    if shares[cell] > 0.0 && crossed.last() != Some(&cell) {
                        crossed.push(cell);
                    }
                    if let Some(at) = own.of(p) {
                        first.get_or_insert(at);
                        last = Some(at);
                    }
                }
                let (Some(first), Some(last)) = (first, last) else { continue };
                // What rays along it see first from either end, credited to every cell it crosses: a ray that gets
                // to one, through those before it, sees that.
                for &i in &crossed {
                    count(&mut cells[i].seen[from_start], first.0, first.1);
                    count(&mut cells[i].seen[from_end], last.0, last.1);
                }
            }
        }
    }
    let mut sites = Vec::new();
    for (i, stats) in cells.into_iter().enumerate().filter(|(i, _)| shares[*i] > 0.0) {
        let site = site_of(i);
        let [x, y, z] = site;
        let faces = stats.seen;
        // One pick for all of its faces and itself: faces that see alike are of one kind.
        let u = hash(picks, x * s, y * s, z * s) as u64;
        let pick = |kinds: &Counts| -> (Kind, Op) {
            // In an order of their own: the columns' isn't one.
            let mut kinds = kinds.clone();
            kinds.sort_by_key(|e| e.0);
            let total: u32 = kinds.iter().map(|e| e.2).sum();
            let mut pick = ((u * total as u64) >> 32) as u32;
            let &(kind, op, _) = kinds
                .iter()
                .find(|e| {
                    pick < e.2 || {
                        pick -= e.2;
                        false
                    }
                })
                .expect("a kind");
            (kind, op)
        };
        let mut all: Counts = Vec::new();
        for (kind, op, n) in faces.iter().flatten() {
            match all.iter_mut().find(|e| e.0 == *kind) {
                Some(e) => e.2 += n,
                None => all.push((*kind, *op, *n)),
            }
        }
        // Its share, of each kind as much as its faces see of it, each kind's of what the ones before leave, so that
        // together they stop as much as the cell's share; each with the faces seen.
        let faces = faces.map(|counts| pick(&counts).0);
        let total = all.iter().map(|e| e.2).sum::<u32>().max(1) as f64;
        let mut before = 0.0f64;
        for &(kind, op, n) in &all {
            let part = shares[i] * n as f64 / total;
            let look = Look { kind, faces: faces.iter().any(|k| *k != kind).then_some(faces) };
            let picked = to_pick(hash(picks.wrapping_add(kind as u64 + 1), x * s, y * s, z * s));
            sites.push((site, look, op, share(part / (1.0 - before).max(f64::EPSILON)), picked));
            before += part;
        }
    }
    // In an order of their own, as sites are put in it.
    sites.sort_by_key(|e| e.0);
    sites
}

thread_local! {
    /** While `checking_skipped` runs: what drew where it said it may not. */
    static CHECKING: RefCell<Option<Vec<String>>> = const { RefCell::new(None) };
}

/**
 * Runs `f`, what it generates that says it may not draw in a box (a
 * field's lines about their ground, a population standing elsewhere)
 * drawn there all the same: returns where it drew.
 */
pub fn checking_skipped<R>(f: impl FnOnce() -> R) -> (R, Vec<String>) {
    let outer = CHECKING.replace(Some(Vec::new()));
    let value = f();
    (value, CHECKING.replace(outer).unwrap_or_default())
}

/** Whether `checking_skipped` runs. */
fn checking() -> bool {
    CHECKING.with_borrow(Option::is_some)
}

/** Keeps, while checking, that what `key` names drew at `site`, sites `s` apart, in `clip`, which it said it may not. */
fn broke(key: &str, site: [i64; 3], s: i64, clip: &Box3) {
    let message =
        format!("{key} draws at site {site:?}, sites {s} apart, in {:?} (lattice units), where it says it may not", units_of(clip, s));
    CHECKING.with_borrow_mut(|broken| broken.as_mut().expect("checking").push(message));
}

/** The lattice units of the sites `s` apart of `clip`. */
fn units_of(clip: &Box3, s: i64) -> Box3 {
    Box3 { min: clip.min.map(|v| v * s), max: clip.max.map(|v| (v - 1) * s + 1) }
}

/**
 * Whether a population's things may draw in `b` (lattice units): anywhere,
 * unless they stand on a plan of heights (Population::on): each within N
 * and its reach of its cell's centre, its anchor in the cell and at most
 * N off the plan, so within 2N and its reach of where it stands.
 */
fn stands_within(population: &Population, b: &Box3) -> bool {
    let Some(ground) = &population.on else { return true };
    let (n, [rx, ry, rz]) = (population.cell, population.reach);
    let (around, along) = (2 * n + rx.max(rz), 3 * n + ry);
    let Some((lo, hi)) = ground.range([b.min[0] - around, b.min[2] - around], [b.max[0] + around, b.max[2] + around]) else { return true };
    b.min[1] <= hi + along && b.max[1] > lo - along
}

/** Lines of a field's spans a side that are asked together whether they may draw (About): a box's own would reach from the ground into the air. */
const LINES: i64 = 32;

/** Draws a field into `clip`, a medium keeping a site with the chance a ray crossing it is stopped; returns the work. */
fn draw_field(field: &Field, seed: u64, s: i64, clip: &Box3, put: &mut Put) -> Work {
    let mut work = Work::default();
    let op = field.op;
    let field_seed = hash_text(field.name, seed);
    // A medium's share of a site, from its extinction.
    let share_of = |extinction: Option<f64>| extinction.map_or(SHARE, |e| share(1.0 - js::exp(-e * CHORD * s as f64)));
    let picked = |x: i64, y: i64, z: i64| to_pick(hash(field_seed, x * s, y * s, z * s));
    // Where column (i, j) of sites reads dithered plans, in the s × s columns it stands for (Plan::dithered): the same
    // for every field, so a column's fields read one point. Along a row of columns the offsets across it are a Weyl
    // sequence (the golden ratio's) from a phase hashed per row, so any run of them is spread evenly: a street along
    // the row is read by as many columns as it covers, give or take one, not by chance. None at level 0.
    let bits = s.trailing_zeros();
    let offset = |i: i64, j: i64| -> [i64; 2] {
        if bits == 0 {
            return [0, 0];
        }
        let weyl = |n: i64, row: i64, axis: i64| {
            let phase = hash64(seed, 0x0f00_7041, row, axis << 8 | bits as i64);
            ((n as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15).wrapping_add(phase) >> (64 - bits)) as i64 - s / 2
        };
        [weyl(j, i, 0), weyl(i, j, 2)]
    };
    match &field.values {
        FieldValues::At(at) => {
            for z in clip.min[2]..clip.max[2] {
                for y in clip.min[1]..clip.max[1] {
                    let mut x = clip.min[0] + ((clip.min[0] + y + z) & 1);
                    work.reads += 0.max((clip.max[0] - x + 1) / 2) as u64;
                    while x < clip.max[0] {
                        if let Some((kind, extinction)) = plan::footprint(s, offset(x, z), || at(x * s, y * s, z * s, s)) {
                            put(kind.into(), op, share_of(extinction), picked(x, y, z), x, y, z);
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
            let tie = Tie::new(s);
            let mut site = [0i64; 3];
            assert!(axis == 1 || field.about.is_none(), "field {}: spans along y only are about a plan of heights", field.name);
            // Block by block of lines, those that may not meet the clip skipped; while checking, drawn to see they don't.
            let mut drew = None;
            let mut record = |_: Look, _: Op, share: u32, _: u32, x: i64, y: i64, z: i64| drew = drew.or((share > 0).then_some([x, y, z]));
            for (bu, bv) in (lo[a]..hi[a]).step_by(LINES as usize).flat_map(|u| (lo[b]..hi[b]).step_by(LINES as usize).map(move |v| (u, v)))
            {
                let (to_u, to_v) = ((bu + LINES).min(hi[a]), (bv + LINES).min(hi[b]));
                let skipped = field.about.as_ref().is_some_and(|about| {
                    let along = [lo[axis] * s, (hi[axis] - 1) * s];
                    !about.may_meet([bu * s, bv * s], [(to_u - 1) * s, (to_v - 1) * s], along, s)
                });
                if skipped && !checking() {
                    continue;
                }
                let put: &mut Put = if skipped { &mut record } else { &mut *put };
                for u in bu..to_u {
                    for v in bv..to_v {
                        site[a] = u;
                        site[b] = v;
                        // A line along x or z reads plans at one z or x: dithered across it only.
                        let mut moved = offset(site[0], site[2]);
                        if axis != 1 {
                            moved[axis / 2] = 0;
                        }
                        work.reads += 1;
                        let spans = plan::footprint(s, moved, || spans(u * s, v * s, s));
                        if let Some((about, slack)) = field.about.as_ref().and_then(|about| Some((about, about.slack(s)?))) {
                            let g = about.of.at(u * s, v * s);
                            for span in spans.iter().filter(|span| span.from < span.to) {
                                assert!(
                                    span.from >= g - about.below - slack && span.to <= g + about.above + slack,
                                    "field {}: span [{}, {}) at column ({}, {}) reaches past {} under and {} over its ground {g}, \
                                 give or take {slack} at sites {s} apart (Field::about); see \"Rules\" in {DOCS}",
                                    field.name,
                                    span.from,
                                    span.to,
                                    u * s,
                                    v * s,
                                    about.below,
                                    about.above
                                );
                            }
                        }
                        for span in spans {
                            // s is a power of two: shifting is dividing, rounded down.
                            let first = (-((-span.from) >> s.trailing_zeros())).max(lo[axis]);
                            let mut w = first + ((first + u + v) & 1);
                            let share = share_of(span.extinction);
                            while w * s <= span.to && w < hi[axis] {
                                site[axis] = w;
                                if tie.within(axis, site.map(|c| c * s), span.from, span.to, 1) {
                                    put(span.kind.into(), op, share, picked(site[0], site[1], site[2]), site[0], site[1], site[2]);
                                }
                                w += 2;
                            }
                        }
                    }
                }
            }
            if let Some(site) = drew {
                broke(field.name, site, s, clip);
            }
        }
    }
    work
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
    nearest_unit_site(p.map(|c| c.div_euclid(s)))
}

/** The site nearest to `v`, in units of the sites' spacing (nearest_site of a point divided by it). */
fn nearest_unit_site(v: At) -> [i64; 3] {
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

/**
 * Each thing `population` makes in the cells of N·FCC whose indices are
 * in `cells` ([first, last] along each axis), with its cell: those whose
 * anchors are in them, asked of each cell or once of each line of them;
 * returns the work of asking.
 */
fn each_thing(generator: &Generator, population: &Population, cells: [(i64, i64); 3], mut each: impl FnMut(&Cell, Point, Node)) -> Work {
    let mut work = Work::default();
    let n = population.cell;
    let cell = |c: [i64; 3]| Cell::fcc(population, generator.seed, c);
    let mut make = |cell: &Cell, anchor: Point| {
        if let Some(ground) = &population.on {
            let [x, y, z] = anchor;
            let g = ground.at(x.round() as i64, z.round() as i64);
            assert!(
                (y - g as f64).abs() <= n as f64,
                "{} is anchored at {anchor:?}, {:.0} units off the ground it stands on (Population::on), more than its cell's width {n}; see \"Rules\" in {DOCS}",
                cell.key(),
                y - g as f64
            );
        }
        if let Some(node) = (population.make)(anchor, cell) {
            each(cell, anchor, node);
        }
    };
    match &population.anchor {
        Anchors::Cells(anchor) => {
            let [(i0, i1), (j0, j1), (k0, k1)] = cells;
            for i in i0..=i1 {
                for j in j0..=j1 {
                    for k in (k0 + ((k0 + i + j) & 1)..=k1).step_by(2) {
                        let cell = cell([i, j, k]);
                        work.cells += 1;
                        if let Some(p) = anchor(&cell).filter(|p| nearest_site(at(*p), n) == [i, j, k]) {
                            work.owned += 1;
                            make(&cell, p);
                        }
                    }
                }
            }
        }
        Anchors::Lines { axis, anchor } => {
            let (axis, [a, b]) = (*axis, across(*axis));
            for u in cells[a].0..=cells[a].1 {
                for v in cells[b].0..=cells[b].1 {
                    let line = Line { axis, across: [u, v], centre: [u * n, v * n], n, population: population.name, seed: generator.seed };
                    work.cells += 1;
                    let Some(p) = anchor(&line) else { continue };
                    // Owned by the cell of the line it is in, if the box reaches it.
                    let owner = nearest_site(at(p), n);
                    if owner[a] == u && owner[b] == v && (cells[axis].0..=cells[axis].1).contains(&owner[axis]) {
                        work.owned += 1;
                        make(&cell(owner), p);
                    }
                }
            }
        }
        Anchors::Points { points, anchor } => {
            // Cells are leaves along the axes the points span; along the others, those their anchors are nearest.
            let spans = points.spans();
            work.cells += (0..3).filter(|a| spans[*a]).map(|a| (cells[a].1 - cells[a].0 + 1) as u64).product::<u64>();
            for spot in points.points_in(cells.map(|c| c.0 * n), cells.map(|c| (c.1 + 1) * n)) {
                let Some(p) = anchor(&spot) else { continue };
                let along = |a: usize| (p[a].round() as i64 + n / 2).div_euclid(n);
                if (0..3).any(|a| !spans[a] && !(cells[a].0..=cells[a].1).contains(&along(a))) {
                    continue;
                }
                let [i, j, k] = [0, 1, 2].map(|a| if spans[a] { spot.leaf[a].div_euclid(n) } else { 0 });
                let centre = [0, 1, 2].map(|a| if spans[a] { spot.leaf[a] + n / 2 } else { p[a].round() as i64 });
                let (mark, nth) = (spot.mark, spot.nth);
                work.owned += 1;
                make(&Cell { i, j, k, centre, n: n / 2, mark, nth, population: population.name, seed: generator.seed }, p);
            }
        }
    }
    work
}

/** The box a thing of `population` in `cell` may reach: N and the population's reach from the cell's centre. */
fn reach_of(population: &Population, cell: &Cell) -> Box3 {
    let (c, n, r) = (cell.centre, cell.n, population.reach);
    Box3 { min: [0, 1, 2].map(|i| c[i] - n - r[i]), max: [0, 1, 2].map(|i| c[i] + n + r[i]) }
}

/**
 * The things `population` makes in the cells of N·FCC whose indices are
 * in `cells` ([first, last] along each axis), as `generate` makes them:
 * each one's key, its bounds and the box its population's reach allows
 * it. For checks that look at more things than a box draws
 * (worldgen::contract).
 */
pub fn things_in(generator: &Generator, population: &Population, cells: [(i64, i64); 3]) -> Vec<(String, Box3, Box3)> {
    let mut things = Vec::new();
    each_made(generator, population, cells, |key, bounds, allowed, _| things.push((key, bounds, allowed)));
    things
}

/** As `things_in`, each with the thing itself, to look into (worldgen::survey); returns how many cells were asked. */
pub fn each_made(
    generator: &Generator,
    population: &Population,
    cells: [(i64, i64); 3],
    mut each: impl FnMut(String, Box3, Box3, &Node),
) -> u64 {
    each_thing(generator, population, cells, |cell, _, node| each(cell.key(), bounds_of(&node), reach_of(population, cell), &node)).cells
}

/**
 * The anchor of the thing `population` makes in cell (i, j, k) of N·FCC
 * (i + j + k even), and the thing, as `generate` makes them; none if it
 * makes none there.
 */
pub fn made_in(generator: &Generator, population: &Population, i: i64, j: i64, k: i64) -> Option<(Point, Node)> {
    let (anchor, owner) = anchor_in(generator, population, i, j, k)?;
    let cell = Cell::fcc(population, generator.seed, [i, j, k]);
    (owner == [i, j, k]).then(|| (population.make)(anchor, &cell)).flatten().map(|node| (anchor, node))
}

/**
 * The anchors of the things `population` makes in the cells whose
 * indices are in `cells` ([first, last] along each axis), as `generate`
 * makes them: anchored at points, those of the points of these leaves.
 */
pub fn anchors_in(generator: &Generator, population: &Population, cells: [(i64, i64); 3]) -> Vec<Point> {
    let mut anchors = Vec::new();
    each_thing(generator, population, cells, |_, anchor, _| anchors.push(anchor));
    anchors
}

/**
 * The anchor asked for cell (i, j, k) of `population`, or for its line,
 * if any, and the cell it is in, which owns a thing there if it is this
 * one; none anchored at points.
 */
pub fn anchor_in(generator: &Generator, population: &Population, i: i64, j: i64, k: i64) -> Option<(Point, [i64; 3])> {
    let n = population.cell;
    let anchor = match &population.anchor {
        Anchors::Cells(anchor) => anchor(&Cell::fcc(population, generator.seed, [i, j, k])),
        Anchors::Lines { axis, anchor } => {
            let ([a, b], c) = (across(*axis), [i, j, k]);
            anchor(&Line {
                axis: *axis,
                across: [c[a], c[b]],
                centre: [c[a] * n, c[b] * n],
                n,
                population: population.name,
                seed: generator.seed,
            })
        }
        // A leaf holds any number: see anchors_in.
        Anchors::Points { .. } => None,
    }?;
    Some((anchor, nearest_site(at(anchor), n)))
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
    use crate::noise::fx;

    #[test]
    fn a_long_lathe_reaches_its_widest_circle() {
        // 1000 units long, at 3 : 4 : 0, so its widest circle reaches 4, 3 and 5 units along x, y and z.
        let shape = lathe([0.0; 3], [600.0, 800.0, 0.0], |_| 5.0, 1, Fill::Const(1.0), Op::Fill);
        assert_eq!((shape.bounds.min, shape.bounds.max), ([-4, -3, -5], [605, 804, 6]));
    }

    /** `f` of 0 to `n`, in order, on every core: many seeds take long to measure one by one. */
    fn on_threads<T: Send>(n: usize, f: impl Fn(usize) -> T + Sync) -> Vec<T> {
        let per_thread = n.div_ceil(std::thread::available_parallelism().map_or(1, |t| t.get())).max(1);
        let f = &f;
        std::thread::scope(|scope| {
            let parts: Vec<_> = (0..n)
                .step_by(per_thread)
                .map(|from| scope.spawn(move || (from..n.min(from + per_thread)).map(f).collect::<Vec<_>>()))
                .collect();
            parts.into_iter().flat_map(|part| part.join().expect("measured")).collect()
        })
    }

    #[test]
    fn a_measured_far_form_keeps_what_rays_see() {
        // A solid block 64 units wide, painted with a sheet of glass on top two fine sites deep, and a sparse cloud:
        // measured at sites 16 apart from sites 4 apart.
        // Made on each thread: a node is not Sync.
        let block = || Node {
            far: Some(Far::measured(fx(2.0))),
            ..Node::of(vec![
                cuboid([0.0; 3], [64.0; 3], 1, SOLID, Op::Fill),
                cuboid([0.0, 56.0, 0.0], [64.0, 64.0, 64.0], 2, SOLID, Op::Paint),
            ])
        };
        let one = block();
        let sites = measure(&one, "block", "blocks", bounds_of(&one), fixed(2.0), 16, 1);
        assert!(
            (1..4).all(|y| (1..4).all(|x| (1..4).all(|z| (x + y + z) % 2 == 1 || sites.iter().any(|s| s.0 == [x, y, z])))),
            "its inside is all drawn"
        );
        // Rays straight down see the glass on its top row of cells, half covered, through the faces towards them;
        // where that is dropped, the cells under it, whose faces up the columns from above credit with the glass
        // too. Rays from the sides see the block through the faces towards them, but in the top row, where the
        // glass shows at their tops. Each block picks its cells' kinds anew.
        let seen = |sites: &Sites, ray: [i64; 3]| {
            let looks: std::collections::HashMap<[i64; 3], Look> = sites.iter().map(|s| (s.0, s.1)).collect();
            let face = |look: &Look, k: usize| look.faces.map_or(look.kind, |faces| faces[k]);
            // The faces a ray along `ray` may enter by: those facing back along it.
            let faces: Vec<usize> =
                (0..FACE_KINDS).filter(|&k| (0..3).map(|c| VOXEL_FACES[k][c] as i64 * ray[c]).sum::<i64>() < 0).collect();
            let (mut glass, mut hits) = (0, 0);
            for u in (2..64).step_by(4) {
                for v in (2..64).step_by(4) {
                    // From above, or from −x, at the middle height.
                    let along = |t: i64| if ray[1] < 0 { [u, 127 - t, v] } else { [t - 64, 32 + (u % 16) - 8, v] };
                    let Some(look) = (0..192).find_map(|t| looks.get(&nearest_site(along(t).map(|v| v * ONE), 16))) else { continue };
                    hits += faces.len();
                    glass += faces.iter().filter(|&&k| face(look, k) == 2).count();
                }
            }
            glass as f64 / hits as f64
        };
        let blocks: Vec<Sites> = on_threads(16, |i| {
            let block = block();
            measure(&block, &format!("block {i}"), "blocks", bounds_of(&block), fixed(2.0), 16, 1)
        });
        let from_above = blocks.iter().map(|sites| seen(sites, [0, -1, 0])).sum::<f64>() / 16.0;
        let from_the_side = blocks.iter().map(|sites| seen(sites, [1, 0, 0])).sum::<f64>() / 16.0;
        // Over these 16 blocks, 0.79 from above.
        assert!(from_above > 0.65, "rays from above see the glass on top {from_above} of the time");
        assert!(from_the_side < 0.1, "rays from the side see the glass {from_the_side} of the time");
        // A sparse cloud's shares of its about 134 cells (a ball's volume over a site's, 2 · 16³) add up to about a
        // quarter of them, each cloud's within a cell or two of that: it is not kept or dropped whole. Its fine
        // sites, each kept with a chance of 1 − 0.98⁴, stop 0.27 of the rays along an axis through a cell inside
        // it, traced through their voxels; counted by columns, not pairs, a sixth of them were kept.
        let cloud = || Node { far: Some(Far::measured(fx(2.0))), ..Node::of(vec![ball([0.0; 3], 64.0, 1, Fill::Const(0.02), Op::Fill)]) };
        let kept: Vec<f64> = on_threads(16, |i| {
            let cloud = cloud();
            let sites = measure(&cloud, &format!("cloud {i}"), "clouds", bounds_of(&cloud), fixed(2.0), 16, 1);
            sites.iter().map(|s| s.3 as f64 / SHARE as f64).sum()
        });
        assert!(kept.iter().all(|n| (26.0..38.0).contains(n)), "sparse clouds keep {kept:?} of their about 134 cells");
    }

    #[test]
    fn a_cells_pairs_of_columns_are_counted() {
        // Every fine site near a few cells of a level, assigned to its nearest: each cell's columns holding one,
        // per axis, and the pairs of neighbours either of which does, as pairs_of_cell counts them for any cell.
        for r in [2i64, 4, 8, 16] {
            for centre in [[0, 0, 0], [1, 1, 0], [-1, 0, 1], [0, 2, 0], [3, -1, 2]] {
                let mut columns: [std::collections::HashSet<(i64, i64)>; 3] = Default::default();
                let c = centre.map(|v| v * r);
                for x in c[0] - r..=c[0] + r {
                    for y in c[1] - r..=c[1] + r {
                        for z in c[2] - r..=c[2] + r {
                            if (x + y + z) & 1 == 0 && nearest_site([x, y, z].map(|v| v * ONE), r) == centre {
                                columns[0].insert((y, z));
                                columns[1].insert((z, x));
                                columns[2].insert((x, y));
                            }
                        }
                    }
                }
                let pairs = columns.map(|held| {
                    let mut pairs = std::collections::HashSet::new();
                    for &(i, j) in &held {
                        for (u, v) in [(i - 1, j), (i + 1, j), (i, j - 1), (i, j + 1)] {
                            pairs.insert(if (i, j) < (u, v) { ((i, j), (u, v)) } else { ((u, v), (i, j)) });
                        }
                    }
                    pairs.len() as u32
                });
                assert_eq!(pairs, pairs_of_cell(r), "cells {r} sites wide, at {centre:?}");
            }
            println!("{r}: {:?} halves of a column, of a shadow of {}", pairs_of_cell(r), 4 * r * r);
        }
    }

    /**
     * Levels of detail by construction: at sites s apart, a tube holds
     * what is within its radius of the curve, give or take s / 2, as the
     * curve is drawn through the points of a level it strays from by
     * no more.
     */
    #[test]
    fn a_tube_is_within_half_a_site_of_its_curve() {
        let curve = Curve::wandering(3, [8.0, 4.0, 2.0, 1.0].map(fx).to_vec());
        let ends = [[0.0, 0.0, 0.0], [96.0, 40.0, -24.0]];
        let line: Vec<Point> = curve.points(ends.map(|e| e.map(fx))).iter().map(|p| p.map(Fx::to_f64)).collect();
        let shape = tube(&curve, ends[0], ends[1], 3.0, 1, SOLID, Op::Fill);
        let distance = |p: Point| {
            let to = |a: Point, b: Point| {
                let (ab, ap) = ([0, 1, 2].map(|k| b[k] - a[k]), [0, 1, 2].map(|k| p[k] - a[k]));
                let t = ((0..3).map(|k| ab[k] * ap[k]).sum::<f64>() / (0..3).map(|k| ab[k] * ab[k]).sum::<f64>()).clamp(0.0, 1.0);
                js::hypot(&[0, 1, 2].map(|k| ap[k] - t * ab[k]))
            };
            line.windows(2).map(|s| to(s[0], s[1])).fold(f64::INFINITY, f64::min)
        };
        let b = shape.bounds;
        for s in [1, 2, 4, 8, 16] {
            let inside = shape.inside(0, s);
            // A hundredth of a unit for rounding.
            let slack = s as f64 / 2.0 + 0.01;
            for x in b.min[0].div_euclid(s)..=b.max[0].div_euclid(s) {
                for y in b.min[1].div_euclid(s)..=b.max[1].div_euclid(s) {
                    for z in b.min[2].div_euclid(s)..=b.max[2].div_euclid(s) {
                        let d = distance([x, y, z].map(|v| (v * s) as f64));
                        let holds = inside.holds([x, y, z].map(|v| v * s * ONE));
                        assert!(holds || d > 3.0 - slack, "at sites {s} apart, ({x}, {y}, {z}) is {d:.3} from the curve, not held");
                        assert!(!holds || d <= 3.0 + slack, "at sites {s} apart, ({x}, {y}, {z}) is {d:.3} from the curve, held");
                    }
                }
            }
        }
    }

    #[test]
    fn a_gable_narrows_to_its_ridge() {
        // 8 wide across x, 4 high: at height h, the sites within 4 · (1 − h / 4) of x = 4.
        let shape = gable([0.0; 3], [8.0, 4.0, 6.0], 2, 1, SOLID, Op::Fill);
        let inside = shape.inside(0, 1);
        let holds = |x: i64, y: i64| inside.holds([x, y, 2].map(|v| v * ONE));
        assert!(holds(0, 0) && holds(7, 0) && holds(4, 3) && holds(3, 2) && holds(6, 1));
        assert!(!holds(1, 3) && !holds(8, 0) && !holds(4, 4) && !holds(7, 2));
    }

    /** A generator of `layers`, of one kind. */
    fn layers(layers: Vec<Layer>) -> Generator {
        Generator {
            kinds: vec![KindDef { name: "stone", kind_type: 0, colors: &["#808080"], walk_through: false, glows: false, medium: false }],
            seed: 1,
            layers,
        }
    }

    /** Ground `height(x)` high, said to rise at most `slope` a column: eight units of it about it. */
    fn ground(height: fn(i64) -> i64, slope: i64) -> Generator {
        let of = Rc::new(Sloped::new(Rc::new(plan::Plan::new("ground", 1, move |x, _| height(x))), slope));
        let field = Field::columns("ground", 0.0, Op::Fill, move |x, _, _| vec![Span::solid(height(x) - 8, height(x) + 1, 0)]);
        layers(vec![Layer::Field(field.about(of, 8, 1))])
    }

    #[test]
    fn a_field_about_its_ground_is_drawn_only_where_its_lines_may_be() {
        let flat = ground(|_| 0, 0);
        let (across, above) = (Box3 { min: [0, -16, 0], max: [64, 16, 8] }, Box3 { min: [0, 16, 0], max: [64, 48, 8] });
        let drawn = |b: &Box3| checking_skipped(|| generate(&flat, b, 0).pieces.alpha.iter().any(|&a| a > 0));
        assert_eq!(drawn(&across), (true, Vec::new()));
        assert_eq!(drawn(&above), (false, Vec::new()), "skipped above it, and nothing would be drawn there");
    }

    #[test]
    #[should_panic(expected = "field ground: span [-8, 1) at column (0, 0) reaches past 2 under and 1 over its ground 0")]
    fn refuses_a_field_reaching_past_what_it_says() {
        let flat = Rc::new(Sloped::new(Rc::new(plan::Plan::new("flat", 1, |_, _| 0)), 0));
        let field = Field::columns("ground", 0.0, Op::Fill, |_, _, _| vec![Span::solid(-8, 1, 0)]).about(flat, 2, 1);
        generate(&layers(vec![Layer::Field(field)]), &Box3 { min: [0, -16, 0], max: [8, 16, 8] }, 0);
    }

    #[test]
    fn checking_finds_what_a_broken_promise_skips() {
        // A spike at x = 12 the slope's rule never sees (no neighbour of it is read): sampled every 8, it is skipped.
        let spiked = ground(|x| if x == 12 { 200 } else { 0 }, 0);
        let b = Box3 { min: [0, 45, 0], max: [32, 55, 4] };
        assert!(generate(&spiked, &b, 2).pieces.alpha.iter().all(|&a| a == 0), "skipped");
        let (_, broken) = checking_skipped(|| generate(&spiked, &b, 2));
        assert!(broken.len() == 1 && broken[0].starts_with("ground draws at site [3, 49, 0]"), "{broken:?}");
    }

    #[test]
    #[should_panic(expected = "rocks 0,0,0 is anchored at [0.0, 0.0, 0.0], 100 units off the ground it stands on (Population::on)")]
    fn refuses_a_thing_anchored_off_the_ground_it_stands_on() {
        // In its cell, but the ground is 100 units under it.
        let low = Rc::new(Sloped::new(Rc::new(plan::Plan::new("low", 1, |_, _| -100)), 0));
        let rocks = Population::new("rocks", 1.0, 16)
            .on(low)
            .anchor(|cell| (cell.centre == [0; 3]).then_some([0.0; 3]))
            .make(|p, _| Some(Node::of(vec![ball(p, 2.0, 0, SOLID, Op::Fill)])));
        generate(&layers(vec![Layer::Population(rocks)]), &Box3 { min: [-8, -100, -8], max: [8, 8, 8] }, 0);
    }

    #[test]
    fn faces_on_a_plane_of_sites_stay_where_level_0_has_them() {
        // Ground up to y = 0 and boxes from y = 0 up, on a plane of sites at every level, each drawn alone.
        let ground = Layer::Field(Field {
            name: "ground",
            order: 0.0,
            op: Op::Fill,
            values: FieldValues::Spans { axis: 1, spans: Box::new(|_, _, _| vec![Span { from: -4096, to: 0, kind: 0, extinction: None }]) },
            about: None,
        });
        let n = 1 << 14;
        let block = |top: f64| {
            Layer::Population(Population {
                name: "block",
                order: 1.0,
                cell: n,
                reach: [0; 3],
                size: ANY_SIZE,
                anchor: Anchors::Cells(Box::new(|cell| (cell.centre == [0; 3]).then_some([0.0; 3]))),
                make: Box::new(move |_, _| {
                    Some(Node::of(vec![cuboid([-8192.0, 0.0, -8192.0], [8192.0, top, 8192.0], 0, SOLID, Op::Fill)]))
                }),
                far: None,
                on: None,
            })
        };
        let kinds = vec![KindDef { name: "stone", kind_type: 1, colors: &["#808080"], walk_through: false, glows: false, medium: false }];
        let alone = |layer: Layer| Generator { kinds: kinds.clone(), seed: 1, layers: vec![layer] };
        let (ground, solid) = (alone(ground), alone(block(4096.0)));
        for level in 0..6 {
            let s = 1i64 << level;
            let b = Box3 { min: [0, -32 / s - 2, 0], max: [128, 32 / s + 2, 128] };
            let (below, above) = (generate(&ground, &b, level), generate(&solid, &b, level));
            let (mut tops, mut lines) = (0, 0);
            for x in 0..128 {
                for z in 0..128 {
                    let sites = (b.min[1]..b.max[1]).filter(|y| (x + y + z) & 1 == 0);
                    let (mut top, mut bottom) = (i64::MIN, i64::MAX);
                    for y in sites {
                        let (g, h) = (below.get(x, y, z) != 0, above.get(x, y, z) != 0);
                        assert!(g != h, "level {level}: ({x}, {y}, {z}) is in {} of the two", g as u8 + h as u8);
                        if g { top = top.max(y) } else { bottom = bottom.min(y) }
                    }
                    assert_eq!(top + 2, bottom, "level {level}: the line at ({x}, {z}) is split once");
                    // A site's voxel reaches a site's spacing along the line.
                    tops += (top + 1) * s;
                    lines += 1;
                }
            }
            // Level 0's ground is half a unit low; a site on the plane counts as in it with chance ½ − 1/(2s), so
            // the mean is within a few s / √lines of that, not (s − 1) / 2 lower.
            let mean = tops as f64 / lines as f64;
            assert!((mean + 0.5).abs() <= s as f64 / 16.0, "level {level}: the ground's top is at {mean} on average");
            // A slab one site thick keeps the whole plane: half of it and half the next would let rays through.
            let slab = generate(&alone(block(s as f64)), &b, level);
            assert!((0..128).all(|x| (0..128).all(|z| (x + z) & 1 != 0 || slab.get(x, 0, z) != 0)), "level {level}: the slab has holes");
        }
    }
}
