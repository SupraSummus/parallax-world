//! What a host needs to show a world: its generator, where the camera
//! starts, and the heights to generate it in.

use crate::generator::Generator;

pub struct World {
    pub generator: Generator,
    /** Where the camera starts, lattice units. */
    pub spawn: [i64; 3],
    /** The heights it has anything at, lattice units, [min, max). */
    pub heights: (i64, i64),
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
}
