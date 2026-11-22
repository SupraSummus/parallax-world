//! The contract for a component of a world (worldgen::checks::contract's
//! `check`, narrowed to it), to work on one part at a time:
//!
//! cargo run --release --example check -- valley towns/block/church
//! cargo run --release --example check -- moor steadings/hut levels 3 alone
//!
//! for world and component (`towns/block/church`), then `seed` (42),
//! `levels` (the world's) and `alone` (the world cut to it) optional.
//! Held to a snapshot by the binary, not in git:
//! CONTRACT_BLESS=1 takes one before a change, to see what it does.

use worldgen::checks::contract::{breaks, check, hold_to_baseline};
use worldgen::entry::SEED;
use worlds::{all, by_name};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let usage = "check <world> <component> [seed <n>] [levels <n>] [alone]";
    let (Some(world), Some(component)) = (args.first(), args.get(1)) else { panic!("{usage}") };
    let Some(entry) = by_name(world) else {
        let names: Vec<&str> = all().map(|e| e.name).collect();
        eprintln!("no world {world}; the worlds are {}", names.join(", "));
        std::process::exit(1);
    };
    let (mut seed, mut levels, mut alone) = (SEED, entry.levels, false);
    let mut words = args[2..].iter();
    while let Some(word) = words.next() {
        let mut number = || words.next().and_then(|n| n.parse::<u64>().ok()).unwrap_or_else(|| panic!("a number after {word}: {usage}"));
        match word.as_str() {
            "seed" => seed = number(),
            "levels" => levels = number() as u32,
            "alone" => alone = true,
            _ => panic!("{word}? {usage}"),
        }
    }
    let name = format!("{world}, {component}{}, seed {seed}", if alone { " alone" } else { "" });
    let made = (entry.make)(seed);
    let made = if alone { made.alone(component) } else { made };
    let checked = check(&[(&name, &made)], levels, &[component]);
    let snapshot = std::env::current_exe().expect("this example").with_file_name("contract").join(format!("{name}.txt"));
    std::fs::create_dir_all(snapshot.parent().expect("a directory")).expect("the snapshots' directory");
    hold_to_baseline(&breaks(&checked, entry.accuracy), snapshot.to_str().expect("a path"), true);
}
