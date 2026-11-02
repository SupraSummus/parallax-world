# Agent Development Guide

Entry point for AI agents working on this codebase. Read this first.

## Documents

| Document | Purpose | Read when |
|----------|---------|-----------|
| [README.md](README.md) | Overview and quick start | First |
| [docs/product.md](docs/product.md) | What the product does and doesn't do | Adding or changing features |
| [docs/architecture.md](docs/architecture.md) | Slicing, caching and compositing architecture | Touching `src/lib` |
| [docs/testing.md](docs/testing.md) | Test layout and conventions | Writing tests |
| [docs/todo.md](docs/todo.md) | Measured or derived improvements not done yet | Looking for what to do next |
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
    ├── world.ts             # World: voxel grids by level of detail (generated in parts), spawn, toy world types
    ├── generator.ts         # Generating any box of a world alone, at any level of detail
    ├── forest.ts            # Forest scene generator, on generator.ts (UNIT_SIZE lives here)
    ├── city.ts              # City scene generator: streets, buildings, maze of rooms
    ├── noise.ts             # Seeded hash, value noise and PRNG for the generators
    ├── cells.ts             # Cells of the coarser lattices N·FCC that slices are made of
    ├── slice-selection.ts   # Slices, chunks and the eyes they are baked from (pure functions)
    ├── view.ts              # Camera orientation, the 12 faces (FCC directions), projection
    ├── renderer.ts          # Slices and chunks in view, caching, screen placement (no DOM)
    ├── occlusion.ts         # Which screen cells nearer layers cover (bake/src/coverage.rs), to skip chunks behind them
    ├── chunk-baker.ts       # Bakes whole chunks with bake/ (WebAssembly); the bake worker's message protocol
    ├── worker-bakery.ts     # Pool of bake workers the renderer sends missing chunks to
    ├── bake.worker.ts       # Bake worker entry
    ├── compositor.ts        # Draws the renderer's frames with WebGL 2
    ├── bake.ts              # Voxel face geometry and shaded palettes handed to the baker
    └── utils.ts             # cn() for class names
viewer.html                  # Reference 3D viewer page (second Vite entry)
src/viewer/
├── main.ts                  # three.js scene, orbit camera
└── mesh.ts                  # World -> triangle mesh of exposed rhombic faces
bake/                        # Chunk baker and coverage in Rust, compiled to WebAssembly (no dependencies)
└── src/
    ├── lib.rs               # Baker: cached cell surfaces, one chunk per call; the WebAssembly exports
    ├── world.rs             # Grid and voxel faces as handed over, read per face
    ├── cells.rs             # cells.ts, with a table per level
    ├── slicing.rs           # The parts of slice-selection.ts baking needs
    ├── surface.rs           # A cell's voxels that can show
    ├── faces.rs             # Which voxel faces a chunk shows, baked onto its centre plane
    ├── fill.rs              # Faces filled into pixels, gap filling, solid blocks
    └── coverage.rs          # Which screen cells nearer layers cover, on the main thread; its WebAssembly exports
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

`pnpm lint` and `pnpm test` cover the Rust in `bake/` too
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
3. **Architecture changes** update `docs/architecture.md` and the affected
   tests.
4. **TypeScript strict** (`strictTypeChecked` ESLint rules). Follow existing
   patterns in neighbouring files.
5. **Semantic line breaks in Markdown** ([sembr.org](https://sembr.org/)).
   One sentence per line,
   and break after a clause where it helps (comma, semicolon, colon, dash).
   Keep lines under about 80 characters, except for links and code.
   Apply it lazily: to paragraphs you write or edit, no mass reformatting.
