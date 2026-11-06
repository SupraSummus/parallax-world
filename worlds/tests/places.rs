//! No world's things prefer places on the grids it is generated on
//! (worldgen::places): a test each world, its seeds on threads.

use worldgen::places::preferences;
use worlds::WORLDS;

/** The seeds things are gathered at. */
const SEEDS: &[u64] = &[42, 1, 2, 3, 4, 5, 6, 7];

/** Populations known to prefer places, by world (docs/todo.md); each must still, or come off. */
const KNOWN: &[(&str, &str)] =
    &[("valley", "towns"), ("valley", "landmarks"), ("valley", "steadings"), ("moor", "steadings"), ("learned", "settlements")];

/** The things of each population of the world `name` prefer no places, but those KNOWN; prints each one's strongest preference. */
fn even(name: &str) {
    let entry = worlds::by_name(name).expect("a world of that name");
    let mut broken = Vec::new();
    for p in preferences(entry.make, SEEDS) {
        println!("{name}: {p}");
        match (p.broken(), KNOWN.contains(&(name, p.population))) {
            (true, false) => broken.push(format!("{name}: {p}")),
            (false, true) => broken.push(format!("{name}: {} prefers no places now: take it off KNOWN", p.population)),
            _ => {}
        }
    }
    assert!(broken.is_empty(), "{}", broken.join("\n"));
}

/** A test per world named, and the names. */
macro_rules! worlds {
    ($($name:ident),*) => {
        mod even {
            $(
                #[test]
                fn $name() {
                    super::even(stringify!($name));
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
