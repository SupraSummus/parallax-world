//! The worlds, built on worldgen's engine with its public API only, and
//! the registry hosts start them from by name (docs/generators.md). The
//! native frontend links them; the web generates them as WebAssembly
//! (wasm.rs, src/lib/worldgen.ts).

pub mod fields;
pub mod forest;
pub mod town;
pub mod valley;
pub mod wasm;

use worldgen::World;
use worldgen::grid::Box3;

/** A world hosts can start by name, with the region the contract checks it in. */
pub struct Entry {
    pub name: &'static str,
    /** The world of a seed. */
    pub make: fn(u64) -> World,
    /** A region with a bit of everything in it, lattice units (see worldgen::contract::check). */
    pub region: Box3,
    /** The level up to which it keeps what a camera sees. */
    pub levels: u32,
}

/** Every world; their indices are the web's (worldgen_start). */
pub const WORLDS: &[Entry] = &[
    // Near the spawn point: path, trees, undergrowth, from under the ground to above the crowns.
    Entry { name: "forest", make: forest::forest, region: Box3 { min: [-128, -64, -384], max: [128, 384, -128] }, levels: 6 },
    // The first town's south edge: fields, its first row of houses and the main street, trees of its yards and of the forest.
    Entry { name: "valley", make: valley::valley, region: Box3 { min: [-128, -48, 760], max: [256, 208, 1016] }, levels: 5 },
];

/** The world named `name`. */
pub fn by_name(name: &str) -> Option<&'static Entry> {
    WORLDS.iter().find(|e| e.name == name)
}
