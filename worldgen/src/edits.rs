//! Edits: what a world's level-0 sites hold since it was generated. The
//! lattice never moves; matter moves between its sites: a site cut, or
//! left by what fell, holds nothing (`Held::default()`); one fallen to
//! holds what fell there. A coarser site holds what the level-0 site it
//! is read at holds (generator.rs's `middle`), as the generator reads
//! levels. A site's matter holds on to its neighbours as its `Holds` say,
//! which move with it; generated matter holds on across every face
//! (`holds`), as worlds give no bodies yet. Hosts and light take sites
//! from `make_sites` (tiles.rs), which carves edits in, so what is drawn
//! and what is lit agree. Each edit is counted (`epoch`), for hosts to make
//! again what was made before it.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};

use lattice::{FastMap, HOLDS_ALL, Holds};

use crate::grid::{Box3, Held, VoxelGrid};
use crate::sites;

/** Level-0 sites edited: what each holds, and across which faces that holds on. */
pub type Sites = FastMap<[i64; 3], (Held, Holds)>;

/** Across which faces what a level-0 site holds holds on, edited or as generated. */
pub fn holds(edits: &Sites, site: [i64; 3]) -> Holds {
    edits.get(&site).map_or(HOLDS_ALL, |e| e.1)
}

/** A world's edits, shared by the threads that make its tiles and light them. */
#[derive(Default)]
pub struct Edits {
    sites: RwLock<Arc<Sites>>,
    epoch: AtomicU64,
}

impl Edits {
    /**
     * Has each of `changes` hold what it says, at its level-0 site. If any
     * changed, the edit's count (`epoch`) and the box of those that did.
     */
    pub fn change(&self, changes: impl IntoIterator<Item = ([i64; 3], (Held, Holds))>) -> Option<(u64, Box3)> {
        let mut sites = self.sites.write().expect("edits");
        let map = Arc::make_mut(&mut sites);
        let mut changed: Option<Box3> = None;
        for (site, held) in changes {
            if map.insert(site, held) != Some(held) {
                let b = changed.unwrap_or(Box3 { min: site, max: site });
                changed = Some(Box3 { min: [0, 1, 2].map(|a| b.min[a].min(site[a])), max: [0, 1, 2].map(|a| b.max[a].max(site[a] + 1)) });
            }
        }
        // Counted under the lock: what reads the count, then the sites, reads them as of that count or later.
        changed.map(|b| (self.epoch.fetch_add(1, Ordering::SeqCst) + 1, b))
    }

    /** The edits so far, kept as they are now. */
    pub fn sites(&self) -> Arc<Sites> {
        self.sites.read().expect("edits").clone()
    }
}

/** Has `grid`, of `level`, hold at each site what `edits` has its level-0 site read (generator.rs's `middle`) hold. */
pub fn carve(edits: &Sites, grid: &mut VoxelGrid, level: u32) {
    let s = 1i64 << level;
    let b = grid.clip();
    for (site, (held, _)) in edits {
        let p = site.map(|v| v.div_euclid(s));
        if (0..3).all(|a| site[a] == p[a] * s + sites::middle(s)) && b.holds(p) {
            grid.put(p[0], p[1], p[2], held);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lattice::FACE_KINDS;

    /**
     * An edit at [5, 5, 1] shows at the sites read for it: none at level 1
     * (it reads even ones), [1, 1, 0] at 2; taken out of whole voxels, or
     * put in none.
     */
    #[test]
    fn an_edit_shows_at_every_level_that_reads_it() {
        let b = Box3 { min: [0; 3], max: [8; 3] };
        let mut full = VoxelGrid::new(&b);
        (0..512).for_each(|i| full.put_whole(i % 8, i / 8 % 8, i / 64, [[100; 3]; FACE_KINDS]));
        let whole = full.held(0, 0, 0);
        for (into, held) in [(&full, Held::default()), (&VoxelGrid::new(&b), whole)] {
            let edits = Edits::default();
            assert_eq!(edits.change([([5, 5, 1], (held, 0))]), Some((1, Box3 { min: [5, 5, 1], max: [6, 6, 2] })));
            assert_eq!(edits.change([([5, 5, 1], (held, 0))]), None, "as it is already");
            for (level, read) in [(0, vec![[5, 5, 1]]), (1, vec![]), (2, vec![[1, 1, 0]])] {
                let mut grid = into.clone();
                carve(&edits.sites(), &mut grid, level);
                let sites = (0..512).map(|i| [i % 8, i / 8 % 8, i / 64]);
                let edited: Vec<[i64; 3]> = sites.filter(|&[x, y, z]| grid.held(x, y, z) == held).collect();
                assert_eq!(edited, read, "level {level}");
            }
        }
    }
}
