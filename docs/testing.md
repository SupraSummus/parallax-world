# Testing

`cargo test` for the workspace's crates (`Cargo.toml`).
Commands are in `package.json`:

```bash
pnpm test                       # every crate's tests, but the worlds' checks over seeds
pnpm test:worlds                # every world's contract, directions and places
cargo test --release -p parallax-native world::   # one crate's, by name
```

`pnpm test` leaves out the worlds' checks over seeds
(`worlds/tests/contract.rs`, `directions.rs`, `places.rs`),
most of the time testing takes;
only `worldgen/`, `worlds/` and `fcc/` change what they check.
`pnpm test:worlds` runs them, as CI always does;
one world's: `cargo test --release -p worlds --test contract valley`.

## Layout

| File | Covers |
|------|--------|
| `bake/src/cells.rs` | Cells of N·FCC and the baker's table of them |
| `bake/src/lod.rs` | Cells: every site drawn first at the finest level claiming it; no ray meets a finer level after a coarser |
| `native/src/engine/chunks.rs` | Culling on the GPU leaves the picture unchanged |
| `native/src/view.rs` | Camera basis, faces, projection, culling |
| `native/src/engine/chunks.rs` | Moving keeps most chunks |
| `native/src/world.rs` | A world on one thread is as one on some; `Level::meets`: every cell holding a voxel, or a see-through one, and of 8·FCC, 32·FCC and smaller no other |
| `native/src/engine/chunks/renderer.rs` | Cells said to hold nothing that shows, none of their voxels meeting them or all buried, bake empty |
| `native/src/engine/chunks/occlusion.rs`, `native/src/engine/chunks.rs` | Rays hide what is wholly behind where they stop and nothing else, cast as far as what asks is and on from there, those that wait going on once told to; hiding cells on the CPU draws fewer chunks, and once its probes' cells are in, leaves the picture unchanged |
| `native/src/engine/chunks.rs` | What the GPU's buffers cannot hold is left out, the farthest first |
| `native/src/engine/chunks/compositor.rs`, `native/src/engine/voxels.rs` | Their shaders valid as wgpu would find them, without a GPU (naga); the structs they share laid out alike in WGSL and Rust |
| `fcc/src/lib.rs` | Tiles hold their cubes alike along every axis; a table of tiles in blocks holds what a map would, and a copy changes alone; a part of a grid holds its box's sites as the grid does; each face reads the look through the voxels' faces towards it |
| `fcc/src/walk.rs`, `worlds/tests/forest.rs` | Walking: the ground, steps, walls, jumps; the forest's path through its ferns |
| `worlds/tests/contract.rs`, `worldgen/tests/contract.rs`, `worldgen/tests/shapes.rs` | Every world, the engine's test worlds and each solid keep the contract (`worldgen/src/checks/contract.rs`): generated in pieces equals generated whole; each stratum's cells look on average as what is in them two levels finer, or as their baseline says they don't (`worlds/data/contract/`, `worldgen/data/contract/`; `docs/checks.md`); each solid in the worst case |
| `worlds/tests/cost.rs` | What generating each world costs, counted, stays within a quarter of its baseline (`worlds/data/cost/`; "Cost" in `docs/checks.md`) |
| `worlds/src/path.rs` | Whether a column is by a path agrees with its distance to the path's curve, round a steep, fast-bending spine |
| `worlds/src/valley/railway.rs` | The valley's railway keeps its grade and its band of the valley, out of towns; it bends gently; few bridges and tunnels, none a stub, one through each spur and over each gorge on its side |
| `worldgen/src/plan.rs` | Plans: finer ones refused, slopes kept, claims apart; a column read while threads keep others is whole or not found |
| `worldgen/src/points.rs` | Points: a cell holds its children's, whichever is asked first; they stand where their mark has a chance, as often as it is; a blur is its cells' counts weighed, wherever its blocks fall |
| `worlds/tests/places.rs` | No world's things prefer places on its grids (`worldgen/src/checks/places.rs`), but those known to (`KNOWN`, docs/todo.md) |
| `worlds/tests/directions.rs`, `worldgen/src/checks/directions.rs` | No world's directions follow the lattice's, but around the places known to (`KNOWN`, docs/todo.md); walls along the axes or the diagonals do; hills, gentle or steep, rows along z, and walls turned any way a seed likes don't |
| `worlds/tests/forest.rs`, `worlds/tests/valley.rs`, `worlds/tests/city.rs` | The scenes: determinism, contents, spawn; threads sharing a world make its tiles as one alone |

## Conventions

- Assert observable behaviour (what is drawn, frames, what is made), not
  private fields. Every test must be able to fail.
- Prefer one invariant test over a grid of inputs to many near-identical
  example tests. For a claim about all inputs, check it on seeded random
  samples so failures reproduce.
- The worlds' checks list their known exceptions
  (`OVER_BUDGET`, `KNOWN`) and fail both ways:
  on a new failure, and on a listed one that now passes, to take it off.
  Change a list only with the reason, in the commit or docs/todo.md;
  never pick seeds, places or points to dodge a check.
- Pixels are checked only by `native/`'s tests that draw on the GPU.
  They need Vulkan (Mesa's software one will do),
  so `pnpm test` skips them (`#[ignore]`) and CI's `native-gpu` job runs them
  (`cargo test --release -p parallax-native -- --ignored`).
  With Mesa 26 (LLVM 21) on a CPU with AVX-512,
  give them `GALLIUM_OVERRIDE_CPU_CAPS=avx`, as CI does
  ([docs/todo.md](todo.md)).
  Otherwise look in the window or the browser (`pnpm dev`),
  or at a picture saved by `pnpm shot` (see AGENTS.md).
