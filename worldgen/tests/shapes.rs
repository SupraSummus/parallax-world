//! Each solid the engine draws (a tube's pieces are capsules, or cones
//! where its radius changes) keeps the contract (worldgen::checks::contract) to the levels its case names, wherever
//! it falls against the sites: many of one case, of sizes and places
//! drawn at random, on a ground.
//! Bright on dark, as small shares of a bright colour differ the most in
//! sRGB, so a world's palette can only hide what breaks here.

use worldgen::World;
use worldgen::checks::contract::{Measure, margins, whole_equals_parts};
use worldgen::curve::Curve;
use worldgen::generator::{
    ANY_SIZE, Field, Generator, KindDef, Layer, Node, Op, Point, Population, SOLID, Shape, Span, ball, capsule, cuboid, ellipsoid, gable,
    lathe, tube,
};
use worldgen::grid::Box3;
use worldgen::noise::{Rng, fx};

static KINDS: &[KindDef] = &[
    KindDef { name: "ground", kind_type: 1, colors: &["#222222"], walk_through: false, glows: false, medium: false },
    KindDef { name: "solid", kind_type: 3, colors: &["#eeeeee"], walk_through: false, glows: false, medium: false },
];
const GROUND: u8 = 0;
const BRIGHT: u8 = 1;

/** A solid at `p`, on the ground, its sizes and height over it drawn from `random`. */
type Make = fn(Point, &mut Rng) -> Shape;

/** `size` give or take a third. */
fn about(random: &mut Rng, size: f64) -> f64 {
    size * (2.0 / 3.0 + random.next() * 2.0 / 3.0)
}

/** A box about `size`, along x or z, about `lift` over the ground. */
fn block(p: Point, size: [f64; 3], lift: f64, random: &mut Rng) -> Shape {
    let s = size.map(|v| about(random, v));
    let s = if random.next() < 0.5 { s } else { [s[2], s[1], s[0]] };
    let min = [p[0] - s[0] / 2.0, p[1] + about(random, lift), p[2] - s[2] / 2.0];
    cuboid(min, [min[0] + s[0], min[1] + s[1], min[2] + s[2]], BRIGHT, SOLID, Op::Fill)
}

/**
 * The cases: a name, the cell one stands in (every other one of a square
 * grid), and the levels it keeps the contract to, against the level two
 * finer and from above (`margins`). Past them it breaks: planned work
 * in docs/todo.md, and the levels only rise.
 */
const CASES: &[(&str, i64, Make, u32, u32)] = &[
    ("speck", 4, |p, r| block(p, [1.0, 1.0, 1.0], 4.0, r), 5, 5),
    ("block", 16, |p, r| block(p, [6.0, 6.0, 6.0], 0.0, r), 5, 5),
    ("big block", 48, |p, r| block(p, [24.0, 24.0, 24.0], 0.0, r), 5, 3),
    ("wall", 24, |p, r| block(p, [16.0, 20.0, 1.0], 0.0, r), 5, 1),
    ("thick wall", 24, |p, r| block(p, [16.0, 20.0, 3.0], 0.0, r), 5, 2),
    ("plate", 24, |p, r| block(p, [16.0, 1.0, 16.0], 12.0, r), 5, 3),
    ("post", 8, |p, r| block(p, [1.0, 16.0, 1.0], 0.0, r), 5, 1),
    ("beam", 16, |p, r| block(p, [16.0, 1.0, 1.0], 8.0, r), 5, 5),
    ("ball", 8, |p, r| ball([p[0], p[1] + about(r, 3.0), p[2]], about(r, 1.5), BRIGHT, SOLID, Op::Fill), 5, 5),
    ("big ball", 24, |p, r| ball([p[0], p[1] + about(r, 8.0), p[2]], about(r, 8.0), BRIGHT, SOLID, Op::Fill), 5, 3),
    (
        "disc",
        24,
        |p, r| {
            let (a, b) = (about(r, 8.0), about(r, 1.0));
            ellipsoid([p[0], p[1] + about(r, 10.0), p[2]], [a, b, a], BRIGHT, SOLID, Op::Fill)
        },
        5,
        3,
    ),
    (
        "stick",
        16,
        |p, r| {
            let (l, turn, y) = (about(r, 12.0), r.next() * std::f64::consts::TAU, p[1] + about(r, 4.0));
            let q = [p[0] + l * turn.cos(), y + l * (r.next() - 0.5), p[2] + l * turn.sin()];
            capsule([p[0], y, p[2]], q, about(r, 0.5), BRIGHT, SOLID, Op::Fill)
        },
        5,
        5,
    ),
    (
        "bent cone",
        24,
        |p, r| {
            let (h, w) = (about(r, 20.0), about(r, 2.0));
            let bend = Curve::wandering(r.next_u64(), vec![fx(2.0), fx(1.0)]);
            tube(&bend, [p[0], p[1] - 1.0, p[2]], [p[0], p[1] + h, p[2]], move |t| w * (1.0 - t * 2.0 / 3.0), BRIGHT, SOLID, Op::Fill)
        },
        5,
        5,
    ),
    (
        "cone",
        24,
        |p, r| {
            let (h, w) = (about(r, 24.0), about(r, 4.0));
            lathe(p, [p[0], p[1] + h, p[2]], move |t| w * (1.0 - t), BRIGHT, SOLID, Op::Fill)
        },
        5,
        5,
    ),
    (
        "roof",
        32,
        |p, r| {
            let (l, w, h) = (about(r, 16.0), about(r, 12.0), about(r, 8.0));
            let min = [p[0] - l / 2.0, p[1], p[2] - w / 2.0];
            gable(min, [min[0] + l, min[1] + h, min[2] + w], 0, BRIGHT, SOLID, Op::Fill)
        },
        5,
        3,
    ),
];

