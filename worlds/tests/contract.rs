//! Every world keeps the contract (worldgen::checks::contract) in regions holding
//! everything it is made of around spawn, on average over several seeds,
//! and far away in its registry entry's far regions: a test each, so they
//! run in parallel, and a thread each seed.

use worldgen::checks::contract::{check, check_far, strata_kept};
use worlds::{Entry, REGIONS_SEED, WORLDS};

/** The seeds each world is checked at: each stratum's margins are averaged over all of them. */
const SEEDS: &[u64] = &[REGIONS_SEED, 1, 2, 3];

/**
 * `entry`'s world keeps the contract (worldgen::checks::contract) at SEEDS,
 * in regions holding everything it is made of around spawn, each
 * stratum's margins averaged over all of them; generated in parts too at
 * REGIONS_SEED; and in its far regions at REGIONS_SEED. CONTRACT_ONLY,
 * names of strata separated by commas ("towns › church,railway"), checks
 * only regions holding them. Panics with what breaks it.
 */
fn keeps_the_contract(entry: &Entry) {
    let only = std::env::var("CONTRACT_ONLY").unwrap_or_default();
    let only: Vec<&str> = only.split(',').map(str::trim).filter(|o| !o.is_empty()).collect();
    let only = &only;
    let (checked, panicked) = std::thread::scope(|scope| {
        let seeds: Vec<_> = SEEDS
            .iter()
            .map(|&seed| {
                // World and Generator aren't Send: each thread makes its own.
                scope.spawn(move || {
                    let world = (entry.make)(seed);
                    // Parts at one seed: they check how a world is drawn, which seeds don't change.
                    check(&format!("{} (seed {seed})", entry.name), &world, entry.levels, entry.from_above, seed == REGIONS_SEED, only)
                })
            })
            .collect();
        // Far regions at one seed too: they check how fields are read far away.
        let far = (!entry.far.is_empty() && only.is_empty()).then(|| {
            scope.spawn(move || {
                let world = (entry.make)(REGIONS_SEED);
                let far: Vec<_> = entry.far.iter().map(|r| r.of(&world)).collect();
                check_far(&format!("{} (seed {REGIONS_SEED})", entry.name), world, &far, entry.far_levels)
            })
        });
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
        if let Some(Err(panic)) = far.map(|t| t.join()) {
            panicked.push(format!("far: {}", message(panic)));
        }
        (checked, panicked)
    });
    let broken: Vec<String> = panicked.into_iter().chain(strata_kept(&checked)).collect();
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

worlds!(forest, valley, moor, fog, learned, city);

#[test]
fn every_world_is_tested() {
    let names: Vec<&str> = WORLDS.iter().map(|e| e.name).collect();
    assert_eq!(names, TESTED, "add the world to worlds! above");
}
