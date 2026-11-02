# Product Requirements

A proof of concept for rendering a 3D voxel world as a stack of cached 2D
depth slices composited with parallax, instead of rasterising the scene in 3D
every frame. The demo exists to show (and measure) that idea.
Voxels are rhombic dodecahedra on the face-centred cubic (FCC) lattice,
not cubes; scenes keep their shapes (flat walls along the axes,
rectangular buildings), only their surfaces are made of rhombi.

## Goals

- **Cheap camera movement.** Moving mostly redraws cached images; only
  chunks coming into view, or of a level whose eye moved, are rasterised.
- **Close to true perspective.** Nothing that perspective would hide shows
  by more than a set number of pixels.
- **Event-driven.** Nothing renders while the camera is idle.
- **Visible mechanics.** Users can see the slices, toggle them, and watch
  cache statistics react to their movement.

## Features

**Camera** – Arrow keys or dragging on the scene turn it all around
(up to straight up or down). W/S forward/back and A/D left/right along
where it looks (level), Space/Shift up/down, adjustable move speed, field
of view
(5°–120° across the longer screen side) and render distance (40–800
units; a voxel's neighbours are √2 units away).

**Slicing** – Twelve stacks of slices, one per direction to a voxel's
nearest neighbours (the faces of a rhombic dodecahedron around the
camera), each cut along the lattice's layers across it. Near the camera a
slice is one layer of voxels; farther away, a slab of 2, 4, … 64 layers,
and beyond that 64 layers of coarser voxels,
from the world at a coarser level of detail
(every 2nd, 4th, … site each way, standing for the sites nearest it),
so slices grow with viewing distance and stay the same shape at every
scale, and far voxels stay about two pixels across at 90°. Each
is split into chunks, bricks laid like a wall's, each the shadow of a
column of cells of a coarser lattice still, rasterised only in view and
as seen from the site of the slice's lattice nearest the camera (its eye).
Slices are thin enough that
geometry perspective would hide (e.g. stone under the lake's water
surface) never shows by more than a set angle ("Max Occlusion Leak",
0.1–6°, default 1.5°: 17 px at 90° on a 1280 px wide window).
The field of view only zooms the picture, without re-slicing,
so zoomed in, the same slices leak more pixels;
the statistics show the slice detail and how many pixels that is.
"Fill Gaps" (on by default) grows faces that run from one
slice into the next, where nothing else is drawn in their chunk, so thin
things like a floor seen from above don't show sky between slices.
"Skip Covered Chunks" (on by default) doesn't draw chunks that nearer
ones cover, nor bakes them; the picture is the same, it is there to
compare. "Show Face Seams" (off by default) draws lines where the twelve
stacks meet on screen. See [architecture.md](architecture.md).

**Worlds** – Procedurally generated, regenerable with a new seed; the camera
moves to the world's spawn point.
Generating takes a second or two, with a spinner meanwhile.
- *Forest* (default): a 96 m × 160 m autumn forest valley, voxels 0.35 m
  apart: a winding path, spruces, birches and oaks, ferns, bushes, boulders
  and fallen logs, and a ridge closing the far end. ~2.8M voxels.
- *City*: a 116 m × 116 m district planned on a 0.25 m grid, four voxels
  (0.18 m apart) per plan cell,
  for checking flat walls, thin frames and openings:
  3 × 3 blocks of 1–3 storey buildings whose rooms form a maze of doorways,
  with windows, balconies, courtyards, furniture, street lamps and trees.
  The camera starts in front of an open entrance. ~3M voxels.
- Hills, mountains, hills with trees, lake with island: small 80×80 toy
  scenes of blocks (four voxels each) without a real-world scale.

**3D viewer** – "Open in 3D Viewer" renders the same world (same seed) as an
ordinary triangle mesh of the voxels' rhombic faces with three.js and an orbit camera, starting from the
parallax camera with the same perspective (field of view) and fog. It is
the reference for checking what a scene looks like, independent of the
slice renderer.

**Statistics panel** – Session totals (updates, average render time, average
cache efficiency, voxels rendered) and details of the last update (layers,
slice detail, leak bound in pixels, chunks in view reused vs newly baked, chunks still baking,
chunks covered, cached chunk pixels). Chunks bake in background workers, so moving never
waits for them; meanwhile a cell shows an older bake, or nothing.

## Non-goals (for the POC)

- Rolling the camera.
- Mobile/touch controls.
- Lighting beyond flat colours, distance fog and simple face shading.

## Open ideas

- Budget chunk baking per frame: a frame that brings many new chunks
  into view (moving, or turning to new faces) still takes up to ~300 ms,
  more in software WebGL.
- Measure the seams between faces against ray-cast perspective.
- Generate the world on demand, in a worker, instead of one fixed extent
  on the main thread at start:
  the forest can already be generated in any box ([generators.md](generators.md)).
- Hold less of the world: every worker has its own 35–50 MB copy;
  a shared buffer or only the voxels with an open face would do.
- Describe the city in metres, like the forest, instead of boxes on its
  0.25 m plan.
- Make the 12 stacks cheaper:
  with four or more faces in view, each with its own near slices,
  a first bake looking along an axis takes ~10 s on one thread,
  even with the Rust baker.
  Baking finer than the screen shows is part of it:
  slices of size 4 and 8 get 2–3× the pixels they cover on screen.
- Port the toy worlds to the forest's metric scale, or drop them.
- Find covered chunks faster: it still adds ~17 ms to a render with 12 faces,
  mostly testing sub-blocks one by one in `cover` (`bake/src/coverage.rs`).
- Find covered chunks more exactly: half of the chunks drawn show no pixel.
- Build `LayerIndex` off the main thread or in Rust: ~0.4 s at start on the forest.
- Measure the WebGL compositor on real GPUs (headless Chromium only has a
  software one); if draw calls matter, pack chunks into a texture atlas.
- Paint cut cross-sections with the colour of the surface above them: a 2D
  prototype allowed slices twice as thick at the same leak.
- Fill gaps in screen space (only pixels no layer covers) instead of
  growing faces.
