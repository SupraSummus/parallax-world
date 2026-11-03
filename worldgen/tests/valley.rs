//! The valley: the forest, a town and fields on one land.

use std::rc::Rc;

use worldgen::forest::{Relief, UNIT_SIZE};
use worldgen::generator::{generate, materials_of};
use worldgen::grid::Box3;
use worldgen::town::Towns;
use worldgen::valley::valley;

fn unit(metres: f64) -> i64 {
    (metres / UNIT_SIZE) as i64
}

#[test]
fn the_first_town_has_clear_streets_and_a_spire() {
    let (generator, spawn) = valley(42);
    let materials = materials_of(&generator);
    assert!(materials.len() <= 256, "{} materials", materials.len());
    let town = Towns::new(42, Rc::new(Relief::new(42))).near(spawn[2] + unit(200.0)).expect("a town ahead");
    assert!(town.z - spawn[2] > unit(100.0), "the camera starts outside the town");

    // Along the main street's middle, 20 m into the town: nothing from a metre above it to the sky.
    let street =
        Box3 { min: [town.x - 2, town.level + 8, town.z - unit(60.0)], max: [town.x + 2, town.level + unit(30.0), town.z - unit(40.0)] };
    let grid = generate(&generator, &street, 0);
    assert!(grid.cells.iter().all(|id| *id == 0), "something stands on the main street");

    // The spire: copper 27 m and more above the streets, at level 2.
    let level = 2;
    let s = 1i64 << level;
    let above = Box3 {
        min: [((town.x - unit(150.0)) / s) & !1, (town.level + unit(27.0)) / s, (town.z - unit(150.0)) / s],
        max: [(town.x + unit(150.0)) / s, (town.level + unit(40.0)) / s, (town.z + unit(150.0)) / s],
    };
    let grid = generate(&generator, &above, level);
    let copper = grid.cells.iter().filter(|id| **id != 0 && materials[**id as usize].color == "#5f9a86").count();
    assert!(copper > 0, "no spire");
}
