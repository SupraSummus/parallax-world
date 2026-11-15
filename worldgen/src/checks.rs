//! Checks of worlds, for tests (docs/generators.md, "Checks"): the
//! contract every world keeps, each cell against what is in it (cells),
//! what a world is made of (survey), and
//! whether its layout follows the grids it is generated on (places) or the
//! lattice's directions (directions).

pub mod cells;
pub mod contract;
pub mod directions;
pub mod places;
pub mod survey;
