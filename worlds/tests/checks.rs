//! Every world of this crate keeps the contract, its directions and
//! places, and its cost baseline (worldgen's entry.rs): a test each, so
//! they run in parallel.

worldgen::world_tests!(worlds::WORLDS; forest, valley, moor, fog, learned, city);