/** Many of a case on a ground at y = 0, one anywhere in each cell it stands in. */
fn world(make: Make, cell: i64) -> World {
    let ground = Field::columns("ground", 0.0, Op::Fill, |_, _, _| [Span { from: -64, to: 0, kind: GROUND, extinction: None }]);
    // At every level, though cells narrower than the sites cost fourfold a level: how shapes are drawn, to the coarsest.
    let things = Population::new("things", 1.0, cell)
        .reach(32)
        .size(ANY_SIZE)
        .anchor(|cell| {
            let mut random = cell.random("where");
            let mut at = |c: i64| c as f64 + cell.n as f64 * (random.next() - 0.5);
            (cell.j == 0).then(|| [at(cell.centre[0]), 0.0, at(cell.centre[2])])
        })
        .make(move |p, cell| Some(Node::of(vec![make(p, &mut cell.random("size"))])));
    let generator = Generator { kinds: KINDS.to_vec(), seed: 5, layers: vec![Layer::Field(ground), Layer::Population(things)] };
    World { generator, spawn: [0; 3], heights: (-64, 64), points: Vec::new() }
}

const LEVELS: u32 = 5;

#[test]
fn each_solid_keeps_the_contract_to_its_levels() {
    let region = Box3 { min: [-128, -16, -128], max: [128, 48, 128] };
    let failed: Vec<String> = std::thread::scope(|scope| {
        let runs: Vec<_> = CASES
            .iter()
            .map(|&(name, cell, make, finer, above)| {
                scope.spawn(move || {
                    let world = world(make, cell);
                    for level in [0, 2, LEVELS] {
                        whole_equals_parts(&world.generator, &region, level);
                    }
                    let report = margins(name, &world, &[region], LEVELS, LEVELS);
                    // The last level before the first that breaks, of each.
                    let kept = |above: bool| {
                        let broken = report.margins.iter().filter(|m| m.broken() && (m.measure == Measure::FromAbove) == above);
                        broken.map(|m| m.level - 1).min().unwrap_or(LEVELS)
                    };
                    let kept = (kept(false), kept(true));
                    (kept != (finer, above)).then(|| format!("{name} keeps it to {kept:?}, not {:?}:\n{}", (finer, above), report.broken()))
                })
            })
            .collect();
        runs.into_iter().filter_map(|run| run.join().expect("ran")).collect()
    });
    assert!(failed.is_empty(), "levels against the level two finer and from above:\n{}", failed.join("\n"));
}
