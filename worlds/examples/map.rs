//! A world on a ground from above, to see its layout at a glance: each
//! column's topmost voxel as its faces up show it, lighter the higher it
//! is, +z up. Over all its heights: a world with none (the cheese) has no top.
//! Writes a PPM:
//!
//! cargo run --release --example map -- valley 42 2 -1600 -400 3200 4000 map.ppm
//!
//! for world (by name, as in worlds::all), seed, level, the area's min x and z, width and depth (lattice
//! units), and the file.

use lattice::light::{linear_to_srgb, srgb_to_linear};
use worldgen::generator::generate;
use worldgen::grid::{Box3, VOXEL_FACES};
use worlds::{all, by_name};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let number = |i: usize| args[i].parse::<i64>().expect("a number");
    let seed = number(1) as u64;
    let Some(entry) = by_name(&args[0]) else {
        let names: Vec<&str> = all().map(|e| e.name).collect();
        eprintln!("no world {}; the worlds are {}", args[0], names.join(", "));
        std::process::exit(1);
    };
    let world = (entry.make)(seed);
    if world.heights.is_none() {
        eprintln!("{} has no heights, so no top to be seen from", args[0]);
        std::process::exit(1);
    }
    let generator = &world.generator;
    let level = number(2) as u32;
    let s = 1i64 << level;
    let (x0, z0, nx, nz) = (2 * (number(3) / s).div_euclid(2), number(4) / s, number(5) / s, number(6) / s);
    let (min_y, max_y) = world.heights_at(level);
    let mut image = vec![0u8; (nx * nz * 3) as usize];
    // A ray straight down sees a voxel through its four faces up alike (lattice's `facing`): their mean.
    let up: Vec<usize> = (0..VOXEL_FACES.len()).filter(|&k| VOXEL_FACES[k][1] > 0).collect();
    // In tiles, as hosts generate it.
    const TILE: i64 = 64;
    for tz in (0..nz).step_by(TILE as usize) {
        for tx in (0..nx).step_by(TILE as usize) {
            let b = Box3 { min: [x0 + tx, min_y, z0 + tz], max: [x0 + tx + TILE, max_y, z0 + tz + TILE] };
            let grid = generate(generator, &b, level);
            for z in b.min[2]..b.max[2].min(z0 + nz) {
                for x in b.min[0]..b.max[0].min(x0 + nx) {
                    let Some(y) = (min_y..max_y).rev().find(|&y| grid.get(x, y, z) != 0) else {
                        continue;
                    };
                    let light = 0.75 + 0.25 * (y - min_y) as f64 / (max_y - min_y) as f64;
                    let at = ((nz - 1 - (z - z0)) * nx + (x - x0)) as usize * 3;
                    for (i, shift) in [0, 8, 16].into_iter().enumerate() {
                        let channel = |k: usize| srgb_to_linear(((grid.face(x, y, z, k) >> shift) & 255) as f32 / 255.0);
                        let mean = up.iter().map(|&k| channel(k)).sum::<f32>() / up.len() as f32;
                        image[at + i] = (255.0 * linear_to_srgb(mean) as f64 * light) as u8;
                    }
                }
            }
        }
    }
    let mut out = format!("P6 {nx} {nz} 255\n").into_bytes();
    out.extend(image);
    std::fs::write(&args[7], out).expect("written");
}
