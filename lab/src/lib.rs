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
//! The third makes the grammar's two sides of a cut agree, so cross
//! streets cross: a cut keeps the strips running across it in training as
//! guides for everything below it, which chooses its rules to fit them
//! and snaps its cuts onto them (grammar.rs; geometry.rs counts crossings).
//! And it samples a whole area by regimes (regimes.rs): towns at the
//! sites of a plan, the grammar in their boxes down to rules of canopy and
//! ground, a texture model (W0's) everywhere else.
//!
//! The grammar with shared splits is also a world's: `export` writes a
//! sample of its rules as the table worlds/src/learned.rs draws towns from.
//!
//! The fourth asks for one learner of any scene, flat or 3D: a pyramid of
//! learned codes, each level drawn from the one above by a small local
//! network (masked.rs; masked_fcc.rs on the worlds' FCC lattice, where
//! coarse cells overlap). It learns statistics and new layouts, but breaks
//! long thin things (lanes, tunnels) into pieces. The scenes are in
//! scenes.rs.
//!
//! The fifth draws addresses instead of labels (exemplar.rs,
//! exemplar_fcc.rs): each cell points into an example, so whole pieces
//! come through. It keeps lanes whole and islands apart, but cuts houses
//! and street grids where pieces of two towns meet.
//!
//! The sixth scores them without asking how like the examples they are,
//! which a copy wins (critic.rs): can a small network tell their windows
//! from real ones, at 16, 64 and 256 m; how much do they copy verbatim.
//! Checked first on maps whose verdict is known (unseen real towns, the
//! training crops, patchworks of training tiles), it puts synthesis by
//! coordinates ahead of the grammar and the masked model, whose streets
//! and blocks give them away at 64 m. But on real places it barely sees
//! streets cut into pieces; a profile (profile.rs) does: per label, the
//! sizes of its parts, its widths and its distances to the others, set
//! against real places' beyond how much those differ among themselves.
//!
//! The seventh learns from real places (real.rs, from OpenStreetMap by
//! scripts/overture.py) and draws to a plan (plan.rs: the shares of
//! built, forest, fields and water around each 16 m), held-out places' or
//! painted ones (blend.rs). Both models follow a plan and blend where it
//! says, as real places blend at their edges; the critic sets each window
//! against real ones of the same plan, to judge the filling-in apart from
//! the plan. But shares can't hold what is long and thin: no model drew a
//! painted river, and streets fell into pieces. So a plan has lines too:
//! main roads and water, given exactly, the rest drawn around them.
//! Rivers and roads come through whole; the streets the models draw
//! themselves still fall into pieces.
//!
//! Why, for the masked model: given a real place's codes from 8 m up, its
//! lower levels join the streets; from 16 m up, they don't. Not for what
//! the codes hold (128 a level, or which faces of a cell a label crosses)
//! nor for how well each level is learned (a wider network, or attention
//! among a cell's slots, hold out no fewer bits): of thousands of cells
//! drawn, a few stray, and each cuts a street. Drawn surer than learned,
//! streets join but everything comes out wider; four times the steps
//! barely help.
//!
//! The examples run the experiments (`cargo run --release --example
//! segment`, then `synth`; `gather`, then `geometry`, `shared` and
//! `regimes`; `masked`, `masked_fcc`; `exemplar`, `exemplar_fcc`;
//! `critic`; scripts/overture.py, then `blend`), print
//! Markdown tables and write pictures to out/; the worlds' columns are
//! generated once and cached there.

pub mod critic;
pub mod exemplar;
pub mod exemplar_fcc;
pub mod experiment;
pub mod fcc_stats;
pub mod geometry;
pub mod grammar;
pub mod hierarchy;
pub mod image;
pub mod masked;
pub mod masked_fcc;
pub mod mixture;
pub mod net;
pub mod plan;
pub mod png;
pub mod profile;
pub mod real;
pub mod regimes;
pub mod scenes;
pub mod stats;
pub mod synth;
pub mod template;
pub mod towns;
pub mod valley;
pub mod vol;
pub mod wfc;
