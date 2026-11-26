//! Statics, an experiment (docs/statics.md): does what is built stand, and
//! what falls when it does not? Matter stays on the lattice; what falls is
//! taken off it whole.
//!
//! At every level of 2^l·Z³, the worlds' octree (lattice.rs), a node is
//! a face between two cells, and a cell joins its faces by beams, each a
//! section and a strength (model.rs). Level 0's are its cells' material's, a full cell's
//! (`SECTION`); a coarser level's are measured from a finer one against
//! a full cell's (gather.rs), or given as a generator likes. What lies
//! deep in bulk is held (`hold_bulk`). The forces are all nodes' at once,
//! found sooner by bodies within bodies (bodies.rs); a beam whose stress
//! passes its strength breaks, and what no longer holds to the ground
//! falls (`settle`).

// Small matrices and vectors by index read as the algebra does.
#![allow(clippy::needless_range_loop)]

pub mod bodies;
pub mod gather;
pub mod lattice;
pub mod model;
