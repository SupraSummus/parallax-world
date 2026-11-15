//! Short forms for writing worlds: constructors of fields, populations,
//! nodes and solid shapes, so a world says what it draws without struct
//! literals and boxes. The long forms stay:
//! they are the model (generator.rs). See "Writing a world" in
//! docs/generators.md.

use crate::curve::Curve;
use crate::generator::{
    Anchors, Cell, Child, Children, DEEP, DOCS, Far, Field, FieldValues, Holds, Kind, Line, Meets, Node, Op, Point, Population, Radius,
    SOLID, Shape, Shapes, Span, ball, bounds_of_shapes, capsule, cuboid, ellipsoid, gable, lathe, tube,
};
use crate::grid::Box3;
use std::rc::Rc;

use crate::noise::Rng;
use crate::plan::{self, Sloped};
use crate::points::{Points, Spot};

impl Field {
    /** A kind, with a medium's extinction if it is one, at each site: `at(x, y, z)`, lattice units. */
    pub fn at(name: &'static str, order: f64, op: Op, at: impl Fn(i64, i64, i64) -> Option<(Kind, Option<f64>)> + 'static) -> Field {
        Field { name, order, op, values: FieldValues::At(Box::new(at)), meets: None }
    }

    /** Spans along each line of sites parallel to `axis`: `spans(u, v)`, the line's other coordinates in axis order (FieldValues::Spans). */
    pub fn spans<S: Into<Vec<Span>>>(
        name: &'static str,
        order: f64,
        op: Op,
        axis: usize,
        spans: impl Fn(i64, i64) -> S + 'static,
    ) -> Field {
        let spans = Box::new(move |u, v| spans(u, v).into());
        Field { name, order, op, values: FieldValues::Spans { axis, spans }, meets: None }
    }

    /**
     * Shapes, each of `op`, cutting or filling the world, not a thing:
     * `near(box)`, those that may reach the box (lattice units), each the
     * same, in the same order, whichever box asks (FieldValues::Shapes).
     * What it meets follows from their bounds, asked till one reaches the box.
     */
    pub fn shapes<S: IntoIterator<Item = Shape>>(name: &'static str, order: f64, op: Op, near: impl Fn(&Box3) -> S + 'static) -> Field {
        let near = Rc::new(near);
        let reaching = near.clone();
        let values = FieldValues::Shapes(Box::new(move |b| near(b).into_iter().collect()));
        Field { name, order, op, values, meets: None }.meets(move |b, spacing| {
            let reaches = plan::footprint(spacing, [0, 0], || reaching(b).into_iter().any(|s| s.bounds.overlaps(b)));
            if reaches { Meets::Any } else { Meets::Nothing }
        })
    }

    /** Spans along each column, up y, all of them: `spans(x, z)`, lattice units; a `Column` or the spans. */
    pub fn columns<S: Into<Vec<Span>>>(name: &'static str, order: f64, op: Op, spans: impl Fn(i64, i64) -> S + 'static) -> Field {
        Field::spans(name, order, op, 1, spans)
    }

    /** Its spans along y from `below` under to `above` over `of` at their column (About): a rule; what it meets follows. */
    pub fn about(self, of: Rc<Sloped>, below: i64, above: i64) -> Field {
        self.about_as(About { of, below, above, under: None })
    }

    /**
     * A ground: its spans about `of` as `about` says, and solid `under`
     * under them, down without end (About's `under`): solid to the bottom.
     */
    pub fn ground(self, of: Rc<Sloped>, below: i64, above: i64, under: impl Into<Kind>) -> Field {
        self.about_as(About { of, below, above, under: Some(under.into()) })
    }

    /** Its spans held to `about` as they are made, and what it meets following from it. */
    fn about_as(self, about: About) -> Field {
        let name = self.name;
        let FieldValues::Spans { axis: 1, spans } = self.values else {
            panic!("field {name}: spans along y only are about a plan of heights")
        };
        let about = Rc::new(about);
        let held = about.clone();
        let spans = Box::new(move |x, z| {
            let spans = spans(x, z);
            held.hold(name, x, z, &spans);
            spans
        });
        Field { values: FieldValues::Spans { axis: 1, spans }, ..self }.meets(move |b, spacing| about.meets(b, spacing))
    }

    /**
     * What it draws in a box (lattice units) at sites a spacing apart,
     * `meets(box, spacing)`, along any axis (Field::meets): a rule;
     * drawn only where it may be, a box all of a kind without asking.
     */
    pub fn meets(self, meets: impl Fn(&Box3, i64) -> Meets + 'static) -> Field {
        Field { meets: Some(Box::new(meets)), ..self }
    }
}

/**
 * Where a field's spans along y are: from `below` under to `above` over
 * `of` at their column, give or take what a line of sites stands for
 * (`slack`), and under that `under` if it says, down without end (a span
 * from DEEP). A rule, checked on each line's spans as they are made; what
 * the field meets follows (Sloped::range).
 */
struct About {
    of: Rc<Sloped>,
    below: i64,
    above: i64,
    /** A solid kind under it all: the ground, solid to the bottom. */
    under: Option<Kind>,
}

