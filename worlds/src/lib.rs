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
use worldgen::grid::Box3;
use worldgen::units::m;

/**
 * How deep a surface on ground of another kind lies (snow on rock):
 * as deep as level 4, the valley's coarsest checked, draws it.
 * Thinner, finer levels hold ground where level 4 holds the surface.
 */
pub const SURFACE: i64 = m(4.0);

/** A box around a place a world names (worldgen::World::point), lattice units from it: the same feature at any seed. */
pub struct Region {
    pub at: &'static str,
    pub min: [i64; 3],
    pub max: [i64; 3],
}

impl Region {
    const fn at(at: &'static str, min: [i64; 3], max: [i64; 3]) -> Region {
        Region { at, min, max }
    }

    /** Where it is in `world`. */
    pub fn of(&self, world: &World) -> Box3 {
        let p = world.point(self.at).unwrap_or_else(|| panic!("no point named {}", self.at));
        Box3 { min: [0, 1, 2].map(|i| p[i] + self.min[i]), max: [0, 1, 2].map(|i| p[i] + self.max[i]) }
    }
}

/** A world hosts can start by name, with how far the contract checks it, and the regions its costs are measured in. */
pub struct Entry {
    pub name: &'static str,
    /** The world of a seed. */
    pub make: fn(u64) -> World,
    /**
     * Regions with, between them, the costliest of it: where
     * worldgen::cost measures its tiles (tests/cost.rs). The contract
     * finds its own (worldgen::checks::contract::regions).
     */
    pub regions: &'static [Region],
    /** The level up to which its cells look as what is in them (worldgen::checks::contract). */
    pub levels: u32,
    /**
     * How closely its strata do, on average, by name: a thing's or a
     * layer's covers its parts (Accuracy::of); DEFAULT for the rest.
     */
    pub accuracy: &'static [(&'static str, Accuracy)],
    /**
     * Wide regions whose fields are seen from above at coarse levels
     * (worldgen::checks::contract::check_far); only their x and z count.
     */
    pub far: &'static [Region],
    /** The level up to which its far regions keep their fields' look from above, from level 4. */
    pub far_levels: u32,
}

/** The seed a world's regions were laid out at; its costs, parts and far regions are checked at it alone. */
pub const REGIONS_SEED: u64 = 42;

/** Every world. */
pub const WORLDS: &[Entry] = &[
    // Near the spawn point: path, trees, undergrowth, from under the ground to above the crowns.
    Entry {
        name: "forest",
        make: forest::forest,
        regions: &[Region::at("spawn", [-71, -115, -136], [185, 333, 120])],
        levels: 6,
        accuracy: &[],
        far: &[],
        far_levels: 6,
    },
    // The first town's south edge: fields, its first row of houses and the main street, trees of its yards and of the forest;
    // and its centre: houses back to back, a cross street, the square with its fountain and church.
    // Landmarks (landmarks.rs), from under the ground to over the crowns: the first chapel by the path ahead of the camera,
    // its churchyard and the forest round it; and the first windmill, its sails, fields and forest round it.
    // A mountainside behind a crest above the treeline: scree and rock, and alpine meadows among them at some seeds.
    // Moorland on the valley's floor, a steading by the path from 4.5 km on: heather and grass, boulders, its hut, path and ring.
    // The railway's first station: its building, the platform, the track and the formation either side, forest or fields round them.
    // Far away, all of that town and a few metres of fields around it: its streets and paving are finer than the sites there;
    // and the sea off the coast, from 16 m out: its shallows, where far sites are taller than the water is deep.
    Entry {
        name: "valley",
        make: valley::valley,
        regions: &[
            Region::at("town", [-128, -38, -840], [256, 218, -584]),
            Region::at("town", [-64, -38, -192], [320, 298, 256]),
            Region::at("chapel", [-227, -92, -128], [157, 356, 192]),
            Region::at("windmill", [-193, -90, -162], [191, 358, 158]),
            Region::at("mountainside", [-128, -189, -128], [128, 151, 128]),
            Region::at("steading", [-197, -50, -178], [219, 174, 206]),
            Region::at("station", [-192, -60, -256], [192, 240, 256]),
        ],
        levels: 4,
        accuracy: &[],
        far: &[Region::at("town", [-704, -38, -736], [704, 298, 736]), Region::at("coast", [-1536, -38, -704], [-128, 298, 704])],
        far_levels: 6,
    },
    // The first steading: its hut, the cairns and the ring north of it, heather and grass, from under the ground to over the roof.
    // Not checked far away: its coarse levels show less of its mist from above ("Spans of a medium" in docs/todo.md).
    Entry {
        name: "moor",
        make: valley::moor::moor,
        regions: &[Region::at("spawn", [-113, -46, 9], [143, 50, 393])],
        levels: 5,
        accuracy: &[],
        far: &[],
        far_levels: 6,
    },
    // The moor in fog (moor.rs's FOG), mist over all of it: where see-through voxels cost a host most.
    Entry {
        name: "fog",
        make: valley::moor::fog,
        regions: &[Region::at("spawn", [-113, -46, 9], [143, 50, 393])],
        levels: 5,
        accuracy: &[],
        far: &[],
        far_levels: 5,
    },
    // The first settlement's main street where a cross street meets it (at REGIONS_SEED): houses facing both, yards, trees, paving.
    // Far away, all of its crop: streets, paving and yards, fields beside them, the forest round it.
    Entry {
        name: "learned",
        make: learned::learned,
        regions: &[Region::at("settlement", [64, -34, -96], [448, 222, 288])],
        levels: 4,
        accuracy: &[],
        far: &[Region::at("settlement", [-1536, -34, -1536], [1536, 302, 1536])],
        far_levels: 6,
    },
    // The front row of blocks: the road before them, their entrances and the rooms behind, from under the road to over the roofs.
    // To level 3: at level 4, sites 2 m apart, a building's measured outline (city.rs, INSIDE) is kept or lost a site at a time.
    // Far away, the whole district: its markings, kerbs and paving slabs are finer than the sites there.
    Entry {
        name: "city",
        make: city::city,
        regions: &[Region::at("spawn", [-478, -20, -16], [482, 108, 304])],
        levels: 3,
        accuracy: &[],
        far: &[Region::at("spawn", [-478, -20, -32], [482, 108, 928])],
        far_levels: 6,
    },
    // The cavern the camera starts in, the caverns, tunnels and eyes around it, their pillars, stairs and crystals.
    // To level 3: from level 4 on, caves have no pillars, stairs or crystals (cheese/src/lib.rs, SIZE).
    // Endless, it has no sky to be seen from.
    Entry {
        name: "cheese",
        make: cheese::cheese,
        regions: &[Region::at("spawn", [-128, -128, -128], [128, 128, 128])],
        levels: 3,
        accuracy: &[],
        far: &[],
        far_levels: 6,
    },
    // All of Sponza's atrium, from its middle: floor, colonnades, galleries, the roof's edge (scene.rs).
    #[cfg(feature = "sponza")]
    Entry {
        name: "sponza",
        make: sponza,
        regions: &[Region::at("spawn", [-242, -46, -150], [242, 160, 150])],
        levels: 5,
        accuracy: &[],
        far: &[],
        far_levels: 6,
    },
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
