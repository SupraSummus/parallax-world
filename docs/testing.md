# Testing

Vitest in Node,
with jsdom only in UI tests that opt in (`// @vitest-environment jsdom`),
and `cargo test` for the Rust in `bake/`, `worldgen/`, `worlds/` and `native/`.
Commands are in `package.json`:

```bash
pnpm test                       # run both once: pnpm test:rust, then test:web
pnpm exec vitest                # watch mode (after pnpm wasm)
pnpm exec vitest -t "cache"     # filter by test name
pnpm exec vitest run bake       # files matching "bake"
```

## Layout

| File | Covers |
|------|--------|
| `src/lib/cells.test.ts`, `bake/src/cells.rs` | Cells of N·FCC and the baker's table of them |
| `src/lib/slice-selection.test.ts`, `bake/src/slicing.rs` | Slices, bricks and eyes; every site in exactly one slice; slicings paired as they re-tile |
| `bake/src/lod.rs` | Native's cells: every site drawn first at the finest level claiming it; no ray meets a finer level after a coarser |
| `src/lib/relief.test.ts` | Slices drawn in relief against ray-cast perspective: what they leak by detail |
| `src/lib/chunk-baker.test.ts`, `bake/src/splats.rs` | Baked chunks' splats against rays cast along the axis and, in relief, slanted |
| `bake/src/fill.rs`, `bake/src/coverage.rs` | Splats filled into a mask; solid blocks and their sums |
| `src/lib/occlusion.test.ts`, `native/src/main.rs` | Skipping covered chunks and clusters leaves the picture unchanged (natively, culling on the GPU) |
| `src/lib/pillars.test.ts` | Each column a pillar over its run shows a lawn without the soil under it; without runs, stripes |
| `src/lib/view.test.ts`, `native/src/view.rs` | Camera basis, faces, projection, culling |
| `native/src/renderer.rs` | Moving keeps most chunks |
| `native/src/main.rs` | What the GPU's buffers cannot hold is left out, the farthest first |
| `bake/src/walk.rs`, `worlds/tests/forest.rs` | Walking: the ground, steps, walls, jumps; the forest's path through its ferns |
| `src/lib/voxel-grid.test.ts` | Storage on the lattice, `LayerIndex` |
| `src/lib/renderer.test.ts` | Slices and chunks in view, placement, caching, stats (on a small world) |
| `src/lib/worker-bakery.test.ts` | Baking in workers (simulated) gives the same frames; what is baked first, and when not at all |
| `src/lib/renderer-regressions.test.ts` | Previously fixed rendering bugs |
| `src/lib/world.test.ts` | The worlds listed; a generated world's grids put together from parts generated alone |
| `worlds/tests/contract.rs`, `worldgen/tests/contract.rs`, `worldgen/tests/shapes.rs` | Every world, the engine's test worlds and each solid keep the contract (`worldgen/src/contract.rs`): generated in pieces equals generated whole, levels agree on what rays see, and seen from above with level 0 |
| `worlds/tests/cost.rs` | What generating each world costs, counted, stays within a quarter of its baseline (`worlds/data/cost/`; "Cost" in `docs/generators.md`) |
| `worlds/src/railway.rs` | The valley's railway keeps its grade and its side of the valley, out of towns; it bends gently; it has bridges and tunnels, none a stub |
| `worldgen/src/points.rs` | Points: a cell holds its children's, whichever is asked first; they stand where their mark has a chance, as often as it is; a blur is its cells' counts weighed, wherever its blocks fall |
| `worlds/tests/places.rs` | No world's things prefer places on its grids (`worldgen/src/places.rs`), but those known to (`KNOWN`, docs/todo.md) |
| `worlds/tests/directions.rs`, `worldgen/src/directions.rs` | No world's directions follow the lattice's, but around the places known to (`KNOWN`, docs/todo.md); walls along the axes or the diagonals do; hills, gentle or steep, rows along z, and walls turned any way a seed likes don't |
| `worlds/tests/forest.rs`, `worlds/tests/valley.rs`, `worlds/tests/city.rs` | The scenes: determinism, contents, spawn |
| `src/lib/worldgen.test.ts` | The worlds as the web lists and gets them from WebAssembly |
| `src/App.test.tsx` | UI panels, keyboard movement and turning, walking |

`src/lib/picture.test-helper.ts` stands in for the compositor in tests that compare pictures,
`src/lib/small-forest.test-helper.ts` for the forest where it would be too slow,
and `src/lib/hills.test-helper.ts` is a small scene with trees, a world of voxels, for the renderer's tests.
`src/lib/scene.test-helper.ts` makes such worlds of hand-made voxels,
their coarser levels sampled at their sites.
`src/dom.test-helper.ts`, imported by UI tests, stubs what jsdom lacks:
a no-op 2D canvas context
(no WebGL, so `compositor.ts` is only checked in the browser),
`ResizeObserver`, and a timer-based `requestAnimationFrame`.

## Conventions

- Assert observable behaviour (layer contents, frames, stats), not
  private fields. Every test must be able to fail: no `expect(true)` or
  `toBeGreaterThanOrEqual(0)` placeholders.
- To check what was drawn, rasterise the chunks (`rasterise` in
  `picture.test-helper.ts`) and read their placements, as
  `renderer-regressions.test.ts` does.
- Prefer one invariant test over a grid of inputs to many near-identical
  example tests. For a claim about all inputs, check it on seeded random
  samples so failures reproduce.
- The worlds' checks list their known exceptions
  (`KNOWN_BREAKS`, `OVER_BUDGET`, `KNOWN`) and fail both ways:
  on a new failure, and on a listed one that now passes, to take it off.
  Change a list only with the reason, in the commit or docs/todo.md;
  never pick seeds, places or points to dodge a check.
- Tag exports that only tests use `@internal`;
  `pnpm knip:production` checks that everything else is used outside tests.
- Pixels are checked only by native tests that draw on the GPU.
  They need Vulkan (Mesa's software one will do),
  so `pnpm test` skips them (`#[ignore]`) and CI's `native-gpu` job runs them
  (`cargo test` in `native/` with `-- --ignored`).
  Otherwise look in the browser (`pnpm dev`),
  or without a GPU with `pnpm shot` (see AGENTS.md).
