# Testing

Vitest in Node,
with jsdom only in UI tests that opt in (`// @vitest-environment jsdom`),
and `cargo test` for the baker's Rust in `bake/`.
Commands are in `package.json`:

```bash
pnpm test                       # build the baker, run both once (CI)
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
| `src/lib/bake.test.ts` | Baked chunks pixel by pixel against rays cast from the eye |
| `bake/src/fill.rs`, `bake/src/coverage.rs` | Filling faces into pixels; solid blocks and their sums |
| `src/lib/occlusion.test.ts` | Skipping covered chunks leaves the picture unchanged |
| `src/lib/view.test.ts` | Camera basis, faces, projection, culling |
| `src/lib/voxel-grid.test.ts` | Storage on the lattice, `LayerIndex`, coarsening |
| `src/lib/renderer.test.ts` | Slices and chunks in view, placement, caching, stats (on a small world) |
| `src/lib/worker-bakery.test.ts` | Baking in workers (simulated) gives the same frames; what is baked first, and when not at all |
| `src/lib/renderer-regressions.test.ts` | Previously fixed rendering bugs |
| `src/lib/generator.test.ts` | Every world generated in pieces equals it generated whole, at several levels |
| `src/lib/forest.test.ts`, `src/lib/city.test.ts`, `src/lib/world.test.ts` | The scenes: determinism, budgets, spawn |
| `src/viewer/mesh.test.ts` | Reference viewer mesh |
| `src/App.test.tsx` | UI panels, keyboard movement and turning |

`src/lib/picture.test-helper.ts` stands in for the compositor in tests that compare pictures.
`src/dom.test-helper.ts`, imported by UI tests, stubs what jsdom lacks:
a no-op 2D canvas context
(no WebGL, so `compositor.ts` is only checked in the browser),
`ResizeObserver`, and a timer-based `requestAnimationFrame`.

## Conventions

- Assert observable behaviour (layer contents, frames, stats), not
  private fields. Every test must be able to fail: no `expect(true)` or
  `toBeGreaterThanOrEqual(0)` placeholders.
- To check what was drawn, read the chunks' pixels and placements, as
  `renderer-regressions.test.ts` does.
- Prefer one invariant test over a grid of inputs to many near-identical
  example tests. For a claim about all inputs, check it on seeded random
  samples (`forAll` in `leak-bound.test.ts`) so failures reproduce.
- Tag exports that only tests use `@internal`;
  `pnpm knip:production` checks that everything else is used outside tests.
- Visual output is not covered by unit tests; check it in the browser
  (`pnpm dev`).
