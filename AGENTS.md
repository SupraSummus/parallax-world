# Agent Development Guide

Entry point for AI agents working on this codebase. Read this first.

## Documents

| Document | Purpose | Read when |
|----------|---------|-----------|
| [README.md](README.md) | Overview and quick start | First |
| [docs/product.md](docs/product.md) | What the product does and doesn't do | Adding or changing features |
| [docs/architecture.md](docs/architecture.md) | Slicing, caching and compositing architecture | Touching `src/lib` |
| [docs/testing.md](docs/testing.md) | Test layout and conventions | Writing tests |
| [docs/todo.md](docs/todo.md) | Planned work: known problems and open ideas | Looking for what to do next |
| [docs/generators.md](docs/generators.md) | Generating worlds in pieces: layers, things, rules | Writing or changing a world generator |

Each concept has one home:
build/test commands live in `package.json` scripts,
architecture in `docs/architecture.md`,
product behaviour in `docs/product.md`,
test conventions in `docs/testing.md`,
planned work in `docs/todo.md`,
agent rules here.
Link instead of duplicating.
Docs go in `docs/` with lower-case names;
only conventional files (README, AGENTS, LICENSE) stay at the root.

## Source layout

```
src/
├── main.tsx                 # Entry: error boundary + App
├── App.tsx                  # UI panels, keyboard movement loop
├── ErrorFallback.tsx
├── main.css                 # Tailwind + design tokens
├── components/
│   └── ui/                  # shadcn/ui primitives (generated; CLI config in components.json)
└── lib/
    ├── fcc.ts               # Voxel geometry: rhombic dodecahedra of the FCC lattice
    ├── voxel-grid.ts        # Voxel storage on the FCC lattice, in bricks (typed arrays); coarsening it
    ├── world.ts             # World: voxel grids by level of detail (generated in tiles around the camera), spawn, toy world types
    ├── worldgen.ts          # The forest of worldgen/ (WebAssembly), any box at any level
    ├── city.ts              # City scene generator: streets, buildings, maze of rooms
    ├── noise.ts             # Seeded hash and PRNG for the city
    ├── cells.ts             # Cells of the coarser lattices N·FCC that slices are made of
    ├── slice-selection.ts   # Slices, chunks and the eyes they are baked from (pure functions)
    ├── view.ts              # Camera orientation, the 12 faces (FCC directions), projection
    ├── renderer.ts          # Slices and chunks in view, caching, screen placement (no DOM)
    ├── occlusion.ts         # Which screen cells nearer layers cover (bake/src/coverage.rs), to skip chunks behind them
    ├── chunk-baker.ts       # Bakes whole chunks with bake/ (WebAssembly); the bake worker's message protocol
    ├── wasm.ts              # The WebAssembly module (worldgen/ with bake/), inlined; an instance per use
    ├── worker-bakery.ts     # Pool of bake workers the renderer sends missing chunks to
    ├── bake.worker.ts       # Bake worker entry
    ├── compositor.ts        # Draws the renderer's frames with WebGL 2: chunks' splats, instanced
    ├── bake.ts              # Voxel face geometry and shaded palettes handed to the baker
    └── utils.ts             # cn() for class names
viewer.html                  # Reference 3D viewer page (second Vite entry)
src/viewer/
├── main.ts                  # three.js scene, orbit camera
└── mesh.ts                  # World -> triangle mesh of exposed rhombic faces
bake/                        # Chunk baker and coverage in Rust (no dependencies), in worldgen/'s WebAssembly
└── src/
    ├── lib.rs               # Baker: cached cell surfaces, one chunk per call; the WebAssembly exports
    ├── world.rs             # Grid, tiles of grids, and voxel faces as handed over, read per face
    ├── cells.rs             # cells.ts, with a table per level
    ├── slicing.rs           # The parts of slice-selection.ts baking needs
    ├── surface.rs           # A cell's voxels that can show
    ├── splats.rs            # Which voxels a chunk shows, as hexagons on its centre plane; gap filling
    ├── fill.rs              # Splats filled into a mask, solid blocks (for coverage)
    └── coverage.rs          # Which screen cells nearer layers cover, on the main thread; its WebAssembly exports
worldgen/                    # World generation, moving to integers, into the baker's grids; the web's WebAssembly
└── src/
    ├── js.rs                # Float math the same on every target (libm), named after JS's Math
    ├── noise.rs, grid.rs    # Integer hashes, random numbers, fixed-point noise; voxel-grid.ts's bricks
    ├── generator.rs         # The framework: shapes, nodes, fields, populations, drawing any box at any level
    ├── forest.rs            # The forest (UNIT_SIZE lives here)
    └── wasm.rs              # Its WebAssembly exports (the module holds bake/'s too)
native/                      # Spike of a native frontend (eframe + wgpu), linking bake/; see src/main.rs
├── .cargo/config.toml       # Builds for x86-64-v3: floor and ceil inline (see there)
├── dump.test.ts             # Dumps a view from the TS renderer, to check it against (only with DUMP set)
└── src/
    ├── main.rs              # The window and camera; --shot, --bench, --walk, --check-requests against a dump
    ├── dump.rs              # Reads a dump: the baker's tables, a view, its grids and the chunks it wants
    ├── tables.rs            # bake.ts: the voxel geometry and shaded palettes the bakers are given
    ├── world.rs             # world.ts for generated worlds: worldgen's tiles, those the renderer wants first, made and indexed on threads
    ├── slicing.rs           # slice-selection.ts: slices, chunks, eyes
    ├── view.rs              # view.ts: faces, which rays and screen boxes they draw
    ├── layer_index.rs       # LayerIndex (voxel-grid.ts): where each face's layers hold voxels
    ├── renderer.rs          # renderer.ts: chunks in view, cached, stand-ins, coverage (bake's Coverage)
    ├── bakery.rs            # Worker threads baking chunks, a baker each sharing the tiles
    ├── compositor.rs        # compositor.ts with wgpu: splats uploaded per chunk, placed relative to the camera
    ├── shot.rs              # --shot: one frame drawn offscreen on the GPU, saved as a PNG, no window
    └── compositor.wgsl
```

