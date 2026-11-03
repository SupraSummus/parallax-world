//! World generation: the generator framework and the forest, into grids
//! the baker reads, moving to integers (docs/generators.md). The native
//! frontend links it; the web generates with it as WebAssembly (wasm.rs,
//! src/lib/worldgen.ts).

pub mod forest;
pub mod generator;
pub mod grid;
pub mod js;
pub mod noise;
pub mod wasm;
