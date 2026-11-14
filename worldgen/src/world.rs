//! What a host needs to show a world: its generator, where the camera
//! starts, and the heights to generate it in; and the places it names,
//! for checks and tools to find its features at any seed.
//!
//! Heights are a world's own: a ground's sky over it, and the rock under
//! it no one sees. Checks look at a world in cubes of the lattice around
//! places, each within its heights (`within`); some still take y as up
//! (docs/todo.md, "Drop y as up").

use crate::generator::Generator;
use crate::grid::Box3;

/** Finds a place of a world, lattice units. */
pub type Find = Box<dyn Fn() -> [i64; 3]>;

/**
 * Heights as good as endless, for a world nothing bounds along y (the
 * cheese): whatever is asked of it is made where asked (plan::along).
 */
pub const ENDLESS: (i64, i64) = (-(1 << 40), 1 << 40);

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
     * voxels reach into them. A site on the top's plane stands for the half
     * of its voxel below it, and may be drawn ("Faces" in docs/generators.md).
     */
    pub fn heights_at(&self, level: u32) -> (i64, i64) {
        let s = 1i64 << level;
        (self.heights.0.div_euclid(s), self.heights.1.div_euclid(s) + 1)
    }

    /** What of box `b` (lattice units) lies within its heights. */
    pub fn within(&self, b: Box3) -> Box3 {
        let (min, max) = (self.heights.0.max(b.min[1]), self.heights.1.min(b.max[1]));
        Box3 { min: [b.min[0], min, b.min[2]], max: [b.max[0], max.max(min), b.max[2]] }
    }

    /** The place named `name`: "spawn", or one of its points. */
    pub fn point(&self, name: &str) -> Option<[i64; 3]> {
        if name == "spawn" {
            return Some(self.spawn);
        }
        self.points.iter().find(|(n, _)| *n == name).map(|(_, find)| find())
    }
}
