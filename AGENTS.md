# Agent Development Guide

Entry point for AI agents working on this codebase. Read this first.

## Documents

| Document | Purpose | Read when |
|----------|---------|-----------|
| [README.md](README.md) | Overview and quick start | First |
| [docs/product.md](docs/product.md) | What the product does and doesn't do | Adding or changing features |
| [docs/architecture.md](docs/architecture.md) | Slicing, caching and compositing architecture; the crates | Touching `web/src/lib` or the Rust |
| [docs/testing.md](docs/testing.md) | Test layout and conventions | Writing tests |
| [docs/todo.md](docs/todo.md) | Planned work: known problems and open ideas | Looking for what to do next |
| [docs/generators.md](docs/generators.md) | Generating worlds in pieces: layers, things, rules | Writing or changing a world generator |
| [docs/light.md](docs/light.md) | Light: what the prototype gets wrong, what was tried, what it costs | Touching `worldgen/src/light.rs` or native's lighting |
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

A map: each file's header comment says what is in it.

```
web/                         # The web frontend: React and WebGL 2, its Rust in WebAssembly
├── index.html, vite.config.ts, tsconfig.json
├── native.html              # The native frontend in a browser: native/ in WebAssembly, on WebGPU
├── bindgen/                 # pnpm bindgen: binds that WebAssembly to the page (into web/native/, not in git)
├── components.json          # shadcn/ui's CLI config
├── wasm/src/                # The WebAssembly module: the exports the page calls
│   ├── worlds.rs            # The worlds, for worldgen.ts
│   ├── baker.rs             # The baker, for chunk-baker.ts
│   ├── index.rs             # The layer index, for voxel-grid.ts
│   ├── walk.rs              # Walking, for walk.ts
│   └── coverage.rs          # Screen cells nearer layers cover, for occlusion.ts
└── src/
    ├── main.tsx             # Entry: error boundary + App
    ├── App.tsx              # UI panels, keyboard movement loop
    ├── ErrorFallback.tsx
    ├── main.css             # Tailwind + design tokens
    ├── components/ui/       # shadcn/ui primitives (generated)
    └── lib/
        ├── fcc.ts           # Voxel geometry on the FCC lattice
        ├── voxel-grid.ts    # Voxel storage in bricks; LayerIndex
        ├── world.ts         # World: grids by level of detail, in tiles around the camera
        ├── worldgen.ts      # The worlds of worlds/, any box at any level
        ├── cells.ts         # Cells of the coarser lattices N·FCC
        ├── slice-selection.ts # Slices and chunks (pure functions)
        ├── view.ts          # Camera, the 12 faces, projection
        ├── walk.ts          # Walking on the ground
        ├── renderer.ts      # Slices and chunks in view, caching, screen placement (no DOM)
        ├── occlusion.ts     # Skipping chunks behind what nearer layers cover
        ├── chunk-baker.ts   # Bakes chunks with bake/; the bake worker's protocol
        ├── wasm.ts          # The WebAssembly module, inlined; an instance per use
        ├── worker-bakery.ts # Pool of bake workers
        ├── bake.worker.ts   # Bake worker entry
        ├── compositor.ts    # Draws frames with WebGL 2
        └── utils.ts         # cn() for class names
Cargo.toml                   # The Rust crates' workspace; lab/ a member, not a default one
.cargo/config.toml           # Builds for x86-64-v3 (see there)
scripts/shot.mjs             # pnpm shot: a view in headless Chromium, saved once complete
scripts/voxelize.py          # A glTF scene as voxels, for worlds/src/scene.rs
fcc/src/                     # The FCC lattice and its voxels as every crate sees them (no dependencies)
├── lib.rs                   # Faces, lattices, levels, Grid, tiles
└── light.rs                 # The light from voxels' faces, as hosts hand it over
bake/src/                    # Chunk baker and walking, on fcc/: native links it, web/wasm/ exports it
├── lib.rs                   # Baker: one chunk per call
├── cells.rs                 # cells.ts, with a table per level
├── slicing.rs               # slice-selection.ts's slices and chunks
├── surface.rs               # A cell's voxels that can show
├── tables.rs                # The voxel's faces and shades per view face
├── splats.rs                # What a chunk shows: splats, reliefs, media
├── lod.rs                   # Native's chunks: whole cells, levels by depth
├── fill.rs                  # Splats filled into masks (for coverage)
└── walk.rs                  # A body that falls, collides, steps and jumps
worldgen/                    # The world-generation engine; no worlds (docs/generators.md)
├── src/
│   ├── generator.rs         # The framework: nodes, fields, populations
│   ├── generator/           # Any box at any level: forms, drawables, shapes, things
│   ├── build.rs             # Short forms to write worlds with
│   ├── grid.rs              # voxel-grid.ts's bricks, each site's look
│   ├── noise.rs             # Integer hashes, fixed point (Fx), noise
│   ├── units.rs             # Metres per lattice unit
│   ├── js.rs                # Float math the same on every target
│   ├── column.rs            # A heightfield's spans
│   ├── plan.rs              # Plans by column: heights, land, claims
│   ├── domain.rs            # Domains a world is composed of
│   ├── curve.rs             # Curves: lines through space
│   ├── points.rs            # Points: where things are; rooms
│   ├── frame.rs             # Things built in frames of their own
│   ├── recent.rs            # What was used last, kept
│   ├── world.rs             # World: what a host needs to show one
│   ├── tiles.rs             # A world's tiles, as hosts and light take them
│   ├── picture.rs           # Rays through a world's voxels; pictures as PNGs
│   ├── light.rs             # Light, a prototype (docs/light.md)
│   ├── light/reference.rs   # Its reference: rays traced through level 0
│   ├── cost.rs              # What generating costs, against a baseline
│   └── checks/              # The contract, the survey, places, directions: for tests
├── data/                    # The light reference's values; the test worlds' contract baselines
└── tests/                   # The engine on test worlds; each solid against its baseline
worlds/                      # The worlds, on worldgen's public API only
├── cheese/                  # The cheese, a crate of its own
├── src/
│   ├── lib.rs               # WORLDS: the registry hosts start worlds from by name
│   ├── path.rs              # Spines and paths along them
│   ├── forest.rs            # The forest, on ground other worlds share
│   ├── forest/              # Its trees (trees.rs) and what lies on its floor (floor.rs)
│   ├── valley.rs            # The valley: its domains on one land
│   ├── valley/
│   │   ├── course.rs        # The valley's course and its path's route
│   │   ├── town.rs          # Towns along the path
│   │   ├── fields.rs        # Farmland around them
│   │   ├── landmarks.rs     # Keeps, chapels, ponds, farmsteads, windmills
│   │   ├── moor.rs          # The moor; alone, the worlds moor and fog
│   │   ├── mountains.rs     # The valley's ranges
│   │   └── railway.rs       # The valley's railway
│   ├── city.rs              # The city: blocks of buildings on a plan
│   ├── learned.rs           # Settlements drawn by a grammar learned in lab/
│   └── scene.rs             # Scenes made elsewhere, read from a file
├── data/                    # Baselines of cost and contract, the forest's stands, the town grammar;
│                            # scenes/ is not in git (docs/generators.md, "Scenes")
├── tests/                   # Every world keeps the contract and its cost baseline
└── examples/                # map, look, check, cost, bench: a world looked at, checked, measured
lab/                         # Experiment, linted but not tested by pnpm (see src/lib.rs); scripts/overture.py fetches its real places
native/src/                  # The native frontend: egui on winit and wgpu; in browsers too (web/native.html)
├── main.rs                  # The camera, the panel, the command line
├── window.rs                # The window: egui on winit and wgpu
├── gpu.rs                   # The device; offscreen pictures; shaders checked without a GPU
├── world.rs                 # world.ts for generated worlds: tiles made and lit on threads
├── view.rs                  # view.ts: faces, rays, screen boxes
├── engine.rs                # What draws: Engine, Picture, Painter; the engines by name
└── engine/
    ├── chunks.rs            # The chunks engine, as the web draws
    ├── chunks/              # Its renderer, occlusion, bakery, compositor (and .wgsl)
    └── voxels.rs, voxels.wgsl # The voxels engine: rays through the tiles' voxels
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
After changing `worldgen/`, `worlds/` or `fcc/`, also run `pnpm test:worlds`:
the worlds' checks over seeds, which `pnpm test` leaves out.

`pnpm lint` and `pnpm test` cover the Rust
in `fcc/`, `bake/`, `worldgen/`, `worlds/`, `native/` and `web/wasm/` too
(clippy with warnings as errors, rustfmt, `cargo test`);
`pnpm dev`, `pnpm build` and `pnpm test:web` rebuild its WebAssembly first,
the native frontend's for browsers with it (clippy sees only the host's code:
what is `cfg(target_arch = "wasm32")` is checked by building it).
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
A software GPU takes the cores the native world's threads work on:
to time those, moving, add `--stroll SECONDS --by-work`
(`native/src/main.rs`), which gives them each frame's time to themselves.

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
