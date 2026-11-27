//! What a host needs to show a world: its generator, where the camera
//! starts, and the heights it has anything at, if it is bounded so; and
//! the places it names, for checks and tools to find its features at any
//! seed. One for all the threads that make and light its tiles.
//!
//! Heights are a world's own: a ground's sky over it, and the rock under
//! it no one sees. A world endless every way (the cheese) has none, nor a
//! sky. Checks look at a world in cubes of the lattice around places, each
//! within its heights if it has them (`within`).

use crate::edits::Edits;
use crate::generator::Generator;
use crate::grid::Box3;

/** Finds a place of a world, lattice units. */
pub type Find = Box<dyn Fn() -> [i64; 3] + Send + Sync>;

/**
 * The axis hosts show every world up along: its heights lie along it,
 * tiles are made together along it, rock is found under them along it;
 * pictures and checks look down it. The engine has none
 * (docs/architecture.md, section 5); hosts keep it +y, and a world could
 * name its own ("Assume no ground" in docs/todo.md).
 */
pub const UP: usize = 1;

/**
 * The heights hosts tile a world with none in: as good as endless, and
 * far within what their coordinates hold.
 */
const ENDLESS: (i64, i64) = (-(1 << 40), 1 << 40);

pub struct World {
    pub generator: Generator,
    /** Where the camera starts, lattice units. */
    pub spawn: [i64; 3],
    /**
     * The heights it has anything at, along UP, lattice units, [min,
     * max), with a sky over them; none for a world endless every way.
     */
    pub heights: Option<(i64, i64)>,
    /**
     * Places it has at any seed, by name, besides the spawn: a town, a
     * windmill. Found only when asked, as some take a search.
     */
    pub points: Vec<(&'static str, Find)>,
    /** What its sites hold since it was generated: none as made. */
    pub edits: Edits,
}

impl World {
    /**
     * The heights it has anything at in units of `level`: the sites whose
     * voxels reach into them. A site on the top's plane stands for the half
     * of its voxel below it, and may be drawn ("Faces" in docs/generators.md).
     * With none, as good as endless.
     */
    pub fn heights_at(&self, level: u32) -> (i64, i64) {
        let (s, (low, high)) = (1i64 << level, self.heights.unwrap_or(ENDLESS));
        (low.div_euclid(s), high.div_euclid(s) + 1)
    }

    /** What of box `b` (lattice units) lies within its heights, if it has them. */
    pub fn within(&self, b: Box3) -> Box3 {
        let Some((low, high)) = self.heights else { return b };
        let from = low.max(b.min[UP]);
        b.slab(UP, from, high.min(b.max[UP]).max(from))
    }

    /** Its places by name: "spawn", then its points, each found when the iterator reaches it. */
    pub fn places(&self) -> impl Iterator<Item = (&'static str, [i64; 3])> + '_ {
        std::iter::once(("spawn", self.spawn)).chain(self.points.iter().map(|(name, find)| (*name, find())))
    }

    /** The place named `name`, of `places`, finding no other. */
    pub fn point(&self, name: &str) -> Option<[i64; 3]> {
        if name == "spawn" {
            return Some(self.spawn);
        }
        self.points.iter().find(|(n, _)| *n == name).map(|(_, find)| find())
    }

    /** Its `component` alone (`Generator::alone`), where it is in the world: its spawn, heights and places. */
    pub fn alone(self, component: &str) -> World {
        World { generator: self.generator.alone(component), ..self }
    }
}
