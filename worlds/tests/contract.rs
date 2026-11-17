//! Every world keeps the contract (worldgen::checks::contract) in regions holding
//! everything it is made of around spawn, on average over several seeds,
//! and far away in its registry entry's far regions: a test each, so they
//! run in parallel, and a thread each seed, and for the parts and far regions.

use std::sync::mpsc;

use worldgen::checks::contract::{Break, breaks, check, check_far, hold_to_baseline, parts_equal_whole};
use worlds::{Entry, REGIONS_SEED, WORLDS};

/** The seeds each world is checked at: each stratum's margins are averaged over all of them. */
const SEEDS: &[u64] = &[REGIONS_SEED, 1, 2, 3];

/**
 * `entry`'s world keeps the contract (worldgen::checks::contract) at SEEDS,
 * in regions holding everything it is made of around spawn, each
 * stratum's margins averaged over all of them; generated in parts too at
 * REGIONS_SEED; and in its far regions at REGIONS_SEED. CONTRACT_ONLY,
 * names of strata separated by commas ("towns/church,railway"), checks
 * only regions holding them. Panics where it breaks it but as its
 * baseline, data/contract/<name>.txt, says.
 */
fn keeps_the_contract(entry: &Entry) {
    let only = std::env::var("CONTRACT_ONLY").unwrap_or_default();
    let only: Vec<&str> = only.split(',').map(str::trim).filter(|o| !o.is_empty()).collect();
    let only = &only;
    let (checked, panicked) = std::thread::scope(|scope| {
        // Parts at one seed: they check how a world is drawn, which seeds don't change. On a thread of their own, in the
        // regions that seed's thread finds, while it goes on with the rest.
        let (to_parts, regions) = mpsc::channel();
        let seeds: Vec<_> = SEEDS
            .iter()
            .map(|&seed| {
                let to_parts = (seed == REGIONS_SEED).then(|| to_parts.clone());
                // World and Generator aren't Send: each thread makes its own.
                scope.spawn(move || {
                    let world = (entry.make)(seed);
                    let surveyed = |boxes: &[_]| {
                        if let Some(to_parts) = to_parts {
                            to_parts.send(boxes.to_vec()).expect("the parts' thread waiting");
                        }
                    };
                    check(&format!("{} (seed {seed})", entry.name), &world, entry.levels, only, surveyed)
                })
            })
            .collect();
        drop(to_parts);
        // No regions if their seed's thread panicked before it found them: that thread says why.
        let parts = scope.spawn(move || {
            if let Ok(boxes) = regions.recv() {
                let name = format!("{} (seed {REGIONS_SEED})", entry.name);
                parts_equal_whole(&name, &(entry.make)(REGIONS_SEED), &boxes, entry.levels);
            }
        });
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
        if let Err(panic) = parts.join() {
            panicked.push(format!("parts: {}", message(panic)));
        }
        if let Some(Err(panic)) = far.map(|t| t.join()) {
            panicked.push(format!("far: {}", message(panic)));
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
