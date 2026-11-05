# Product Requirements

A proof of concept for rendering a 3D voxel world as a stack of cached 2D
depth slices composited with parallax, instead of rasterising the scene in 3D
every frame. The demo exists to show (and measure) that idea.
Voxels are rhombic dodecahedra on the face-centred cubic (FCC) lattice,
not cubes; scenes keep their shapes (flat walls along the axes,
rectangular buildings), only their surfaces are made of rhombi.

## Goals

- **Cheap camera movement.** Moving redraws cached chunks; only
  chunks coming into view are baked.
- **Close to true perspective.** Nothing that perspective would hide shows
  by more than a set angle.
- **Event-driven.** Nothing renders while the camera is idle.
- **Visible mechanics.** Users can see the slices, toggle them, and watch
  cache statistics react to their movement.

## Features

**Camera** – Arrow keys or dragging on the scene turn it all around
(up to straight up or down). W/S forward/back and A/D left/right along
where it looks (level), Space/Shift up/down; adjustable field
of view (across the longer screen side) and render distance.

**Slicing** – Twelve stacks of slices, one per direction to a voxel's
nearest neighbours (the faces of a rhombic dodecahedron around the
camera), each cut along the lattice's layers across it.
Slices grow thicker with viewing distance, and far ones draw the world
at a coarser level of detail, so far voxels stay a pixel or two across.
Slices are split into chunks, baked only in view, as seen along their
direction, and cached: moving bakes only what comes into view.
A chunk is a list of its visible voxels,
each painted as a hexagon of one colour, its outline along that direction,
drawn at its voxel's depth, so a slice is a relief,
in the colour of its voxel's face towards it:
an impressionist picture of the scene, not its exact faces.
A voxel may have a different material on each of its 12 faces,
so a coarse voxel at a house's eaves is roof from above and wall from the side.
How thick slices are follows "Max Occlusion Leak":
how far geometry perspective hides (e.g. the ground under the moor's
pools) would show were slices drawn flat.
Drawn in relief, it hardly shows at all,
so the setting now trades detail against slices.
The field of view only zooms the picture, without re-slicing.
"Skip Covered Chunks" (on by default) doesn't bake chunks that nearer
ones cover, nor (on the web) draw them;
the picture is the same, it is there to compare.
"Show Face Seams" (off by default) draws lines where the twelve
stacks meet on screen.
"One Voxel per Column" (off by default, `?first=1`) keeps of each column
along a stack's direction only its nearest voxel: about half as much to bake
and draw, each a pillar over the voxels behind it in the column,
so the ground shows no stripes where the view slants;
but what a column shows along it is its nearest voxel's colour,
so a lawn's shades and a road's markings stretch along it.
"Pillars over Columns" (on by default, `?pillars=0`) draws them so;
off, each column is its nearest voxel alone, as baked, and the stripes show.
"Flat Voxels" (off by default) draws slices flat, as "Max Occlusion Leak" assumes,
and "Voxels in Relief" (on by default) in relief;
with both, each voxel is a pillar from its slice's plane to its depth
(and over its column, with "One Voxel per Column"; `?flat=1`, `?relief=0`).
See [architecture.md](architecture.md).

**Worlds** – Procedurally generated, regenerable with a new seed; the camera
moves to the world's spawn point.
A spinner shows while a world generates.
- *Forest* (default): an endless autumn forest valley on hills kilometres
  wide: a winding path, spruces, birches and oaks, ferns, bushes, boulders
  and fallen logs.
  Its finest voxels, 0.125 m, show what is lost further away:
  leaves on twigs, bark and roots, blades of grass,
  berries, mushrooms, cones, pebbles and fallen leaves.
  Generated around the camera in the background, finest first,
  and again where the camera goes:
  each level of detail reaches twice as far as the one before, to 11 km.
- *Valley*: the forest, with a town on its path
  every kilometre or so, the first ahead of the camera:
  streets of plastered houses with gable roofs, windows, doors and shops,
  a square with a fountain, a church and its spire, lamps, yard trees,
  and fields around it (wheat, stubble and haystacks, ploughed rows,
  meadows, hedges), the forest thinning towards them.
  Generated like the forest.
- *City*: a district of a few blocks,
  for checking flat walls, thin frames and openings:
  low buildings whose rooms form a maze of doorways,
  with windows, balconies, courtyards, furniture, street lamps and trees,
  on lawn.
  The camera starts in front of an open entrance.
  Generated like the forest.

**Views in the URL** – The page's query can give the world, its seed,
the camera, where it looks, the field of view and the render distance,
as the 3D viewer's does, so a view can be shown again.

**3D viewer** – "Open in 3D Viewer" renders the same world (same seed) as an
ordinary triangle mesh of the voxels' rhombic faces with three.js and an orbit camera, starting from the
parallax camera with the same perspective (field of view) and fog,
with coarser voxels farther away, to the render distance. It is
the reference for checking what a scene looks like, independent of the
slice renderer.

**Statistics panel** – Session totals (updates, average render time, average
cache efficiency, voxels rendered) and details of the last update (layers,
slice detail, chunks in view reused vs newly baked,
chunks still baking, levels of detail still generating,
chunks covered, cached chunk bytes).
Chunks bake in background workers, so moving never waits for them;
meanwhile a cell shows an older bake, or nothing.

## Non-goals (for the POC)

- Rolling the camera.
- Mobile/touch controls.
- Lighting beyond flat colours, distance fog and simple face shading.
