//! Every world keeps the contract (worldgen::contract) in regions holding
//! everything it is made of around spawn, at several seeds, and far away
//! in its registry entry's far regions: a test each, so they run in
//! parallel, and a thread each seed.

use worldgen::contract::{check, check_far};
use worlds::{Entry, REGIONS_SEED, WORLDS};

/** The seeds each world is checked at. */
const SEEDS: &[u64] = &[REGIONS_SEED, 1, 2, 3];

/**
 * Where a world is known to break the contract at a seed: in regions
 * holding a stratum whose name holds this (worldgen::survey). They must
 * still break, so the list only shrinks; each is planned work in
 * docs/todo.md.
 */
const KNOWN_BREAKS: &[(&str, u64, &str)] =
    &[("valley", 1, "landmarks sail"), ("valley", 2, "landmarks sail"), ("valley", 3, "landmarks sail")];

/**
 * `entry`'s world keeps the contract at each of SEEDS, in regions
 * holding everything it is made of around spawn (worldgen::contract::
 * regions), generated in parts too at REGIONS_SEED, but KNOWN_BREAKS;
 * and in its far regions at REGIONS_SEED. CONTRACT_ONLY, names of
 * strata separated by commas ("towns › church,railway"), checks only
 * regions holding them. Panics with each seed's breaks.
 */
fn keeps_the_contract(entry: &Entry) {
    let only = std::env::var("CONTRACT_ONLY").unwrap_or_default();
    let only: Vec<&str> = only.split(',').map(str::trim).filter(|o| !o.is_empty()).collect();
    let only = &only;
    let broken: Vec<String> = std::thread::scope(|scope| {
        let mut threads: Vec<_> = SEEDS
            .iter()
            .map(|&seed| {
                // World and Generator aren't Send: each thread makes its own.
                let thread = scope.spawn(move || {
                    let world = (entry.make)(seed);
                    // Parts at one seed: they check how a world is drawn, which seeds don't change.
                    let name = format!("{} (seed {seed})", entry.name);
                    let known: Vec<&str> = KNOWN_BREAKS.iter().filter(|k| (k.0, k.1) == (entry.name, seed)).map(|k| k.2).collect();
                    check(&name, &world, entry.levels, entry.from_above, seed == REGIONS_SEED, only).assert_kept(&known)
                });
                (format!("seed {seed}"), thread)
            })
            .collect();
        // Far regions at one seed too: they check how fields are read far away.
        if !entry.far.is_empty() && only.is_empty() {
            let name = format!("{} (seed {REGIONS_SEED})", entry.name);
            threads.push((
                "far".into(),
                scope.spawn(move || {
                    let world = (entry.make)(REGIONS_SEED);
                    let far: Vec<_> = entry.far.iter().map(|r| r.of(&world)).collect();
                    check_far(&name, world, &far)
                }),
            ));
        }
        threads
            .into_iter()
            .filter_map(|(what, thread)| {
                let panic = thread.join().err()?;
                let message = panic.downcast_ref::<String>().cloned().or_else(|| panic.downcast_ref::<&str>().map(|s| s.to_string()));
                Some(format!("{what}: {}", message.unwrap_or_default()))
            })
            .collect()
    });
    assert!(broken.is_empty(), "{}", broken.join("\n"));
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

worlds!(forest, valley, moor, learned, city);

#[test]
fn every_world_is_tested() {
    let names: Vec<&str> = WORLDS.iter().map(|e| e.name).collect();
    assert_eq!(names, TESTED, "add the world to worlds! above");
}
