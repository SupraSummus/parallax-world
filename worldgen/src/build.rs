//! Short forms for writing worlds: constructors of fields, populations,
//! nodes and solid shapes, so a world says what it draws without struct
//! literals and boxes. The long forms stay:
//! they are the model (generator.rs). See "Writing a world" in
//! docs/generators.md.

use crate::curve::Curve;
use crate::generator::{
    ANY_SIZE, Anchors, Cell, Child, Children, Far, Field, FieldValues, Kind, Line, Node, Op, Point, Population, Radius, SOLID, Shape,
    Shapes, Span, ball, bounds_of_shapes, capsule, cuboid, ellipsoid, gable, lathe, tube,
};
use crate::grid::Box3;
use std::rc::Rc;

use crate::noise::Rng;
use crate::plan::Sloped;
use crate::points::{Points, Spot};

impl Field {
    /** A kind, with a medium's extinction if it is one, at each site: `at(x, y, z, spacing)`, lattice units. */
    pub fn at(name: &'static str, order: f64, op: Op, at: impl Fn(i64, i64, i64, i64) -> Option<(Kind, Option<f64>)> + 'static) -> Field {
        Field { name, order, op, values: FieldValues::At(Box::new(at)), within: None }
    }

    /** Spans along each line of sites parallel to `axis`: `spans(u, v, spacing)`, the line's other coordinates in axis order. */
    pub fn spans<S: Into<Vec<Span>>>(
        name: &'static str,
        order: f64,
        op: Op,
        axis: usize,
        spans: impl Fn(i64, i64, i64) -> S + 'static,
    ) -> Field {
        Field { name, order, op, values: FieldValues::Spans { axis, spans: Box::new(move |u, v, s| spans(u, v, s).into()) }, within: None }
    }

    /** Spans along each column, up y: `spans(x, z, spacing)`, lattice units; a `Column` or the spans. */
    pub fn columns<S: Into<Vec<Span>>>(name: &'static str, order: f64, op: Op, spans: impl Fn(i64, i64, i64) -> S + 'static) -> Field {
        Field::spans(name, order, op, 1, spans)
    }

    /** Drawn only in the boxes `within(box, spacing)` says it may draw in (Within), lattice units. */
    pub fn within(self, within: impl Fn(&Box3, i64) -> bool + 'static) -> Field {
        Field { within: Some(Box::new(within)), ..self }
    }
}

/** One number for every axis, or one per axis. */
pub trait PerAxis {
    fn per_axis(self) -> [i64; 3];
}

impl PerAxis for i64 {
    fn per_axis(self) -> [i64; 3] {
        [self; 3]
    }
}

impl PerAxis for [i64; 3] {
    fn per_axis(self) -> [i64; 3] {
        self
    }
}

impl Population {
    /**
     * Things in the cells of `cell`·FCC: none until it is given `make`.
     * Each is anchored at its cell's centre unless given an anchor,
     * reaches nowhere past its cell unless given `reach`, and is made at
     * every level unless given a `size`.
     */
    pub fn new(name: &'static str, order: f64, cell: i64) -> Population {
        Population {
            name,
            order,
            cell,
            reach: [0; 3],
            size: ANY_SIZE,
            anchor: Anchors::Cells(Box::new(|c| Some(c.centre.map(|v| v as f64)))),
            make: Box::new(|_, _| None),
            far: None,
            within: None,
        }
    }

    /** How far a thing reaches past N from its cell's centre: one number, or per axis. */
    pub fn reach(self, reach: impl PerAxis) -> Population {
        Population { reach: reach.per_axis(), ..self }
    }

    /** Things are made while sites are at most `size` apart; beyond, the far field is drawn, if any. */
    pub fn size(self, size: i64) -> Population {
        Population { size, ..self }
    }

    /** Where a cell's thing is: the cell owns one only if its anchor is in it. */
    pub fn anchor(self, anchor: impl Fn(&Cell) -> Option<Point> + 'static) -> Population {
        Population { anchor: Anchors::Cells(Box::new(anchor)), ..self }
    }

    /**
     * Where the thing of each line of cells along `axis` is, asked once
     * however many of them a box reaches: the cell of the line it is in
     * owns it (`Line::pick` picks a point on a surface across the axis).
     */
    pub fn anchor_along(self, axis: usize, anchor: impl Fn(&Line) -> Option<Point> + 'static) -> Population {
        Population { anchor: Anchors::Lines { axis, anchor: Box::new(anchor) }, ..self }
    }

    /**
     * A thing at each of `points`, anchored where `anchor` lifts it to
     * along the axes they don't span (onto a ground); its cells are their
     * leaves, and its cell's `mark` the point's.
     */
    pub fn anchor_points(self, points: std::rc::Rc<Points>, anchor: impl Fn(&Spot) -> Option<Point> + 'static) -> Population {
        Population { cell: points.leaf, anchor: Anchors::Points { points, anchor: Box::new(anchor) }, ..self }
    }

