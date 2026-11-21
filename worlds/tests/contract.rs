//! Every world keeps the contract (worldgen::checks::contract) in regions holding
//! everything it is made of around spawn, on average over several seeds:
//! a test each, so they run in parallel, and a thread each seed.

use worldgen::checks::contract::{Break, breaks, check, hold_to_baseline};
use worlds::{Entry, REGIONS_SEED, WORLDS};

/** The seeds each world is checked at: each stratum's margins are averaged over all of them. */
const SEEDS: &[u64] = &[REGIONS_SEED, 1, 2, 3];

/**
 * `entry`'s world keeps the contract (worldgen::checks::contract) at SEEDS,
 * in regions holding everything it is made of around spawn, each
 * stratum's margins averaged over all of them. CONTRACT_ONLY, names of strata separated by commas
 * ("towns/church,railway"), checks only regions holding them. Panics
 * where it breaks it but as its baseline, data/contract/<name>.txt, says.
 */
fn keeps_the_contract(entry: &Entry) {
    let only = std::env::var("CONTRACT_ONLY").unwrap_or_default();
    let only: Vec<&str> = only.split(',').map(str::trim).filter(|o| !o.is_empty()).collect();
    let only = &only;
    let (checked, panicked) = std::thread::scope(|scope| {
        // Each seed is a world of its own.
        let seeds: Vec<_> = SEEDS
            .iter()
            .map(|&seed| scope.spawn(move || check(&format!("{} (seed {seed})", entry.name), &(entry.make)(seed), entry.levels, only)))
            .collect();
        let message = |panic: Box<dyn std::any::Any + Send>| {
            panic.downcast_ref::<String>().cloned().or_else(|| panic.downcast_ref::<&str>().map(|s| s.to_string())).unwrap_or_default()
        };
        let (mut checked, mut panicked) = (Vec::new(), Vec::new());
        for (seed, thread) in SEEDS.iter().zip(seeds) {
            match thread.join() {
                Ok(c) => checked.push(c),
                Err(panic) => panicked.push(format!("seed {seed}: {}", message(panic))),
            }
        }
        (checked, panicked)
    });
    // What panicked has no baseline: it always breaks.
    let mut breaks = breaks(&checked, entry.accuracy);
    breaks.extend(panicked.into_iter().map(|message| Break { of: None, message }));
    hold_to_baseline(&breaks, &format!("{}/data/contract/{}.txt", env!("CARGO_MANIFEST_DIR"), entry.name), only.is_empty());
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
