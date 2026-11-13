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
use crate::grid::{ALPHA, BRICK, Box3, VoxelGrid};
use crate::js;
use crate::noise::{Fixed, Fx, ONE, Rng, fixed, float, hash, hash_text, hash64};
use crate::plan::{self, Sloped};
use crate::points::{Points, Spot};

mod drawables;
mod forms;
mod generate;
mod shapes;
#[cfg(test)]
mod tests;
mod things;

pub use drawables::*;
pub use forms::*;
pub use generate::*;
pub use shapes::*;
pub use things::*;

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
 * (`slack`), and under that `under` if it says, down without end (a span
 * from DEEP). A rule, checked wherever it draws; so it is asked to draw
 * only where that may be (Sloped::range), and under it fills `under`.
 */
pub struct About {
    pub of: Rc<Sloped>,
    pub below: i64,
    pub above: i64,
    /** A solid kind under it all: the ground, solid to the bottom. */
    pub under: Option<Kind>,
}

/** Where a span reaches down without end from: under everything (About's `under`). */
pub const DEEP: i64 = i64::MIN / 4;

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

    /**
     * What lines in [min, max] (x, z, lattice units, inclusive) draw in
     * heights [y0, y1], at sites `spacing` apart: whatever they draw, all
     * `under` (a kind), or nothing.
     */
    fn meets(&self, min: [i64; 2], max: [i64; 2], [y0, y1]: [i64; 2], spacing: i64) -> Meets {
        let (Some((lo, hi)), Some(slack)) = (self.of.range(min, max), self.slack(spacing)) else { return Meets::Any };
        match self.under {
            _ if y0 > hi + self.above + slack => Meets::Nothing,
            Some(kind) if y1 < lo - self.below - slack => Meets::All(kind),
            None if y1 < lo - self.below - slack => Meets::Nothing,
            _ => Meets::Any,
        }
    }
}

/** What a field's lines draw in a box, by what they are `About`. */
#[derive(Clone, Copy, PartialEq, Eq)]
enum Meets {
    Any,
    All(Kind),
    Nothing,
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
    /** For messages (worldgen::checks::contract's margins). */
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
