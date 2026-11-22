//! Plans read by fields, at every level (ground/src/plan.rs).

use std::sync::Arc;

use fcc::FACE_KINDS;
use ground::columns;
use ground::plan::Plan;
use worldgen::generator::{Generator, KindDef, Layer, Op, Span, generate};
use worldgen::grid::{ALPHA, Box3, COLOUR_ONE};

const ROCK: u8 = 0;
const GRASS: u8 = 1;

fn world(layer: Layer) -> Generator {
    let kinds = vec![
        KindDef { name: "rock", kind_type: 3, colors: &["#777777"], walk_through: false, glows: false, medium: false },
        KindDef { name: "grass", kind_type: 1, colors: &["#44aa44"], walk_through: false, glows: false, medium: false },
    ];
    Generator { kinds, seed: 1, layers: vec![layer] }
}

/**
 * Grass with a stripe of rock a unit wide every 8 along x, from a plan
 * of scale 1, dithered or not: the share of rock in what the sites drawn
 * at `level` show, by their red (a site over several columns mixes their
 * kinds), and the grid's looks.
 */
fn stripes(dithered: bool, level: u32) -> (f64, Vec<u32>) {
    let make = |x: i64, _| if x.rem_euclid(8) == 0 { ROCK } else { GRASS };
    let plan = Arc::new(if dithered { Plan::dithered("stripes", 1, make) } else { Plan::new("stripes", 1, make) });
    let ground = columns("ground", 0.0, Op::Fill, move |x, z| vec![Span::solid(-8, 0, plan.at(x, z))]);
    let (generator, b) = (world(Layer::Field(ground)), Box3 { min: [0, -8 >> level, 0], max: [4096 >> level, 0, 64] });
    let grid = generate(&generator, &b, level);
    let sites = (b.min[2]..b.max[2]).flat_map(|z| (b.min[1]..b.max[1]).flat_map(move |y| (b.min[0]..b.max[0]).map(move |x| (x, y, z))));
    let sites: Vec<_> = sites.collect();
    let looks: Vec<u32> = sites.iter().map(|&(x, y, z)| grid.get(x, y, z)).collect();
    // Linear red, rock's #77 and grass's #44, of what each face of each site shows over how much it stops.
    let (mut red, mut stopped) = (0.0, 0.0);
    for &(x, y, z) in &sites {
        for k in 0..FACE_KINDS {
            let (alpha, colour) = grid.light(x, y, z, k).0;
            (red, stopped) = (red + colour[0] as f64 / COLOUR_ONE as f64, stopped + alpha as f64 / ALPHA as f64);
        }
    }
    let [rock, grass] = [0x77, 0x44].map(|c: u8| fcc::light::srgb_to_linear(c as f32 / 255.0) as f64);
    ((red / stopped - grass) / (rock - grass), looks)
}

#[test]
fn fields_read_a_dithered_plan_as_often_as_it_covers() {
    // Lines of sites 4 apart read where they are: every other one is on a stripe. Read at a point of the 4 × 4
    // columns each stands for, one in eight.
    let (point, dithered) = (stripes(false, 2).0, stripes(true, 2).0);
    assert!((point - 0.5).abs() < 0.01, "read where each line is: {point}");
    assert!((dithered - 0.125).abs() < 0.01, "dithered: {dithered}");
    assert_eq!(stripes(false, 0).1, stripes(true, 0).1, "level 0 reads where each line is");
}
