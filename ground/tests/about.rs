//! Fields about a ground and populations on it: drawn only where they
//! may be, and held to it (ground/src/about.rs).

use std::sync::Arc;

use ground::column::Column;
use ground::plan::{Plan, Sloped};
use ground::{FieldAbout, PopulationOn, columns};
use worldgen::generator::{Generator, KindDef, Layer, Node, Op, Population, SOLID, Span, Whole, ball, checking_skipped, generate, whole};
use worldgen::grid::{ALPHA, Box3, is_site};

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
    let of = Arc::new(Sloped::new(Arc::new(Plan::new("ground", 1, move |x, _| height(x))), slope));
    let field = columns("ground", 0.0, Op::Fill, move |x, _| vec![Span::solid(height(x) - 8, height(x) + 1, 0)]);
    layers(vec![Layer::Field(field.about(of, 8, 1))])
}

/** Ground `height(x)` high, said to rise at most `slope` a column: a surface about it, stone under it. */
fn solid_ground(height: fn(i64) -> i64, slope: i64) -> Generator {
    let of = Arc::new(Sloped::new(Arc::new(Plan::new("ground", 1, move |x, _| height(x))), slope));
    let field = columns("ground", 0.0, Op::Fill, move |x, _| Column::new(height(x)).surface(0, 4).under(0));
    layers(vec![Layer::Field(field.ground(of, 4, 1, 0))])
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
    let flat = Arc::new(Sloped::new(Arc::new(Plan::new("flat", 1, |_, _| 0)), 0));
    let field = columns("ground", 0.0, Op::Fill, |_, _| vec![Span::solid(-8, 1, 0)]).about(flat, 2, 1);
    generate(&layers(vec![Layer::Field(field)]), &Box3 { min: [0, -16, 0], max: [8, 16, 8] }, 0);
}

#[test]
fn checking_finds_what_a_broken_promise_skips() {
    // A spike at x = 12 the slope's rule never sees (no neighbour of it is read): sampled every 8, it is skipped.
    let spiked = ground(|x| if x == 12 { 200 } else { 0 }, 0);
    let b = Box3 { min: [0, 45, 0], max: [32, 55, 4] };
    assert!(generate(&spiked, &b, 2).pieces.alpha.iter().all(|&a| a == 0), "skipped");
    let (_, broken) = checking_skipped(|| generate(&spiked, &b, 2));
    assert!(broken.len() == 1 && broken[0].starts_with("ground draws where it says it may not: at site [3, 49, 0]"), "{broken:?}");
}

#[test]
fn a_field_under_its_ground_is_solid_to_the_bottom_and_says_so() {
    let world = solid_ground(|x| x / 4, 1);
    for level in 0..4 {
        let s = 1 << level;
        // Deep under it, across it, over it.
        for (y, expected) in [(-4096, Some(Whole::Buried(0))), (-16, None), (1024, Some(Whole::Empty))] {
            let b = Box3 { min: [0, y / s, 0], max: [64, y / s + 32, 16] };
            assert_eq!(whole(&world, &b, level), expected, "level {level}, at {y}");
            let (grid, broken) = checking_skipped(|| generate(&world, &b.widen(1), level));
            assert_eq!(broken, Vec::<String>::new());
            let sites =
                || (b.min[2]..b.max[2]).flat_map(|z| (b.min[1]..b.max[1]).flat_map(move |y| (b.min[0]..b.max[0]).map(move |x| [x, y, z])));
            let alpha = |[x, y, z]: [i64; 3]| grid.alpha(x, y, z);
            match expected {
                Some(Whole::Buried(_)) => assert!(sites().filter(|&[x, y, z]| is_site(x, y, z)).all(|p| alpha(p) == ALPHA)),
                Some(Whole::Empty) => assert!(sites().all(|p| alpha(p) == 0)),
                None => {}
            }
            // Drawn as the field would, without asking it.
            assert_eq!(generate(&world, &b, level).alpha(0, b.min[1], 0), grid.alpha(0, b.min[1], 0));
        }
    }
}

#[test]
#[should_panic(expected = "field ground: at column (0, 0), kind Some(0) doesn't reach up from DEEP to -6, 4 under its ground 0")]
fn refuses_a_field_not_under_its_ground_as_it_says() {
    let flat = Arc::new(Sloped::new(Arc::new(Plan::new("flat", 1, |_, _| 0)), 0));
    let field = columns("ground", 0.0, Op::Fill, |_, _| vec![Span::solid(-4, 1, 0)]).ground(flat, 4, 1, 0);
    generate(&layers(vec![Layer::Field(field)]), &Box3 { min: [0, -16, 0], max: [8, 16, 8] }, 0);
}

#[test]
fn checking_finds_a_field_not_under_its_ground_where_it_says() {
    // A hole at x = 12 the slope's rule never sees, sampled every 8: filled under the ground all the same.
    let holed = solid_ground(|x| if x == 12 { -200 } else { 0 }, 0);
    let b = Box3 { min: [0, -55, 0], max: [32, -45, 4] };
    let (_, broken) = checking_skipped(|| generate(&holed, &b, 2));
    assert!(broken.len() == 1 && broken[0].starts_with("ground isn't all kind 0, as it says: at site [3, -55, 0]"), "{broken:?}");
}

#[test]
fn makes_no_thing_anchored_off_the_ground_it_stands_on() {
    // The ground is 100 units under it.
    let low = Arc::new(Sloped::new(Arc::new(Plan::new("low", 1, |_, _| -100)), 0));
    let rocks = Population::new("rocks", 1.0, 16)
        .on(low)
        .at(|_| vec![([0.0; 3], ())], |_, anchor| Some(Node::of(vec![ball(anchor.at, 2.0, 0, SOLID, Op::Fill)])));
    let grid = generate(&layers(vec![Layer::Population(rocks)]), &Box3 { min: [-8, -100, -8], max: [8, 8, 8] }, 0);
    assert!(grid.pieces.alpha.iter().all(|&a| a == 0));
}
