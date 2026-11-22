//! Columns: the spans of a heightfield's line of sites (a field's
//! `Spans` along y), built so that each shows at every level as it does
//! at level 0.
//!
//! A site holds what its spans stop of its voxel (`cover` in
//! generator/drawables.rs), so a span thinner than the sites is held in
//! part. A line of sites along an axis has a site only every two
//! spacings (x + y + z is even at a level's sites): on a line whose site
//! is mostly over a thin layer, rays down pass it to the site under it,
//! and the layer fades to what it lies on, as a turf a unit thick on
//! stone does from level 2 (`a_thin_layer_tops_the_ground_at_every_level`).
//! So every layer a `Column` lays is `thick`, what `fill_below` fills
//! aside: from the side, where a slope shows it, it then hides what it
//! lies on. A medium on the ground is as tall as it is,
//! held in part by the sites it reaches. A heightfield's columns close
//! its slopes' sides by reaching down past the columns around (`sides`).
//! The spacing is that of the sites read for (`plan::spacing`), so a
//! world declares a column once, for every level. See "Writing a world"
//! in docs/generators.md.

use crate::generator::{DEEP, Holds, Kind, Span};
use crate::plan::spacing;

/**
 * A span `length` long as it stays at every level: at least two spacings
 * of the sites read for, as a line of sites along an axis has a site every
 * two. A solid grows away from where it is seen (a layer down from its top),
 * a medium standing free up from what it stands on.
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
     * A medium of `kind` from `from`, `height` tall, stopping `extinction`
     * per unit of a ray's path (see Holds), standing free: `thick`, and
     * then as much thinner as it is taller, as dense along
     * a ray through it. In the site of the ground under it alone, it would
     * show only in the colour of the ground's pieces (VoxelGrid's `partial`).
     */
    pub fn medium(from: i64, height: i64, kind: impl Into<Kind>, extinction: f64) -> Span {
        let tall = thick(height);
        Span { from, to: from + tall, kind: kind.into(), holds: Holds::Medium(extinction * height as f64 / tall as f64) }
    }
}

/**
 * A column's neighbours: along the axes, then the diagonals. A voxel shares
 * faces with sites of all eight columns, the diagonals' at its own height.
 */
pub const AROUND: [(i64, i64); 8] = [(-1, 0), (1, 0), (0, -1), (0, 1), (-1, -1), (-1, 1), (1, -1), (1, 1)];

/** The lowest top of the columns a spacing around (x, z) (AROUND), `ground` giving a column's top. */
pub fn lowest_around(ground: impl Fn(i64, i64) -> i64, x: i64, z: i64) -> i64 {
    let s = spacing();
    AROUND.iter().map(|&(dx, dz)| ground(x + dx * s, z + dz * s)).min().expect("eight")
}

/**
 * The spans of a column whose ground's top unit is `height`, at the sites
 * read for, built from its top: layers stack down from it, media up from
 * it. Its spans come out bottom up, what fills up to a level (`fill_to`) last.
 */
pub struct Column {
    height: i64,
    /** What is under the top, top down, and what stands on it, bottom up. */
    below: Vec<Span>,
    above: Vec<Span>,
}

impl Column {
    pub fn new(height: i64) -> Column {
        Column { height, below: Vec::new(), above: Vec::new() }
    }

    /** Where the ground's layers end so far, going down: the top of the next. */
    fn floor(&self) -> i64 {
        self.below.last().map_or(self.height + 1, |s| s.from)
    }

    fn ceiling(&self) -> i64 {
        self.above.last().map_or(self.height + 1, |s| s.to)
    }

    /** A layer of `kind` `depth` deep, `thick`, under the layers so far (the first is the ground's top). */
    pub fn surface(self, kind: impl Into<Kind>, depth: i64) -> Column {
        let bottom = self.floor() - thick(depth);
        self.fill_below(kind, bottom)
    }

    /**
     * `kind` under the layers so far, down to `bottom` and no further:
     * thinner than two sites, or nothing. For what shows only from the
     * side, such as soil down to a slope's lower neighbours.
     */
    pub fn fill_below(mut self, kind: impl Into<Kind>, bottom: i64) -> Column {
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
        let bottom = lowest.min(self.height - depth) - 2 * spacing();
        self.fill_below(kind, bottom)
    }

    /** `kind` under the layers so far, down without end: the ground solid to the bottom (`Field::ground`). */
    pub fn under(self, kind: impl Into<Kind>) -> Column {
        self.fill_below(kind, DEEP)
    }

    /**
     * A medium `height` tall on what stands on the ground so far, as tall
     * as it is: it lies on that where the field paints, and shows on it
     * from above as often as it stops a ray down, at every level.
     */
    pub fn medium(mut self, kind: impl Into<Kind>, height: i64, extinction: f64) -> Column {
        let from = self.ceiling();
        self.above.push(Span { from, to: from + height, kind: kind.into(), holds: Holds::Medium(extinction) });
        self
    }

