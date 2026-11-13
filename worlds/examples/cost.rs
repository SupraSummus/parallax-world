//! What generating a world's host tiles costs, by layer and level
//! (worldgen::cost), with what looks costly flagged:
//!
//! cargo run --release --example cost -- valley
//! cargo run --release --example cost -- moor 7 levels 0 1 2 3 at 0 0 0 at 512 0 512 all
//!
//! for world, then optionally seed (REGIONS_SEED), `levels` (all a host makes), places to
//! look at with `at` (lattice units; the spawn and the centres of the
//! world's regions there, around the places it names), and `all` for
//! every count of every tile.

use worldgen::cost::{places, report};
use worlds::{REGIONS_SEED, WORLDS, by_name};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(entry) = args.first().and_then(|name| by_name(name)) else {
        let names: Vec<&str> = WORLDS.iter().map(|e| e.name).collect();
        eprintln!("cost <world> [seed] [levels <l>...] [at <x y z>...] [all]; the worlds are {}", names.join(", "));
        std::process::exit(1);
    };
    let number = |i: usize| args.get(i).and_then(|a| a.parse::<i64>().ok());
    let seed = number(1).map_or(REGIONS_SEED, |n| n as u64);
    let (mut levels, mut at, mut all) = (Vec::new(), Vec::new(), false);
    let mut i = 1 + number(1).is_some() as usize;
    while i < args.len() {
        match args[i].as_str() {
            "levels" => {
                while let Some(level) = number(i + 1) {
                    levels.push(level as u32);
                    i += 1;
                }
            }
            "at" => {
                let p = [1, 2, 3].map(|k| number(i + k).expect("at x y z"));
                at.push((format!("{},{},{}", p[0], p[1], p[2]), p));
                i += 3;
            }
            "all" => all = true,
            other => panic!("{other}?"),
        }
        i += 1;
    }
    if levels.is_empty() {
        levels = (0..fcc::GRID_LEVELS as u32).collect();
    }
    if at.is_empty() {
        let world = (entry.make)(seed);
        let regions: Vec<_> = entry.regions.iter().map(|r| r.of(&world)).collect();
        at = places(&world, &regions);
    }
    let report = report(&format!("{} (seed {seed})", entry.name), &|| (entry.make)(seed), &at, &levels);
    print!("{report}");
    if all {
        print!("{}", report.baseline());
    }
}
