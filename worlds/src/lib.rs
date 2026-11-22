//! The worlds, built on worldgen's engine with its public API only, and
//! the registry hosts start them from by name (docs/generators.md): those
//! here, and those in crates of their own (cheese/), each crate's WORLDS.

pub mod city;
pub mod forest;
pub mod learned;
pub mod path;
pub mod scene;
pub mod valley;

use worldgen::Entry;
#[cfg(feature = "sponza")]
use worldgen::World;
use worldgen::units::m;

/**
 * How deep a surface on ground of another kind lies (snow on rock):
 * as deep as level 4, the valley's coarsest checked, draws it.
 * Thinner, finer levels hold ground where level 4 holds the surface.
 */
pub const SURFACE: i64 = m(4.0);

/** The worlds of this crate (worldgen's entry.rs). */
pub const WORLDS: &[Entry] = &[
    forest::FOREST,
    valley::VALLEY,
    valley::moor::MOOR,
    valley::moor::FOG,
    learned::LEARNED,
    city::CITY,
    #[cfg(feature = "sponza")]
    Entry { name: "sponza", make: sponza, levels: 5, accuracy: &[], known: &[] },
];

/** Every world hosts start by name: each crate's WORLDS. */
pub fn all() -> impl Iterator<Item = &'static Entry> {
    [WORLDS, cheese::WORLDS].into_iter().flatten()
}

/** The world named `name`. */
pub fn by_name(name: &str) -> Option<&'static Entry> {
    all().find(|e| e.name == name)
}

/** Crytek's Sponza at twice its size (docs/generators.md, "Scenes"); the same at every seed. */
#[cfg(feature = "sponza")]
fn sponza(_seed: u64) -> World {
    // The eye 1.75 m over its floor, which is a metre over its lowest point (a drain's), both scaled.
    scene::Scene::read(include_bytes!("../data/scenes/sponza.bin")).world(2.0 * 2.75)
}
