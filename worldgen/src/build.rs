//! Short forms for writing worlds: constructors of fields, populations,
//! nodes and solid shapes, so a world says what it draws without struct
//! literals and boxes. The long forms stay:
//! they are the model (generator.rs). See "Writing a world" in
//! docs/generators.md.

use crate::curve::Curve;
use crate::generator::{
    Anchor, Child, Children, Far, Field, FieldMeets, FieldValues, Holds, Kind, Make, Meets, Node, Op, Point, Population, Radius, SOLID,
    Shape, Shapes, Span, Stands, ball, bounds_of_shapes, capsule, cuboid, ellipsoid, gable, lathe, lattice, tube,
};
use crate::grid::Box3;

use crate::noise::Rng;
use crate::sites::spacing;

/**
 * A span `length` long as it stays at every level: at least two spacings
 * of the sites read for, as a line of sites along an axis has a site every
 * two (generator.rs, Span). Grown away from the end that shows: a layer
 * from its top, a medium standing free from what it stands on.
 */
pub fn thick(length: i64) -> i64 {
    length.max(2 * spacing())
}

impl Span {
    /** Solid `kind` in [from, to). */
    pub fn solid(from: i64, to: i64, kind: impl Into<Kind>) -> Span {
        Span { from, to, kind: kind.into(), holds: Holds::Solid }
    }

    /**
     * A medium of `kind` from `from`, `length` long, stopping `extinction`
     * per unit of a ray's path (see Holds), standing free: `thick`, and
     * then as much thinner as it is longer, as dense along a ray through
     * it. In the site of what it stands on alone, it would show only in
     * the colour of that site's pieces (VoxelGrid's `partial`).
     */
    pub fn medium(from: i64, length: i64, kind: impl Into<Kind>, extinction: f64) -> Span {
        let long = thick(length);
        Span { from, to: from + long, kind: kind.into(), holds: Holds::Medium(extinction * length as f64 / long as f64) }
    }
}

impl Field {
    /** A kind, with a medium's extinction if it is one, at each site: `at(x, y, z)`, lattice units. */
    pub fn at(
        name: &'static str,
        order: f64,
        op: Op,
        at: impl Fn(i64, i64, i64) -> Option<(Kind, Option<f64>)> + Send + Sync + 'static,
    ) -> Field {
        Field { name, order, op, values: FieldValues::At(Box::new(at)), meets: None }
    }

    /** Spans along each line of sites parallel to `axis`: `spans(u, v)`, the line's coordinates across it, as `across` orders them (FieldValues::Spans). */
    pub fn spans<S: Into<Vec<Span>>>(
        name: &'static str,
        order: f64,
        op: Op,
        axis: usize,
        spans: impl Fn(i64, i64) -> S + Send + Sync + 'static,
    ) -> Field {
        let spans = Box::new(move |u, v| spans(u, v).into());
        Field { name, order, op, values: FieldValues::Spans { axis, spans }, meets: None }
    }

    /**
     * Shapes, each of `op`, cutting or filling the world, not a thing:
     * `near(box)`, those that may reach the box (lattice units), each the
     * same, in the same order, whichever box asks (FieldValues::Shapes).
     * What it meets follows from their bounds (`whole`).
     */
    pub fn shapes<S: IntoIterator<Item = Shape>>(
        name: &'static str,
        order: f64,
        op: Op,
        near: impl Fn(&Box3) -> S + Send + Sync + 'static,
    ) -> Field {
        let values = FieldValues::Shapes(Box::new(move |b| near(b).into_iter().collect()));
        Field { name, order, op, values, meets: None }
    }

    /**
     * What it draws in a box (lattice units) at sites a spacing apart,
     * `meets(box, spacing)`, along any axis (Field::meets): a rule;
     * drawn only where it may be, a box all of a kind without asking.
     * Rules add up: what one said before, if more than anything, holds.
     */
    pub fn meets(self, meets: impl Fn(&Box3, i64) -> Meets + Send + Sync + 'static) -> Field {
        let meets: Box<FieldMeets> = match self.meets {
            None => Box::new(meets),
            Some(before) => Box::new(move |b, spacing| match before(b, spacing) {
                Meets::Any => meets(b, spacing),
                said => said,
            }),
        };
        Field { meets: Some(meets), ..self }
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
     * Things at the sites of `cell`·FCC (`lattice`), N: none until it is
     * given `make`, or other points (`at`). Each reaches nowhere past its
     * anchor unless given `reach`, and is made while sites are at most 2N
     * apart unless given a `size`.
     */
    pub fn new(name: &'static str, order: f64, cell: i64) -> Population {
        Population {
            name,
            order,
            cell,
            reach: [0; 3],
            size: 2 * cell,
            things: Box::new(|_, _| {}),
            far: None,
            stands: None,
            measured: Default::default(),
        }
    }

    /** How far a thing reaches from its anchor: one number, or per axis. */
    pub fn reach(self, reach: impl PerAxis) -> Population {
        Population { reach: reach.per_axis(), ..self }
    }

    /** Things are made while sites are at most `size` apart; beyond, the far field is drawn, if any. */
    pub fn size(self, size: i64) -> Population {
        Population { size, ..self }
    }

    /**
     * A thing at each of the points `points` gives in a box (lattice
     * units), if `make` makes one of it: each point once, the same whoever
     * asks, as a pure function of position does. It may give points
     * outside the box: only those in it are asked for.
     */
    pub fn at<T: 'static>(
        self,
        points: impl Fn(&Box3) -> Vec<(Point, T)> + Send + Sync + 'static,
        make: impl Fn(&T, &Anchor) -> Option<Node> + Send + Sync + 'static,
    ) -> Population {
        let things = move |b: &Box3, each: &mut dyn FnMut(Point, &Make<'_>)| {
            for (p, of) in points(b) {
                each(p, &|anchor| make(&of, anchor));
            }
        };
        Population { things: Box::new(things), ..self }
    }

    /** The thing at each site of N·FCC (`lattice`), if it has one. */
    pub fn make(self, make: impl Fn(&Anchor) -> Option<Node> + Send + Sync + 'static) -> Population {
        let n = self.cell;
        self.at(move |b| lattice(n, b).into_iter().map(|p| (p.map(|v| v as f64), ())).collect(), move |_, anchor| make(anchor))
    }

    /**
     * Whether an anchor of its things may lie in a box (lattice units),
     * `stands(box)` (Population::stands): its points are only those.
     * They add up: where one said before they may.
     */
    pub fn stands(self, stands: impl Fn(&Box3) -> bool + Send + Sync + 'static) -> Population {
        let stands: Box<Stands> = match self.stands {
            None => Box::new(stands),
            Some(before) => Box::new(move |b| before(b) && stands(b)),
        };
        Population { stands: Some(stands), ..self }
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
    pub fn lazy(bounds: Box3, seed: u64, make: impl Fn(&mut Rng) -> Vec<Shape> + Send + Sync + 'static) -> Node {
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

    pub fn gable(min: Point, max: Point, ridge: usize, up: [i64; 3], kind: impl Into<Kind>) -> Shape {
        gable(min, max, ridge, up, kind.into(), SOLID, Op::Fill)
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
