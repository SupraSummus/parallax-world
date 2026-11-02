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
  by more than a set angle.
- **Event-driven.** Nothing renders while the camera is idle.
- **Visible mechanics.** Users can see the slices, toggle them, and watch
  cache statistics react to their movement.

## Features

**Camera** – Arrow keys or dragging on the scene turn it all around
(up to straight up or down). W/S forward/back and A/D left/right along
where it looks (level), Space/Shift up/down, adjustable move speed, field
of view (across the longer screen side) and render distance.

**Slicing** – Twelve stacks of slices, one per direction to a voxel's
nearest neighbours (the faces of a rhombic dodecahedron around the
camera), each cut along the lattice's layers across it.
Slices grow thicker with viewing distance, and far ones draw the world
at a coarser level of detail, so far voxels stay a pixel or two across.
Slices are split into chunks, rasterised only in view and cached
while the camera stays near where they were baked from.
Slices are thin enough that
geometry perspective would hide (e.g. stone under the lake's water
surface) never shows by more than a set angle ("Max Occlusion Leak").
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
A spinner shows while a world generates.
- *Forest* (default): an autumn forest valley:
  a winding path, spruces, birches and oaks, ferns, bushes, boulders
  and fallen logs, and a ridge closing the far end.
- *City*: a district of a few blocks,
  for checking flat walls, thin frames and openings:
  low buildings whose rooms form a maze of doorways,
  with windows, balconies, courtyards, furniture, street lamps and trees.
  The camera starts in front of an open entrance.
- Hills, mountains, hills with trees, lake with island: small toy
  scenes of blocks without a real-world scale.

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
