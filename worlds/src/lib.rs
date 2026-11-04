//! The worlds, built on worldgen's engine with its public API only, and
//! the registry hosts start them from by name (docs/generators.md). The
//! native frontend links them; the web generates them as WebAssembly
//! (wasm.rs, src/lib/worldgen.ts).

pub mod fields;
pub mod forest;
pub mod learned;
pub mod moor;
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
    /**
     * Regions with, between them, a bit of everything in it at REGIONS_SEED, lattice units
     * (see worldgen::contract::check); at other seeds they follow its surface up or down.
     */
    pub regions: &'static [Box3],
    /** The level up to which it keeps what a camera sees. */
    pub levels: u32,
    /**
     * Wide regions whose fields are seen from above at coarse levels
     * (worldgen::contract::check_far); only their x and z count.
     */
    pub far: &'static [Box3],
    /** The level up to which it keeps what is seen from above, against level 0: at most `levels`. */
    pub from_above: u32,
}

/** The seed a world's regions are chosen at. */
pub const REGIONS_SEED: u64 = 42;

/** Every world; the web lists them by itself (worldgen_worlds) and starts them by index (worldgen_start). */
pub const WORLDS: &[Entry] = &[
    // Near the spawn point: path, trees, undergrowth, from under the ground to above the crowns;
    // seen from above at levels 5 and 6, 64 and 128 m of forest around it.
    Entry {
        name: "forest",
        make: forest::forest,
        regions: &[Box3 { min: [-128, -64, -384], max: [128, 384, -128] }],
        levels: 6,
        from_above: 6,
        far: &[],
    },
    // The first town's south edge: fields, its first row of houses and the main street, trees of its yards and of the forest;
    // and its centre: houses back to back, a cross street, the square with its fountain and church.
    // Far away, all of that town and a few metres of fields around it: its streets and paving are finer than the sites there.
    Entry {
        name: "valley",
        make: valley::valley,
        regions: &[Box3 { min: [-128, -48, 760], max: [256, 208, 1016] }, Box3 { min: [-64, -48, 1408], max: [320, 288, 1856] }],
        levels: 4,
        // At level 4, sites 2 m apart, a house's edges, of one kind seen from any side, grow up to half a site
        // and show walls from above, 0.11 off at the town's edge ("Media are isotropic" in docs/generators.md).
        from_above: 3,
        far: &[Box3 { min: [-704, -48, 864], max: [704, 288, 2336] }],
    },
    // The first steading: its hut, the cairns and the ring north of it, heather and grass, from under the ground to over the roof.
    // Far away, 175 m around it: the stream, narrower than the sites there, and patches of heather and grass.
    Entry {
        name: "moor",
        make: moor::moor,
        regions: &[Box3 { min: [-64, -32, -192], max: [192, 64, 192] }],
        levels: 5,
        from_above: 3,
        far: &[Box3 { min: [-640, -32, -640], max: [768, 64, 768] }],
    },
    // The first settlement's main street where a cross street meets it: houses facing both, yards, trees, paving.
    // Far away, all of its crop: streets, paving and yards, fields beside them, the forest round it.
    Entry {
        name: "learned",
        make: learned::learned,
        regions: &[Box3 { min: [64, -48, 1984], max: [448, 208, 2368] }],
        levels: 4,
        from_above: 3,
        far: &[Box3 { min: [-1536, -48, 544], max: [1536, 288, 3616] }],
    },
];

/** The world named `name`. */
pub fn by_name(name: &str) -> Option<&'static Entry> {
    WORLDS.iter().find(|e| e.name == name)
}
