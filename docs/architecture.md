# Rendering Architecture

The world is cut into slices, each a slab between two layers of a
coarser FCC lattice made of whole cells of a finer one,
and each slice into chunks, bricks of the lattice seen along its axis;
each chunk in view is baked once, as seen along the slice's axis,
into a list of its visible voxels,
each a hexagon of one colour (a splat) at its site,
that of its face towards the camera,
and WebGL draws the splats in perspective, each at its own depth:
a slice is a relief.
Voxels are rhombic dodecahedra on the face-centred cubic (FCC) lattice
(section 6).
Sections 1–3 describe one stack of slices, looking along one of the
lattice's 12 nearest-neighbour directions;
the camera looks all around with 12, one per face of a rhombic
dodecahedron around it (section 4).
Seen along such a direction, the lattice is a stack of flat layers
LAYER = 1/√2 apart, and a voxel reaches from the layer before its own to
the one after, so neighbouring slices interlock.

```
World (voxels) ──selectSlices──▶ slices ──chunks in view──▶ cached splats ──Frame──▶ WebGL ──▶ screen
   world.ts      slice-selection.ts, cells.ts                 renderer.ts          compositor.ts
                                                               │      ▲
                                                 missing chunks▼      │splats
                                                 bake workers (worker-bakery.ts)
```

## 1. Slice selection (`slice-selection.ts`, `cells.ts`)

Two constraints: slices must be thin enough for their distance
(`detail`, section 2), and named the same wherever the camera is,
so they can be cached.

### Cells of coarser lattices

Every second site in each direction of the FCC lattice is again an FCC
lattice, twice as coarse, and so on: N·FCC for N = 1, 2, 4, …
(`cells.ts`).
In a face's lattice coordinates (section 4) its sites are the (a, b, c)
with all three multiples of N and a/N, b/N, c/N of one parity,
so it has its own layers along every face's axis, N layers apart.
Each site belongs to the cell of the nearest site of N·FCC,
a rhombic dodecahedron N times a voxel's.
Sites exactly between cells, on a cell's face or corner,
go to the cell with the greatest c, then b, then a:
the minimal tie-break, and the same along every face.
A cell holds N³ sites, in layers c0 − N to c0 + N − 1 around its own
site's layer c0 (`ownedLayers`; at N = 2, c0 − 2 to c0).
Cells are as round as rhombic dodecahedra:
no direction is preferred.

