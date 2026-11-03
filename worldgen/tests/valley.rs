//! The valley: the forest, a town and fields on one land.

use std::rc::Rc;

use worldgen::forest::{Relief, UNIT_SIZE};
use worldgen::generator::generate;
use worldgen::grid::Box3;
use worldgen::town::Towns;
use worldgen::valley::valley;

fn unit(metres: f64) -> i64 {
    (metres / UNIT_SIZE) as i64
}

#[test]
fn the_forest_keeps_off_the_first_towns_main_street() {
    let (generator, spawn) = valley(42);
    let town = Towns::new(42, Rc::new(Relief::new(42))).near(spawn[2] + unit(200.0)).expect("a town ahead");
    assert!(town.z - spawn[2] > unit(100.0), "the camera starts outside the town");
    // Along the main street's middle, 20 m into the town: nothing from a metre above it to the sky.
    let street =
        Box3 { min: [town.x - 2, town.level + 8, town.z - unit(60.0)], max: [town.x + 2, town.level + unit(30.0), town.z - unit(40.0)] };
    assert!(generate(&generator, &street, 0).cells.iter().all(|id| *id == 0), "something stands on the main street");
}
