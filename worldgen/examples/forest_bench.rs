//! Times generating forest tiles on one thread, with a hash of what they hold to check optimisations change nothing.
use std::hash::{DefaultHasher, Hash, Hasher};
use std::time::Instant;
use worldgen::forest::{forest, heights};
use worldgen::generator::generate;
use worldgen::grid::Box3;

fn main() {
    let start = Instant::now();
    let (generator, spawn) = forest(12345);
    println!("forest() in {:.1} ms", start.elapsed().as_secs_f64() * 1000.0);
    for level in [0u32, 4] {
        let s = 1i64 << level;
        let (min_y, max_y) = heights(level);
        let (cx, cz) = (spawn[0].div_euclid(s * 128) * 128, spawn[2].div_euclid(s * 128) * 128);
        let start = Instant::now();
        let (mut voxels, mut hash) = (0, DefaultHasher::new());
        for i in 0..4 {
            let (x, z) = (cx + 128 * (i % 2), cz + 128 * (i / 2));
            let grid = generate(&generator, &Box3 { min: [x, min_y, z], max: [x + 128, max_y, z + 128] }, level).into_grid();
            voxels += grid.cells.len();
            (&grid.bricks, &grid.cells).hash(&mut hash);
        }
        println!(
            "level {level}: 4 tiles in {:.2} s ({voxels} bytes of bricks, hash {:016x})",
            start.elapsed().as_secs_f64(),
            hash.finish()
        );
    }
}
