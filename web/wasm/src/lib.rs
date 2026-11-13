//! The web's WebAssembly module, its only one (wasm.ts): the exports the
//! page calls, each module those of one of its files, over the crates
//! native links as they are.

mod baker;
mod coverage;
mod index;
mod walk;
mod worlds;
