# Agent Development Guide

Entry point for AI agents working on this codebase. Read this first.

## Documents

| Document | Purpose | Read when |
|----------|---------|-----------|
| [README.md](README.md) | Overview and quick start | First |
| [docs/worlds.md](docs/worlds.md) | The worlds: what is in them and how they are seen | Adding or changing what one sees or does |
| [docs/architecture.md](docs/architecture.md) | Levels and tiles, the voxels and flat engines, the lattice; the crates | Touching the Rust |
| [docs/testing.md](docs/testing.md) | Test layout and conventions | Writing tests |
| [docs/todo.md](docs/todo.md) | Planned work: known problems and open ideas | Looking for what to do next |
| [docs/generators.md](docs/generators.md) | Generating worlds in pieces: layers, things, rules | Writing or changing a world generator |
| [docs/checks.md](docs/checks.md) | The contract worlds keep, checks of places and directions, what generating costs | A world's check or cost fails |
| [docs/light.md](docs/light.md) | Light: what the prototype gets wrong, what was tried, what it costs | Touching `light/` or native's lighting |
| [docs/mechanics.md](docs/mechanics.md) | Mechanics on the voxels: matter that moves, bends and breaks, at a level of detail | Touching `mechanics/` or `native/src/mechanics.rs` |
| [docs/statics.md](docs/statics.md) | Statics, an experiment: what stands and what falls, what was measured | Touching `statics/` |
| [.claude/skills/lead/SKILL.md](.claude/skills/lead/SKILL.md) | The lead role: driving changes to main through delegated work and PRs | Only when the user asks for it (`/lead`) |

Each concept has one home:
build/test commands live in `package.json` scripts,
architecture in `docs/architecture.md`,
what one sees and does in `docs/worlds.md`,
test conventions in `docs/testing.md`,
planned work in `docs/todo.md`,
agent rules here.
Link instead of duplicating.
Docs go in `docs/` with lower-case names;
only conventional files (README, AGENTS, LICENSE) stay at the root.

## Source layout

A map: each file's header comment says what is in it.

