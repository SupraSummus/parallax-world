//! What generating each world costs (worldgen::cost) stays within
//! TOLERANCE of its baseline, worlds/data/cost/<world>.txt: counts of
//! work, the same on every machine, at REGIONS_SEED in the host tiles of
//! its places at LEVELS. `COST_BLESS=1 cargo test --release --test cost`
//! in worlds/ writes the baselines anew.

use worldgen::cost::{places, report};
use worlds::{REGIONS_SEED, WORLDS};

const LEVELS: &[u32] = &[0, 2, 4, 6];

/** How much a count may rise or fall from its baseline: a quarter. */
const TOLERANCE: f64 = 0.25;

#[test]
fn costs_keep_to_their_baselines() {
    let bless = std::env::var_os("COST_BLESS").is_some();
    let mut changed = Vec::new();
    for entry in WORLDS {
        let world = (entry.make)(REGIONS_SEED);
        let regions: Vec<_> = entry.regions.iter().map(|r| r.of(&world)).collect();
        let places = places(&world, &regions);
        let report = report(&format!("{} (seed {REGIONS_SEED})", entry.name), &|| (entry.make)(REGIONS_SEED), &places, LEVELS);
        let path = format!("{}/data/cost/{}.txt", env!("CARGO_MANIFEST_DIR"), entry.name);
        if bless {
            std::fs::write(&path, report.baseline()).unwrap_or_else(|e| panic!("{path}: {e}"));
            continue;
        }
        let baseline = std::fs::read_to_string(&path).unwrap_or_else(|_| panic!("no {path}: COST_BLESS=1 writes it"));
        let changes = report.changes(&baseline, TOLERANCE);
        if !changes.is_empty() {
            changed.push(format!("{report}  changed from the baseline:\n    {}", changes.join("\n    ")));
        }
    }
    assert!(changed.is_empty(), "{}\nif that is meant, COST_BLESS=1 writes the baselines anew", changed.join("\n"));
}
