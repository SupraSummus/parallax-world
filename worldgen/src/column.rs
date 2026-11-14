//! Columns: the spans of a heightfield's line of sites (a field's
//! `Spans` along y), built so that each stays visible at every level.
//!
//! A line of sites along an axis has a site only every two spacings: at
//! a level's sites, x + y + z is even, so with x and z fixed, y keeps
//! its parity. A span shorter than two spacings may hold no site at all,
//! and whatever it is (a pond, a layer of turf) vanishes at that level.
//! So every span a `Column` makes is at least two spacings tall, those of
//! `fill_below_exactly` aside, and a thinned medium keeps its density
//! along a ray. A heightfield's columns close its slopes' sides by
//! reaching down past the columns around (`sides`). See "Writing a world"
//! in docs/generators.md.

use crate::generator::{DEEP, Holds, Kind, Span};
use crate::js;

impl Span {
    /** Solid `kind` in [from, to). */
    pub fn solid(from: i64, to: i64, kind: impl Into<Kind>) -> Span {
        Span { from, to, kind: kind.into(), holds: Holds::Solid }
    }

    /**
     * A medium of `kind` from `from`, `height` tall, stopping `extinction`
     * per unit of a ray's path (see Span): at least two sites tall, and
     * then as much thinner as it is taller, as dense along a ray through it.
     */
    pub fn medium(from: i64, height: i64, kind: impl Into<Kind>, extinction: f64, spacing: i64) -> Span {
        let tall = height.max(2 * spacing);
        Span { from, to: from + tall, kind: kind.into(), holds: Holds::Medium(extinction * height as f64 / tall as f64) }
    }
}

/**
 * A column's neighbours: along the axes, then the diagonals. A voxel shares
 * faces with sites of all eight columns, the diagonals' at its own height.
 */
pub const AROUND: [(i64, i64); 8] = [(-1, 0), (1, 0), (0, -1), (0, 1), (-1, -1), (-1, 1), (1, -1), (1, 1)];

