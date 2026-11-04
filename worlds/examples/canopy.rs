//! Study (not for merging): how anisotropic the forest's far canopy is.
//! Sample trees of each species drawn at level 2, and for slabs of them
//! as thick as the sites of levels 4 to 7, their shadow seen from above
//! and from the side, by kind (Shadows::slab_by_axis):
//!
//! cargo run --release --example canopy

use worldgen::generator::{Node, Shadows};
use worldgen::noise::{Rng, hash64};
use worlds::forest::{Species, forest, make_tree};

fn main() {
    let kinds = forest(42).generator.kinds;
    let name = |k: u8| if k == u8::MAX { "all" } else { kinds[k as usize].name };
    let tree_top = (26.0 / worldgen::units::UNIT_SIZE) as i64;
    for (species, label) in [(Species::Spruce, "spruce"), (Species::Birch, "birch"), (Species::Oak, "oak")] {
        let samples: Vec<Node> =
            (0..64).map(|i| make_tree(species, 0.0, 0.0, 0.0, &mut Rng::new(hash64(17, i, 0, 0)), &|_, _| 0)).collect();
        let shadows = Shadows::new(&samples, 2, 1);
        // The whole tree: from above against from the side.
        for (k, [y, z, x]) in shadows.slab_by_axis(0, tree_top) {
            println!(
                "WHOLE {label} {:>12}: from above {y:8.0}, from the side {:8.0} (x {x:.0}, z {z:.0}), above/side {:.2}",
                name(k),
                (x + z) / 2.0,
                2.0 * y / (x + z).max(1.0)
            );
        }
        for level in 4..=7 {
            let s = 1i64 << level;
            for from in (0..tree_top).step_by(s as usize) {
                let to = (from + s).min(tree_top);
                let line: Vec<String> = shadows
                    .slab_by_axis(from, to)
                    .iter()
                    .map(|(k, [y, z, x])| format!("{} above {y:.0} side {:.0}", name(*k), (x + z) / 2.0))
                    .collect();
                println!("SLAB {label} level {level} {from:>3}..{to:>3}: {}", line.join("; "));
            }
        }
    }
}
