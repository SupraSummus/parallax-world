//! Times generating a world's tiles on one thread, with a hash of what they hold to check optimisations change nothing:
//!
//! cargo run --release --example bench -- valley 12345 0 0
//!
//! for world (the forest if none), seed, and where, off spawn along x and z (lattice units):
//! at levels 0, 4 and 6, the four tiles of 128 sites around there, over the world's heights.
use std::hash::{DefaultHasher, Hash, Hasher};
use std::time::Instant;
use worldgen::generator::generate;
use worldgen::grid::Box3;
use worlds::by_name;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let number = |i: usize, default: i64| args.get(i).map_or(default, |a| a.parse().expect("a number"));
    let entry = by_name(args.first().map_or("forest", |a| a.as_str())).expect("a world of that name");
    let start = Instant::now();
    let world = (entry.make)(number(1, 12345) as u64);
    let (generator, spawn) = (&world.generator, world.spawn);
    println!("{}() in {:.1} ms", entry.name, start.elapsed().as_secs_f64() * 1000.0);
    let (x, z) = (spawn[0] + number(2, 0), spawn[2] + number(3, 0));
    for level in [0u32, 4, 6] {
        let s = 1i64 << level;
        let (min_y, max_y) = world.heights_at(level);
        let (cx, cz) = (x.div_euclid(s * 128) * 128, z.div_euclid(s * 128) * 128);
        let start = Instant::now();
        let (mut voxels, mut hash) = (0, DefaultHasher::new());
        for i in 0..4 {
            let (x, z) = (cx + 128 * (i % 2), cz + 128 * (i / 2));
            let grid = generate(generator, &Box3 { min: [x, min_y, z], max: [x + 128, max_y, z + 128] }, level).into_grid(false);
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
