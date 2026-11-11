//! What a host needs to show a world: its generator, where the camera
//! starts, and the heights to generate it in; and the places it names,
//! for checks and tools to find its features at any seed.

use crate::generator::Generator;

/** Finds a place of a world, lattice units. */
pub type Find = Box<dyn Fn() -> [i64; 3]>;

pub struct World {
    pub generator: Generator,
    /** Where the camera starts, lattice units. */
    pub spawn: [i64; 3],
    /** The heights it has anything at, lattice units, [min, max). */
    pub heights: (i64, i64),
    /**
     * Places it has at any seed, by name, besides the spawn: a town, a
     * windmill. Found only when asked, as some take a search.
     */
    pub points: Vec<(&'static str, Find)>,
}

impl World {
    /**
     * The heights it has anything at in units of `level`: the sites whose
     * voxels reach into them, ends included. A site on the top's plane
     * stands for the half of its voxel below it, and may be drawn ("Faces"
     * in docs/generators.md); past level 0, one under the bottom's plane
     * holds half of level 0's site on it (`slices` in generator.rs).
     */
    pub fn heights_at(&self, level: u32) -> (i64, i64) {
        let s = 1i64 << level;
        ((self.heights.0 - s + 1).div_euclid(s), self.heights.1.div_euclid(s) + 1)
    }

    /** The place named `name`: "spawn", or one of its points. */
    pub fn point(&self, name: &str) -> Option<[i64; 3]> {
        if name == "spawn" {
            return Some(self.spawn);
        }
        self.points.iter().find(|(n, _)| *n == name).map(|(_, find)| find())
    }
}
