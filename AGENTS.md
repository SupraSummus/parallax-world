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
    ├── voxel-grid.ts        # Voxel storage on the FCC lattice, in bricks (typed arrays); coarsening it; LayerIndex, made by bake/
    ├── world.ts             # World: voxel grids by level of detail (generated in tiles around the camera), spawn, toy world types
    ├── worldgen.ts          # The worlds of worlds/ (WebAssembly), any box at any level
    ├── city.ts              # City scene generator: streets, buildings, maze of rooms
    ├── noise.ts             # Seeded hash and PRNG for the city
    ├── cells.ts             # Cells of the coarser lattices N·FCC that slices are made of
    ├── slice-selection.ts   # Slices, chunks and the eyes they are baked from (pure functions)
    ├── view.ts              # Camera orientation, the 12 faces (FCC directions), projection
    ├── renderer.ts          # Slices and chunks in view, caching, screen placement (no DOM)
    ├── occlusion.ts         # Which screen cells nearer layers cover (bake/src/coverage.rs), to skip chunks behind them
    ├── chunk-baker.ts       # Bakes whole chunks with bake/ (WebAssembly); the bake worker's message protocol
    ├── wasm.ts              # The WebAssembly module (worlds/ with worldgen/ and bake/), inlined; an instance per use
    ├── worker-bakery.ts     # Pool of bake workers the renderer sends missing chunks to
    ├── bake.worker.ts       # Bake worker entry
    ├── compositor.ts        # Draws the renderer's frames with WebGL 2: chunks' splats, instanced
    └── utils.ts             # cn() for class names
viewer.html                  # Reference 3D viewer page (second Vite entry)
scripts/shot.mjs             # pnpm shot: a view in headless Chromium, saved once complete
src/viewer/
├── main.ts                  # three.js scene, orbit camera
└── mesh.ts                  # World -> triangle mesh of exposed rhombic faces
bake/                        # Chunk baker and coverage in Rust (no dependencies), in worlds/' WebAssembly
└── src/
    ├── lib.rs               # Baker: cached cell surfaces, one chunk per call; the WebAssembly exports
    ├── world.rs             # Grid, tiles of grids, the view's faces' lattices, voxels read per face
    ├── cells.rs             # cells.ts, with a table per level
    ├── slicing.rs           # The parts of slice-selection.ts baking needs
    ├── surface.rs           # A cell's voxels that can show
    ├── tables.rs            # The voxel's faces in lattice coordinates, the materials shaded per view face
    ├── splats.rs            # Which voxels a chunk shows, as hexagons on its centre plane; gap filling
    ├── fill.rs              # Splats filled into a mask, solid blocks (for coverage)
    ├── layer_index.rs       # LayerIndex: where each face's layers hold voxels; its WebAssembly exports
    └── coverage.rs          # Which screen cells nearer layers cover, on the main thread; its WebAssembly exports
worldgen/                    # The world-generation engine, moving to integers, into the baker's grids; no worlds
├── src/
│   ├── js.rs                # Float math the same on every target (libm), named after JS's Math
│   ├── noise.rs, grid.rs    # Integer hashes, random numbers, fixed point (Fx) and noise; voxel-grid.ts's bricks
│   ├── units.rs             # UNIT_SIZE: metres per lattice unit; whole Units, typed apart from Fx
│   ├── generator.rs         # The framework: shapes, nodes, fields, populations, drawing any box at any level
│   ├── build.rs             # Short forms to write worlds with: constructors of fields, populations, nodes, solid shapes
│   ├── column.rs            # A heightfield's spans, each tall enough to keep a site at every level
│   ├── plan.rs              # Plans by column, each from coarser ones only; land domains bid for
│   ├── frame.rs             # Things built in frames of their own: quarter turns, mirrors, walls
│   ├── world.rs             # World: what a host needs to show one (generator, spawn, reach, heights)
│   ├── picture.rs           # Pictures of a world at one level: rays through its voxels, as PNGs
│   └── contract.rs          # The contract every world keeps, as checks for tests
└── tests/contract.rs        # The engine on test worlds: fog, caves, a village; what it refuses
worlds/                      # The worlds, on worldgen's public API only; the web's WebAssembly
├── src/
│   ├── lib.rs               # WORLDS: the registry hosts start worlds from by name, with the regions checked
│   ├── forest.rs            # The forest, on ground it may share
│   ├── town.rs, fields.rs   # Towns along the path, farmland around them: the valley's other domains
│   ├── valley.rs            # The valley: the three domains on one land
│   ├── moor.rs              # The moor: heath, a stream, steadings of a hut and a ring of stones
│   ├── learned.rs           # The learned town: settlements drawn by a split grammar learned in lab/, on the forest's land
│   └── wasm.rs              # Its WebAssembly exports (the module holds bake/'s too)
├── data/town-grammar.bin    # learned.rs's rule table, written by lab's export example
├── tests/                   # Every world in WORLDS keeps the contract; the scenes' contents
└── examples/                # map: a world from above; look: a picture of it (picture.rs); forest_bench: timing forest tiles
lab/                         # Experiment, not built by pnpm: hierarchical WFC rule-sets, pixel conditionals, split grammars with shared splits and regimes, learned from the valley (see src/lib.rs); exports the learned town's rules
native/                      # Spike of a native frontend (eframe + wgpu), linking bake/ and worlds/; see src/main.rs
├── .cargo/config.toml       # Builds for x86-64-v3: floor and ceil inline (see there)
├── dump.test.ts             # Dumps a view from the TS renderer, to check it against (only with DUMP set)
└── src/
    ├── main.rs              # The window and camera; --shot, --bench, --walk, --check-requests against a dump
    ├── dump.rs              # Reads a dump: a view, its faces, grids and the chunks it wants
    ├── world.rs             # world.ts for generated worlds: a World's tiles, those the renderer wants first, made and indexed on threads
    ├── slicing.rs           # slice-selection.ts: slices, chunks, eyes
    ├── view.rs              # view.ts: faces, which rays and screen boxes they draw
    ├── renderer.rs          # renderer.ts: chunks in view, cached, stand-ins, coverage (bake's Coverage)
    ├── bakery.rs            # Worker threads baking chunks, a baker each sharing the tiles, and clustering their splats for the GPU
    ├── compositor.rs        # compositor.ts with wgpu: splats uploaded per chunk in clusters, culled on the GPU against what showed
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

`pnpm lint` and `pnpm test` cover the Rust
in `bake/`, `worldgen/` and `worlds/` too
(clippy with warnings as errors, rustfmt, `cargo test`);
`pnpm dev`, `pnpm build` and `pnpm test` rebuild its WebAssembly first.
CI runs the same scripts plus `pnpm build`. For rendering changes, also look
at the result in the browser (`pnpm dev`); unit tests can't see pixels.
Without a GPU, `pnpm shot` saves a picture of a view once it is complete
(`scripts/shot.mjs`);
with `mesa-vulkan-drivers` installed it draws several times faster.

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
