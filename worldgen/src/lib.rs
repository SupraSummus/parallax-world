//! World generation in Rust: the generator framework (generator.ts) and
//! the forest (forest.ts), into grids the baker reads. Ported from the TS
//! step for step, now moving to integers (docs/generators.md).

pub mod forest;
pub mod generator;
pub mod grid;
pub mod js;
pub mod noise;
