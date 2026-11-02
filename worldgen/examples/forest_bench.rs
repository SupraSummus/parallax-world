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
        let s = 1i64 << level;
        let (min_y, max_y) = heights(level);
        let (cx, cz) = (spawn[0].div_euclid(s * 128) * 128, spawn[2].div_euclid(s * 128) * 128);
        let start = Instant::now();
        let mut voxels = 0;
        for i in 0..4 {
            let (x, z) = (cx + 128 * (i % 2), cz + 128 * (i / 2));
            voxels += generate(&generator, &Box3 { min: [x, min_y, z], max: [x + 128, max_y, z + 128] }, level).cells.len();
        }
        println!("level {level}: 4 tiles in {:.2} s ({voxels} bytes of bricks)", start.elapsed().as_secs_f64());
    }
}
