//! Every world keeps the contract (worldgen::contract) in its registry
//! entry's regions, at several seeds, and far away in its far ones: a
//! test each, so they run in parallel, and a thread each seed.

use worldgen::contract::{check, check_content, check_far, follow_surface};
use worlds::{Entry, REGIONS_SEED, WORLDS};

/** The seeds each world is checked at, but those that name their own. */
const SEEDS: &[u64] = &[REGIONS_SEED, 1, 2, 3];

/**
 * `entry`'s world keeps the contract at each of `seeds`, its regions
 * following its surface from REGIONS_SEED's, and in its far regions at
 * REGIONS_SEED; panics with each seed's breaks.
 */
fn keeps_the_contract(entry: &Entry, seeds: &[u64]) {
    let broken: Vec<String> = std::thread::scope(|scope| {
        let mut threads: Vec<_> = seeds
            .iter()
            .map(|&seed| {
                // World and Generator aren't Send: each thread makes its own.
                let thread = scope.spawn(move || {
                    let world = (entry.make)(seed);
                    let regions = follow_surface(entry.regions, &(entry.make)(REGIONS_SEED), &world);
                    let name = format!("{} (seed {seed})", entry.name);
                    // Parts at one seed: they check how a world is drawn, which seeds don't change.
                    if seed == REGIONS_SEED {
                        check(&name, &world, &regions, entry.levels)
                    } else {
                        check_content(&name, &world, &regions, entry.levels)
                    }
                });
                (format!("seed {seed}"), thread)
            })
            .collect();
        // Far regions at one seed too: they check how fields are read far away.
        if !entry.far.is_empty() {
            let name = format!("{} (seed {REGIONS_SEED})", entry.name);
            threads.push(("far".into(), scope.spawn(move || check_far(&name, (entry.make)(REGIONS_SEED), entry.far))));
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

/**
 * A test per world named, keeps_the_contract::<name>, and the names; a
 * world whose regions hold what they should at some seeds only names
 * them: `name = [seeds]`.
 */
macro_rules! worlds {
    ($($name:ident $(= [$($seed:expr),*])?),*) => {
        mod keeps_the_contract {
            $(
                #[test]
                fn $name() {
                    let entry = worlds::by_name(stringify!($name)).expect("a world of that name");
                    let seeds: &[u64] = worlds!(@seeds $($($seed),*)?);
                    super::keeps_the_contract(entry, seeds);
                }
            )*
        }
        const TESTED: &[&str] = &[$(stringify!($name)),*];
    };
    (@seeds) => { super::SEEDS };
    (@seeds $($seed:expr),+) => { &[$($seed),+] };
}

worlds!(forest, valley, moor);

#[test]
fn every_world_is_tested() {
    let names: Vec<&str> = WORLDS.iter().map(|e| e.name).collect();
    assert_eq!(names, TESTED, "add the world to worlds! above");
}
