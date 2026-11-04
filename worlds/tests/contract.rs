//! Every world keeps the contract (worldgen::contract) in its registry
//! entry's regions: a test each, so they run in parallel.

use worlds::WORLDS;

/** A test per world named, keeps_the_contract::<name>, and the names. */
macro_rules! worlds {
    ($($name:ident),*) => {
        mod keeps_the_contract {
            $(
                #[test]
                fn $name() {
                    let entry = worlds::by_name(stringify!($name)).expect("a world of that name");
                    worldgen::contract::check(entry.name, &(entry.make)(42), entry.regions, entry.levels);
                }
            )*
        }
        const TESTED: &[&str] = &[$(stringify!($name)),*];
    };
}

worlds!(forest, valley);

#[test]
fn every_world_is_tested() {
    let names: Vec<&str> = WORLDS.iter().map(|e| e.name).collect();
    assert_eq!(names, TESTED, "add the world to worlds! above");
}