    /**
     * `kind` on the ground where it is under `level`, up to it
     * (its top unit, as the ground's `height`), as what stands in a hollow.
     * A layer down from its top, `thick`: where it is shallower, it reaches
     * down over the ground's top, so a field painting it keeps it there.
     */
    pub fn fill_to(mut self, kind: impl Into<Kind>, level: i64) -> Column {
        if self.height < level {
            self.above.push(Span::solid(level - thick(level - self.height) + 1, level + 1, kind));
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
    use crate::picture::composited;
    use crate::plan::footprint;

    #[test]
    fn every_solid_span_holds_a_site_at_every_level() {
        // A thin turf over thin soil, a shallow fill: each still on every line of sites, at every level.
        let column = |h: i64, s: i64| footprint(s, [0, 0], || Column::new(h).surface(1, 1).surface(2, 1).fill_to(4, h + 1).spans());
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
    fn a_thin_layer_tops_the_ground_at_every_level() {
        // Turf a unit thick on stone, at heights of every parity: rays down see the turf, near and far. As thin as
        // it is (not two sites), from level 2 they see the stone too, from level 4 up to a third of what they see.
        for h in 0..8 {
            let world = ground(Op::Fill, false, move |_, _| Column::new(h).surface(1, 1).under(0));
            for level in 0..6 {
                let s = (1i64 << level) as f64;
                let grid = generate(&world, &Box3 { min: [0, -4, 0], max: [64, (16 >> level) + 2, 64] }, level);
                let red = |v: f64| fcc::light::srgb_to_linear((v / 255.0) as f32) as f64;
                let turf = (0..1024).map(|i| {
                    let at = |n: u64| 8.0 + 48.0 * crate::noise::hash(n, i, 0, 0) as f64 / u32::MAX as f64;
                    let seen = composited(&grid, [at(1), 16.0 / s - 1.0, at(2)], [0.0, -1.0, 0.0], 16.0 / s + 4.0, 1.0);
                    // How much of what the ray sees is turf's red (0x40) rather than stone's (0x80).
                    let (stopped, r) = seen.iter().fold((0.0, 0.0), |(s, r), (_, seen)| (s + seen.stopped, r + seen.shown[0]));
                    (red(128.0) - r / stopped) / (red(128.0) - red(64.0))
                });
                let turf = turf.sum::<f64>() / 1024.0;
                assert!(turf > 0.99, "height {h}, level {level}: rays down see {turf} of turf");
            }
        }
    }

    #[test]
    fn spans_come_out_bottom_up_and_media_are_as_tall_as_they_are() {
        let spans = footprint(4, [0, 0], || Column::new(10).surface(1, 3).surface(2, 1).fill_below(3, -20).medium(4, 6, 0.5).spans());
        let bounds: Vec<(i64, i64)> = spans.iter().map(|s| (s.from, s.to)).collect();
        assert_eq!(bounds, [(-20, -5), (-5, 3), (3, 11), (11, 17)]);
        assert_eq!(spans[3].holds, Holds::Medium(0.5));
        // A field of columns draws them.
        let world = ground(Op::Fill, false, |_, _| Column::new(0).surface(0, 1));
        let grid = generate(&world, &Box3 { min: [0, -4, 0], max: [4, 4, 4] }, 0);
        assert!((0..4).all(|x| (0..4).all(|z| (-1..=0).any(|y| grid.alpha(x, y, z) != 0))), "the turf is on every column");
    }

    #[test]
    fn sides_close_any_heightfield() {
        // Ground as rough as any: each column's top at random, steps, corners and pits of every height. Light from the
        // sky, along the lattice's directions up (a voxel's faces), reaches nothing two sites under the lowest top of
        // the columns a site's voxel reaches into: not the hollow under the ground, past a cliff's corners or through a
        // steep face. Nearer a cliff's face, the sky shows, as it would.
        let top = |x: i64, z: i64| i64::from(crate::noise::hash(7, x, 0, z) % 40);
        let world = ground(Op::Fill, false, move |x, z| Column::new(top(x, z)).surface(0, 1).sides(0, lowest_around(top, x, z), 3));
        for level in 0..4 {
            let s = 1 << level;
            let b = Box3 { min: [0, -48 / s, 0], max: [24, 48 / s, 24] };
            let grid = generate(&world, &b, level);
            for (x, z) in (2..22).flat_map(|x| (2..22).map(move |z| (x, z))) {
                let lowest = (-1..=1).flat_map(|i| (-1..=1).map(move |j| top((x + i) * s, (z + j) * s))).min().expect("nine");
                for y in (b.min[1]..(lowest - 2 * s).div_euclid(s)).filter(|&y| crate::grid::is_site(x, y, z)) {
                    for n in crate::picture::FACES.iter().filter(|n| n[1] > 0) {
                        let (p, d) = ([x, y, z].map(|v| v as f64), n.map(|v| v as f64 / std::f64::consts::SQRT_2));
                        let exits = |i: usize| match d[i] {
                            v if v > 0.0 => (b.max[i] as f64 - p[i]) / v,
                            v if v < 0.0 => (b.min[i] as f64 - p[i]) / v,
                            _ => f64::INFINITY,
                        };
                        // Only rays out by the box's top: past its sides, what is there is not known.
                        if exits(1) > exits(0).min(exits(2)) {
                            continue;
                        }
                        let through = 1.0 - composited(&grid, p, d, exits(1), 1.0).iter().map(|(_, seen)| seen.stopped).sum::<f64>();
                        assert!(
                            through < 0.01,
                            "level {level}: {through} of the sky reaches ({x}, {y}, {z}), under the ground, along {n:?}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn a_thin_medium_shows_on_its_ground_as_often_at_every_level() {
        // Moss that stops half of a ray down, painted on flat ground: rays down see half of it and half the stone,
        // near and far, whether its kind is pieces or see-through.
        let red = |v: f64| fcc::light::srgb_to_linear((v / 255.0) as f32) as f64;
        let shown = |see_through: bool, level: u32| {
            let world = ground(Op::Paint, see_through, |_, _| Column::new(0).surface(0, 1).medium(1, 2, std::f64::consts::LN_2 / 2.0));
            let grid = generate(&world, &Box3 { min: [0, -6, 0], max: [64, 6, 64] }, level);
            let rays = (0..4096).map(|i| {
                let at = |n: u64| 8.0 + 48.0 * crate::noise::hash(n, i, 0, 0) as f64 / u32::MAX as f64;
                let seen = composited(&grid, [at(1), 5.0, at(2)], [0.0, -1.0, 0.0], 10.0, 1.0);
                // How much of what the ray sees is moss's red (0x40) rather than stone's (0x80).
                let (stopped, r) = seen.iter().fold((0.0, 0.0), |(s, r), (_, seen)| (s + seen.stopped, r + seen.shown[0]));
                (red(128.0) - r / stopped) / (red(128.0) - red(64.0))
            });
            rays.sum::<f64>() / 4096.0
        };
        for see_through in [false, true] {
            for level in 0..6 {
                let moss = shown(see_through, level);
                assert!((moss - 0.5).abs() < 0.08, "see-through {see_through}, level {level}: rays down see {moss} of it");
            }
        }
    }

    #[test]
    fn a_sea_shows_as_much_from_above_at_every_level() {
        // Dark water under a thin, paler, denser top, on stone: where a coarse site holds two of them,
        // the top is in front of the water, not in its place nor mixed with it, and the water in front of the floor.
        let kinds = vec![kind(&"#808080", false), kind(&"#2478a0", true), kind(&"#9cc4d6", true)];
        let sea = |_, _| Column::new(0).surface(0, 1).medium(1, 38, 0.8 / 38.0).medium(2, 2, 0.4 / 2.0);
        let world = Generator { kinds, seed: 1, layers: vec![Layer::Field(Field::columns("sea", 0.0, Op::Paint, sea))] };
        let shown = |level: u32| {
            let s = (1i64 << level) as f64;
            let grid = generate(&world, &Box3 { min: [0, -6, 0], max: [64, (40 >> level) + 6, 64] }, level);
            let mut sum = [0.0; 3];
            for i in 0..4096 {
                let at = |n: u64| 8.0 + 48.0 * crate::noise::hash(n, i, 0, 0) as f64 / u32::MAX as f64;
                for (_, seen) in composited(&grid, [at(1), 40.0 / s + 5.0, at(2)], [0.0, -1.0, 0.0], 40.0 / s + 10.0, 1.0) {
                    (0..3).for_each(|c| sum[c] += seen.shown[c] / 4096.0);
                }
            }
            sum
        };
        let fine = shown(0);
        // Not past level 5, where a voxel holds all of the sea. Not nearer than this: a site's share of the water
        // stops red for all its rays, those that meet the top in the site beside it too (0.027 of red at level 4).
        for level in 1..6 {
            let coarse = shown(level);
            for c in 0..3 {
                assert!((coarse[c] - fine[c]).abs() < 0.035, "level {level}: rays down see {coarse:.3?}, at level 0 {fine:.3?}");
            }
        }
    }

    /** A kind of `colour`, a medium or not. */
    fn kind(colour: &'static &'static str, medium: bool) -> crate::generator::KindDef {
        crate::generator::KindDef {
            name: "",
            kind_type: 1,
            colors: std::slice::from_ref(colour),
            walk_through: false,
            glows: false,
            medium,
        }
    }

    /** A world of stone, and moss, see-through or not, a field of `columns` its ground, drawn by `op`. */
    fn ground(op: Op, see_through: bool, columns: impl Fn(i64, i64) -> Column + Send + Sync + 'static) -> Generator {
        // The moss grey, so that see-through it stops every channel alike.
        let kinds = vec![kind(&"#808080", false), kind(&"#404040", see_through)];
        Generator { kinds, seed: 1, layers: vec![Layer::Field(Field::columns("ground", 0.0, op, columns))] }
    }
}
