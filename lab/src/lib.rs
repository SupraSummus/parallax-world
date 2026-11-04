//! A lab, not part of the engine: can a hierarchy of wave-function-collapse
//! rule-sets be learned from a world rather than written?
//!
//! The valley seen from above (valley.rs) is a label image. Cut into
//! blocks, it is fitted as a mixture of adjacency models by EM
//! (mixture.rs): each block's rule-set is its biome, and the biomes form
//! the next, coarser label image, fitted the same way (hierarchy.rs). A new
//! world is then sampled top down (synth.rs): each cell's children by WFC
//! (wfc.rs) with the rule-set of the cell's biome, cell by cell, the
//! children on its edges and corners decided by those alone, so any box is
//! the same generated alone or as part of the whole. stats.rs scores both.
//!
//! The second experiment asks whether geometry can be learned: streets
//! as lines on the edges of blocks, rectangular lots and houses, which
//! neighbour counts in biomes cannot hold. Many towns, from several seeds
//! (towns.rs), against two models sampled top down, any box alone: pixel
//! conditionals with position, on a quadtree of templates (template.rs),
//! and a split grammar of rectangles (grammar.rs); geometry.rs measures
//! straight streets and rectangular roofs.
//!
//! The examples run the experiments (`cargo run --release --example
//! segment`, then `synth`; `gather`, then `geometry`), print Markdown
//! tables and write pictures to out/; the worlds' columns are generated
//! once and cached there.

pub mod experiment;
pub mod geometry;
pub mod grammar;
pub mod hierarchy;
pub mod image;
pub mod mixture;
pub mod png;
pub mod stats;
pub mod synth;
pub mod template;
pub mod towns;
pub mod valley;
pub mod wfc;
