//! A world as hosts start it by name, and what its checks hold it to
//! (docs/checks.md); and the tests each crate of worlds runs on its own
//! (world_tests!), its WORLDS, against baselines in its own data/: so a
//! world is written in a crate of its own, touching no other, and the
//! registry hosts read (worlds' `all`) takes each crate's WORLDS.

use lattice::GRID_LEVELS;

use crate::World;
use crate::checks::contract::{Accuracy, breaks, check, hold_to_baseline};
use crate::checks::directions::directions;
use crate::checks::places::preferences;
use crate::cost::{BUDGET, REMADE, Tile, report};

/** A world hosts can start by name, with how far the checks hold it. */
#[derive(Clone, Copy)]
pub struct Entry {
    pub name: &'static str,
    /** The world of a seed. */
    pub make: fn(u64) -> World,
    /** The level up to which its cells look as what is in them (checks::contract). */
    pub levels: u32,
    /**
     * How closely its strata do, on average, by name: a thing's or a
     * layer's covers its parts (Accuracy::of); DEFAULT for the rest.
     */
    pub accuracy: &'static [(&'static str, Accuracy)],
    /** What it is known to break, each for a reason in docs/todo.md. */
    pub known: &'static [Known],
}

/**
 * What a world is known to break. Listed, it must still, or come off
 * (docs/testing.md): the list only shrinks, and blessing does not move it.
 */
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Known {
    /** Around the place it names, its directions follow the lattice's (checks::directions). */
    Turned(&'static str),
    /** The population's things prefer places on its grids (checks::places). */
    Prefers(&'static str),
    /** Its tile at the place, of the level, is over the cost budget (cost::BUDGET). */
    OverBudget(&'static str, u32),
}

/** The seed worlds are checked at first, and their cost measured at alone. */
pub const SEED: u64 = 42;

/** The seeds the contract checks a world at, together: each stratum is checked where it is cheapest, its margins averaged over all of them. */
const CONTRACT_SEEDS: &[u64] = &[SEED, 1, 2, 3];

/** The seeds pairs are sampled at for directions: the samples, so enough for those known to show. */
const DIRECTION_SEEDS: &[u64] = &[SEED, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23];

/** The seeds things are gathered at for places. */
const PLACE_SEEDS: &[u64] = &[SEED, 1, 2, 3, 4, 5, 6, 7];

/** How much a count of cost may rise or fall from its baseline: a quarter. */
const TOLERANCE: f64 = 0.25;

/**
 * `entry`'s world keeps the contract (checks::contract) at CONTRACT_SEEDS,
 * in regions holding everything it is made of around spawn at any of
 * them, each stratum's margins averaged over all of them. CONTRACT_ONLY,
 * names of strata separated by commas ("towns/church,railway"), checks
 * only regions holding them. Panics where it breaks it but as its
 * baseline, `dir`/data/contract/<name>.txt, says.
 */
pub fn keeps_the_contract(entry: &Entry, dir: &str) {
    let only = std::env::var("CONTRACT_ONLY").unwrap_or_default();
    let only: Vec<&str> = only.split(',').map(str::trim).filter(|o| !o.is_empty()).collect();
    let worlds: Vec<(String, World)> =
        CONTRACT_SEEDS.iter().map(|&seed| (format!("{} (seed {seed})", entry.name), (entry.make)(seed))).collect();
    let worlds: Vec<(&str, &World)> = worlds.iter().map(|(name, world)| (name.as_str(), world)).collect();
    let checked = check(&worlds, entry.levels, &only);
    let path = format!("{dir}/data/contract/{}.txt", entry.name);
    hold_to_baseline(&breaks(&checked, entry.accuracy), &path, only.is_empty());
}

/** Around no place of `entry`'s world its directions follow the lattice's (checks::directions), but those it knows; prints each place's strongest four-fold harmonic. */
pub fn turned(entry: &Entry) {
    let leaks = directions(entry.make, DIRECTION_SEEDS);
    leaks.iter().for_each(|leak| println!("{}: {leak}", entry.name));
    let found = leaks.iter().filter(|l| l.broken()).map(|l| (Known::Turned(l.place), l.to_string()));
    hold_to_known(entry, |k| matches!(k, Known::Turned(_)), found.collect());
}

/** The things of each population of `entry`'s world prefer no places (checks::places), but those it knows; prints each one's strongest preference. */
pub fn even(entry: &Entry) {
    let preferences = preferences(entry.make, PLACE_SEEDS);
    preferences.iter().for_each(|p| println!("{}: {p}", entry.name));
    let found = preferences.iter().filter(|p| p.broken()).map(|p| (Known::Prefers(p.population), p.to_string()));
    hold_to_known(entry, |k| matches!(k, Known::Prefers(_)), found.collect());
}

/**
 * What generating `entry`'s world costs (cost) stays within TOLERANCE of
 * its baseline, `dir`/data/cost/<name>.txt: counts of work, the same on
 * every machine, at SEED in the host tiles of its places
 * (World::places) at every level a host makes (lattice's GRID_LEVELS), odd
 * ones too. `COST_BLESS=1` writes the baseline anew. No plan makes a
 * tile's columns over cost::REMADE times each, and no tile is over the
 * budget (cost::BUDGET, estimated) but those it knows.
 */
pub fn costs_keep_to_their_baseline(entry: &Entry, dir: &str) {
    let levels: Vec<u32> = (0..GRID_LEVELS as u32).collect();
    let names: Vec<(&'static str, [i64; 3])> = (entry.make)(SEED).places().collect();
    let places: Vec<(String, [i64; 3])> = names.iter().map(|&(name, at)| (name.to_string(), at)).collect();
    let report = report(&format!("{} (seed {SEED})", entry.name), &|| (entry.make)(SEED), &places, &levels);
    let path = format!("{dir}/data/cost/{}.txt", entry.name);
    if std::env::var_os("COST_BLESS").is_some() {
        std::fs::write(&path, report.baseline()).unwrap_or_else(|e| panic!("{path}: {e}"));
    } else {
        let baseline = std::fs::read_to_string(&path).unwrap_or_else(|_| panic!("no {path}: COST_BLESS=1 writes it"));
        let changes = report.changes(&baseline, TOLERANCE);
        assert!(
            changes.is_empty(),
            "{report}  changed from the baseline:\n    {}\nif that is meant, COST_BLESS=1 writes the baselines anew",
            changes.join("\n    ")
        );
    }
    let remade: Vec<String> = (report.tiles.iter())
        .flat_map(|t| t.counts.remade().map(move |(plan, times)| format!("{} level {}: plan {plan}, {times:.1} times", t.place, t.level)))
        .collect();
    assert!(remade.is_empty(), "columns made over {REMADE} times each (cost::REMADE):\n{}", remade.join("\n"));
    let name = |t: &Tile| names.iter().find(|(name, _)| *name == t.place).expect("a place of the world").0;
    let over = report.tiles.iter().filter(|t| t.counts.total().ns() > BUDGET);
    let found = over.map(|t| {
        (Known::OverBudget(name(t), t.level), format!("{} ms, over {} ms", t.counts.total().ns() / 1_000_000, BUDGET / 1_000_000))
    });
    hold_to_known(entry, |k| matches!(k, Known::OverBudget(..)), found.collect());
}

/**
 * Holds what a check `found` `entry`'s world to break, each as its known
 * would list it, to those of its known of the kind (`kind`): fails on
 * each found it does not know, and on each it knows not found, to take it off.
 */
fn hold_to_known(entry: &Entry, kind: fn(&Known) -> bool, found: Vec<(Known, String)>) {
    let unknown = found.iter().filter(|(k, _)| !entry.known.contains(k)).map(|(k, why)| format!("{} {k:?}: {why}", entry.name));
    let gone = (entry.known.iter().filter(|k| kind(k) && !found.iter().any(|(f, _)| f == *k)))
        .map(|k| format!("{} {k:?}: known, but broken no more: take it off its known", entry.name));
    let broken: Vec<String> = unknown.chain(gone).collect();
    assert!(broken.is_empty(), "{}", broken.join("\n"));
}

/**
 * The tests of each world named, of a crate's WORLDS, in that crate:
 * keeps_the_contract::<name>, turned::<name> and even::<name> (slow:
 * `pnpm test:worlds`), and cost::<name>, each against the crate's own
 * data/; and that every world of WORLDS is named.
 *
 * ```ignore
 * worldgen::world_tests!(cheese::WORLDS; cheese);
 * ```
 */
#[macro_export]
macro_rules! world_tests {
    ($worlds:expr; $($name:ident),* $(,)?) => {
        /** The world of WORLDS named `name`. */
        fn world(name: &str) -> &'static $crate::Entry {
            $worlds.iter().find(|e| e.name == name).unwrap_or_else(|| panic!("no world {name}"))
        }
        mod keeps_the_contract {
            $(#[test] fn $name() { $crate::entry::keeps_the_contract(super::world(stringify!($name)), env!("CARGO_MANIFEST_DIR")) })*
        }
        mod turned {
            $(#[test] fn $name() { $crate::entry::turned(super::world(stringify!($name))) })*
        }
        mod even {
            $(#[test] fn $name() { $crate::entry::even(super::world(stringify!($name))) })*
        }
        mod cost {
            $(#[test] fn $name() { $crate::entry::costs_keep_to_their_baseline(super::world(stringify!($name)), env!("CARGO_MANIFEST_DIR")) })*
        }
        #[test]
        fn every_world_is_tested() {
            let names: Vec<&str> = $worlds.iter().map(|e| e.name).collect();
            assert_eq!(names, [$(stringify!($name)),*], "name each world of WORLDS in world_tests!");
        }
    };
}
