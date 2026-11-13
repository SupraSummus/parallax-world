//! The valley: the forest, towns, fields, landmarks and moorland on one land.

use std::rc::Rc;

use worldgen::domain::Scope;
use worldgen::generator::{Generator, generate, materials_of};
use worldgen::grid::{Box3, is_site};
use worldgen::picture::{Camera, picture};
use worldgen::units::UNIT_SIZE;
use worlds::landmarks::{LANDMARKS, Landmark, Landmarks, NEAR, Site};
use worlds::railway::Railway;
use worlds::valley::{Terrain, valley};

fn unit(metres: f64) -> i64 {
    (metres / UNIT_SIZE) as i64
}

#[test]
fn the_forest_keeps_off_the_first_towns_main_street() {
    let world = valley(42);
    let spawn = world.spawn;
    let town = Terrain::new(&Scope::new(42)).towns.near(spawn[2] + unit(200.0)).expect("a town ahead");
    assert!(town.z - spawn[2] > unit(100.0), "the camera starts outside the town");
    // Along the main street's middle, 20 m into the town: nothing from a metre above it to the sky.
    let street =
        Box3 { min: [town.x - 2, town.level + 8, town.z - unit(60.0)], max: [town.x + 2, town.level + unit(30.0), town.z - unit(40.0)] };
    assert!(generate(&world.generator, &street, 0).cells.iter().all(|id| *id == 0), "something stands on the main street");
}

/**
 * The landmarks of `seed` near points NEAR apart within 500 m of the
 * valley's middle along its first 8 km, so all those standing there, and
 * the terrain they stand on.
 */
fn landmarks(seed: u64) -> (Vec<Site>, Rc<Terrain>) {
    let world = Scope::new(seed);
    let terrain = Rc::new(Terrain::new(&world));
    let railway = Rc::new(Railway::new(&world.of("railway"), terrain.clone()));
    let landmarks = &Landmarks::new(&world.of("landmarks"), terrain.clone(), railway);
    let points = |from: f64, to: f64| (unit(from)..=unit(to)).step_by(NEAR as usize);
    let mut sites: Vec<Site> =
        points(-500.0, 8000.0).flat_map(|z| points(-500.0, 500.0).filter_map(move |x| landmarks.site(x, z))).collect();
    sites.sort_by_key(|s| s.centre);
    sites.dedup_by_key(|s| s.centre);
    (sites, terrain)
}

#[test]
fn landmarks_of_every_kind_stand_out_of_towns_and_off_the_path() {
    let mut kinds = Vec::new();
    for seed in [42, 1, 2, 3] {
        let (sites, terrain) = landmarks(seed);
        for s in sites {
            kinds.push(s.landmark);
            let ([hx, hz], [cx, cz], reach) = (s.half(), s.centre, s.clearing.0);
            let metres = |from: i64, to: i64| (from..=to).step_by(unit(1.0) as usize);
            // Its clearing: within its reach of the footprint, as it bids.
            let cleared = |x: i64, z: i64| ((x - cx).abs() - hx).max(0).pow(2) + ((z - cz).abs() - hz).max(0).pow(2) <= reach * reach;
            let in_town = metres(cz - hz - reach, cz + hz + reach)
                .any(|z| metres(cx - hx - reach, cx + hx + reach).any(|x| cleared(x, z) && terrain.towns.distance(x, z).0 <= 0));
            assert!(!in_town, "seed {seed}: the {:?} at {:?} or its clearing is in a town", s.landmark, s.centre);
            let on_path = metres(cz - hz, cz + hz).any(|z| (terrain.path_x(z).round_units().0 - cx).abs() - hx < unit(3.0));
            assert!(!on_path, "seed {seed}: the {:?} at {:?} stands on the path", s.landmark, s.centre);
        }
    }
    for l in LANDMARKS {
        assert!(kinds.contains(&l), "no {l:?} in the valleys of four seeds");
    }
}

/** The material ids of the kind `name` of `generator` (as worldgen::generator::materials_of numbers them). */
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
        let (materials, water) = (materials_of(&world.generator), materials_of_kind(&world.generator, "water"));
        let ([hx, hz], [cx, cz], out) = (pond.half(), pond.centre, unit(1.0));
        // Grids start on an even x.
        let x0 = 2 * (cx - hx - out).div_euclid(2);
        let area = Box3 { min: [x0, pond.level - unit(4.0), cz - hz - out], max: [cx + hx + out, pond.level + unit(3.0), cz + hz + out] };
        let grid = generate(&world.generator, &area, 0);
        // The top of the water, and the lowest top of the ground (anything solid) in the ring a metre round its footprint.
        let (mut top, mut rim, mut wet) = (i64::MIN, i64::MAX, 0);
        for z in area.min[2]..area.max[2] {
            for x in area.min[0]..area.max[0] {
                let column = (area.min[1]..area.max[1]).filter(|&y| is_site(x, y, z)).map(|y| (y, grid.get(x, y, z)));
                let (mut ground, mut surface) = (i64::MIN, None);
                for (y, id) in column {
                    if water.contains(&id) {
                        surface = Some(y);
                    } else if !materials[id as usize].walk_through {
                        ground = y;
                    }
                }
                if let Some(y) = surface {
                    (top, wet) = (top.max(y), wet + 1);
                }
                if (x - cx).abs() > hx || (z - cz).abs() > hz {
                    rim = rim.min(ground);
                }
            }
        }
        assert!(wet > 100, "seed {seed}: the pond at {:?} holds no water", pond.centre);
        assert!(top < rim, "seed {seed}: the pond at {:?} stands to {top}, over its rim at {rim}", pond.centre);
    }
}

#[test]
fn the_camera_starts_with_a_chapel_in_view() {
    for seed in [42, 1, 2, 3] {
        let world = valley(seed);
        let eye = world.spawn.map(|v| v as f64);
        // The chapel stands about 35 m ahead.
        let camera = Camera::looking_at(eye, [eye[0], eye[1], eye[2] + 1.0], 70f64.to_radians(), 192, 120, unit(50.0) as f64);
        let shot = picture(&world.generator, &camera, 0, world.heights_at(0));
        let stone = materials_of_kind(&world.generator, "churchStone");
        let seen = shot.material.iter().filter(|m| stone.contains(m)).count();
        // Seen past the trees: at least a patch of 12 by 12 pixels of its walls.
        assert!(seen >= 144, "seed {seed}: the first chapel covers {seen} of {} pixels", shot.material.len());
    }
}
