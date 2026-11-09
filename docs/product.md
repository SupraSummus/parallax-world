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
"Walk on Ground" (F) walks instead, as in Minecraft:
the camera is the eye of a body 1.8 m tall that falls onto the ground,
is stopped by walls and trunks, steps up and down what is half a metre high,
jumps (Space) and sneaks (Shift);
it walks through plants and water, as each world marks its kinds.

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
How thick slices are follows "Slice Detail":
a slice is at most 1/detail of its distance thick.
Thinner slices draw coarser voxels sooner,
so that the world generated around the camera holds all they show.
Thicker ones are faster, but leak where a column along a slice's direction
has a gap (a valley between two hills):
only what is before it is drawn, and farther slices show through.
The field of view only zooms the picture, without re-slicing.
"Skip Covered Chunks" (on by default) doesn't bake chunks that nearer
ones cover, nor (on the web) draw them;
the picture is the same, it is there to compare.
"Show Face Seams" (off by default) draws lines where the twelve
stacks meet on screen.
Each slice is a picture along its stack's direction with a depth per point:
of each column along it, only the nearest voxel,
drawn as a pillar over the voxels behind it in the column,
so the ground shows no stripes where the view slants;
but what a column shows along it is its nearest voxel's colour,
so a lawn's shades and a road's markings stretch along it.
See [architecture.md](architecture.md).

**Worlds** – Procedurally generated, regenerable with a new seed; the camera
moves to the world's spawn point.
A spinner shows while a world generates.
- *Forest*: an endless autumn forest valley on hills kilometres
  wide: a winding path, spruces, birches and oaks, ferns, bushes, boulders
  and fallen logs.
  Its finest voxels, 0.125 m, show what is lost further away:
  leaves on twigs, bark and roots, blades of grass,
  berries, mushrooms, cones, pebbles and fallen leaves.
  Generated around the camera in the background, finest first,
  and again where the camera goes:
  each level of detail reaches twice as far as the one before, to 11 km.
- *Valley* (default, the flagship new structures are added to): the forest, with a town on its path
  every kilometre or so, the first ahead of the camera:
  hamlets, villages and market towns,
  each in a style of its own: southern plaster under low red tiles,
  northern walls under steep slate, timber frames, or brick with stepped gables.
  Houses with gable roofs, windows, doors and shops, front gardens and fences,
  a square or a green with a fountain, a well or a market hall,
  a church and its spire, lamps or an avenue of trees,
  some market towns walled with gates on their main street,
  and fields around each (wheat, stubble and haystacks, ploughed rows,
  meadows, hedges), the forest thinning towards them.
  Moorland on the uplands under the treeline,
  and along stretches of the valley kilometres apart:
  heather, grass and bog, pools in hollows, boulders,
  and steadings of a drystone hut and a ring of standing stones.
  Landmarks standing alone: a ruined keep on a hilltop, a watchtower,
  wayside chapels, ponds, farmsteads and windmills.
  Mountain ranges along both sides: crests, peaks and passes,
  side valleys, meadows and scree above the treeline, snow on the peaks.
  A railway up one side of the valley:
  ballast, sleepers and rails in cuttings and on embankments,
  bridges on piers and lined tunnels where the ground is far from the track,
  and a station by each town it passes on the ground.
  Generated like the forest.
- *Moor*: the valley's moorland alone: rolling heath, a stream and pools,
  boulders, and steadings with a path of cairns between hut and ring.
  Generated like the forest.
- *Learned*: the forest, with settlements along its path
  drawn by a split grammar learned from the valley's towns
  (`lab/`), not written.
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
so a view can be shown again.

**Statistics panel** – Session totals (updates, average render time)
and details of the last update (layers,
slice detail, chunks in view reused vs newly baked,
chunks still baking, levels of detail still generating,
chunks covered, cached chunk bytes).
Chunks bake in background workers, so moving never waits for them;
meanwhile a cell shows an older bake, or nothing.

## Non-goals (for the POC)

- Rolling the camera.
- Mobile/touch controls.
- Lighting beyond flat colours, distance fog and simple face shading.
