//! The contract for a component of a world (worldgen::checks::contract's
//! `check_component`), to work on one part at a time:
//!
//! cargo run --release --example check -- valley towns/block/church
//! cargo run --release --example check -- moor steadings/hut places 4 levels 3 alone
//!
//! for world and component (`towns/block/church`), then `seed` (42),
//! `places` (8), `levels` (the world's) and `alone` (the world cut to it)
//! optional. Held to a snapshot by the binary, not in git:
//! CONTRACT_BLESS=1 takes one before a change, to see what it does.

use worldgen::checks::contract::{breaks, check_component, covers, hold_to_baseline};
use worlds::{REGIONS_SEED, WORLDS, by_name};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let usage = "check <world> <component> [seed <n>] [places <n>] [levels <n>] [alone]";
    let (Some(world), Some(component)) = (args.first(), args.get(1)) else { panic!("{usage}") };
    let Some(entry) = by_name(world) else {
        let names: Vec<&str> = WORLDS.iter().map(|e| e.name).collect();
        eprintln!("no world {world}; the worlds are {}", names.join(", "));
        std::process::exit(1);
    };
    let (mut seed, mut places, mut levels, mut alone) = (REGIONS_SEED, 8, entry.levels, false);
    let mut words = args[2..].iter();
    while let Some(word) = words.next() {
        let mut number = || words.next().and_then(|n| n.parse::<u64>().ok()).unwrap_or_else(|| panic!("a number after {word}: {usage}"));
        match word.as_str() {
            "seed" => seed = number(),
            "places" => places = number() as usize,
            "levels" => levels = number() as u32,
            "alone" => alone = true,
            _ => panic!("{word}? {usage}"),
        }
    }
    let name = format!("{world}, {component}{}, seed {seed}, {places} places", if alone { " alone" } else { "" });
    let make = || if alone { (entry.make)(seed).alone(component) } else { (entry.make)(seed) };
    let checked = check_component(&name, &make, component, places, levels);
    // What is declared for it, or for what it is in.
    let declared: Vec<_> = entry.accuracy.iter().filter(|(d, _)| covers(d, component)).copied().collect();
    let snapshot = std::env::current_exe().expect("this example").with_file_name("contract").join(format!("{name}.txt"));
    std::fs::create_dir_all(snapshot.parent().expect("a directory")).expect("the snapshots' directory");
    hold_to_baseline(&breaks(&[checked], &declared), snapshot.to_str().expect("a path"), true);
}
