//! Statics, an experiment (docs/statics.md): does what is built stand, and
//! what falls when it does not? Matter stays on the lattice; what falls is
//! taken off it whole.
//!
//! Voxels of 2^l·FCC (lattice.rs) are rigid, joined by bonds on the faces
//! they share, each a spring with a share of its face's area (model.rs).
//! The forces are all voxels' at once, found sooner by bodies within
//! bodies (bodies.rs); a face whose stress passes its material's strength
//! breaks, and what no longer holds to the ground falls (`settle`).

// Small matrices and vectors by index read as the algebra does.
#![allow(clippy::needless_range_loop)]

pub mod bodies;
pub mod lattice;
pub mod model;