## Workflow

Before and after changing code, run the `package.json` scripts and fix
errors (lint warnings are advisory):

```bash
pnpm check   # lint, typecheck, knip, knip:production, then test
```

`pnpm test` takes most of that time.
While iterating, run only the affected test files
([docs/testing.md](docs/testing.md));
run `pnpm check` again only after changing code.

`pnpm lint` and `pnpm test` cover the Rust in `bake/` and `worldgen/` too
(clippy with warnings as errors, rustfmt, `cargo test`);
`pnpm dev`, `pnpm build` and `pnpm test` rebuild its WebAssembly first.
CI runs the same scripts plus `pnpm build`. For rendering changes, also look
at the result in the browser (`pnpm dev`); unit tests can't see pixels.

## Rules

1. **No dangling files.** Every file is imported, referenced by config, or
   linked from this file or README.md. When removing a file, remove its
   references. `knip` enforces this for code.
2. **Minimal docs.** Prefer code comments for implementation details. Delete
   outdated docs rather than letting them mislead. When changing code, grep
   the docs for mentions and update them.
   Don't restate what the code says (constants, step-by-step logic):
   name it and say why.
   Measured numbers go stale; give one only when the point needs it.
3. **Architecture changes** update `docs/architecture.md` and the affected
   tests.
4. **TypeScript strict** (`strictTypeChecked` ESLint rules). Follow existing
   patterns in neighbouring files.
5. **Semantic line breaks in Markdown** ([sembr.org](https://sembr.org/)).
   One sentence per line,
   and break after a clause where it helps (comma, semicolon, colon, dash).
   Keep lines under about 80 characters, except for links and code.
   Apply it lazily: to paragraphs you write or edit, no mass reformatting.
