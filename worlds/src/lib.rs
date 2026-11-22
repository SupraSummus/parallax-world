//! The worlds, built on worldgen's engine with its public API only, and
//! the registry hosts start them from by name (docs/generators.md): those
//! here, and those in crates of their own (cheese/).

pub mod city;
pub mod forest;
pub mod learned;
pub mod path;
pub mod scene;
pub mod valley;

use worldgen::World;
use worldgen::checks::contract::Accuracy;
use worldgen::units::m;

/**
 * How deep a surface on ground of another kind lies (snow on rock):
 * as deep as level 4, the valley's coarsest checked, draws it.
 * Thinner, finer levels hold ground where level 4 holds the surface.
 */
pub const SURFACE: i64 = m(4.0);

/** A world hosts can start by name, with how far the contract checks it. */
pub struct Entry {
    pub name: &'static str,
    /** The world of a seed. */
    pub make: fn(u64) -> World,
    /** The level up to which its cells look as what is in them (worldgen::checks::contract). */
    pub levels: u32,
    /**
     * How closely its strata do, on average, by name: a thing's or a
     * layer's covers its parts (Accuracy::of); DEFAULT for the rest.
     */
    pub accuracy: &'static [(&'static str, Accuracy)],
}

/** The seed worlds are checked at first, and their cost measured at alone. */
pub const SEED: u64 = 42;

/** Every world. */
pub const WORLDS: &[Entry] = &[
    Entry { name: "forest", make: forest::forest, levels: 6, accuracy: &[] },
    Entry { name: "valley", make: valley::valley, levels: 4, accuracy: &[] },
    Entry { name: "moor", make: valley::moor::moor, levels: 5, accuracy: &[] },
    // The moor in fog (moor.rs's FOG), mist over all of it: where see-through voxels cost a host most.
    Entry { name: "fog", make: valley::moor::fog, levels: 5, accuracy: &[] },
    Entry { name: "learned", make: learned::learned, levels: 4, accuracy: &[] },
    // To level 3: at level 4, sites 2 m apart, a building's measured outline (city.rs, INSIDE) is kept or lost a site at a time.
    Entry { name: "city", make: city::city, levels: 3, accuracy: &[] },
    // To level 3: from level 4 on, caves have no pillars, stairs or crystals (cheese/src/lib.rs, SIZE).
    Entry { name: "cheese", make: cheese::cheese, levels: 3, accuracy: &[] },
    #[cfg(feature = "sponza")]
    Entry { name: "sponza", make: sponza, levels: 5, accuracy: &[] },
];

/** The world named `name`. */
pub fn by_name(name: &str) -> Option<&'static Entry> {
    WORLDS.iter().find(|e| e.name == name)
}

/** Crytek's Sponza at twice its size (docs/generators.md, "Scenes"); the same at every seed. */
#[cfg(feature = "sponza")]
fn sponza(_seed: u64) -> World {
    // The eye 1.75 m over its floor, which is a metre over its lowest point (a drain's), both scaled.
    scene::Scene::read(include_bytes!("../data/scenes/sponza.bin")).world(2.0 * 2.75)
}
