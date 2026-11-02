# Parallax Voxel Engine

A proof of concept that renders a 3D voxel world (rhombic dodecahedra on
the face-centred cubic lattice) as a stack of cached 2D
depth slices composited with WebGL. Slices get thicker with
distance, but never so thick that they show more than a few pixels of what
true perspective would hide; camera movement mostly just moves cached
images.

Live demo: <https://suprasummus.github.io/parallax-world/> (deployed from `main`).

## Design principles

What the design is held to, beyond speed:

1. **Structure comes from the lattice, not the screen.**
   Slices, chunks and every other partition follow the FCC lattice,
   and their boundaries run between voxels.
   A rectangle of pixels imposed from outside is a foreign element.
2. **Self-similarity.**
   Every coarser level is again an FCC lattice of rhombic dodecahedra:
   the same rule and the same geometry at every scale,
   with no special case for coarser levels.
3. **Isotropy over convenience.**
   No direction is preferred:
   the world must look the same along each of the 12 directions.
   This outranks nesting (a tree of cells) and ease of implementation.
4. **Elegance is a criterion in its own right.**
   Results should be visible, not only measured.

## Quick start

Chunks are baked by Rust compiled to WebAssembly (`bake/`),
so besides Node and pnpm you need [Rust](https://rustup.rs/)
with its WebAssembly target:

```bash
rustup target add wasm32-unknown-unknown
pnpm install
pnpm dev
```

Controls: W/S forward/back, A/D left/right, Space/Shift up/down, arrow
keys or drag to look around.
"Open in 3D Viewer" shows the same scene with a conventional (three.js)
renderer and an orbit camera.

Other scripts (`test`, `lint`, `typecheck`, `knip`, `build`) are in
`package.json`.

## Documentation

| Document | Description |
|----------|-------------|
| **[AGENTS.md](AGENTS.md)** | Guide for AI agents (start here) |
| [docs/product.md](docs/product.md) | Product requirements |
| [docs/architecture.md](docs/architecture.md) | Rendering architecture |
| [docs/testing.md](docs/testing.md) | Testing guide |

## License

MIT, see [LICENSE](LICENSE).
