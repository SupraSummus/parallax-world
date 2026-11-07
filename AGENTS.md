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
| [.claude/skills/lead/SKILL.md](.claude/skills/lead/SKILL.md) | The lead role: driving changes to main through delegated work and PRs | Only when the user asks for it (`/lead`) |

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
    ├── voxel-grid.ts        # Voxel storage on the FCC lattice, in bricks (typed arrays), a material per voxel face; LayerIndex, made by bake/
    ├── world.ts             # World: voxel grids by level of detail (generated in tiles around the camera), spawn; tests' scenes, a grid per level given
    ├── worldgen.ts          # The worlds of worlds/ (WebAssembly), any box at any level
    ├── cells.ts             # Cells of the coarser lattices N·FCC that slices are made of
    ├── slice-selection.ts   # Slices, chunks and the leak bound (pure functions)
    ├── view.ts              # Camera orientation, the 12 faces (FCC directions), projection
    ├── walk.ts              # Walking on the ground with bake/src/walk.rs (WebAssembly)
    ├── renderer.ts          # Slices and chunks in view, caching, screen placement (no DOM)
    ├── occlusion.ts         # Which screen cells nearer layers cover (bake/src/coverage.rs), to skip chunks behind them
    ├── chunk-baker.ts       # Bakes whole chunks with bake/ (WebAssembly); the bake worker's message protocol
    ├── wasm.ts              # The WebAssembly module (worlds/ with worldgen/ and bake/), inlined; an instance per use
    ├── worker-bakery.ts     # Pool of bake workers the renderer sends missing chunks to
    ├── bake.worker.ts       # Bake worker entry
    ├── compositor.ts        # Draws the renderer's frames with WebGL 2: chunks' splats as instanced prisms
    └── utils.ts             # cn() for class names
scripts/shot.mjs             # pnpm shot: a view in headless Chromium, saved once complete
bake/                        # Chunk baker, coverage and walking in Rust (no dependencies), in worlds/' WebAssembly
└── src/
    ├── lib.rs               # Baker: one chunk per call; the WebAssembly exports
    ├── world.rs             # Grid, tiles of grids, the view's faces' lattices, voxels read per face
    ├── cells.rs             # cells.ts, with a table per level
    ├── slicing.rs           # slice-selection.ts: slices, chunks, the leak bound (native's too)
    ├── surface.rs           # A cell's voxels that can show
    ├── tables.rs            # The voxel's faces in lattice coordinates, the materials shaded per view face
    ├── splats.rs            # Which voxels a chunk shows: each column's nearest, a hexagon at its site, in relief, over its run
    ├── fill.rs              # Splats filled into a mask for every slant, solid blocks (for coverage)
    ├── layer_index.rs       # LayerIndex: where each face's layers hold voxels; its WebAssembly exports
    ├── walk.rs              # Walking: a body that falls, collides, steps and jumps; its WebAssembly exports
    └── coverage.rs          # Which screen cells nearer layers cover, on the main thread; its WebAssembly exports
