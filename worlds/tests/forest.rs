//! The forest: determinism, contents, spawn.

use std::collections::HashSet;

use worldgen::generator::{generate, materials_of};
use worldgen::grid::{Box3, VoxelGrid, is_site};
use worldgen::units::UNIT_SIZE;
use worlds::forest::forest;

/** 48 m × 80 m ahead of the spawn point, all heights, and the spawn point. */
fn generated(seed: u64) -> (VoxelGrid, Vec<u8>, [i64; 3]) {
    let world = forest(seed);
    let (min_y, max_y) = world.heights_at(0);
    let (generator, spawn) = (world.generator, world.spawn);
    let unit = |metres: f64| (metres / UNIT_SIZE) as i64;
    // Grids start on an even x.
    let x = 2 * (spawn[0] - unit(24.0)).div_euclid(2);
    let area = Box3 { min: [x, min_y, spawn[2] - unit(1.0)], max: [x + unit(48.0), max_y, spawn[2] + unit(79.0)] };
    let types = materials_of(&generator).iter().map(|m| m.material_type).collect();
    (generate(&generator, &area, 0), types, spawn)
}

/** Every voxel: x, y, z and its material type. */
fn voxels(grid: &VoxelGrid, types: &[u8]) -> Vec<(i64, i64, i64, u8)> {
    let [x0, y0, z0] = grid.min;
    let [nx, ny, nz] = grid.n;
    let mut all = Vec::new();
    for z in z0..z0 + nz {
        for y in y0..y0 + ny {
            let mut x = x0 + ((x0 + y + z) & 1);
            while x < x0 + nx {
                let id = grid.get(x, y, z);
                if id != 0 {
                    all.push((x, y, z, types[id as usize]));
                }
                x += 2;
            }
        }
    }
    all
}

#[test]
fn forest_is_deterministic_has_ground_trees_and_a_clear_spawn() {
    let (grid, types, spawn) = generated(42);
    let all = voxels(&grid, &types);

    // Deterministic for a seed.
    let (again, _, again_spawn) = generated(42);
    assert_eq!(again_spawn, spawn);
    assert!(again.cells == grid.cells && again.bricks == grid.bricks);

    // Ground under every column of the 48 m × 80 m area.
    let columns: HashSet<(i64, i64)> = all.iter().map(|&(x, _, z, _)| (x, z)).collect();
    assert_eq!(columns.len() as f64, (48.0 / UNIT_SIZE) * (80.0 / UNIT_SIZE));

    // Trees between 10 m and 25 m tall: the longest vertical run of wood in any column (its sites are every other y).
    let mut wood: Vec<(i64, i64, i64)> = all.iter().filter(|v| v.3 == 4).map(|&(x, y, z, _)| (x, z, y)).collect();
    wood.sort();
    let (mut longest, mut run) = (0, 0);
    for (i, &(x, z, y)) in wood.iter().enumerate() {
        run = if i > 0 && wood[i - 1] == (x, z, y - 2) { run + 1 } else { 1 };
        longest = longest.max(run);
    }
    let tallest = 2.0 * longest as f64 * UNIT_SIZE;
    assert!(tallest > 10.0 && tallest < 25.0, "tallest run of wood {tallest} m");

    // The camera starts in open air with a clear view 4 m ahead.
    let [sx, sy, sz] = spawn;
    for dz in 0..(4.0 / UNIT_SIZE) as i64 {
        for dx in -1..=1 {
            for dy in -1..=1 {
                if is_site(sx + dx, sy + dy, sz + dz) {
                    assert_eq!(grid.get(sx + dx, sy + dy, sz + dz), 0, "a voxel at ({}, {}, {})", sx + dx, sy + dy, sz + dz);
                }
            }
        }
    }
}

#[test]
fn a_body_walks_the_path_through_its_ferns() {
    use bake::walk::{COLLISION_LEVEL, Ground, WalkInput, Walker};
    let world = forest(7);
    let level = COLLISION_LEVEL;
    let [x, z] = [world.spawn[0], world.spawn[2]].map(|v| v >> level);
    let (min_y, max_y) = world.heights_at(level);
    let area = Box3 { min: [2 * ((x - 32) / 2), min_y, z - 8], max: [x + 32, max_y, z + 128] };
    let tiles = fcc::Tiles::one(generate(&world.generator, &area, level).into_grid(true));
    let walk_through: Vec<bool> = materials_of(&world.generator).iter().map(|m| m.walk_through).collect();
    let ground = Ground { tiles: &tiles, walk_through: &walk_through };
    let mut body = Walker::at(world.spawn.map(|v| v as f64));
    let forward = WalkInput { forward: 1.0, ..WalkInput::default() };
    for _ in 0..6 * 60 {
        body = body.walk(forward, 1.0 / 60.0, &ground);
    }
    // At walking speed, 4.3 m/s, all the way: unstopped by the ferns by the path.
    let walked = (body.eye()[2] - world.spawn[2] as f64) * UNIT_SIZE;
    assert!(walked > 25.0, "walked {walked:.1} m");
}
