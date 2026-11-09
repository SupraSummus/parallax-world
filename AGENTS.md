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
    ├── slice-selection.ts   # Slices and chunks (pure functions)
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
scripts/voxelize.py          # A glTF scene as voxels of the FCC lattice, for worlds/src/scene.rs (numpy, Pillow)
bake/                        # Chunk baker, coverage and walking in Rust (no dependencies), in worlds/' WebAssembly
└── src/
    ├── lib.rs               # Baker: one chunk per call; the WebAssembly exports
    ├── world.rs             # Grid, tiles of grids, the view's faces' lattices, voxels read per face
    ├── cells.rs             # cells.ts, with a table per level
    ├── slicing.rs           # slice-selection.ts's slices' shapes and chunks, for the web's baker
    ├── surface.rs           # A cell's voxels that can show
    ├── tables.rs            # The voxel's faces in lattice coordinates, the materials shaded per view face
    ├── splats.rs            # Which voxels a chunk shows, each a hexagon at its site: of a slice, each column's nearest over its run; of a cell, each a ray can enter
    ├── light.rs             # The light from voxels' faces, as hosts that light a world hand it over
    ├── lod.rs               # Native's chunks: whole cells of the lattice, each face's levels by depth along it
    ├── fill.rs              # Splats filled into a mask for every slant, solid blocks (for coverage)
    ├── layer_index.rs       # LayerIndex: where each face's layers hold voxels; its WebAssembly exports
    ├── walk.rs              # Walking: a body that falls, collides, steps and jumps; its WebAssembly exports
    └── coverage.rs          # Which screen cells nearer layers cover, on the main thread; its WebAssembly exports
