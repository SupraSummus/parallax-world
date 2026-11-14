//! No world's directions follow the lattice's (worldgen::checks::directions):
//! a test each world, its seeds on threads.

use worldgen::checks::directions::directions;
use worlds::WORLDS;

/** The seeds pairs are sampled at: the samples, so enough for the KNOWN to show. */
const SEEDS: &[u64] = &[42, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23];

/** Places known to follow the lattice's directions, by world (docs/todo.md); each must still, or come off. */
const KNOWN: &[(&str, &str)] = &[("valley", "town"), ("city", "spawn")];

/** Around no place of the world `name` its directions follow the lattice's, but those KNOWN; prints each place's strongest four-fold harmonic. */
fn turned(name: &str) {
    let entry = worlds::by_name(name).expect("a world of that name");
    let mut broken = Vec::new();
    for leak in directions(entry.make, SEEDS) {
        println!("{name}: {leak}");
        match (leak.broken(), KNOWN.contains(&(name, leak.place))) {
            (true, false) => broken.push(format!("{name}: {leak}")),
            (false, true) => broken.push(format!("{name}: around {} it follows the lattice no more: take it off KNOWN", leak.place)),
            _ => {}
        }
    }
    assert!(broken.is_empty(), "{}", broken.join("\n"));
}

/** A test per world named, and the names. */
macro_rules! worlds {
    ($($name:ident),*) => {
        mod turned {
            $(
                #[test]
                fn $name() {
                    super::turned(stringify!($name));
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
