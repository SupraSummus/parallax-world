//! The valley: the forest, towns, fields and landmarks on one land.

use std::rc::Rc;

use worldgen::generator::{Generator, generate, materials_of};
use worldgen::grid::{Box3, is_site};
use worldgen::picture::{Camera, picture};
use worldgen::units::UNIT_SIZE;
use worlds::forest::Relief;
use worlds::landmarks::{LANDMARKS, Landmark, Landmarks, Site};
use worlds::town::Towns;
use worlds::valley::{Terrain, valley};

fn unit(metres: f64) -> i64 {
    (metres / UNIT_SIZE) as i64
}

#[test]
fn the_forest_keeps_off_the_first_towns_main_street() {
    let world = valley(42);
    let spawn = world.spawn;
    let town = Towns::new(42, Rc::new(Relief::new(42))).near(spawn[2] + unit(200.0)).expect("a town ahead");
    assert!(town.z - spawn[2] > unit(100.0), "the camera starts outside the town");
    // Along the main street's middle, 20 m into the town: nothing from a metre above it to the sky.
    let street =
        Box3 { min: [town.x - 2, town.level + 8, town.z - unit(60.0)], max: [town.x + 2, town.level + unit(30.0), town.z - unit(40.0)] };
    assert!(generate(&world.generator, &street, 0).cells.iter().all(|id| *id == 0), "something stands on the main street");
}

/** The landmarks of `seed` along the valley's first 8 km, in the regions within 500 m of its path, and the terrain under them. */
fn landmarks(seed: u64) -> (Vec<Site>, Terrain) {
    let landmarks = Landmarks::new(seed, Rc::new(Terrain::new(seed)));
    let sites = (-2..36).flat_map(|k| (-2..=2).map(move |i| (i, k))).filter_map(|(i, k)| landmarks.of_region(i, k)).collect();
    (sites, Terrain::new(seed))
}

#[test]
fn landmarks_of_every_kind_stand_out_of_towns_off_the_path_and_below_the_mountains() {
    let mut kinds = Vec::new();
    for seed in [42, 1, 2, 3] {
        let (sites, terrain) = landmarks(seed);
        for s in sites {
            kinds.push(s.landmark);
            let ([hx, hz], [cx, cz]) = (s.half(), s.centre);
            let reach = s.clearing().0;
            let mut columns = (cz - hz - reach..=cz + hz + reach)
                .step_by(unit(1.0) as usize)
                .flat_map(|z| (cx - hx - reach..=cx + hx + reach).step_by(unit(1.0) as usize).map(move |x| (x, z)));
            assert!(
                columns.all(|(x, z)| terrain.towns.distance(x, z).0 > 0),
                "seed {seed}: the {:?} at {:?} or its clearing is in a town",
                s.landmark,
                s.centre
            );
            for z in cz - hz..=cz + hz {
                let path = terrain.path_x(z).round_units().0;
                assert!((path - cx).abs() - hx >= unit(3.0), "seed {seed}: the {:?} at {:?} stands on the path", s.landmark, s.centre);
            }
            assert!(s.level < unit(40.0), "seed {seed}: the {:?} at {:?} stands up in the mountains", s.landmark, s.centre);
        }
    }
    for l in LANDMARKS {
        assert!(kinds.contains(&l), "no {l:?} in the valleys of four seeds");
    }
}

/** The material ids of the kind `name` of `generator`, in order (see worldgen::generator::materials_of). */
fn materials_of_kind(generator: &Generator, name: &str) -> std::ops::Range<u8> {
    let at = generator.kinds.iter().position(|k| k.name == name).expect("a kind of that name");
    let first = 1 + generator.kinds[..at].iter().map(|k| k.colors.len()).sum::<usize>();
    first as u8..(first + generator.kinds[at].colors.len()) as u8
}

#[test]
fn ponds_hold_their_water_below_the_ground_round_them() {
    for seed in [42, 1] {
        let (sites, _) = landmarks(seed);
        let pond =
            sites.iter().filter(|s| s.landmark == Landmark::Pond).min_by_key(|s| s.centre[0].abs() + s.centre[1].abs()).expect("a pond");
        let world = valley(seed);
        let types: Vec<u8> = materials_of(&world.generator).iter().map(|m| m.material_type).collect();
        let ([hx, hz], [cx, cz]) = (pond.half(), pond.centre);
        let out = unit(1.0);
        // Grids start on an even x.
        let x0 = 2 * (cx - hx - out).div_euclid(2);
        let area = Box3 { min: [x0, pond.level - unit(4.0), cz - hz - out], max: [cx + hx + out, pond.level + unit(3.0), cz + hz + out] };
        let grid = generate(&world.generator, &area, 0);
        // The top of the water, and the least top of the ground in the ring a metre round its footprint.
        let (mut water, mut rim, mut wet) = (i64::MIN, i64::MAX, 0);
        for z in area.min[2]..area.max[2] {
            for x in area.min[0]..area.max[0] {
                let column = (area.min[1]..area.max[1]).filter(|&y| is_site(x, y, z)).map(|y| (y, types[grid.get(x, y, z) as usize]));
                let (mut ground, mut top) = (i64::MIN, i64::MIN);
                for (y, t) in column {
                    match t {
                        13 => top = y,
                        1..=3 => ground = y,
                        _ => {}
                    }
                }
                if top > i64::MIN {
                    (water, wet) = (water.max(top), wet + 1);
                }
                if (x - cx).abs() > hx || (z - cz).abs() > hz {
                    rim = rim.min(ground);
                }
            }
        }
        assert!(wet > 100, "seed {seed}: the pond at {:?} holds no water", pond.centre);
        assert!(water < rim, "seed {seed}: the pond at {:?} stands to {water}, over its rim at {rim}", pond.centre);
    }
}

#[test]
fn the_camera_starts_with_a_chapel_in_view() {
    for seed in [42, 1, 2, 3] {
        let world = valley(seed);
        let eye = world.spawn.map(|v| v as f64);
        let camera = Camera::looking_at(eye, [eye[0], eye[1], eye[2] + 1.0], 70f64.to_radians(), 192, 120, unit(120.0) as f64);
        let shot = picture(&world.generator, &camera, 0, world.heights_at(0));
        let stone = materials_of_kind(&world.generator, "churchStone");
        let seen = shot.material.iter().filter(|m| stone.contains(m)).count();
        // Seen through the trees: at least a patch of 10 by 10 pixels of its walls.
        assert!(seen >= 100, "seed {seed}: the first chapel covers {seen} of {} pixels", shot.material.len());
    }
}