worldgen/                    # The world-generation engine, moving to integers, into the baker's grids; no worlds
├── src/
│   ├── js.rs                # Float math the same on every target (libm), named after JS's Math
│   ├── noise.rs, grid.rs    # Integer hashes, random numbers, fixed point (Fx) and noise; voxel-grid.ts's bricks, each site's alpha, and the voxels it shows
│   ├── units.rs             # UNIT_SIZE: metres per lattice unit; whole Units, typed apart from Fx
│   ├── generator.rs         # The framework: shapes, nodes, fields, populations, drawing any box at any level
│   ├── build.rs             # Short forms to write worlds with: constructors of fields, populations, nodes, solid shapes
│   ├── column.rs            # A heightfield's spans, each tall enough to keep a site at every level
│   ├── plan.rs              # Plans by column, each from coarser ones only; land domains bid for; claims, things kept apart anywhere
│   ├── curve.rs             # Curves: lines through space made top-down within narrowing corridors; where one passes nearest a point
│   ├── points.rs            # Points: where things are, drawn top down, so a cell of any size knows how many it holds; blurs of their counts
│   ├── frame.rs             # Things built in frames of their own: quarter turns, mirrors, walls
│   ├── world.rs             # World: what a host needs to show one (generator, spawn, heights), the places it names
│   ├── picture.rs           # Pictures of a world at one level: rays through its voxels, as PNGs, or through its sites' alpha
│   ├── light.rs             # Light (a prototype, for look and native's --light): sky, sun and what glows, each level's from coarser levels'; held to light/reference.rs, rays traced through level 0
│   ├── cost.rs              # What generating costs: work counted by layer, reported for a world's tiles, against a baseline
│   ├── places.rs            # Whether things prefer places: positions modulo periods, as on a grid, over seeds
│   ├── directions.rs        # Whether a world's directions follow the lattice's: four-fold harmonics of pairs' directions, over seeds
│   ├── survey.rs            # What a world is made of: its layers asked around spawn, things taken apart, strata to check
│   └── contract.rs          # The contract every world keeps, as checks for tests, in regions holding every stratum: rays see each site by its alpha, its colour and glow
├── data/light-reference.txt # The reference's light in light.rs's test scenes, written by its test reference_values
└── tests/contract.rs        # The engine on test worlds: fog, caves, a village, knobs on a wall; what it refuses
worlds/                      # The worlds, on worldgen's public API only; the web's WebAssembly
├── src/
│   ├── lib.rs               # WORLDS: the registry hosts start worlds from by name, with the regions checked around their places
│   ├── forest.rs            # The forest, on ground it may share
│   ├── path.rs              # The forest's path along the valley: its centre line by z, and a curve through it
│   ├── town.rs, fields.rs   # Towns along the path, farmland around them: the valley's other domains
│   ├── landmarks.rs         # The valley's fourth domain: a keep, a watchtower, chapels, ponds, farmsteads, windmills
│   ├── valley.rs            # The valley: the six domains on one land
│   ├── mountains.rs         # The valley's ranges: massifs, crests, peaks and passes, side valleys; treeline, meadows, scree, rock, snow
│   ├── moor.rs              # The moor: heath, a stream or pools, steadings of a hut and a ring of stones; the valley's fifth domain
│   ├── railway.rs           # The valley's railway: a line along it made by cost, cut, filled, bridged and tunnelled; stations by towns
│   ├── city.rs              # The city: blocks of buildings whose rooms form a maze, boxes on a plan of quarter metres
│   ├── learned.rs           # The learned town: settlements drawn by a split grammar learned in lab/, on the forest's land
│   ├── scene.rs             # Scenes made elsewhere: voxels read from a file (scripts/voxelize.py); lib.rs adds Sponza, with its feature
│   └── wasm.rs              # Its WebAssembly exports (the module holds bake/'s too)
├── data/town-grammar.bin    # learned.rs's rule table, written by lab's export example
├── data/scenes/             # Scenes' voxels, not in git (their licences): see docs/generators.md, "Scenes"
├── data/cost/               # Each world's cost baseline, which tests/cost.rs holds it to
├── tests/                   # Every world in WORLDS keeps the contract and its cost baseline; the scenes' contents
└── examples/                # map: a world from above; look: a picture of it, of any place it names (picture.rs), lit (light.rs); cost: what its tiles cost (cost.rs); bench: timing a world's tiles
lab/                         # Experiment, not built by pnpm: split grammars with shared splits, learned from the valley's towns; a masked model of any volume, flat, 3D or on the FCC lattice; synthesis by coordinates into examples; a critic and profiles that score any of them; real places (scripts/overture.py) drawn to a plan (see src/lib.rs); exports the learned town's rules
native/                      # Spike of a native frontend (egui on winit and wgpu), linking bake/ and worlds/; see src/main.rs
├── .cargo/config.toml       # Builds for x86-64-v3: floor and ceil inline (see there)
└── src/
    ├── main.rs              # The camera (flying or walking, F) and panel; --shot, --bench, --stroll
    ├── window.rs            # The window: egui on winit and wgpu, its GPU device made again when lost
    ├── world.rs             # world.ts for generated worlds: a World's tiles, those the renderer wants first, made and indexed on threads
    ├── view.rs              # view.ts: faces, which rays and screen boxes they draw
    ├── renderer.rs          # Chunks in view: cells of the lattice (bake's lod.rs), not slices; cached, finest and nearest first
    ├── bakery.rs            # Worker threads baking chunks, a baker each sharing the tiles (lit, with a Light each), and clustering their splats for the GPU
    ├── compositor.rs        # compositor.ts with wgpu, for cells: splats uploaded per chunk in clusters, by depth, culled on the GPU against what showed; see-through by chance or blended
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
in `bake/`, `worldgen/`, `worlds/` and `native/` too
(clippy with warnings as errors, rustfmt, `cargo test`);
`pnpm dev`, `pnpm build` and `pnpm test:web` rebuild its WebAssembly first.
CI runs the same scripts plus `pnpm build`, in jobs side by side,
and `native/`'s tests that draw, on Mesa's software Vulkan.
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

0. **Simplicity first.** README.md's design principles open with Hoare's
   motto: make it so simple that there are obviously no deficiencies.
   Judge every change by it, and prefer deleting to adding.
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
