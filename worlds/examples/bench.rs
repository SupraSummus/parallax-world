//! Times generating a world's tiles on one thread, with a hash of what they hold to check optimisations change nothing:
//!
//! cargo run --release --example bench -- valley 12345 0 0 [levels...]
//!
//! for world (the forest if none), seed, and where, off spawn along x and z (lattice units):
//! at each level given (0, 4 and 6 if none), four tiles (native's cubes of 128 sites, light::tile_box) side by side there, at the spawn's height,
//! what may show of them (shown).
use std::hash::{DefaultHasher, Hash, Hasher};
use std::time::Instant;
use worldgen::generator::{generate, shown};
use worldgen::light::{TILE, tile_box};
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
    let levels: Vec<u32> = args.iter().skip(4).map(|a| a.parse().expect("a level")).collect();
    for level in if levels.is_empty() { vec![0, 4, 6] } else { levels } {
        let s = 1i64 << level;
        let heights = world.heights_at(level);
        let [cx, cy, cz] = [x, spawn[1], z].map(|v| v.div_euclid(s * TILE));
        let start = Instant::now();
        let (mut voxels, mut hash) = (0, DefaultHasher::new());
        for i in 0..4 {
            let b = shown(generator, &tile_box(heights, [cx + i % 2, cy, cz + i / 2]), level);
            let grid = generate(generator, &b, level).into_grid(false);
            voxels += grid.cells.len();
            (&grid.bricks, &grid.cells, &grid.solid, &grid.views, &grid.glow).hash(&mut hash);
        }
        println!(
            "level {level}: 4 tiles in {:.1} ms ({voxels} bytes of bricks, hash {:016x})",
            start.elapsed().as_secs_f64() * 1000.0,
            hash.finish()
        );
    }
}
