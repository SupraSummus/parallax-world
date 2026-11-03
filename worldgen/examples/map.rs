//! A world from above, to see its layout at a glance: each column's
//! topmost voxel, lighter the higher it is, +z up. Writes a PPM:
//!
//! cargo run --release --example map -- valley 42 2 -1600 -400 3200 4000 map.ppm
//!
//! for world, seed, level, the area's min x and z, width and depth (lattice
//! units), and the file.

use worldgen::forest::{forest, heights};
use worldgen::generator::{generate, materials_of};
use worldgen::grid::Box3;
use worldgen::valley::valley;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let number = |i: usize| args[i].parse::<i64>().expect("a number");
    let seed = number(1) as u64;
    let (generator, _) = if args[0] == "valley" { valley(seed) } else { forest(seed) };
    let level = number(2) as u32;
    let s = 1i64 << level;
    let (x0, z0, nx, nz) = (2 * (number(3) / s).div_euclid(2), number(4) / s, number(5) / s, number(6) / s);
    let colours: Vec<u32> = materials_of(&generator).iter().map(|m| m.rgb()).collect();
    let (min_y, max_y) = heights(level);
    let mut image = vec![0u8; (nx * nz * 3) as usize];
    // In tiles, as the web generates it.
    const TILE: i64 = 64;
    for tz in (0..nz).step_by(TILE as usize) {
        for tx in (0..nx).step_by(TILE as usize) {
            let b = Box3 { min: [x0 + tx, min_y, z0 + tz], max: [x0 + tx + TILE, max_y, z0 + tz + TILE] };
            let grid = generate(&generator, &b, level);
            for z in b.min[2]..b.max[2].min(z0 + nz) {
                for x in b.min[0]..b.max[0].min(x0 + nx) {
                    let Some((y, id)) =
                        (min_y..max_y).rev().filter(|y| (x + y + z) & 1 == 0).map(|y| (y, grid.get(x, y, z))).find(|(_, id)| *id != 0)
                    else {
                        continue;
                    };
                    let (c, light) = (colours[id as usize], 0.75 + 0.25 * (y - min_y) as f64 / (max_y - min_y) as f64);
                    let at = ((nz - 1 - (z - z0)) * nx + (x - x0)) as usize * 3;
                    for (i, shift) in [16, 8, 0].into_iter().enumerate() {
                        image[at + i] = (((c >> shift) & 255) as f64 * light) as u8;
                    }
                }
            }
        }
    }
    let mut out = format!("P6 {nx} {nz} 255\n").into_bytes();
    out.extend(image);
    std::fs::write(&args[7], out).expect("written");
}
