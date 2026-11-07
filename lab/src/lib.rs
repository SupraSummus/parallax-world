//! A lab, not part of the engine: can a world be learned from examples
//! rather than written?
//!
//! Three first tries are dropped; git history has them (114c077). A
//! hierarchy of WFC rule-sets learned the valley's biomes by EM and drew
//! any box alone, but biomes as bags of neighbour pairs hold no geometry:
//! streets as lines, rectangular lots and houses. Pixel conditionals on a
//! quadtree of templates, and rule-sets filling in around the grammar's
//! towns, gave way to the split grammar alone: geometry.rs's measures of
//! straight streets and rectangular roofs ranked all three below it.
//!
//! The split grammar (grammar.rs) learns many towns, from several seeds
//! (towns.rs); geometry.rs measures straight streets, rectangular roofs
//! and whether cross streets cross. They do once a cut's two sides agree:
//! a cut keeps the strips running across it in training as guides for
//! everything below it, which chooses its rules to fit them and snaps its
//! cuts onto them. The grammar is also a world's: `export` writes a
//! sample of its rules as the table worlds/src/learned.rs draws towns
//! from, the forest where a rule is texture.
//!
//! One learner of any scene, flat or 3D: a pyramid of learned codes, each
//! level drawn from the one above by a small local network (masked.rs;
//! masked_fcc.rs on the worlds' FCC lattice, where coarse cells overlap).
//! It learns statistics and new layouts, but breaks long thin things
//! (lanes, tunnels) into pieces. The scenes are in scenes.rs.
//!
//! Drawing addresses instead of labels (exemplar.rs, exemplar_fcc.rs):
//! each cell points into an example, so whole pieces come through. It
//! keeps lanes whole and islands apart, but cuts houses and street grids
//! where pieces of two towns meet.
//!
//! The critic (critic.rs) scores them without asking how like the
//! examples they are, which a copy wins: can a small network tell their
//! windows from real ones, at 16, 64 and 256 m; how much do they copy
//! verbatim. Checked first on maps whose verdict is known (unseen real
//! towns, the training crops, patchworks of training tiles), it puts
//! synthesis by coordinates ahead of the grammar and the masked model,
//! whose streets and blocks give them away at 64 m. But on real places it
//! barely sees streets cut into pieces; a profile (profile.rs) does: per
//! label, the sizes of its parts, its widths and its distances to the
//! others, set against real places' beyond how much those differ among
//! themselves.
//!
//! Real places (real.rs, from OpenStreetMap by scripts/overture.py) drawn
//! to a plan (plan.rs: the shares of built, forest, fields and water
//! around each 16 m), held-out places' or painted ones (blend.rs). Both
//! models follow a plan and blend where it says, as real places blend at
//! their edges; the critic sets each window against real ones of the same
//! plan, to judge the filling-in apart from the plan. But shares can't
//! hold what is long and thin: no model drew a painted river, and streets
//! fell into pieces. So a plan has lines too: main roads and water, given
//! exactly, the rest drawn around them. Rivers and roads come through
//! whole; the streets the models draw themselves still fall into pieces.
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
//! gather`, then `shared` and `export`; `masked`, `masked_fcc`;
//! `exemplar`, `exemplar_fcc`; `critic`; scripts/overture.py, then
//! `blend`), print Markdown tables and write pictures to out/; the
//! worlds' columns are generated once and cached there.

pub mod critic;
pub mod exemplar;
pub mod exemplar_fcc;
pub mod fcc_stats;
pub mod geometry;
pub mod grammar;
pub mod image;
pub mod masked;
pub mod masked_fcc;
pub mod net;
pub mod plan;
pub mod png;
pub mod profile;
pub mod real;
pub mod scenes;
pub mod stats;
pub mod towns;
pub mod valley;
pub mod vol;

/** Where the examples write: their pictures, and what they cache. */
pub const OUT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/out");
