//! The places a world names at a seed (`World::places`: its spawn, a town,
//! a windmill…), each where it is, in lattice units and metres: where to
//! point `look`, `pnpm shot --camera` or the page's query.
//!
//! cargo run --release --example places -- valley 42
//!
//! for world (by name, as in worlds::all) and seed (42 if not given).

use worldgen::entry::SEED;
use worldgen::units::UNIT_SIZE;
use worlds::{all, by_name};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(entry) = args.first().and_then(|name| by_name(name)) else {
        let names: Vec<&str> = all().map(|e| e.name).collect();
        eprintln!("places <world> [seed]; the worlds are {}", names.join(", "));
        std::process::exit(1);
    };
    let seed = args.get(1).map_or(SEED, |s| s.parse().expect("a seed"));
    let world = (entry.make)(seed);
    for (name, p) in world.places() {
        let metres = p.map(|v| format!("{:.1}", v as f64 * UNIT_SIZE));
        println!("{name:>14}  {:>7} {:>6} {:>7}   ({} m)", p[0], p[1], p[2], metres.join(", "));
    }
}