worldgen/                    # The world-generation engine, moving to integers, into the baker's grids; no worlds
├── src/
│   ├── js.rs                # Float math the same on every target (libm), named after JS's Math
│   ├── noise.rs, grid.rs    # Integer hashes, random numbers, fixed point (Fx) and noise; voxel-grid.ts's bricks
│   ├── units.rs             # UNIT_SIZE: metres per lattice unit; whole Units, typed apart from Fx
│   ├── generator.rs         # The framework: shapes, nodes, fields, populations, drawing any box at any level
│   ├── build.rs             # Short forms to write worlds with: constructors of fields, populations, nodes, solid shapes
│   ├── column.rs            # A heightfield's spans, each tall enough to keep a site at every level
│   ├── plan.rs              # Plans by column, each from coarser ones only; land domains bid for; claims, things kept apart anywhere
│   ├── curve.rs             # Curves: lines through space made top-down within narrowing corridors; where one passes nearest a point
│   ├── frame.rs             # Things built in frames of their own: quarter turns, mirrors, walls
│   ├── world.rs             # World: what a host needs to show one (generator, spawn, heights), the places it names
│   ├── picture.rs           # Pictures of a world at one level: rays through its voxels, as PNGs
│   ├── cost.rs              # What generating costs: work counted by layer, reported for a world's tiles, against a baseline
│   ├── places.rs            # Whether things prefer places: positions modulo periods, as on a grid, over seeds
│   ├── directions.rs        # Whether a world's directions follow the lattice's: four-fold harmonics of pairs' directions, over seeds
│   └── contract.rs          # The contract every world keeps, as checks for tests
└── tests/contract.rs        # The engine on test worlds: fog, caves, a village, knobs on a wall; what it refuses
worlds/                      # The worlds, on worldgen's public API only; the web's WebAssembly
├── src/
│   ├── lib.rs               # WORLDS: the registry hosts start worlds from by name, with the regions checked around their places
│   ├── forest.rs            # The forest, on ground it may share
│   ├── path.rs              # The forest's path along the valley: its centre line by z, and a curve through it
│   ├── town.rs, fields.rs   # Towns along the path, farmland around them: the valley's other domains
│   ├── landmarks.rs         # The valley's fourth domain: a keep, a watchtower, chapels, ponds, farmsteads, windmills
│   ├── valley.rs            # The valley: the five domains on one land
│   ├── mountains.rs         # The valley's ranges: massifs, crests, peaks and passes, side valleys; treeline, meadows, scree, rock, snow
│   ├── moor.rs              # The moor: heath, a stream or pools, steadings of a hut and a ring of stones; the valley's fifth domain
│   ├── city.rs              # The city: blocks of buildings whose rooms form a maze, boxes on a plan of quarter metres
│   ├── learned.rs           # The learned town: settlements drawn by a split grammar learned in lab/, on the forest's land
│   └── wasm.rs              # Its WebAssembly exports (the module holds bake/'s too)
├── data/town-grammar.bin    # learned.rs's rule table, written by lab's export example
├── data/cost/               # Each world's cost baseline, which tests/cost.rs holds it to
├── tests/                   # Every world in WORLDS keeps the contract and its cost baseline; the scenes' contents
└── examples/                # map: a world from above; look: a picture of it, of any place it names (picture.rs); cost: what its tiles cost (cost.rs); bench: timing a world's tiles
lab/                         # Experiment, not built by pnpm: hierarchical WFC rule-sets, pixel conditionals, split grammars with shared splits and regimes, learned from the valley; a masked model of any volume, flat, 3D or on the FCC lattice; synthesis by coordinates into examples; a critic and profiles that score any of them; real places (scripts/overture.py) drawn to a plan (see src/lib.rs); exports the learned town's rules
native/                      # Spike of a native frontend (egui on winit and wgpu), linking bake/ and worlds/; see src/main.rs
├── .cargo/config.toml       # Builds for x86-64-v3: floor and ceil inline (see there)
├── dump.test.ts             # Dumps a view from the TS renderer, to check it against (only with DUMP set)
└── src/
    ├── main.rs              # The camera (flying or walking, F) and panel; --shot, --bench, --walk, --stroll, --check-requests against a dump
    ├── window.rs            # The window: egui on winit and wgpu, its GPU device made again when lost
    ├── dump.rs              # Reads a dump: a view, its faces, grids and the chunks it wants
    ├── world.rs             # world.ts for generated worlds: a World's tiles, those the renderer wants first, made and indexed on threads
    ├── view.rs              # view.ts: faces, which rays and screen boxes they draw
    ├── renderer.rs          # renderer.ts: chunks in view, cached, which cells to bake as occlusion.rs found
    ├── occlusion.rs         # bake's Coverage on a thread of its own: which cells in view are covered, so not baked
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
`pnpm dev`, `pnpm build` and `pnpm test:web` rebuild its WebAssembly first.
CI runs the same scripts plus `pnpm build`, in jobs side by side.
For rendering changes, also look
at the result in the browser (`pnpm dev`); unit tests can't see pixels.
Without a GPU, `pnpm shot` saves a picture of a view once it is complete
(`scripts/shot.mjs`);
`pnpm shot:native --view QUERY --shot out.png` saves the native frontend's,
several times faster, though it still differs in places
([docs/todo.md](docs/todo.md)).
Both take the page's view query, e.g. `'seed=1&world=moor&look=30,-10'`,
and want Mesa's software Vulkan (`apt-get install mesa-vulkan-drivers`):
without it the native one cannot draw, the web's draws several times slower.

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