/** The lowest top of the columns `spacing` around (x, z) (AROUND), `ground` giving a column's top. */
pub fn lowest_around(ground: impl Fn(i64, i64) -> i64, x: i64, z: i64, spacing: i64) -> i64 {
    AROUND.iter().map(|&(dx, dz)| ground(x + dx * spacing, z + dz * spacing)).min().expect("eight")
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

    /**
     * `kind` under the layers so far, at least `depth` under the top and
     * two spacings past `lowest`, the lowest of the columns around
     * (`lowest_around`): the sides a slope shows, closed. A span's ends
     * round to its column's sites, up to a spacing each; with less, rays
     * slipped through steep faces, and without the diagonals, past a
     * cliff's corners into the hollow under the ground.
     */
    pub fn sides(self, kind: impl Into<Kind>, lowest: i64, depth: i64) -> Column {
        let bottom = lowest.min(self.height - depth) - 2 * self.spacing;
        self.fill_below_exactly(kind, bottom)
    }

    /** `kind` under the layers so far, down without end: the ground solid to the bottom (`Field::under`). */
    pub fn under(self, kind: impl Into<Kind>) -> Column {
        self.fill_below_exactly(kind, DEEP)
    }

    /**
     * A medium `height` tall on what stands on the ground so far, as
     * `Span::medium`. Thinner than a spacing, a coat on what it stands on
     * instead: the site under its top shows it as often as all of it
     * stops a ray down through it. Grown to two sites, its top would
     * stand a spacing or more over where it is, and rays look down on it
     * that much nearer.
     */
    pub fn medium(mut self, kind: impl Into<Kind>, height: i64, extinction: f64) -> Column {
        let (from, s) = (self.ceiling(), self.spacing);
        let medium = if height < s {
            let to = from + height;
            Span { from: to - 2 * s, to, kind: kind.into(), holds: Holds::Coat(1.0 - js::exp(-extinction * height as f64)) }
        } else {
            Span::medium(from, height, kind, extinction, s)
        };
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
        let spans = Column::new(10, 4).surface(1, 3).surface(2, 1).fill_below(3, -20).medium(4, 6, 0.5).spans();
        let bounds: Vec<(i64, i64)> = spans.iter().map(|s| (s.from, s.to)).collect();
        assert_eq!(bounds, [(-20, -5), (-5, 3), (3, 11), (11, 19)]);
        assert_eq!(spans[3].holds, Holds::Medium(0.5 * 6.0 / 8.0));
        // A field of columns draws them.
        let world = ground(Op::Fill, |_, _, s| Column::new(0, s).surface(0, 1));
        let grid = generate(&world, &Box3 { min: [0, -4, 0], max: [4, 4, 4] }, 0);
        assert!((0..4).all(|x| (0..4).all(|z| (-1..=0).any(|y| grid.get(x, y, z) != 0))), "the turf is on every column");
    }

    #[test]
    fn sides_close_any_heightfield() {
        // Ground as rough as any: each column's top at random, steps, corners and pits of every height. From the air
        // above, along the lattice's 12 neighbours (a voxel's faces), nothing under any column's top is reached.
        let top = |x: i64, z: i64| i64::from(crate::noise::hash(7, x, 0, z) % 40);
        let world = ground(Op::Fill, move |x, z, s| Column::new(top(x, z), s).surface(0, 1).sides(0, lowest_around(top, x, z, s), 3));
        for level in 0..4 {
            let s = 1 << level;
            let b = Box3 { min: [0, -48 / s, 0], max: [24, 48 / s, 24] };
            let grid = generate(&world, &b, level);
            let mut air: Vec<[i64; 3]> =
                (0..24 * 24).map(|i| [i % 24, b.max[1] - 1, i / 24]).filter(|&[x, y, z]| crate::grid::is_site(x, y, z)).collect();
            let mut seen: std::collections::HashSet<[i64; 3]> = air.iter().copied().collect();
            while let Some([x, y, z]) = air.pop() {
                assert!(y * s > top(x * s, z * s), "level {level}: ({x}, {y}, {z}) under the ground, reached from the air");
                for [dx, dy, dz] in crate::picture::FACES {
                    let p = [x + dx, y + dy, z + dz];
                    if b.holds(p) && grid.get(p[0], p[1], p[2]) == 0 && seen.insert(p) {
                        air.push(p);
                    }
                }
            }
        }
    }

    #[test]
    fn a_thin_medium_shows_on_its_ground_as_often_as_it_stops_rays_down_it() {
        // Stopping half of what comes down, on flat ground: half its top sites show it, whatever the field's op, at
        // every level whose sites are further apart than it is tall.
        for op in [Op::Fill, Op::Paint] {
            let world = ground(op, |_, _, s| Column::new(0, s).surface(0, 1).medium(1, 2, std::f64::consts::LN_2 / 2.0));
            for level in 2..6 {
                let grid = generate(&world, &Box3 { min: [0, -4, 0], max: [96, 4, 96] }, level);
                let tops: Vec<u8> =
                    (0..96 * 96).filter_map(|i| (-4..4).rev().map(|y| grid.get(i % 96, y, i / 96)).find(|id| *id != 0)).collect();
                let shown = tops.iter().filter(|id| **id == 2).count() as f64 / tops.len() as f64;
                assert!((shown - 0.5).abs() < 0.03, "level {level}: {shown} of the top sites show it");
            }
        }
    }

    /** A world of stone, and moss (material 2), a field of `columns` its ground, drawn by `op`. */
    fn ground(op: Op, columns: impl Fn(i64, i64, i64) -> Column + 'static) -> Generator {
        let kind = |name, colour| crate::generator::KindDef {
            name,
            kind_type: 1,
            colors: std::slice::from_ref(colour),
            walk_through: false,
            glows: false,
            medium: false,
        };
        let kinds = vec![kind("stone", &"#808080"), kind("moss", &"#408040")];
        Generator { kinds, seed: 1, layers: vec![Layer::Field(Field::columns("ground", 0.0, op, columns))] }
    }
}
