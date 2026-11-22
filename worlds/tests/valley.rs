//! The valley: the forest, towns, fields, landmarks and moorland on one land.

use std::sync::Arc;

use ground::domain::Scope;
use worldgen::generator::{Layer, generate, generate_shown, kind_of};
use worldgen::grid::{ALPHA, Box3, is_site};
use worldgen::picture::{Camera, picture};
use worldgen::tiles::{TILE, make_tiles, tile_box};
use worldgen::units::UNIT_SIZE;
use worlds::valley::landmarks::{LANDMARKS, Landmark, Landmarks, NEAR, Site};
use worlds::valley::railway::Railway;
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
    assert!(generate(&world.generator, &street, 0).shown.iter().all(|shown| !shown), "something stands on the main street");
}

/**
 * A host makes the tiles one above another as one box (make_tiles): each
 * is as made alone wherever a voxel may show, here those of the columns
 * around the spawn. Buried, a site may differ: rock under a box is of one
 * shade from its bottom up.
 */
#[test]
fn tiles_made_together_show_what_each_shows_alone() {
    let world = valley(42);
    for level in [0, 3] {
        let heights = world.heights_at(level);
        let [x, y, z] = world.spawn.map(|v| (v >> level).div_euclid(TILE));
        let columns = (x - 2..=x + 2).flat_map(|x| (z - 2..=z + 2).map(move |z| [x, y, z]));
        let mut shown = 0;
        for (key, together) in &columns.flat_map(|key| make_tiles(&world, level, key)).collect::<Vec<_>>() {
            let b = tile_box(heights, *key);
            let alone = generate_shown(&world.generator, &b, level, worldgen::world::UP).to_grid();
            let faces = |g: &fcc::Grid, [x, y, z]: [i32; 3]| std::array::from_fn::<_, { fcc::FACE_KINDS }, _>(|k| g.face(x, y, z, k));
            let held = |g: &fcc::Grid, p @ [x, y, z]: [i32; 3]| (g.get(x, y, z), faces(g, p), g.solid(x, y, z), g.glow(x, y, z));
            let whole = |p: [i32; 3]| [&alone, together].iter().all(|g| fcc::alpha_of(g.get(p[0], p[1], p[2])) == ALPHA);
            let sites =
                (b.min[0]..b.max[0]).flat_map(|x| (b.min[1]..b.max[1]).flat_map(move |y| (b.min[2]..b.max[2]).map(move |z| [x, y, z])));
            for p in sites.filter(|&[x, y, z]| is_site(x, y, z)).map(|p| p.map(|v| v as i32)) {
                let around = fcc::VOXEL_FACES.iter().flat_map(|f| [1, -1].map(|n| [0, 1, 2].map(|a| p[a] + n * f[a])));
                // Whole, and all around it in the tile too: no ray gets to it.
                if whole(p) && around.filter(|q| b.holds(q.map(i64::from))).all(whole) {
                    continue;
                }
                assert_eq!(held(together, p), held(&alone, p), "level {level}, site {p:?}");
                shown += u32::from(alone.get(p[0], p[1], p[2]) != 0);
            }
        }
        assert!(shown > 10_000, "level {level}: only {shown} voxels that may show");
    }
}

/** Nothing standing over a tunnel reaches into its bore: the railway's bores are carved last, the track in them drawn after. */
/**
 * A world is one for all the threads that make its tiles, and what it
 * keeps is a function of it alone: the columns of tiles around the spawn,
 * of a fine, a middle and a coarse level, are the same made by threads
 * together as by one.
 */
#[test]
fn threads_sharing_a_world_make_its_tiles_as_one_alone() {
    const THREADS: usize = 4;
    let (alone, shared) = (valley(42), valley(42));
    let [x, y, z] = alone.spawn;
    let keys: Vec<(u32, [i64; 3])> = [0, 3, 6]
        .into_iter()
        .flat_map(|level| {
            (0..16).map(move |i| (level, [(x >> level) / TILE + i % 4 - 2, (y >> level) / TILE, (z >> level) / TILE + i / 4 - 2]))
        })
        .collect();
    let made = |world: &worldgen::World, (level, key): (u32, [i64; 3])| -> Vec<_> {
        make_tiles(world, level, key).into_iter().map(|(key, grid)| (key, grid.bricks, grid.cells, grid.views, grid.glow)).collect()
    };
    let alone: Vec<_> = keys.iter().map(|&key| made(&alone, key)).collect();
    std::thread::scope(|scope| {
        for thread in 0..THREADS {
            let (keys, alone, shared) = (&keys, &alone, &shared);
            scope.spawn(move || {
                for i in (thread..keys.len()).step_by(THREADS) {
                    assert!(made(shared, keys[i]) == alone[i], "tile {:?} of level {}", keys[i].1, keys[i].0);
                }
            });
        }
    });
}