```
web/                         # The frontend in a browser
├── index.html               # The page: native/ in WebAssembly, on WebGPU
├── worker.js                # A thread of native/ in the page: a Web Worker on its memory
├── vite.config.js           # The page isolated while developing, so its threads share memory
├── public/isolated.js       # A service worker that isolates the page where the server does not
└── bindgen/                 # pnpm bindgen: binds that WebAssembly to the page (into web/native/, not in git)
Cargo.toml                   # The Rust crates' workspace; lab/ a member, not a default one
.cargo/config.toml           # Builds for x86-64-v3, and WebAssembly with threads (see there)
scripts/voxelize.py          # A glTF scene as voxels, for worlds/src/scene.rs
lattice/src/                 # The cubic lattice and its voxels as every crate sees them (no dependencies)
├── lib.rs                   # Faces, levels, Grid, tiles
├── light.rs                 # The light from voxels' faces, as hosts hand it over
├── ray.rs                   # Rays across the voxels, exactly, face to face
└── walk.rs                  # A body that falls, collides, steps and jumps
bake/src/                    # Chunks baked for native's flat engine (docs/architecture.md, section 3)
├── face.rs                  # The 18 faces: the lattice seen along the cube's axes and edges
├── lod.rs                   # Which cells of the octree a face draws, by level
├── pillars.rs               # A cell's columns as pillars, and its medium slab by slab
└── lib.rs                   # Baker: a cell's tiles read, its pillars and medium made
worldgen/                    # The world-generation engine, no direction up in it; no worlds (docs/generators.md)
├── src/
│   ├── generator.rs         # The framework: nodes, fields, populations
│   ├── generator/           # Any box at any level: forms, drawables, shapes, things
│   ├── build.rs             # Short forms to write worlds with
│   ├── grid.rs              # Voxel storage in bricks, each site's look
│   ├── edits.rs             # What a world's sites hold since it was generated, at every level
│   ├── noise.rs             # Integer hashes, fixed point (Fx), noise; seeds by name (Scope)
│   ├── units.rs             # Metres per lattice unit
│   ├── sites.rs             # The sites a layer reads for: their spacing
│   ├── js.rs                # Float math the same on every target
│   ├── curve.rs             # Curves: lines through space
│   ├── points.rs            # Points: where things are; rooms
│   ├── frame.rs             # Things built in frames of their own
│   ├── recent.rs            # What was used last, kept; small values by slot, read without waiting
│   ├── world.rs             # World: what a host needs to show one
│   ├── entry.rs             # Entry: a world by name, what its checks hold it to; its tests (world_tests!)
│   ├── tiles.rs             # A world's tiles, as hosts and light take them
│   ├── picture.rs           # Pictures of a world, by rays through its voxels
│   ├── png.rs               # A small PNG encoder, every crate's
│   ├── scalar.rs            # Scalars: numbers bounded over boxes, what layers are drawn within
│   ├── land.rs              # Land: space shared out by bids, each share a scalar
│   ├── cost.rs              # What generating costs, against a baseline (docs/checks.md)
│   └── checks/              # The contract, the survey, places, directions: for tests (docs/checks.md)
├── data/                    # The test worlds' contract baselines
└── tests/                   # The engine on test worlds; each solid against its baseline
ground/                      # Worlds on a ground, on worldgen's public API: up is +y
├── src/
│   ├── plan.rs              # Plans by column: heights, claims
│   ├── column.rs            # A heightfield's spans
│   ├── about.rs             # Fields about a ground, things on it
│   └── land.rs              # Land on a ground: the lie of the land, its pulls; shares by column
└── tests/                   # Fields about a ground; plans read by fields
light/                       # Light, a prototype (docs/light.md), on worldgen's tiles
├── src/lib.rs               # Light: each site's, settled round by round, shared by threads
├── src/relax.rs             # Light kept settled site by site as the world changes (Settling)
├── src/reference.rs         # Its reference: rays traced through level 0
└── data/                    # The reference's values
worlds/                      # The worlds, on worldgen's and ground's public API only
├── cheese/                  # The cheese, a crate of its own, with its tests and baselines;
│                            # its parts: the holed cheese (lib.rs), the mine (mine.rs)
├── src/
│   ├── lib.rs               # all: the registry hosts start worlds from by name, each crate's WORLDS
│   ├── path.rs              # Spines and paths along them
│   ├── forest.rs            # The forest, on ground other worlds share
│   ├── forest/              # Its trees (trees.rs) and what lies on its floor (floor.rs)
│   ├── valley.rs            # The valley: its parts on one land
│   ├── valley/
│   │   ├── course.rs        # The valley's course and its path's route
│   │   ├── town.rs          # Towns along the path
│   │   ├── fields.rs        # Farmland around them
│   │   ├── landmarks.rs     # Keeps, chapels, ponds, farmsteads, windmills, mazes
│   │   ├── moor.rs          # The moor; alone, the worlds moor and fog
│   │   ├── mountains.rs     # The valley's ranges
│   │   ├── railway.rs       # The valley's railway
│   │   ├── caves.rs         # Caves in the mountains' faces
│   │   ├── badlands.rs      # Red rock east of the ranges
│   │   └── coast.rs         # Lighthouses and wrecks by the sea
│   ├── city.rs              # The city: blocks of buildings on a plan
│   ├── learned.rs           # Settlements drawn by a grammar learned in lab/
│   └── scene.rs             # Scenes made elsewhere, read from a file
├── data/                    # Baselines of cost and contract, the forest's stands, the town grammar;
│                            # scenes/ is not in git (docs/generators.md, "Scenes")
├── tests/                   # Its worlds keep the contract and their cost baselines (checks.rs); scenes
└── examples/                # map, look, places, check, cost, bench: a world looked at, checked, measured
mechanics/src/               # Mechanics on the voxels (docs/mechanics.md), local/ in its place
├── lib.rs                   # Matter: cells welded across faces where both hold on, contacts; a step; hops
├── element.rs               # A cell as a finite element: a co-rotated cube of eight nodes
├── solve.rs                 # Conjugate gradients, V-cycles over the octree
└── tests.rs                 # Posts dropped, toppling, holding on or not; beams out of a wall, spinning
local/src/                   # Mechanics by local rules (docs/mechanics.md, "By local rules"), driven by native/src/mechanics.rs
├── lib.rs                   # Cells as rigid cubes, beams across the faces they hold on across; explicit steps
├── crew.rs                  # Threads that work a step's rounds together, a barrier between phases
└── tests.rs                 # mechanics/'s tests, on it; the same on one thread and on several
statics/                     # Statics, an experiment (docs/statics.md): what stands
├── src/                     # Faces and beams (model.rs) on the octree 2^l·Z³ (lattice.rs);
│                            # a level measured from the one below (gather.rs); the forces (bodies.rs)
└── examples/statics.rs      # The experiments
lab/                         # Experiment, linted but not tested by pnpm (see src/lib.rs); scripts/overture.py fetches its real places
native/src/                  # The frontend: egui on winit and wgpu; in browsers too (web/index.html)
├── main.rs                  # The camera, the panel, the command line
├── window.rs                # The window: egui on winit and wgpu
├── gpu.rs                   # The device; offscreen pictures; shaders checked without a GPU
├── threads.rs               # Threads natively and in browsers, which the thread that draws never waits for
├── world.rs                 # The world's tiles around the camera, made and lit on threads
├── view.rs                  # The camera's basis and focal length
├── eye.rs                   # How light shows: the eye adapting, its curve
├── inspect.rs               # The voxel under the pointer, as the voxels engine finds it
├── settler.rs               # The light kept settled on a thread of its own (--settle), lighting bricks
├── cut.rs                   # Cutting: what a stroke where one looks takes out of the world
├── mechanics.rs, mechanics/ # The world as one lattice, the part awake moving: mechanics on a thread
├── engine.rs                # What draws: Engine, Picture, Painter
└── engine/
    ├── voxels.rs, voxels.wgsl # The voxels engine: rays through the tiles' voxels
    ├── chunks.rs              # The flat engine: its renderer on a thread of its own
    └── chunks/                # Its renderer (the cells picked and hidden), bakery,
                               # occlusion, and compositor (compositor.rs, .wgsl)
```

