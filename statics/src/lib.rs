//! Statics, an experiment (docs/statics.md): does what is built stand, and
//! what falls when it does not? Matter stays on the lattice; what falls is
//! dropped whole, not moved.
//!
//! The model (model.rs) is rigid voxels joined by springs on the faces they
//! share: each face carries a force and a moment, and breaks once the
//! stress at one of its corners, or its shear, passes its material's
//! strength. Its solve is equilibrium under weight; what is no longer joined
//! to held ground falls (`settle`). Voxels are those of 2^l·FCC
//! (lattice.rs), each face a rhombus of the rhombic dodecahedron.
//!
//! Coarser (coarse.rs): voxels gathered into rigid bodies, each a cell's
//! connected piece, the fine faces between them kept as they are. Loads
//! gather exactly (mass, centre); strength does not, as it is the weakest
//! section's, which a body hides inside it. So a body's inside is checked
//! alone under the forces on its boundary (`local_check`), and bodies are
//! best split where they are narrow.

// Small matrices and vectors by index read as the algebra does.
#![allow(clippy::needless_range_loop)]

pub mod coarse;
pub mod lattice;
pub mod model;
