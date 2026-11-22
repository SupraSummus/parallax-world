//! The world-generation engine: the generator framework and frames to
//! build worlds with, into grids the baker reads, moving to integers
//! (docs/generators.md); a World for hosts, and the contract every world
//! keeps. No direction is up in it: grounds, columns and plans over them
//! are the ground crate's, on it. The worlds themselves are in worlds/.

pub mod build;
pub mod checks;
pub mod cost;
pub mod curve;
pub mod frame;
pub mod generator;
pub mod grid;
pub mod js;
pub mod noise;
pub mod picture;
pub mod points;
pub mod recent;
pub mod sites;
pub mod tiles;
pub mod units;
pub mod world;

pub use world::World;
