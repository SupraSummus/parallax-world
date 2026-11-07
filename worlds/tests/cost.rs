//! What generating each world costs (worldgen::cost) stays within
//! TOLERANCE of its baseline, worlds/data/cost/<world>.txt: counts of
//! work, the same on every machine, at REGIONS_SEED in the host tiles of
//! its places at LEVELS. `COST_BLESS=1 cargo test --release --test cost`
//! in worlds/ writes the baselines anew. And no tile is over the budget
//! (cost::BUDGET, estimated) but those of OVER_BUDGET, which only shrinks,
//! and no plan makes a tile's columns over cost::REMADE times each.

use worldgen::cost::{BUDGET, REMADE, Tile, places, report};
use worlds::{REGIONS_SEED, WORLDS};

const LEVELS: &[u32] = &[0, 2, 4, 6];

/** How much a count may rise or fall from its baseline: a quarter. */
const TOLERANCE: f64 = 0.25;

/**
 * Tiles known to be over the budget: (world, place, level). A tile over
 * it fails unless listed here, and a listed one fails once within it, to
 * be taken off: a ratchet, blessing does not move it.
 */
const OVER_BUDGET: &[(&str, &str, u32)] = &[("valley", "region1", 4), ("valley", "region1", 6)];

#[test]
fn costs_keep_to_their_baselines() {
    let bless = std::env::var_os("COST_BLESS").is_some();
    let (mut changed, mut budget, mut remade) = (Vec::new(), Vec::new(), Vec::new());
    for entry in WORLDS {
        let world = (entry.make)(REGIONS_SEED);
        let regions: Vec<_> = entry.regions.iter().map(|r| r.of(&world)).collect();
        let places = places(&world, &regions);
        let report = report(&format!("{} (seed {REGIONS_SEED})", entry.name), &|| (entry.make)(REGIONS_SEED), &places, LEVELS);
        let over = |t: &Tile| t.counts.total().ns() > BUDGET;
        for t in report.tiles.iter().filter(|t| over(t) && !OVER_BUDGET.contains(&(entry.name, &t.place, t.level))) {
            budget.push(format!("{} {} level {}: {} ms, over it", entry.name, t.place, t.level, t.counts.total().ns() / 1_000_000));
        }
        for (_, place, level) in OVER_BUDGET.iter().filter(|e| e.0 == entry.name) {
            if !report.tiles.iter().any(|t| t.place == *place && t.level == *level && over(t)) {
                budget.push(format!("{} {place} level {level}: listed, but not over it: take it off", entry.name));
            }
        }
        for t in &report.tiles {
            for (plan, times) in t.counts.remade() {
                remade.push(format!("{} {} level {}: plan {plan}, {times:.1} times", entry.name, t.place, t.level));
            }
        }
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
    assert!(remade.is_empty(), "columns made over {REMADE} times each (cost::REMADE):\n{}", remade.join("\n"));
    assert!(
        budget.is_empty(),
        "against the budget of {} ms a tile, estimated (cost::BUDGET), and OVER_BUDGET in this file:\n{}",
        BUDGET / 1_000_000,
        budget.join("\n")
    );
}
