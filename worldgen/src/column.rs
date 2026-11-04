//! Columns: the spans of a heightfield's line of sites (a field's
//! `Spans` along y), built so that each stays visible at every level.
//!
//! A line of sites along an axis has a site only every two spacings: at
//! a level's sites, x + y + z is even, so with x and z fixed, y keeps
//! its parity. A span shorter than two spacings may hold no site at all,
//! and whatever it is (a pond, a layer of turf) vanishes at that level.
//! So every span a `Column` makes is at least two spacings tall, those of
//! `fill_below_exactly` aside, and a thinned medium keeps its density
//! along a ray. See "Writing a world" in docs/generators.md.

use crate::generator::{Kind, Span};

impl Span {
    /** Solid `kind` in [from, to). */
    pub fn solid(from: i64, to: i64, kind: impl Into<Kind>) -> Span {
        Span { from, to, kind: kind.into(), extinction: None }
    }

    /**
     * A medium of `kind` from `from`, `height` tall, stopping `extinction`
     * per unit of a ray's path (see Span): at least two sites tall, and
     * then as much thinner as it is taller, as dense along a ray through it.
     */
    pub fn medium(from: i64, height: i64, kind: impl Into<Kind>, extinction: f64, spacing: i64) -> Span {
        let tall = height.max(2 * spacing);
        Span { from, to: from + tall, kind: kind.into(), extinction: Some(extinction * height as f64 / tall as f64) }
    }
}

/**
 * The spans of a column whose ground's top unit is `height`, at sites
 * `spacing` apart, built from its top: layers stack down from it, media
 * up from it. Its spans come out bottom up, water last.
 */
pub struct Column {
    height: i64,
    spacing: i64,
    /** What is under the top, top down, and what stands on it, bottom up. */
    below: Vec<Span>,
    above: Vec<Span>,
}

impl Column {
    pub fn new(height: i64, spacing: i64) -> Column {
        Column { height, spacing, below: Vec::new(), above: Vec::new() }
    }

    /** Where the ground's layers end so far, going down: the top of the next. */
    fn floor(&self) -> i64 {
        self.below.last().map_or(self.height + 1, |s| s.from)
    }

    fn ceiling(&self) -> i64 {
        self.above.last().map_or(self.height + 1, |s| s.to)
    }

    /** A layer of `kind` `depth` deep, under the layers so far (the first is the ground's top), at least two sites. */
    pub fn surface(mut self, kind: impl Into<Kind>, depth: i64) -> Column {
        let to = self.floor();
        self.below.push(Span::solid(to - depth.max(2 * self.spacing), to, kind));
        self
    }

    /** `kind` under the layers so far, down to `bottom`, or two sites if that is less. */
    pub fn fill_below(self, kind: impl Into<Kind>, bottom: i64) -> Column {
        let bottom = bottom.min(self.floor() - 2 * self.spacing);
        self.fill_below_exactly(kind, bottom)
    }

    /**
     * `kind` under the layers so far, down to `bottom` and no further:
     * thinner than two sites, or nothing. For what shows only from the
     * side, such as soil down to a slope's lower neighbours.
     */
    pub fn fill_below_exactly(mut self, kind: impl Into<Kind>, bottom: i64) -> Column {
        let to = self.floor();
        if bottom < to {
            self.below.push(Span::solid(bottom, to, kind));
        }
        self
    }

    /** A medium `height` tall on what stands on the ground so far, as `Span::medium`. */
    pub fn medium(mut self, kind: impl Into<Kind>, height: i64, extinction: f64) -> Column {
        let medium = Span::medium(self.ceiling(), height, kind, extinction, self.spacing);
        self.above.push(medium);
        self
    }

    /**
     * Water (or any `kind`) where the ground is under `level`, up to it
     * (its top unit, as the ground's `height`). At least two sites deep:
     * where it is shallower, it reaches down over the ground's top, so
     * a field painting it keeps it there.
     */
    pub fn water_to(mut self, kind: impl Into<Kind>, level: i64) -> Column {
        if self.height < level {
            let from = (self.height + 1).min(level + 1 - 2 * self.spacing);
            self.above.push(Span::solid(from, level + 1, kind));
        }
        self
    }

    /** Its spans, bottom up. */
    pub fn spans(self) -> Vec<Span> {
        let mut spans = self.below;
        spans.reverse();
        spans.extend(self.above);
        spans
    }
}

impl From<Column> for Vec<Span> {
    fn from(column: Column) -> Vec<Span> {
        column.spans()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::generator::{Field, Generator, Layer, Op, generate};
    use crate::grid::Box3;

    #[test]
    fn every_span_holds_a_site_at_every_level() {
        // A thin turf over soil, a thin medium on it, shallow water: each still on every line of sites, at every level.
        let column = |h: i64, s: i64| Column::new(h, s).surface(1, 1).fill_below(2, h - 2).medium(3, 1, 0.5).water_to(4, h + 1).spans();
        for level in 0..6 {
            let s = 1 << level;
            for h in -40..40 {
                for line in 0..2 {
                    for span in column(h, s) {
                        // Sites on this line along y, in lattice units: y ≡ line (mod 2) at the level.
                        let first = span.from.div_euclid(s) + i64::from(span.from.rem_euclid(s) != 0);
                        let site = (first..).find(|y| (y - line).rem_euclid(2) == 0).expect("a site") * s;
                        assert!(site < span.to, "level {level}, height {h}: {}..{} has no site of line {line}", span.from, span.to);
                    }
                }
            }
        }
    }

    #[test]
    fn spans_come_out_bottom_up_and_media_keep_their_density() {
        let spans = Column::new(10, 4).surface(1, 3).surface(2, 1).fill_below(3, -20).medium(4, 2, 0.5).spans();
        let bounds: Vec<(i64, i64)> = spans.iter().map(|s| (s.from, s.to)).collect();
        assert_eq!(bounds, [(-20, -5), (-5, 3), (3, 11), (11, 19)]);
        assert_eq!(spans[3].extinction, Some(0.5 * 2.0 / 8.0));
        // A field of columns draws them.
        let layers = vec![Layer::Field(Field::columns("ground", 0.0, Op::Fill, |_, _, s| Column::new(0, s).surface(0, 1)))];
        let world =
            Generator { kinds: vec![crate::generator::KindDef { name: "stone", kind_type: 1, colors: &["#808080"] }], seed: 1, layers };
        let grid = generate(&world, &Box3 { min: [0, -4, 0], max: [4, 4, 4] }, 0);
        assert!((0..4).all(|x| (0..4).all(|z| (-1..=0).any(|y| grid.get(x, y, z) != 0))), "the turf is on every column");
    }
}