    /** The thing at an anchor, of a cell, if it has one. */
    pub fn make(self, make: impl Fn(Point, &Cell) -> Option<Node> + 'static) -> Population {
        Population { make: Box::new(make), ..self }
    }

    /** What it draws while its sites are further apart than its `size`: a medium as dense as its things. */
    /**
     * Its things standing on `ground`: each anchored on it, in the cell
     * that owns it, and so within its cell's width (N) and N and its reach
     * of where it stands; drawn only near the ground there (Within).
     * Given its cell and reach first.
     */
    pub fn on(self, ground: Rc<Sloped>) -> Population {
        let ([x, y, z], n) = (self.reach, self.cell);
        let (around, along) = (2 * n + x.max(z), 2 * n + y);
        self.within(move |b, spacing| ground.about(b, spacing, around, along, along))
    }

    /** Its things drawn only in the boxes `within(box, spacing)` says they may draw in (Within), lattice units. */
    pub fn within(self, within: impl Fn(&Box3, i64) -> bool + 'static) -> Population {
        Population { within: Some(Box::new(within)), ..self }
    }

    pub fn far(self, far: Field) -> Population {
        Population { far: Some(far), ..self }
    }
}

impl Child {
    /** A finer thing of a node, drawn while sites are at most `size` apart (ANY_SIZE for always). */
    pub fn new(name: impl Into<String>, size: i64, node: Node) -> Child {
        Child { name: name.into(), size, node }
    }
}

impl Node {
    /** A thing of nothing but its children, its bounds theirs. */
    pub fn group(children: Vec<Child>) -> Node {
        Node::of(Vec::new()).with_children(children)
    }

    /** Shapes made only once read, by `make` from a random sequence of `seed`, within `bounds`. */
    pub fn lazy(bounds: Box3, seed: u64, make: impl Fn(&mut Rng) -> Vec<Shape> + 'static) -> Node {
        Node { shapes: Shapes::lazy(seed, make), bounds: Some(bounds), children: None, far: None }
    }

    /** With `children`, and bounds holding them and what it had. */
    pub fn with_children(self, children: Vec<Child>) -> Node {
        assert!(self.children.is_none(), "a node's children are given once");
        let boxes: Vec<Box3> = std::iter::once(self.bounds()).chain(children.iter().map(|c| c.node.bounds())).collect();
        Node { bounds: Some(bounds_of_shapes(&boxes)), children: Some(Children::Fixed(children)), ..self }
    }

    /** Drawn as `far` while sites are further apart than its detail. */
    pub fn with_far(self, far: impl Into<Option<Far>>) -> Node {
        Node { far: far.into(), ..self }
    }
}

/**
 * Solid shapes that fill empty sites: the common case, without `SOLID,
 * Op::Fill`. `paint` and `carve` change how one meets the world; a shape
 * kept with a chance (`Fill`) is made with the long form (`ball` and so
 * on), since how many rays it stops depends on it from the start.
 */
impl Shape {
    pub fn ball(centre: Point, radius: f64, kind: impl Into<Kind>) -> Shape {
        ball(centre, radius, kind.into(), SOLID, Op::Fill)
    }

    pub fn ellipsoid(centre: Point, radii: Point, kind: impl Into<Kind>) -> Shape {
        ellipsoid(centre, radii, kind.into(), SOLID, Op::Fill)
    }

    pub fn capsule(p: Point, q: Point, radius: impl Into<Radius>, kind: impl Into<Kind>) -> Shape {
        capsule(p, q, radius, kind.into(), SOLID, Op::Fill)
    }

    pub fn tube(curve: &Curve<3>, p: Point, q: Point, radius: impl Into<Radius>, kind: impl Into<Kind>) -> Shape {
        tube(curve, p, q, radius, kind.into(), SOLID, Op::Fill)
    }

    pub fn lathe(p: Point, q: Point, radius: impl Into<Radius>, kind: impl Into<Kind>) -> Shape {
        lathe(p, q, radius, kind.into(), SOLID, Op::Fill)
    }

    pub fn cuboid(min: Point, max: Point, kind: impl Into<Kind>) -> Shape {
        cuboid(min, max, kind.into(), SOLID, Op::Fill)
    }

    pub fn gable(min: Point, max: Point, ridge: usize, kind: impl Into<Kind>) -> Shape {
        gable(min, max, ridge, kind.into(), SOLID, Op::Fill)
    }

    /** Over whatever is drawn before it. */
    pub fn paint(mut self) -> Shape {
        self.op = Op::Paint;
        self
    }

    /** Cutting its sites out of its thing (never out of the world: see "Model"). */
    pub fn carve(mut self) -> Shape {
        self.op = Op::Carve;
        self
    }
}
