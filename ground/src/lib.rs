//! Worlds on a ground: what worldgen, in which no direction is up, does
//! not know. Up is +y. Plans decide what is where column by column (x, z),
//! heights among them; columns lay a ground's layers down from its top;
//! fields and populations say they are about a ground, or stand on it;
//! land is shared by bids for columns. Built on worldgen's public API only.
//! See "Composing" and "Writing a world" in docs/generators.md.

pub mod about;
pub mod column;
pub mod land;
pub mod plan;

pub use about::{FieldAbout, PopulationOn, columns, under};

const DOCS: &str = "docs/generators.md";