## Workflow

Before and after changing code, run the `package.json` scripts and fix
errors:

```bash
pnpm check   # lint, the WebAssembly built, then test
```

`pnpm test` takes most of that time.
While iterating, run only the affected tests
([docs/testing.md](docs/testing.md));
run `pnpm check` again only after changing code.
After changing `worldgen/`, `ground/`, `worlds/` or `lattice/`, also run `pnpm test:worlds`:
the worlds' checks over seeds, which `pnpm test` leaves out.

`pnpm lint` is clippy with warnings as errors, and rustfmt.
Clippy sees only the host's code:
what is `cfg(target_arch = "wasm32")` is checked by building it
(`pnpm wasm`, which `pnpm dev` and `pnpm build` run first).
CI runs the same scripts plus `pnpm build`, in jobs side by side,
and `native/`'s tests that draw, on Mesa's software Vulkan.
For rendering changes, also look at the result,
in the window or the browser (`pnpm dev`); unit tests can't see pixels.
Without a GPU, `pnpm shot --world moor --look 30,-10 --shot out.png`
saves a picture of a view once it is complete,
headless (`pnpm shot --help` lists the options),
and wants Mesa's software Vulkan (`apt-get install mesa-vulkan-drivers`).
A software GPU takes the cores the world's threads work on:
to time those, moving, add `--stroll SECONDS --by-work`,
which gives them each frame's time to themselves.

## Rules

0. **Simplicity first.** README.md's design principles open with Hoare's
   motto: make it so simple that there are obviously no deficiencies.
   Judge every change by it, and prefer deleting to adding.
1. **No dangling files.** Every file is imported, referenced by config, or
   linked from this file or README.md. When removing a file, remove its
   references.
2. **Minimal docs.** Prefer code comments for implementation details. Delete
   outdated docs rather than letting them mislead. When changing code, grep
   the docs for mentions and update them.
   Don't restate what the code says (constants, step-by-step logic):
   name it and say why.
   Measured numbers go stale; give one only when the point needs it.
3. **Architecture changes** update `docs/architecture.md` and the affected
   tests.
4. **Follow existing patterns** in neighbouring files.
5. **Semantic line breaks in Markdown** ([sembr.org](https://sembr.org/)).
   One sentence per line,
   and break after a clause where it helps (comma, semicolon, colon, dash).
   Keep lines under about 80 characters, except for links and code.
   Apply it lazily: to paragraphs you write or edit, no mass reformatting.
