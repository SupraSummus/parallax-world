//! Fields and populations on a ground: spans along y, up, about a plan of
//! heights, and things standing on it, each a rule the engine asks where
//! to draw them (Field::meets, Population::stands).
//! See "Writing a world" in docs/generators.md.

use std::sync::Arc;

use worldgen::generator::{DEEP, Field, FieldValues, Holds, Kind, Meets, Op, Point, Population, Span};
use worldgen::grid::Box3;
use worldgen::points::{Points, Spot};
use worldgen::sites::{spacing, where_asked};

use crate::DOCS;
use crate::plan::Sloped;

/** Spans along each column, up y, all of them: `spans(x, z)`, lattice units; a `Column` or the spans. */
pub fn columns<S: Into<Vec<Span>>>(name: &'static str, order: f64, op: Op, spans: impl Fn(i64, i64) -> S + Send + Sync + 'static) -> Field {
    Field::spans(name, order, op, 1, spans)
}

/** What a field drawing nothing over `top` along y (lattice units) draws in box `b`: nothing over it, anything under (Field::meets). */
pub fn under(top: i64, b: &Box3) -> Meets {
    if b.min[1] > top { Meets::Nothing } else { Meets::Any }
}

/** A field of `columns` about a ground. */
pub trait FieldAbout {
    /** Its spans along y from `below` under to `above` over `of` at their column (About): a rule; what it meets follows. */
    fn about(self, of: Arc<Sloped>, below: i64, above: i64) -> Field;

    /**
     * A ground: its spans about `of` as `about` says, and solid `under`
     * under them, down without end (About's `under`): solid to the bottom.
     */
    fn ground(self, of: Arc<Sloped>, below: i64, above: i64, under: impl Into<Kind>) -> Field;
}

impl FieldAbout for Field {
    fn about(self, of: Arc<Sloped>, below: i64, above: i64) -> Field {
        about_as(self, About { of, below, above, under: None })
    }

    fn ground(self, of: Arc<Sloped>, below: i64, above: i64, under: impl Into<Kind>) -> Field {
        about_as(self, About { of, below, above, under: Some(under.into()) })
    }
}

/** `field`'s spans held to `about` as they are made, and what it meets following from it. */
fn about_as(field: Field, about: About) -> Field {
    let name = field.name;
    let FieldValues::Spans { axis: 1, spans } = field.values else {
        panic!("field {name}: spans along y only are about a plan of heights")
    };
    let about = Arc::new(about);
    let held = about.clone();
    let spans = Box::new(move |x, z| {
        let spans = spans(x, z);
        held.hold(name, x, z, &spans);
        spans
    });
    Field { values: FieldValues::Spans { axis: 1, spans }, ..field }.meets(move |b, spacing| about.meets(b, spacing))
}

/**
 * The points of `points`, a process across y, in box `b`'s columns, each
 * standing `height(x, z)` high: things on a ground (Population::at).
 */
pub fn standing(points: &Points, b: &Box3, height: impl Fn(i64, i64) -> i64) -> Vec<(Point, Spot)> {
    let spots = points.points_in(b.min, b.max).into_iter();
    spots.map(|spot| ([spot.at[0] as f64, height(spot.at[0], spot.at[2]) as f64, spot.at[2] as f64], spot)).collect()
}

/** A population on a ground. */
pub trait PopulationOn {
    /**
     * Its things standing on `ground`, each anchored at most N (its cell)
     * off it: on the ground the level's sites read where one is,
     * over a box its range (Population::stands).
     */
    fn on(self, ground: Arc<Sloped>) -> Population;
}

impl PopulationOn for Population {
    fn on(self, ground: Arc<Sloped>) -> Population {
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
}

/**
 * Where a field's spans along y are: from `below` under to `above` over
 * `of` at their column, give or take what a line of sites stands for
 * (`slack`), and under that `under` if it says, down without end (a span
 * from DEEP). A rule, checked on each line's spans as they are made; what
 * the field meets follows (Sloped::range).
 */
struct About {
    of: Arc<Sloped>,
    below: i64,
    above: i64,
    /** A solid kind under it all: the ground, solid to the bottom. */
    under: Option<Kind>,
}

impl About {
    /**
     * How far past `below` and `above` the ground a line reaches, at sites
     * `spacing` apart: down past the eight columns a spacing around it
     * (`Column::sides`), the diagonals' up to twice the slope times
     * that off; without one, any way.
     */
    fn sides(&self, spacing: i64) -> Option<i64> {
        Some(2 * self.of.slope()? * spacing)
    }

    /**
     * How far past them a line's spans may reach: its `sides`, and two
     * sites more, as spans are `thick` (worldgen's build.rs). Asked of a box,
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
        let s = spacing();
        let Some(slack) = self.slack(s) else { return };
        // The column itself, wherever the spans read the ground.
        let g = where_asked(|| self.of.at(x, z));
        let (low, high) = (g - self.below - slack, g + self.above + slack);
        let under = |span: &Span| self.under == Some(span.kind) && span.holds == Holds::Solid;
        for span in spans.iter().filter(|span| span.from < span.to) {
            assert!(
                (span.from >= low || under(span)) && span.to <= high,
                "field {name}: span [{}, {}) at column ({x}, {z}) reaches past {} under and {} over its ground {g}, \
                 give or take {slack} at sites {s} apart (FieldAbout::about); see \"Rules\" in {DOCS}",
                span.from,
                span.to,
                self.below,
                self.above
            );
        }
        assert!(
            self.under.is_none() || spans.iter().any(|span| under(span) && span.from <= DEEP && span.to >= low),
            "field {name}: at column ({x}, {z}), kind {:?} doesn't reach up from DEEP to {low}, {} under its ground {g}, \
             give or take {slack} at sites {s} apart (FieldAbout::ground); see \"Rules\" in {DOCS}",
            self.under,
            self.below,
        );
    }
}