Cells of different N do not nest.
A cell of 2N·FCC cuts in half the 12 cells of N·FCC
centred on its faces, and in six the 6 at its four-fold corners;
a tree of nested cells would need cells of a preferred direction
(the README's design principles put isotropy first).
So each level is used on its own.

### Slices are slabs of whole cells

A slice (`SliceId`) of level k is a slab between two layers of 2^k·FCC:
its atoms with their own site in the 2^k layers from layer · 2^k.
Its atoms are the cells of 2^ℓ·FCC, ℓ = `atomLevel(k, s)` = max(0, k − s),
s the atom shift (at most 4, see below):
up to level s single voxels, slabs of 1 to 2^s layers of them;
above, 2^s layers of cells of 2, 4, … up to 256 layers, the coarsest grid's.
Slabs of levels with the same atoms nest: one of level k + 1 is two of level k.
A slice splits no atom, except where the atoms grow (cutoffs, below):
there the smaller cells along the boundary are shared by two levels.

**Levels of detail.** A world is a grid per level of detail
(`World.level`):
grid ℓ holds the sites of 2^ℓ·FCC, in its own units,
each a voxel standing for its cell.
A slice draws its atoms as the voxels of grid ℓ.
In that grid's units it is a slice of level k − ℓ of single voxels,
so the baker bakes it as one and scales the chunk back
(`in_grid` in `bake/src/lib.rs`).
Every world is generated at each level (`docs/generators.md`),
around the camera, each grid reaching twice as far as the one before.
A grid is made of tiles fixed in the world:
when the camera goes far enough from its middle,
it moves to the camera, and only the tiles it lacks are generated
(`World.missingParts`).
A level with no grid yet is no special case: its grid has no tiles.
The bake workers generate the tiles, finest level first;
a level's slices show once its grid is in,
and a moved grid replaces the old one once all its tiles are.
The native spike (`native/src/world.rs`) has no grids, only tiles:
cubes, alike along every axis, so no direction is up to how they are asked for
(`tile_box` in `worldgen/src/light.rs`; those outside the world's heights are empty),
made in the order the renderer wants them:
a cell is baked once the world has every tile it reads (`chunk_reads`),
and until then it asks for them, nearest first.
Then come the other tiles within `GRID_REACH` of each level the chunks read,
so that turning finds them made, and far ones are dropped;
for the voxels engine, those within `voxels::REACH`, as far as its rays read.
A tile never changes once made,
so no chunk or cell surface goes stale as tiles come.
Tests' scenes of hand-made voxels are given a grid per level,
the coarser ones sampled at their sites (`src/lib/scene.test-helper.ts`).
A slice is about 1/detail of its distance thick (section 2),
so from level s on, a slab of 2^s atoms,
a voxel stays about the same size on screen,
about 1 / (2^s · detail) radians (`docs/todo.md`).

**Grid reach.** Each grid holds the world `GRID_REACH` of its units
around the camera (`World.holds`, a square with its corners cut),
and slices read no grid farther:
the atom shift is the largest whose slices keep within it (`atomShift`).
So thinner slices (more detail) take coarser atoms sooner,
voxels on screen stay within about twice their size at the default detail,
and the grids' bytes are the same at every detail.
Atoms twice as fine on screen would need `GRID_REACH` twice as large,
four times the bytes in every worker.

Why many layers of atoms and not one:
cells of neighbouring layers interlock, as voxels do.
Along the axis, a layer of rhombic dodecahedra N layers apart is 2N deep,
in columns alternating between even and odd layers;
detail limits a slice's depth (section 2),
so slices of single layers of cells would take twice as many.
In a slab of R layers of cells only its two faces are ragged:
it is about (R + 1)·N deep for R·N layers, 1.06 times for R = 16.
(No other direction does much better:
along [111] a layer of cells is 1.5 times as deep as layers are apart.)

A slice's atoms' sites are every N-th of its slab's layers, and its
voxels, N wide, reach N layers past them:
`thickness` = 2^k + N;
its centre plane, where its chunks are baked and drawn,
is halfway through that (`sliceCentre`).
The sites of finer grids an atom stands for span `siteLayers`
from the slab's first layer (`ownedLayers` around each atom's own).
A ray within a face's rhombus (section 4) meets the cells of a face's
layers in order, as it meets voxels;
only voxels on a cell's boundary, half in one cell and half in the next,
can be met out of order by a ray, by up to a voxel.
A slice is named by face, level, atoms, slab and cutoff (below),
so it always holds the same voxels and can be cached.

### Camera selection

`selectSlices(cameraZ, near, far, detail)` walks away from the camera
level by level.
Level 0 starts at the first layer at least `near` ahead
(a nearer slice, scaled by focal / d, could fill the screen).
Each next level begins at `until`,
the first of its slabs (a multiple of 2^(k+1)) far enough for its
thickness, at most a / detail with a the distance to its near side.
Level 0 is always allowed: a voxel can't be cut.

From level 6 on, the next level's atoms are larger,
and cells of two sizes do not nest:
near `until`, a site's cell of one size may lie before it
while its cell of the other lies after.
So the level's slabs near `until` get it as their cutoff
and hold only the sites whose larger cell begins before it;
the next level holds the rest (`selectSlices` gives the margins).
So every site in range is in exactly one slice,
which `slice-selection.test.ts` checks site by site.
A level's last slices may have their centre planes up to two layers
farther than the next level's first,
but hold sites before that level begins, so they are drawn first.

From camera z = 0 with detail 1 (`level:slab`, `|cutoff`):

```
0:1 0:2 0:3  1:2 1:3  2:2 2:3  3:2 3:3  4:2 4:3  5:2 5:3
6:2 6:3|256 6:4|256  7:2 7:3|512 7:4|512  8:2 8:3|1024 8:4|1024  9:2 9:3
```

Covering distance n takes O(detail · log n) slices,
1 + 1/R times as many as slabs of whole layers would,
up to `MAX_LEVEL`, then O(n).
Moving the camera along the axis keeps every slice
but a few where levels change (section 5).

### Native: cells instead of slices

The native spike has no slices (`bake/src/lod.rs`, `native/src/engine/chunks/renderer.rs`).
A chunk is a whole cell of 2^L·FCC,
drawn as the voxels of grid max(0, L − S) (`--shift S`)
over the plane through its site;
nothing else, the camera least of all, changes it.
Its whole voxels are the cell's relief (`cell_relief`), a pillar per column:
from the first whole voxel a ray can enter by an open face,
in its colour,
as deep as its side may show:
to the farthest front of the columns around it,
a ray leaving its side entering one of them;
where one has none in the cell, to the column's last voxel,
and on through the solid behind it past the cell, a cell deep at most,
as the next cell's relief starts at what a ray enters, not at its cut.
Like a slice's, a relief is off by an angle that detail bounds (section 2).
Or, drawn by the `splats` engine (`Renderer::relief` off),
its whole voxels a ray can enter are each one hexagon too:
a ray's first voxel is entered so and is in one cell,
so its hexagon, holding its outline, meets the ray;
no runs and no cut faces.
Or, drawn flat (the `flat` engine, an experiment;
`Flat` in `native/src/engine/chunks/compositor.rs`),
its relief is a picture of its columns' colours without their depths,
rows of them in a buffer (`flatten`),
drawn as one quad on a plane fixed in the world:
the camera's perspective moves and scales it.
Where two cells' pictures meet, each on its own plane,
a gap may open as wide as the rays' slant times
how far the edge columns' pillars reach from the planes,
widest along a surface parallel to the axis,
where a picture is a row or two of columns
and their pillars run through the cell.
So the plane is the one they reach least from,
and each frame each edge of a picture moves out by that reach
times the slant of the rays to it;
depth decides where neighbours overlap.
No gaps (`ground_along_a_face_shows_no_gaps`),
but what a deep pillar at an edge holds is smeared.
Its see-through voxels a ray can enter are each one hexagon (`cell_splats`),
drawn in the order a ray meets them (section 4, "See-through");
the relief takes them for empty.
Its tiles hold the sites' media too, with how much each stops
(`VoxelGrid::into_grid`):
a face is open where its neighbour lets rays through,
so a cell holds what shows through see-through voxels too.
The compositor's depth decides between cells:
finer levels in front, and within a level the nearer site
of the voxel of the column where a ray enters its pillar,
which, voxels being the lattice's Voronoi cells, is the one it meets first
where the pillar stands for whole voxels.
The camera only picks the chunks a face composes,
by its layer along the face's axis alone, in whole numbers:
level L claims the cells up to 2Q · 2^L layers ahead,
and draws those not all within the finer level's claims.
Where a cell overlaps finer ones, both draw what is there,
and the finer is in front.
A face's rays cross a level's cells layer after layer,
so they leave a level's claims at most once
and never meet a finer level after a coarser one.
Where they overlap, coarser voxels may show a little past the finer ones' edges.
The renderer keeps a face's cells picked for the eye anywhere near it
and the view turned a little (`Lod::pick`),
each with what it knows of it (`Seen`): its chunk, that it holds nothing,
or what it waits for;
each frame finds those the face composes now among them
(`Lod::composed`), without looking each up,
and picks anew once the eye or view leaves them:
what it knew of cells still picked moves to the new pick,
of the rest it keeps a while.
There is no coverage: every chunk in view is baked,
and the compositor culls what is hidden (section 4).

## 2. Detail and leaks

A slice is at most 1/`detail` of its distance thick (section 1),
the panel's "Slice Detail":
far voxels are small on screen,
so slices grow with distance and a range n takes O(detail · log n) of them.
Detail depends on nothing else, not the field of view or viewport.

Splats are drawn at their voxels' depths (section 3),
so slices show what perspective shows,
but for slivers where a hexagon stands in for a voxel's outline,
and for what a column hides behind a gap in it
(a valley between two hills: only its nearest voxel is kept,
and farther slices show through).
The thicker the slice, the more of such a column it holds:
against rays cast in perspective,
[`src/lib/relief.test.ts`](../src/lib/relief.test.ts)
finds no leak at the most detail
and leaks from slices as thick as their distance;
at the default, a few pixels zoomed in.

## 3. Chunks and splats (`renderer.ts`, `chunk-baker.ts`, `bake/`)

When a chunk cell is first baked, the voxels of the slice in the cell
(and a margin around it) with a face that is not covered by a neighbour,
or a face whose neighbour is in another slice (the slice is cut there,
and the cut must show), are indexed, with a bitmask of their open and cut
faces; buried voxels are dropped (`cell_surface`).
Which slice a site is in comes from a table of its atom over one period
of N·FCC, 2N along each of a, b and c (`bake/src/cells.rs`).
Cells interlock, so a face towards the camera may be cut by a farther slice.
Voxels are read layer by layer from the world's grid in the face's
lattice coordinates (section 4).

**Chunks.** A slice of level k is split into bricks (`chunkCell`).
Seen along the axis, the sites of M·FCC, M = 8 · 2^k, lie in rows M
apart, every 2M in a, alternate rows shifted by M:
the brick around one reaches M either way in a and M/2 in b,
so rows of bricks are offset by half a brick, like a wall's.
Brick (i, j) is row j's i-th, 2M·LAYER by M of the slice's centre plane.
Since a slice's thickness is about its distance / detail,
every chunk covers about the same part of the screen. Only chunks whose
cell reaches the screen are baked; empty ones are cached too, so
their cells are not searched again, but not shown.

**Empty cells.** A chunk draws the voxels of its cell and a margin around it
(`cellMargin`), within a few units of their b.
The renderer looks them up in the layers the slice's sites span.
`LayerIndex` (`voxel-grid.ts`, made by `bake/src/layer_index.rs`)
holds, per face, layer c and block of 8 columns a,
the least and greatest b of its voxels.
A cell with no voxels it can draw is never baked.
For the others, only the part of the cell within reach of those b
is checked against the screen and coverage (section 4),
so a treetop's chunk is not held to the sky above it.

**Baking along the axis.** A chunk is baked as seen along its face's axis:
each voxel at its site, its depth past the centre plane kept.
So it does not depend on the camera, and is baked once:
moving, only chunks coming into view are baked.
It is keyed by its level's grid (`World.version`):
when the grid moves, the level is baked again,
the old chunks standing in meanwhile.

**Splats.** A chunk is a picture of its slice along the face's axis
with a depth per point (`chunk_splats`).
Seen along the axis, a face's columns (a, b) tile the plane,
so a column's nearest voxel is all the axis sees of it:
of each column, the chunk keeps the nearest voxel
with an open or cut face that a ray in the face's rhombus (section 4)
meets from outside.
Each is painted as one hexagon (`Splat`),
the voxel's outline seen along the face's axis
(√2 wide and 2 high, its corners the rhombic dodecahedron's),
across the axis at its site:
its depth, in half layers, rides in its colour's alpha,
as a slice draws its voxels whole (`slice_numbers`);
a cell's splat has its depth as a number of its own,
and its voxel's alpha (`cell_numbers`).
Seen along a ray slanting σ off the axis, the outline is 1 + |σx| times
as wide (the voxel's tips a layer before and after move sideways)
and, within the face's rays, no higher;
the compositor widens it so,
and the splats of a surface leave no gaps from any ray of the face,
within a slice or across slices.
A splat's colour is that of the voxel's face towards the camera, −F
(the face a slice seen along F shows; `Reader` in `bake/` reads it),
shaded as the mean of its open faces' shades
(of its cut faces' where it has none),
each by the axes its normal leans towards
(`face_shade`, `voxel_colour`):
a voxel is shaded by the way its surface faces,
and coloured by what is seen of it along the face.
A host that lights the world
(native's computed lighting, `Lighting` in its panel, or `--light`)
hands the baker instead
the light each face of a voxel it shows sends out
(`bake/src/light.rs`, found by `worldgen/src/light.rs`),
so only what shows is lit:
a splat shows the mean of its open faces',
each as much as it faces the camera.

**Pillars.** A slanted ray sees past a column's edge,
where it would meet the column behind.
Where the axis lies in a surface (a lawn seen along a level face),
a column of it runs through the slice,
and a ray looking down would pass over its nearest voxel
into the next slice, under the surface: stripes.
So a kept voxel stands for its run (`run_ends`),
the solid voxels behind it in its column up to the last a ray may see,
and is drawn as a prism over them (section 4; `src/lib/pillars.test.ts`).
A run shows its first voxel's colour,
so a lawn's shades or a road's markings stretch along the axis,
unless no ray sees that voxel by an open face,
as one kept only for a cut through it:
the solid goes on in the nearer slice, which hides the cut,
and what shows of the pillar is the run behind it.
So it shows the first voxel of its run a ray sees (`run_ends`):
shaded by the cut, it would show the soil under a slope's grass,
or, lit, black, in bands along each slice's near side.

**Painting order.** Voxels by their layer, farthest first.
A pillar spans layers, so no order is right for every ray:
the compositor draws pillars by where a ray enters them,
and where two tie, the later painted.
Voxels of one layer meet only side by side;
widened, their hexagons overlap,
and which shows there is the same for every ray,
so near a slant's extremes a voxel's neighbour in its layer may show
on up to |σx| of its width.
`src/lib/chunk-baker.test.ts` checks against rays cast along the axis
that every voxel one meets first is painted,
and `bake/src/splats.rs` that a slanted ray enters first
the pillar of the column it meets first, or its neighbour so.

**The baker** is Rust (`bake/`), compiled to WebAssembly
with the worlds and their engine, which depend on it
(`worlds/`, `worldgen/`, section 6)
and inlined into the app (`wasm.ts`), so it compiles synchronously
in a worker, on the main thread (for coverage, section 4) or in Node,
with no files to fetch.
`ChunkBaker` hands it a world once:
the grid and the materials' colours.
It knows the faces itself (`lattices`, in the order of `FACES`),
and shades the colours per face (`tables.rs`).
Then each chunk is one call,
and its splats are copied out of the WebAssembly memory.
A chunk's voxels are those of its cell and a margin around it
as wide as its splats may be drawn (`Splat::reach`, `cell_surface`).
The native spike (`native/`) links the baker instead,
a baker per worker thread sharing one copy of the tiles (`share_tiles`),
and has the renderer, slice selection and coverage in Rust too.

Each chunk is clipped to its cell, by where pixels' rays cross its
centre plane: splats that may not be drawn in it are dropped,
and `originX`/`originY` and its extent record where on the centre plane
the rest may draw. A chunk does not depend on the camera's position,
so it is baked once when it first comes into view and never again.
Splats have no resolution: the compositor draws them at the screen's.

**Workers.** In the app, a frame never waits for chunks:
each render lists the missing chunks in view,
and up to 8 workers, each with a copy of the world, bake them
(`WorkerBakery`, `bake.worker.ts`, `ChunkBaker`).
Until a chunk arrives, the cell's latest bake (of its level's older grid)
stands in;
if that one is covered (section 4), nothing is baked.
A cell never baked shows nothing yet, e.g. right after start.
Those cells are baked first, near to far across all faces;
then the re-tiled segments below, those missing fewest chunks first;
then the rest, near to far.
When the camera moves along a face's axis, its slices re-tile
where levels change.
`matchTilings`
pairs the new slices with the ones drawn last:
slices in both stand for themselves,
and the new ones between two of them
for the old ones between the same two;
until a segment's new slices have a bake in every cell they show,
the old ones are drawn instead.
Moving fast, segments often re-tile again before they are baked;
finishing those missing fewest chunks first
shows more of the chunks baked meanwhile.
The list is split by cell between the workers.
Each render's list replaces a worker's previous one (`bakeHandler`),
so workers keep baking without waiting for the main thread,
which may be busy for hundreds of milliseconds.
Racing ahead, workers also bake far chunks that nearer ones turn out to cover.
The app renders in a task queued behind the workers' messages, not in the animation frame:
while a key is held, the browser puts input and frames before other tasks,
and baked chunks would wait until it is released.
Without a bakery (tests), `render()` bakes missing chunks itself.

## 4. Faces, turning and compositing (`view.ts`, `renderer.ts`, `compositor.ts`)

**Faces.** The camera has a yaw and a pitch, no roll (`viewBasis`).
To look all around, the world gets 12 slice stacks,
one per face of a rhombic dodecahedron around the camera,
i.e. one per nearest-neighbour direction F of the lattice (`FACES`, e.g. `+x+z`).
A face is a rotation, forward = F/√2, up = the world axis across F,
right = up × forward, and its stack is sections 1–3 in those coordinates.
The rotation maps sites to sites, and every face sees the same lattice:
site p is at lattice coordinates (a, b, c) = (p · √2·right, p · up, p · F),
all even or all odd, placed at (a·LAYER, b, c·LAYER);
layer c is a rectangular grid of sites, √2 by 2 apart,
and the next layer is the same grid shifted by half a cell.
The world is read in these coordinates (`Reader` in `bake/`), with faces
shaded by the way they face in the world (`palettes` in `bake/`).

Each face draws the rays closer to its axis than to any other face's
(`inFace`): |x| + √2·|y| ≤ z, a rhombus of tangents reaching 1
towards the world axes (where 4 faces meet) and 1/√2 towards the cube's
diagonals (where 3 meet). The faces tile the screen;
the fragment shader discards the rest of their chunks,
deciding by the pixel's ray, the same for all layers of a face,
with a hair of overlap so no seam pixel falls to neither face;
native's by the ray's two largest world components, one face a pixel (`owns`).
Looking straight along a world axis, as the scenes' walls and streets
invite, puts a corner of 4 faces in the middle of the screen.
"Show Face Seams" draws the seams over the picture.
The native panel can also colour the splats by layer or by chunk (`Colours`).

Chunks depend on neither where the camera is nor where it looks,
and detail on nothing else, not the field of view or viewport.
So turning, zooming and resizing keep every slice and chunk;
only chunks coming into view are baked,
and faces coming into view are sliced.

**Placement.** `render()` returns a `Frame`:
the view (orientation, viewport W × H in CSS pixels,
`focal = max(W, H) / 2 / tan(fov/2)` in pixels)
and, face by face, for each slice near to far,
its distance d (to its centre plane), its chunks and its fog.
A chunk draws within a rectangle on the plane z = d in its face's coordinates,
its top-left corner at (originX − camera.x, originY − camera.y, d)
relative to the camera, as wide and high as its extent.
A camera-relative point p goes to (`projector`, and the vertex shader;
right, up and forward are the camera's axes in the face's coordinates):

```
x = W/2 + focal · (p · right) / (p · forward)
y = H/2 − focal · (p · up)    / (p · forward)
```

A face is skipped if the screen shows none of its pyramid.
Chunk cells are searched in the bounding box of the part it shows
(`faceTangents`);
a cell, then its chunk's content, is kept
if its projected corners' bounding box (`screenBox`) overlaps the viewport.

**Covered chunks.** Near layers cover most of what is behind them.
`Coverage` (`bake/src/coverage.rs`, run on the main thread
in its own instance of the baker's WebAssembly)
marks which small square sub-blocks of the screen
the face's layers drawn so far cover;
a cell whose box, plus a margin around it, lies on covered sub-blocks
is neither drawn nor baked,
and a baked chunk is not drawn if each of its tiles
with something drawn lies on covered sub-blocks.
Of a chunk that is drawn, only the clusters of its splats that may show are,
as the GPU's time goes with the splats drawn:
the bake workers group its splats by where their sites are,
each group with a box holding them as drawn from any ray of the face
(`cluster` in `chunk-baker.ts`),
and a cluster whose box lies on covered sub-blocks is left out
(`show_clusters`).
Each pixel shows one face, so coverage starts over for each face,
and sub-blocks outside the face count as covered.
A sub-block is covered once one layer's chunks draw all of the face's part of it,
but different sub-blocks may be covered by different layers:
leaves, trunks and the ground together close what none of them closes alone.
Bakers fill a mask of what a chunk draws on its centre plane,
its splats' pixels at `mask_scale`, about as many per chunk at every level,
and record which blocks of it are all drawn whatever the camera,
and which may be partly (`fill_mask`, `solid_blocks`):
drawn in relief, a splat moves on the plane with the camera,
so a block is all drawn if it is from the axis and the corners of the
rhombus of rays, and maybe drawn if a splat may reach it from any ray;
the blocks under a sub-block, plus a pixel around, must all be drawn.
The mask is coarser than the screen (`todo.md`):
a gap between splats narrower than a mask pixel may hide nothing behind it.
A chunk is handed to `Coverage` when first drawn or tested,
which keeps its tiles and a table of sums of its all-drawn blocks,
so testing the blocks under a rectangle takes four reads;
chunks unused for a while are dropped, and handed over again if needed.
Only chunks already baked cover anything.
When the camera moves, cells coming out from behind nearer layers
show nothing until baked, a frame or two.
The rays through each sub-block are kept per face while the camera only moves,
and turning, found only in the cells the face crosses or holds
(a face's side is a half-plane of the screen).
The native renderer has no coverage (section 1):
it bakes every chunk in view, and its GPU culling hides what is covered.

The `Compositor` draws a frame onto one opaque WebGL 2 canvas:
per chunk the splats of its clusters that may show, instanced,
then the sky behind them.
A chunk's splats are a buffer in cluster order,
and each run of neighbouring clusters shown is one instanced draw.
A splat is a prism of triangles, its hexagon swept along the axis
over its run (section 3), those facing away culled;
without a run, the near hexagon alone.
The camera sees at most the near hexagon and three sides,
so each splat is one triangle strip of those,
the sides picked in the vertex shader.
A prism spans layers, so no painting order is right for every ray
(a lawn's run behind a trunk on it):
each vertex has a depth within its layer's share of the depth range,
by how far along the axis it is.
Splats are opaque, so it draws near to far with a depth test:
what is nearer rejects what is behind before the fragment shader runs
(the forest stacks dozens of screens of chunks). Depth is first the layer's index,
not its distance: a face's slices are parallel planes, so every ray
meets them near to far, and faces share pixels only along seams
(the first face drawn wins there).
Within a layer, a chunk draws only on its own part of the plane,
decided by each pixel's ray, so chunks neither overlap nor leave seams.
A chunk's splats are uploaded when it comes into view,
cluster by cluster, each near to far (its painting order reversed),
so its nearer splats reject most of those behind them;
where two of a cluster are as near, the one painted later shows.
They are deleted when it leaves.
No antialiasing, so voxel edges stay crisp.

The native compositor (`native/src/engine/chunks/compositor.rs`) draws only what may show.
Its chunks are cells' reliefs (section 1),
and the bake workers put a chunk's columns in tiles of 4 × 4,
empty columns too, so that a column's place says where it is,
each tile with the box of its columns' outlines along the axis
and how far before and past the plane they reach:
the culling finds where the camera's rays draw them.
A column's outline is here its centre's Voronoi cell among the columns',
not its voxels', which interlock: these share out the plane.
A tile is drawn as one quad holding its box, seen from the eye;
each pixel's ray crosses its columns' cells one after another
from where it enters the box,
and stops at the first pillar it meets in the cell it crosses
(`march` in `compositor.wgsl`),
writing the depth of the site of that column's voxel there (`depth_of`),
a hair farther the deeper it is past the pillar's front,
so that where pillars of two cells meet one voxel,
the column it is the front of shows,
and tiles can be drawn in any order;
never nearer than the quad, at the tile's nearest front,
so the GPU still tests depth before the pixel marches
(`early_depth_test`).
Each frame it draws the tiles that showed in the last,
reduces their depth to a pyramid of farthest depths,
then tests every tile in view against it on the GPU, a thread each,
draws those newly showing and keeps which show for the next frame
(two-phase occlusion culling).
Draws are listed per tile on the GPU, roughly near to far,
so nearer tiles mostly hide what is behind them early.
It draws into a picture of its own, which the window (or `--shot`) shows.
Where whole voxels are splats (section 1),
the bake workers put them in clusters of a few nearby on its plane instead,
each with the box of their outlines,
drawn and culled as the tiles are,
each splat at the depth of its site,
widened by the slant of the ray to it, as on the web;
a chunk's slot in the table comes to the vertex shader
as its draws' first vertex,
so its splats are uploaded as the workers made them.

**See-through.** Native draws a site's medium see-through
(`KindDef::medium`, as mist) where its pieces don't show;
those come whole, as on the web.
A see-through splat's alpha is how much of the rays
through its voxel's site the voxel stops,
along the voxel's mean chord (worldgen's `CHORD`).
The bake workers put a chunk's see-through splats in clusters,
layer by layer from the nearest,
culled against the reliefs as tiles are,
and the compositor draws them after the reliefs,
in front of them, into a picture of their own,
nearest first, each under those before it;
then lays that over the picture.
That is the order a ray meets their sites in:
a face's rays meet its levels finest first (`lod.rs`),
a level's cells as their sites are near the eye
(at the face between two, the nearer one's site is nearer: they are Voronoi cells),
and a cell's splats layer by layer, its layers being planes across the face's axis.
The cull lists a draw for each see-through cluster in that order,
of none of its splats where it does not show.
Each splat is its voxel's share of a field over its layer,
a tent over twice its outline:
a layer's sites are on a grid twice their outlines apart,
so the tents sum to one across it,
and a ray takes as much of a layer as its path through it is long;
as outlines, half the plane each layer's, ray after ray along a seam
met their edges together, and drew lines.
A tent reaches past its cell, over a neighbour's pixels,
and there the two cells' order is their sites', not the ray's:
a small error where alpha is small, as in a medium.
So that none counts twice,
each pixel is one face's only,
and a coarser chunk leaves out the see-through voxels
whose cells of the next finer level that level draws
(`Lod::finer_reach`; the baker gives each splat its finer cell's layer).
They add up in 32-bit floats:
hundreds of layers of a thin medium, each letting through 0.999,
in 16 rounded every step one way, and stopped a tenth too much.
A test holds a slab of medium to exp(−extinction · path),
thin and thick, along the axis and slanted
(`a_medium_of_splats_lets_through_as_much_as_its_extinction_says`).

**Fog.** A slice at distance d keeps `exp(−d / F)` of its own colour;
the rest is the horizon colour, mixed in per pixel.
F is a fixed share of the render distance (`FOG_PER_RENDER_DISTANCE`),
so the far cut-off stays hazy.
d is measured along the face's axis, not the ray:
fog is the same on both sides of a seam,
but a little lighter towards a face's corners than true distance would make it.
The sky fades from the horizon colour at eye level to blue
higher up, along each pixel's ray, and gets the fog of the render
distance (or of the last slice, if farther).

### Native: another engine, rays through the voxels

The native window switches at run time between engines (`Kind`),
each drawing the same tiles its own way, with its own shaders:
chunks above, their whole voxels reliefs or splats (`reliefs`, `splats`),
rays through the tiles' voxels (`native/src/engine/voxels.rs`),
with no chunks, slices, faces or baking, described here,
or a prototype, rays through slabs along each face (`native/src/engine/slabs.rs`):
per face and level, slabs of layers baked around the eye,
each a picture of its columns' first voxel and run,
baked again on a thread as the eye moves or turns.

They share one interface (`native/src/engine.rs`), in two halves.
On the main thread an `Engine` makes a `Frame` of what a `Look` sees:
a `Picture`, and what it wants of the world (`Wants`);
what it makes of the tiles, on threads of its own, is its own business.
On the GPU its `Painter` keeps what it needs from frame to frame
and draws its pictures into one of its own,
which the window shows under egui's panel, or a shot saves.
A device holds one painter at a time (`Easel`, `native/src/gpu.rs`).
So the window, `--shot`, `--bench`, `--stroll` and the tests
name no engine:
a view is complete when its engine lacks no tile and makes nothing more.

In the voxels engine a ray goes from rhombic dodecahedron to dodecahedron across their faces;
each level's voxels are read up to `REACH` of its own sites from the eye,
then the next coarser level's, as far as the render distance.
The GPU holds of each level the bricks of a window around the eye,
`REACH` all around and a little more;
its table of the bricks' slots in a pool is a torus,
a brick's entry at its coordinates modulo the window's size,
so the window moves a brick at a time
and only the bricks that came into it or left are touched.
Tiles never change once made, and are packed for the GPU where they are made,
so a brick is uploaded once, when it comes into a window.
A brick of a tile not made yet is unknown, not empty,
and rays take the next coarser level there:
what is missing shows coarser, never as a hole.
Empty space is crossed on the lattice too, not by boxes:
beside its bricks, a window counts for each cell of 8·FCC and of 32·FCC
(rhombic dodecahedra about every other corner of its bricks, or of blocks of 4³ of them)
the bricks around it whose voxels meet it, or that are unknown.
A ray in a cell that none meets goes across it at once, face to face,
as it goes across a voxel: whatever it met in the cell would meet the cell.
A ray that runs out of steps shows magenta: a march must not stick,
and if one does, it must not hide.
One ray is traced for every 4 × 4 pixels;
each pixel takes the nearest of the voxels its four nearest samples met
that its own ray enters, so edges stay sharp.
See-through voxels add up in the order the ray meets them,
each stopping `1 − (1 − alpha)^(path / chord)` of what is left,
its path the ray's through the voxel, until little is let through;
a pixel takes the medium in front of what it sees
from those of its samples that saw the same.
Tests hold the window's table to a new one's after any moves and tiles
(`a_window_moved_and_its_tiles_changed_holds_what_a_new_one_would`),
its cells counted hollow, at its edge too, to ones no voxel it holds meets
(`no_voxel_the_window_holds_meets_a_cell_counted_hollow`),
its rays to their steps (`no_ray_of_the_voxels_runs_out_of_steps`),
the picture to the same one with the world and the eye moved by a vector of the lattice
that is not one of the bricks' (`voxels_look_the_same_wherever_the_bricks_fall`)
and a slab of medium to exp(−extinction · path), as the splats' test does.

The panel picks the lighting apart from the engine (`Lighting`):
voxels shaded by the way their faces face, or computed,
by worldgen's `Light`, one model and one cache of light (`Shared`) for every engine;
each engine only shows it its own way.
The splats' bakers light the voxels they show, face by face;
the slabs show none.
For the voxels, the world's threads light each tile once made
of the levels rays read,
after every tile to make (as it takes several times as long),
nearest first in each level's own sites, as rays read them:
each voxel a ray can meet gets one colour, not knowing which face it meets,
the mean of the light from its faces that light gets out by (`Light::seen`).
It goes to the GPU beside the bricks, a word a site, in a pool of its own.
Till a tile is lit, its bricks are not on the GPU, as if it were not made:
rays take the next coarser level there,
so what is not lit yet shows coarser, lit, never shaded by another light.
A tile lit is a new tile in the place of the one made,
so tiles still never change, and its bricks are uploaded as any new tile's.

## 5. Caching

Slices are keyed by face, level, atoms, layer and cutoff (`sliceKey`)
and hold their chunks by cell (`cellKey`), the latest bake beside older ones standing in.
Insertion order of the slice `Map` is the LRU order. Slices and chunks that
leave the view lose their buffers but stay cached so moving back
reuses them: up to `MAX_CACHED_SLICES` slices, and hidden chunks
up to `MAX_HIDDEN_CHUNK_BYTES` (oldest slices' first). Chunks
of cells in view are kept even if they draw nothing, or they would be
baked again on every render.

| Event | Effect |
|---|---|
| x/y movement | Chunks still in view reused; chunks entering the view baked; placements update |
| turning | Slices and chunks reused; chunks entering the view baked |
| field of view change | Slices and chunks reused; only the compositor's focal length changes |
| detail change | Slices still needed are reused |
| small z movement | Slices reused but where levels change: there a few slices get a new cutoff or level, the ones they replace standing in until baked |
| resize | Slices and chunks reused; placements update |
| world regeneration | Whole cache cleared |

Per-render stats (`UpdateStats`) report chunks reused vs shown for the
first time since they were baked, chunks still baking, voxels painted in chunks baked
since the previous render and the bytes of all cached chunks;
`SessionStats` accumulates them. `renderTime` is main-thread time, which
excludes baking in workers.

## 6. Worlds (`world.ts`, `worldgen.ts`)

**The lattice** (`fcc.ts`). Voxels sit on the face-centred cubic lattice:
the integer points (x, y, z) with x + y + z even.
A voxel is a site's Voronoi cell, a rhombic dodecahedron,
the points within |dx| + |dy|, |dy| + |dz| and |dx| + |dz| ≤ 1 of it,
with 12 rhombic faces, one towards each nearest neighbour,
(±1, ±1, 0) and permutations, √2 away; its volume is 2.
It is the lattice of [complex-life](https://github.com/SupraSummus/complex-life),
whose coordinates (q, v, w) stack close-packed layers along one axis,
turned so that the world axes are its 4-fold axes:
x = q + w, y = q + v, z = v + w.
So walls and floors along the axes look alike in every direction,
and the 12 faces of section 4 map sites to sites.

**Storage** (`voxel-grid.ts`). A world is a voxel grid per level of
detail (section 1) plus a spawn point: one material id byte per site of a
box, 0 for empty.
A voxel has a material on each of its 12 faces (`VOXEL_FACES` in `fcc.ts`),
what is seen of it looking at that face:
a coarse voxel stands for its cell, which may look different from each side.
Mostly the faces are its site's material;
the sites whose faces differ are listed apart (`faces`).
Each reader shows the faces it sees:
the baker the face towards its slices' camera (section 3),
rays (`worldgen/src/picture.rs`) the one they enter by.
The box is cut into bricks 8 units wide, 256 sites,
and only bricks with voxels are stored.
A worker reads its copy of the grids for every face (`Reader` in `bake/`),
without turned copies.
Renderer distances are in world units.

**Scenes** describe shapes in metres and fill the sites inside them;
each world picks its own scale, the renderer assumes none.
The worlds have `UNIT_SIZE` metres per unit
(`worldgen/src/units.rs`).
They are endless and can be generated in any box alone
([generators.md](generators.md)),
by `worlds/` on the engine of `worldgen/` (Rust),
in the workers' WebAssembly (`worldgen.ts`);
the app generates one box of either, its extent, into a grid.
The forest's ground is a heightfield sampled per column of sites;
trunks are columns at least 2 units square,
crowns leaf clusters (balls) or a solid of revolution (spruce),
branches tubes and fallen logs capsules, so thin parts stay one piece on the lattice
(a single column of sites only touches at corners).
The city (`worlds/src/city.rs`) is planned on a 0.25 m grid:
each plan cell is a box holding four sites, so walls stay flat and frames one piece.
The renderer only draws faces that no neighbour covers.

## Limitations

- A voxel's hexagon holds its outline, not more:
  near a slant's extremes it may hide a neighbour in its layer that is in
  front (section 3).
- A voxel is painted as one hexagon of one colour holding its outline,
  that of its face towards the camera:
  its other faces' shapes, shades and materials are lost,
  and a silhouette is a row of hexagons.
- Within `NEAR_DEPTH` of the camera along a face's axis nothing is
  drawn, and near voxels seen at a steep angle are the least accurate.
- Inside a voxel (the camera has no collisions), the cut faces of the
  nearest slices fill the screen.
- At a seam, the two faces slice the world differently, and paint a
  layer's voxels in different orders.
  Looking along a world axis, four faces meet in the middle of the screen.
- More faces are in view at once than with a cube's six (four or more,
  against one to three), each with its own near slices,
  so more is baked.
- Slices are slabs of whole cells, drawn in order, but a voxel on a
  cell's boundary is half in the next layer of cells: where it overlaps a
  voxel of the next slice on screen, the nearer slice's one may show
  though the other is in front, by up to a voxel.
- Slabs of whole cells are ragged on both sides,
  (R + 1) / R times as deep as their R layers of atoms (section 1),
  1.06 for the 16 of the far levels,
  so they take about that many more slices than slabs of whole layers.
- The web compositor draws every splat in view every frame (`todo.md`).
- The world's grids are held by the main thread and by each worker.
- Moving far, a level's grid is generated again where the camera went
  (section 1): until it is in, the far edges of its slices show nothing,
  and moving faster than it generates, the near ones too.
