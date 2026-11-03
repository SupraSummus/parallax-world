# Testing

Vitest in Node,
with jsdom only in UI tests that opt in (`// @vitest-environment jsdom`),
and `cargo test` for the Rust in `bake/` and `worldgen/`.
Commands are in `package.json`:

```bash
pnpm test                       # build the WebAssembly, run both once (CI)
pnpm exec vitest                # watch mode (after pnpm wasm)
pnpm exec vitest -t "cache"     # filter by test name
pnpm exec vitest run bake       # files matching "bake"
```

## Layout

| File | Covers |
|------|--------|
| `src/lib/fcc.test.ts` | Voxel geometry |
| `src/lib/cells.test.ts`, `bake/src/cells.rs` | Cells of N·FCC and the baker's table of them |
| `src/lib/slice-selection.test.ts` | Slices, bricks and eyes; every site in exactly one slice |
| `src/lib/leak-bound.test.ts` | The occlusion-leak bound, step by step, then end to end against ray-cast perspective |
| `src/lib/chunk-baker.test.ts`, `bake/src/splats.rs` | Baked chunks' splats against rays cast from the eye |
| `bake/src/fill.rs`, `bake/src/coverage.rs` | Splats filled into a mask; solid blocks and their sums |
| `src/lib/occlusion.test.ts` | Skipping covered chunks leaves the picture unchanged |
| `src/lib/view.test.ts` | Camera basis, faces, projection, culling |
| `src/lib/voxel-grid.test.ts` | Storage on the lattice, `LayerIndex`, coarsening |
| `src/lib/renderer.test.ts` | Slices and chunks in view, placement, caching, stats (on a small world) |
| `src/lib/worker-bakery.test.ts` | Baking in workers (simulated) gives the same frames; what is baked first, and when not at all |
| `src/lib/renderer-regressions.test.ts` | Previously fixed rendering bugs |
| `src/lib/world.test.ts` | Toy worlds; a generated world's grids put together from parts generated alone |
| `worldgen/tests/contract.rs` | Every world generated in pieces equals it generated whole, at several levels |
| `worldgen/tests/forest.rs`, `src/lib/city.test.ts` | The scenes: determinism, contents, spawn |
| `src/lib/worldgen.test.ts` | The forest as the web gets it from WebAssembly |
| `src/viewer/mesh.test.ts` | Reference viewer mesh |
| `src/App.test.tsx` | UI panels, keyboard movement and turning |

`src/lib/picture.test-helper.ts` stands in for the compositor in tests that compare pictures,
and `src/lib/small-forest.test-helper.ts` for the forest where it would be too slow.
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
  samples (`forAll` in `leak-bound.test.ts`) so failures reproduce.
- Tag exports that only tests use `@internal`;
  `pnpm knip:production` checks that everything else is used outside tests.
- Visual output is not covered by unit tests; check it in the browser
  (`pnpm dev`), or without a GPU with `pnpm shot` (see AGENTS.md).
