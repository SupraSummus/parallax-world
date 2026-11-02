//! Times generating forest tiles on one thread: parallax-native --check-grids without the dump.
use std::time::Instant;
use worldgen::forest::{forest, heights};
use worldgen::generator::generate;
use worldgen::grid::Box3;

fn main() {
    let start = Instant::now();
    let (generator, spawn) = forest(12345.0);
    println!("forest() in {:.1} ms", start.elapsed().as_secs_f64() * 1000.0);
    for level in [0u32, 4] {
        let s = (1i64 << level) as f64;
        let (min_y, max_y) = heights(level);
        let (cx, cz) = ((spawn[0] / s / 128.0).floor() * 128.0, (spawn[2] / s / 128.0).floor() * 128.0);
        let start = Instant::now();
        let mut voxels = 0;
        for i in 0..4 {
            let b = Box3 {
                min_x: cx + 128.0 * (i % 2) as f64,
                min_y,
                min_z: cz + 128.0 * (i / 2) as f64,
                max_x: cx + 128.0 * (i % 2 + 1) as f64,
                max_y,
                max_z: cz + 128.0 * (i / 2 + 1) as f64,
            };
            voxels += generate(&generator, &b, level).cells.len();
        }
        println!("level {level}: 4 tiles in {:.2} s ({voxels} bytes of bricks)", start.elapsed().as_secs_f64());
    }
}
