//! The forest (forest.test.ts): determinism, contents, spawn.

use std::collections::HashSet;

use worldgen::forest::{UNIT_SIZE, forest, heights};
use worldgen::generator::{generate, materials_of};
use worldgen::grid::{Box3, VoxelGrid, is_site};

/** 48 m × 80 m ahead of the spawn point, all heights, and the spawn point. */
fn generated(seed: f64) -> (VoxelGrid, Vec<u8>, [f64; 3]) {
    let (generator, spawn) = forest(seed);
    let (min_y, max_y) = heights(0);
    // Grids start on an even x.
    let x = 2.0 * ((spawn[0] - 24.0 / UNIT_SIZE) / 2.0).floor();
    let area =
        Box3 { min_x: x, min_y, min_z: spawn[2] - 1.0 / UNIT_SIZE, max_x: x + 48.0 / UNIT_SIZE, max_y, max_z: spawn[2] + 79.0 / UNIT_SIZE };
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
    let (grid, types, spawn) = generated(42.0);
    let all = voxels(&grid, &types);

    // Deterministic for a seed.
    let (again, _, again_spawn) = generated(42.0);
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
    let [sx, sy, sz] = spawn.map(|v| v as i64);
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
