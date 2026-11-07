//! Every world keeps the contract (worldgen::contract) in its registry
//! entry's regions, at several seeds, and far away in its far ones: a
//! test each, so they run in parallel, and a thread each seed.

use worldgen::World;
use worldgen::contract::{check, check_content, check_far, heights_hold, margins};
use worldgen::grid::Box3;
use worlds::{Entry, REGIONS_SEED, WORLDS};

/** The seeds each world is checked at. */
const SEEDS: &[u64] = &[REGIONS_SEED, 1, 2, 3];

/**
 * Regions where a world is known to break the contract at a seed, by
 * world, seed and the place they are around: checked apart, they must
 * still break, so the list only shrinks; each is planned work in
 * docs/todo.md. Empty now, but a change that moves a place can land on
 * a seed where it breaks (the windmills' clearings from above at level
 * 4 vary by seed: "The canopy is the trees expected" there).
 */
const KNOWN_BREAKS: &[(&str, u64, &str)] = &[];

/**
 * `entry`'s world keeps the contract at each of SEEDS, in its regions
 * around the places they name there but KNOWN_BREAKS, which break it
 * still, and in its far regions at REGIONS_SEED; panics with each seed's
 * breaks.
 */
fn keeps_the_contract(entry: &Entry) {
    let broken: Vec<String> = std::thread::scope(|scope| {
        let mut threads: Vec<_> = SEEDS
            .iter()
            .map(|&seed| {
                // World and Generator aren't Send: each thread makes its own.
                let thread = scope.spawn(move || {
                    let world = (entry.make)(seed);
                    let (known, rest): (Vec<_>, Vec<_>) =
                        entry.regions.iter().partition(|r| KNOWN_BREAKS.contains(&(entry.name, seed, r.at)));
                    let regions: Vec<_> = rest.iter().map(|r| r.of(&world)).collect();
                    let name = format!("{} (seed {seed})", entry.name);
                    // Parts at one seed: they check how a world is drawn, which seeds don't change.
                    if seed == REGIONS_SEED {
                        check(&name, &world, &regions, entry.levels, entry.from_above)
                    } else {
                        check_content(&name, &world, &regions, entry.levels, entry.from_above)
                    }
                    let kept: Vec<&str> = known.iter().filter(|r| keeps(&name, &world, entry, r.of(&world))).map(|r| r.at).collect();
                    assert!(kept.is_empty(), "{name} keeps the contract around {}: remove it from KNOWN_BREAKS", kept.join(", "));
                });
                (format!("seed {seed}"), thread)
            })
            .collect();
        // Far regions at one seed too: they check how fields are read far away.
        if !entry.far.is_empty() {
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

/** Whether `world` keeps the contract's margins and heights in `region`, as `entry` asks. */
fn keeps(name: &str, world: &World, entry: &Entry, region: Box3) -> bool {
    let margins = margins(name, world, &[region], entry.levels, entry.from_above);
    margins.broken().margins.is_empty() && heights_hold(world, &[region], entry.levels).is_empty()
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
