//! Every world keeps the contract (worldgen::checks::contract) in regions holding
//! everything it is made of around spawn, on average over several seeds:
//! a test each, so they run in parallel.

use worldgen::World;
use worldgen::checks::contract::{breaks, check, hold_to_baseline};
use worlds::{Entry, REGIONS_SEED, WORLDS};

/** The seeds each world is checked at, together: each stratum is checked where it is cheapest, its margins averaged over all of them. */
const SEEDS: &[u64] = &[REGIONS_SEED, 1, 2, 3];

/**
 * `entry`'s world keeps the contract (worldgen::checks::contract) at SEEDS,
 * in regions holding everything it is made of around spawn at any of them,
 * each stratum's margins averaged over all of them. CONTRACT_ONLY, names of strata separated by commas
 * ("towns/church,railway"), checks only regions holding them. Panics
 * where it breaks it but as its baseline, data/contract/<name>.txt, says.
 */
fn keeps_the_contract(entry: &Entry) {
    let only = std::env::var("CONTRACT_ONLY").unwrap_or_default();
    let only: Vec<&str> = only.split(',').map(str::trim).filter(|o| !o.is_empty()).collect();
    let worlds: Vec<(String, World)> = SEEDS.iter().map(|&seed| (format!("{} (seed {seed})", entry.name), (entry.make)(seed))).collect();
    let worlds: Vec<(&str, &World)> = worlds.iter().map(|(name, world)| (name.as_str(), world)).collect();
    let checked = check(&worlds, entry.levels, &only);
    let path = format!("{}/data/contract/{}.txt", env!("CARGO_MANIFEST_DIR"), entry.name);
    hold_to_baseline(&breaks(&checked, entry.accuracy), &path, only.is_empty());
}

/** A test per world named, keeps_the_contract::<name>, and the names. */
macro_rules! worlds {
    ($($name:ident),*) => {
        mod keeps_the_contract {
            $(
                #[test]
                fn $name() {
                    let entry = worlds::by_name(stringify!($name)).expect("a world of that name");
                    super::keeps_the_contract(entry);
                }
            )*
        }
        const TESTED: &[&str] = &[$(stringify!($name)),*];
    };
}

worlds!(forest, valley, moor, fog, learned, city, cheese);

#[test]
fn every_world_is_tested() {
    let names: Vec<&str> = WORLDS.iter().map(|e| e.name).collect();
    assert_eq!(names, TESTED, "add the world to worlds! above");
}