#[test]
fn the_railways_bores_are_carved_last() {
    let world = valley(42);
    let mut layers: Vec<(f64, &str)> = world
        .generator
        .layers
        .iter()
        .map(|layer| match layer {
            Layer::Field(f) => (f.order, f.name),
            Layer::Population(p) => (p.order, p.name),
        })
        .collect();
    layers.sort_by(|a, b| a.0.total_cmp(&b.0));
    let last: Vec<&str> = layers[layers.len() - 2..].iter().map(|l| l.1).collect();
    assert_eq!(last, ["railway's tunnels", "railway in its tunnels"]);
}

/**
 * The landmarks of `seed` near points NEAR apart within 500 m of the
 * valley's course along its first 8 km, so all those standing there, and
 * the terrain they stand on.
 */
fn landmarks(seed: u64) -> (Vec<Site>, Arc<Terrain>) {
    let world = Scope::new(seed);
    let terrain = Arc::new(Terrain::new(&world));
    let railway = Arc::new(Railway::new(&world.of("railway"), terrain.clone()));
    let landmarks = &Landmarks::new(&world.of("landmarks"), terrain.clone(), railway);
    let points = |from: f64, to: f64| (unit(from)..=unit(to)).step_by(NEAR as usize);
    let course = |z: i64| terrain.course.x(z).round_units().0;
    let mut sites: Vec<Site> =
        points(-500.0, 8000.0).flat_map(|z| points(-500.0, 500.0).filter_map(move |x| landmarks.site(x + course(z), z))).collect();
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
            let on_path = metres(cz - hz, cz + hz).any(|z| (terrain.path.x(z).round_units().0 - cx).abs() - hx < unit(3.0));
            assert!(!on_path, "seed {seed}: the {:?} at {:?} stands on the path", s.landmark, s.centre);
        }
    }
    for l in LANDMARKS {
        assert!(kinds.contains(&l), "no {l:?} in the valleys of four seeds");
    }
}

#[test]
fn ponds_hold_their_water_below_the_ground_round_them() {
    for seed in [42, 1] {
        let (sites, _) = landmarks(seed);
        let pond =
            sites.iter().filter(|s| s.landmark == Landmark::Pond).min_by_key(|s| s.centre[0].abs() + s.centre[1].abs()).expect("a pond");
        let world = valley(seed);
        let kind = kind_of(&world.generator);
        let ([hx, hz], [cx, cz], out) = (pond.half(), pond.centre, unit(1.0));
        // Grids start on an even x.
        let x0 = 2 * (cx - hx - out).div_euclid(2);
        let area = Box3 { min: [x0, pond.level - unit(4.0), cz - hz - out], max: [cx + hx + out, pond.level + unit(3.0), cz + hz + out] };
        let grid = generate(&world.generator, &area, 0);
        // The top of the water, and the lowest top of the ground (what one stands on) in the ring a metre round its footprint.
        let (mut top, mut rim, mut wet) = (i64::MIN, i64::MAX, 0);
        for z in area.min[2]..area.max[2] {
            for x in area.min[0]..area.max[0] {
                let (mut ground, mut surface) = (i64::MIN, None);
                for y in (area.min[1]..area.max[1]).filter(|&y| is_site(x, y, z)) {
                    if kind(grid.get(x, y, z)).is_some_and(|k| k.name == "water") {
                        surface = Some(y);
                    } else if grid.solid(x, y, z) > ALPHA / 2 {
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
        let kind = kind_of(&world.generator);
        let seen = shot.looks.iter().filter(|&&look| kind(look).is_some_and(|k| k.name == "churchStone")).count();
        // Seen past the trees: at least a patch of 12 by 12 pixels of its walls.
        assert!(seen >= 144, "seed {seed}: the first chapel covers {seen} of {} pixels", shot.looks.len());
    }
}
