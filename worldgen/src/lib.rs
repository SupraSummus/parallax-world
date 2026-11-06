//! The world-generation engine: the generator framework, plans and frames
//! to build worlds with, into grids the baker reads, moving to integers
//! (docs/generators.md); a World for hosts, and the contract every world
//! keeps. The worlds themselves are in worlds/.

pub mod build;
pub mod column;
pub mod contract;
pub mod cost;
pub mod frame;
pub mod generator;
pub mod grid;
pub mod js;
pub mod noise;
pub mod picture;
pub mod places;
pub mod plan;
pub mod units;
pub mod world;

pub use world::World;
