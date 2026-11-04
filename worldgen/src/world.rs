//! What a host needs to show a world: its generator, and where and how far
//! to generate it.

use crate::generator::Generator;

pub struct World {
    pub generator: Generator,
    /** Where the camera starts, lattice units. */
    pub spawn: [i64; 3],
    /** How far around the camera level 0's grid must hold it, lattice units. */
    pub reach: f64,
    /** The heights it has anything at, lattice units, [min, max). */
    pub heights: (i64, i64),
}

impl World {
    /** The heights it has anything at in units of `level`: those of whole sites holding them. */
    pub fn heights_at(&self, level: u32) -> (i64, i64) {
        let s = 1i64 << level;
        (self.heights.0.div_euclid(s), -(-self.heights.1).div_euclid(s))
    }
}
