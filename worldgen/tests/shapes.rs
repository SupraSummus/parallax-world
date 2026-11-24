//! Each solid the engine draws (a tube's pieces are capsules, or cones
//! where its radius changes) keeps the contract (worldgen::checks::contract) as its baseline says, wherever
//! it falls against the sites: many of one case, of sizes and places
//! drawn at random, on a ground.
//! Bright on dark, as small shares of a bright colour differ the most in
//! sRGB, so a world's palette can only hide what breaks here.

use worldgen::World;
use worldgen::checks::contract::{Accuracy, Break, Margin, Region, Report, hold_to_baseline, in_regions};
use worldgen::curve::Curve;
use worldgen::generator::{
    ANY_SIZE, Field, Generator, KindDef, Layer, Node, Op, Point, Population, SOLID, Shape, Span, ball, capsule, cuboid, ellipsoid, gable,
    lathe, lattice, tube,
};
use worldgen::grid::Box3;
use worldgen::noise::{Rng, fx, hash64};

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
 * The cases: a name, and the cell one stands in (every other one of a
 * square grid). Where each breaks the contract, its cells against what is
 * in them (`margins`), and by how much, is its baseline,
 * data/contract/shapes.txt (CONTRACT_BLESS=1 writes it): planned work in
 * docs/todo.md.
 */
const CASES: &[(&str, i64, Make)] = &[
    ("speck", 4, |p, r| block(p, [1.0, 1.0, 1.0], 4.0, r)),
    ("block", 16, |p, r| block(p, [6.0, 6.0, 6.0], 0.0, r)),
    ("big block", 48, |p, r| block(p, [24.0, 24.0, 24.0], 0.0, r)),
    ("wall", 24, |p, r| block(p, [16.0, 20.0, 1.0], 0.0, r)),
    ("thick wall", 24, |p, r| block(p, [16.0, 20.0, 3.0], 0.0, r)),
    ("plate", 24, |p, r| block(p, [16.0, 1.0, 16.0], 12.0, r)),
    ("post", 8, |p, r| block(p, [1.0, 16.0, 1.0], 0.0, r)),
    ("beam", 16, |p, r| block(p, [16.0, 1.0, 1.0], 8.0, r)),
    ("ball", 8, |p, r| ball([p[0], p[1] + about(r, 3.0), p[2]], about(r, 1.5), BRIGHT, SOLID, Op::Fill)),
    ("big ball", 24, |p, r| ball([p[0], p[1] + about(r, 8.0), p[2]], about(r, 8.0), BRIGHT, SOLID, Op::Fill)),
    ("disc", 24, |p, r| {
        let (a, b) = (about(r, 8.0), about(r, 1.0));
        ellipsoid([p[0], p[1] + about(r, 10.0), p[2]], [a, b, a], BRIGHT, SOLID, Op::Fill)
    }),
    ("stick", 16, |p, r| {
        let (l, turn, y) = (about(r, 12.0), r.next() * std::f64::consts::TAU, p[1] + about(r, 4.0));
        let q = [p[0] + l * turn.cos(), y + l * (r.next() - 0.5), p[2] + l * turn.sin()];
        capsule([p[0], y, p[2]], q, about(r, 0.5), BRIGHT, SOLID, Op::Fill)
    }),
    ("bent cone", 24, |p, r| {
        let (h, w) = (about(r, 20.0), about(r, 2.0));
        let bend = Curve::wandering(r.next_u64(), vec![fx(2.0), fx(1.0)]);
        tube(&bend, [p[0], p[1] - 1.0, p[2]], [p[0], p[1] + h, p[2]], move |t| w * (1.0 - t * 2.0 / 3.0), BRIGHT, SOLID, Op::Fill)
    }),
    ("cone", 24, |p, r| {
        let (h, w) = (about(r, 24.0), about(r, 4.0));
        lathe(p, [p[0], p[1] + h, p[2]], move |t| w * (1.0 - t), BRIGHT, SOLID, Op::Fill)
    }),
    ("roof", 32, |p, r| {
        let (l, w, h) = (about(r, 16.0), about(r, 12.0), about(r, 8.0));
        let min = [p[0] - l / 2.0, p[1], p[2] - w / 2.0];
        gable(min, [min[0] + l, min[1] + h, min[2] + w], 0, [0, 1, 0], BRIGHT, SOLID, Op::Fill)
    }),
];

/** Many of a case on a ground at y = 0, one anywhere in each cell it stands in. */
fn world(make: Make, cell: i64) -> World {
    let ground = Field::spans("ground", 0.0, Op::Fill, 1, |_, _| [Span::solid(-64, 0, GROUND)]);
    // At every level, though cells narrower than the sites cost fourfold a level: how shapes are drawn, to the coarsest.
    let things = Population::new("things", 1.0, cell).reach(32).size(ANY_SIZE).at(
        move |b| {
            // A site of each cell on y = 0 of N·FCC, anywhere within N / 2 of its centre along x and z.
            let around = Box3 { min: [b.min[0] - cell, 0, b.min[2] - cell], max: [b.max[0] + cell, 1, b.max[2] + cell] };
            let at = |c: [i64; 3]| {
                let mut random = Rng::new(hash64(5, c[0], c[1], c[2]));
                let mut at = |c: i64| c as f64 + cell as f64 * (random.next() - 0.5);
                ([at(c[0]), 0.0, at(c[2])], ())
            };
            lattice(cell, &around).into_iter().map(at).collect()
        },
        move |_, anchor| Some(Node::of(vec![make(anchor.at, &mut anchor.random(5))])),
    );
    let generator = Generator { kinds: KINDS.to_vec(), seed: 5, layers: vec![Layer::Field(ground), Layer::Population(things)] };
    // Room over what stands on the ground for a site of the coarsest level, as a coarse site's share may reach past it (docs/todo.md).
    World { generator, spawn: [0; 3], heights: Some((-64, 128)), points: Vec::new() }
}

const LEVELS: u32 = 5;

#[test]
fn each_solid_keeps_to_its_baseline() {
    let region = Box3 { min: [-128, -16, -128], max: [128, 48, 128] };
    let breaks: Vec<Break> = std::thread::scope(|scope| {
        let runs: Vec<_> = CASES
            .iter()
            .map(|&(name, cell, make)| {
                scope.spawn(move || {
                    let world = world(make, cell);
                    let (report, exact) = in_regions(name, &world, &[Region::whole(name, region)], LEVELS);
                    let broken = report.broken(Accuracy::DEFAULT);
                    let message = |m: &Margin| format!("{name}: {}", Report { margins: vec![m.clone()], ..broken.clone() });
                    let of = |m: &Margin| Some((name.to_string(), m.level, m.difference(), m.spread));
                    let exact = exact.into_iter().map(|b| Break { of: None, message: format!("{name}: {b}") });
                    broken.margins.iter().map(|m| Break { of: of(m), message: message(m) }).chain(exact).collect::<Vec<_>>()
                })
            })
            .collect();
        runs.into_iter().flat_map(|run| run.join().expect("ran")).collect()
    });
    hold_to_baseline(&breaks, concat!(env!("CARGO_MANIFEST_DIR"), "/data/contract/shapes.txt"), true);
}
