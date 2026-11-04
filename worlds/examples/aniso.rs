//! Study (not for merging): a world's contract margins, with or without
//! view-dependent coarse voxels (WORLDGEN_ANISO, see worldgen::grid::aniso):
//!
//! WORLDGEN_ANISO=1 WORLDGEN_FROM_ABOVE=5 cargo run --release --example aniso -- valley 42 5
//!
//! Prints each margin on a line, then (with WORLDGEN_ANISO_STATS) how many
//! measured coarse sites are seen as another kind from some direction.

use std::collections::{BTreeMap, BTreeSet};

use worldgen::contract::{Measure, follow_surface, margins};
use worldgen::generator::{ANISO_SITES, generate, materials_of};
use worldgen::grid::Box3;
use worlds::{REGIONS_SEED, by_name};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let entry = by_name(&args[0]).expect("a world");
    let seed: u64 = args[1].parse().expect("a seed");
    let levels: u32 = args[2].parse().expect("levels");
    let world = (entry.make)(seed);
    // At other seeds, the regions follow the surface up or down, as the contract's do.
    let regions = follow_surface(entry.regions, &(entry.make)(REGIONS_SEED), &world);
    let report = margins(entry.name, &world, &regions, levels);
    for m in &report.margins {
        let what = match m.measure {
            Measure::Through => "through",
            Measure::Hit => "hit",
            Measure::FromAbove => "above",
        };
        println!("MARGIN region {:?} level {} {what} {:.3}", m.region.min, m.level, m.difference);
    }
    if std::env::var("WORLDGEN_ANISO_STATS").is_ok() {
        println!("{report:#}");
        // What a grid of each region holds: voxels, those seen otherwise from some direction, distinct id tuples.
        println!("MATERIALS {}", materials_of(&world.generator).len());
        for r in &regions {
            for level in 2..=6u32 {
                let s = 1i64 << level;
                let b = Box3 { min: r.min.map(|v| v.div_euclid(s) - 2), max: r.max.map(|v| -(-v).div_euclid(s) + 2) };
                let grid = generate(&world.generator, &b, level);
                let full = grid.cells.iter().filter(|id| **id != 0).count();
                let tuples: BTreeSet<[u8; 6]> = grid.aniso.values().copied().collect();
                println!(
                    "GRID region {:?} level {level}: {full} voxels, {} ({:.1}%) view-dependent, {} distinct id tuples",
                    r.min,
                    grid.aniso.len(),
                    100.0 * grid.aniso.len() as f64 / full.max(1) as f64,
                    tuples.len()
                );
            }
        }
        let sites = ANISO_SITES.lock().expect("not poisoned");
        let mut by_s: BTreeMap<i64, (usize, usize, usize, BTreeSet<[u8; 6]>)> = BTreeMap::new();
        for ((s, _), (kind, seen)) in sites.iter() {
            let e = by_s.entry(*s).or_default();
            e.0 += 1;
            e.1 += seen.iter().any(|k| k != kind) as usize;
            e.2 += (seen[3] != *kind) as usize;
            e.3.insert(*seen);
        }
        for (s, (n, differs, above, combos)) in by_s {
            println!(
                "STATS spacing {s}: {n} measured sites, {differs} ({:.1}%) seen as another kind from some direction, \
                 {above} ({:.1}%) from above; {} distinct kind tuples",
                100.0 * differs as f64 / n as f64,
                100.0 * above as f64 / n as f64,
                combos.len()
            );
        }
    }
}
