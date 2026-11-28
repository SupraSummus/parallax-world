# The worlds

What there is to see in the worlds, and how to look at them.
Their voxels are cubes, a coarser one the eight finer in it.

## Looking at them

**Camera** – Arrow keys or dragging on the scene turn it all around
(up to straight up or down). W/S forward/back and A/D left/right along
where it looks (level), Space/Shift up/down; adjustable field
of view (across the longer screen side) and render distance.
"Walk on Ground" (F) walks instead, as in Minecraft:
the camera is the eye of a body 1.8 m tall that falls onto the ground,
is stopped by walls and trunks, steps up and down what is half a metre high,
jumps (Space) and sneaks (Shift);
it walks through plants and water, as each world marks its kinds.

**Cutting** – C takes a stroke where the camera looks,
within 3 m (`native/src/cut.rs`):
what lies within a quarter metre of the voxel it meets
is taken out of the world, at every level of detail.
Strokes one after another cut deeper, and a notch as an axe cuts one.
What a stroke leaves standing on nothing, or too weak to stand,
moves: it bends, breaks, falls and lands, all on the voxels
([mechanics.md](mechanics.md)):
a tree cut through topples if it leans past its stump,
or if notched first on the side it is to fall to,
and lies where it lands or is caught, its leaves with it,
lying on what it landed on, not stuck to it.
The lattice never moves: matter moves between its sites.
The voxels engine shows it; the chunks engine does not.

**Drawing** – Rays go through the voxels, from one to the next across
their faces, each level of detail as far as it is held around the eye,
then the next coarser.
A voxel may look different through each of its 6 faces,
so a coarse voxel at a house's eaves is roof from above and wall from the side.
See-through voxels (mist, the sea) add up along a ray.
See [architecture.md](architecture.md).

**Light** – Voxels are shaded by the way their faces face,
or lit by light worked out across the world:
the sky, the sun and what glows, each as bright as its slider says.
Lit, the voxels show it as an eye would:
adapting, over a second, to how bright what it looks at is,
so that a dark room or a night shows brighter, but still dark,
and what is bright past a door or round a lamp washes out less;
its exposure and how far it adapts are sliders too.
Settled, that light is kept as it changes and shown as it settles:
it comes in bit by bit, and follows the sliders as they are dragged.

**Views in the URL** – The page's query can give the world, its seed,
the camera, where it looks, the field of view, the render distance
and the light, so a view can be shown again:
the same options the native frontend takes on its command line.

**Panel** – The engine, the field of view and render distance,
the lighting, walking, the flying speed,
and where each frame's time went.

**The voxel under the pointer** (I, or the panel; the voxels engine) –
what the ray through the pointer meets, told beside it:
the voxel that stops it, where, how far, its look and the light on it;
and the see-through voxels before it, and what they let through.

## The worlds

Procedurally generated, by name and seed (`--world`, `--seed`,
or the page's query); the camera starts at the world's spawn point.
Each is generated around the camera in the background, finest first,
and again where the camera goes:
each level of detail reaches twice as far as the one before, to 11 km.
- *Forest*: an endless autumn forest valley on hills kilometres
  wide: a winding path, spruces, birches and oaks, ferns, bushes, boulders
  and fallen logs.
  Trees of about one height keep their crowns apart, with gaps of sky between
  (crown shyness), now and then touching; a taller tree's crown spreads further.
  Its finest voxels, 0.125 m, show what is lost further away:
  leaves on twigs, bark and roots, blades of grass,
  berries, mushrooms, cones, pebbles and fallen leaves.
- *Valley* (default, and where new structures go first):
  the forest, with a town on its path every kilometre or so:
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
  heather, grass and bog, pools in hollows, mist pooled in them, boulders,
  and steadings of a drystone hut, walked into, and a ring of standing stones,
  on gentle ground.
  Landmarks standing alone: a ruined keep on a hilltop, a watchtower,
  wayside chapels, ponds, farmsteads, windmills, and now and then a hedge maze
  with a garden in its middle.
  Caves open in the mountains' faces, framed by rocks:
  passages winding in to chambers hung with dripstone,
  lit by glow-worms; deep in, a pool or crystals,
  and in the first, a camp someone left, its lantern still lit.
  The valley winds, its path along the lowest of its floor.
  Mountain ranges along both sides: crests, peaks and passes, side valleys,
  arêtes and cirques behind the crests,
  meadows and scree above the treeline, snow in high hollows.
  Past a few kilometres of them the land leaves the ranges:
  west, a lowland falling to the sea, its coast ragged with bays and islands;
  east, an upland under the treeline, moor and forest,
  and patches of badlands kilometres across:
  red rock banded in strata, sand and scrub, bleached dead trees,
  hoodoos, buttes and arches.
  On the western shore, lighthouses on its headlands, lit at night,
  and wrecks on its beaches.
  A railway up one side of the valley,
  laid on the ground as it is, along the slopes where that spares it earthworks:
  ballast, sleepers and rails in cuttings and on embankments,
  and, where it can't go round, bridges on piers and lined tunnels,
  and a station by each town it passes on the ground.
- *Moor*: the valley's moorland alone: rolling heath, a stream and pools,
  mist in its hollows, boulders,
  and steadings with a path of cairns between hut and ring.
- *Learned*: the forest, with settlements along its path
  drawn by a split grammar learned from the valley's towns
  (`lab/`), not written.
- *City*: a district of a few blocks,
  for checking flat walls, thin frames and openings:
  low buildings whose rooms form a maze of doorways,
  with windows, balconies, courtyards, furniture, street lamps and trees,
  on lawn.
  The camera starts in front of an open entrance.
- *Cheese*: cheese endless every way,
  holed through with caverns a few metres wide, tunnels between them
  and eyes in their walls, lit by glowing crystals on the caverns' walls
  and by nothing else: it has no sky.
  Walkable: up the caverns' floors, along the tunnels,
  and up the winding stairs round a pillar in each cavern
  into the one over it.
  Here and there, lumps of darker, aged cheese a hundred metres across
  hold a mine instead:
  straight galleries along x and z, a storey apart,
  propped with timber and lit by lanterns,
  with chambers where they meet and stairs down beside some,
  running on into the caverns at the lumps' edges.
  The camera starts in a cavern; the mine's nearest gallery is `mine`.