impl About {
    /**
     * How far past `below` and `above` the ground a line reaches, at sites
     * `spacing` apart: down past the eight columns a spacing around it
     * (column.rs's `sides`), the diagonals' up to twice the slope times
     * that off; without one, any way.
     */
    fn sides(&self, spacing: i64) -> Option<i64> {
        Some(2 * self.of.slope()? * spacing)
    }

    /**
     * How far past them a line's spans may reach: its `sides`, and two
     * sites more, as spans are at least two sites tall. Asked of a box,
     * the two sites are what it is widened by (Field::meets).
     */
    fn slack(&self, spacing: i64) -> Option<i64> {
        Some(self.sides(spacing)? + 2 * spacing)
    }

    /** What its lines draw in box `b` (lattice units), at sites `spacing` apart: whatever they draw, all `under` (a kind), or nothing. */
    fn meets(&self, b: &Box3, spacing: i64) -> Meets {
        let range = self.of.range([b.min[0], b.min[2]], [b.max[0] - 1, b.max[2] - 1]);
        let (Some((lo, hi)), Some(sides)) = (range, self.sides(spacing)) else { return Meets::Any };
        let (y0, y1) = (b.min[1], b.max[1] - 1);
        match self.under {
            _ if y0 > hi + self.above + sides => Meets::Nothing,
            Some(kind) if y1 < lo - self.below - sides => Meets::All(kind),
            None if y1 < lo - self.below - sides => Meets::Nothing,
            _ => Meets::Any,
        }
    }

    /** Panics unless field `name`'s `spans` of column (x, z), at the spacing read for, lie where it says. */
    fn hold(&self, name: &str, x: i64, z: i64, spans: &[Span]) {
        let s = plan::spacing();
        let Some(slack) = self.slack(s) else { return };
        // The column itself, wherever the spans read the ground.
        let g = plan::where_asked(|| self.of.at(x, z));
        let (low, high) = (g - self.below - slack, g + self.above + slack);
        let under = |span: &Span| self.under == Some(span.kind) && span.holds == Holds::Solid;
        for span in spans.iter().filter(|span| span.from < span.to) {
            assert!(
                (span.from >= low || under(span)) && span.to <= high,
                "field {name}: span [{}, {}) at column ({x}, {z}) reaches past {} under and {} over its ground {g}, \
                 give or take {slack} at sites {s} apart (Field::about); see \"Rules\" in {DOCS}",
                span.from,
                span.to,
                self.below,
                self.above
            );
        }
        assert!(
            self.under.is_none() || spans.iter().any(|span| under(span) && span.from <= DEEP && span.to >= low),
            "field {name}: at column ({x}, {z}), kind {:?} doesn't reach up from DEEP to {low}, {} under its ground {g}, \
             give or take {slack} at sites {s} apart (Field::ground); see \"Rules\" in {DOCS}",
            self.under,
            self.below,
        );
    }
}

impl Meets {
    /** What a field drawing nothing over `top` along y (lattice units) draws in box `b`: nothing over it, anything under (Field::meets). */
    pub fn under(top: i64, b: &Box3) -> Meets {
        if b.min[1] > top { Meets::Nothing } else { Meets::Any }
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
     * reaches nowhere past its cell unless given `reach`, and is made
     * while its cells are at least a site wide (2N) unless given a `size`.
     */
    pub fn new(name: &'static str, order: f64, cell: i64) -> Population {
        Population {
            name,
            order,
            cell,
            reach: [0; 3],
            size: 2 * cell,
            anchor: Anchors::Cells(Box::new(|c| Some(c.centre.map(|v| v as f64)))),
            make: Box::new(|_, _| None),
            far: None,
            stands: None,
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
        assert!(
            self.stands.is_none(),
            "population {}: give its points before where it stands (`on`), whose N is their leaves' width",
            self.name
        );
        Population { cell: points.leaf, anchor: Anchors::Points { points, anchor: Box::new(anchor) }, ..self }
    }

    /** The thing at an anchor, of a cell, if it has one. */
    pub fn make(self, make: impl Fn(Point, &Cell) -> Option<Node> + 'static) -> Population {
        Population { make: Box::new(make), ..self }
    }

    /**
     * Whether an anchor of its things may lie in a box (lattice units),
     * `stands(box)` (Population::stands): a rule; asked to draw only where
     * they may reach.
     */
    pub fn stands(self, stands: impl Fn(&Box3) -> bool + 'static) -> Population {
        Population { stands: Some(Box::new(stands)), ..self }
    }

    /**
     * Its things standing on `ground`, each anchored at most N (its cell,
     * as now) off it: on the ground the level's sites read where one is,
     * over a box its range (Population::stands).
     */
    pub fn on(self, ground: Rc<Sloped>) -> Population {
        let n = self.cell;
        self.stands(move |b| {
            let column = b.max[0] - b.min[0] == 1 && b.max[2] - b.min[2] == 1;
            let range = if column {
                Some((ground.at(b.min[0], b.min[2]), ground.at(b.min[0], b.min[2])))
            } else {
                ground.range([b.min[0], b.min[2]], [b.max[0], b.max[2]])
            };
            range.is_none_or(|(lo, hi)| b.min[1] <= hi + n && b.max[1] > lo - n)
        })
    }

    /** What it draws while its sites are further apart than its `size`: a medium as dense as its things. */
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
