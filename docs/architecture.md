# Rendering Architecture

The world is cut into slices, each a slab between two layers of a
coarser FCC lattice made of whole cells of a finer one,
and each slice into chunks, bricks of the lattice seen along its axis;
each chunk in view is rasterised once, flat, as seen from a lattice site
near the camera, into a pixel buffer, and WebGL draws the buffers
on their slices' planes in perspective.
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
World (voxels) ──selectSlices──▶ slices ──chunks in view──▶ cached pixels ──Frame──▶ WebGL ──▶ screen
   world.ts      slice-selection.ts, cells.ts                 renderer.ts          compositor.ts
                                                               │      ▲
                                                 missing chunks▼      │pixels
                                                 bake workers (worker-bakery.ts)
```

## 1. Slice selection (`slice-selection.ts`, `cells.ts`)

Two constraints: slices must be thin enough not to show what perspective
hides (physical, section 2), and named the same wherever the camera is,
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
Its atoms are the cells of 2^ℓ·FCC, ℓ = `atomLevel(k)` = max(0, k − 6):
up to level 6 single voxels, slabs of 1 to 64 layers of them;
above, 64 layers of cells of 2, 4, … 64 layers.
Slabs of levels with the same atoms nest: one of level k + 1 is two of level k.
A slice splits no atom, except where the atoms grow (cutoffs, below):
there the smaller cells along the boundary are shared by two levels.

**Levels of detail.** A world is a grid per level of detail
(`World.levels`):
grid ℓ holds the sites of 2^ℓ·FCC, in its own units,
each a voxel standing for its cell.
A slice draws its atoms as the voxels of grid ℓ.
In that grid's units it is a slice of level k − ℓ of single voxels,
so the baker bakes it as one and scales the chunk back
(`in_grid` in `bake/src/lib.rs`).
Each grid is the one before it coarsened (`coarsen`):
a coarse voxel is full if a quarter of its cell is,
so a floor or wall one voxel thick stays one,
and solids grow by up to half a coarse voxel.
A slice is about 1/detail of its distance thick
(section 2, detail ≈ 9.5 by default),
so from level 6 on, a slab of 64 atoms,
a voxel is 1.4 to 2.7 pixels across at 1280×800 and 90°.
Level 7, the first of coarse voxels, begins about 230 m away in the forest:
up to there level 0's grid must hold the world.

Why many layers of atoms and not one:
cells of neighbouring layers interlock, as voxels do.
Along the axis, a layer of rhombic dodecahedra N layers apart is 2N deep,
in columns alternating between even and odd layers;
a slice is drawn flat and the leak bound limits its depth (section 2),
so slices of single layers of cells would take twice as many.
In a slab of R layers of cells only its two faces are ragged:
it is about (R + 1)·N deep for R·N layers, 1.02 times for R = 64.
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
A slice is named by face, level, slab and cutoff (below),
so it always holds the same voxels and can be cached.

### Camera selection

`selectSlices(cameraZ, near, far, detail)` walks away from the camera
level by level.
Level 0 starts at the first layer at least `near` ahead
(a nearer slice, scaled by focal / d, could fill the screen;
the renderer's four layers also keep it two layers past its eye, section 3).
Each next level begins at `until`,
the first of its slabs (a multiple of 2^(k+1)) far enough for its
thickness, at most a / detail with a the distance to its near side.
`detail` comes from the leak tolerance (section 2).
Level 0 is always allowed: a voxel can't be cut.

From level 6 on, the next level's atoms are larger,
and cells of two sizes do not nest:
a site's cell of one size may lie before `until` while its cell of the
other lies after, by less than 3N (N the smaller cells' spacing).
So level k goes on to the slab starting at `until`,
and its slabs within reach (ending past until − 4N − 2)
get `until` as their cutoff and hold only the sites
whose cell one size up begins before it;
the next level holds the rest.
Such a level is also at least 3N long,
so that a site of its first slabs never belongs to an earlier level.
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
up to `MAX_LEVEL` = 12 (from about 7.6 km in the forest), then O(n).
Moving the camera along the axis keeps every slice
but a few where levels change (section 5).

## 2. Occlusion leaks

A slice is flat: everything in it is drawn at the scale of its centre. In
true perspective, the top and side faces of nearer voxels hide what lies
behind them; in slices, the gap between two slices can instead show a
cross-section of the farther slice. Too-thick slices show what perspective
hides, e.g. stone under the lake's water surface.

Bound, with ε = the "max occlusion leak" setting (default 1.5°):
**geometry that perspective hides never shows by more than the angle ε**
(or, closer than 2 · `detail` layers, by more than two layer spacings;
see below).
It is an angle because slicing knows neither the field of view nor the viewport.
The compositor makes it ε · `focal` pixels (`leakPixels`):
17 at 90° on a 1280 px wide view, 4× that zoomed in 4×.

**Derivation.** Every step below is stated and checked on random samples,
plus end to end against ray-cast perspective, in
[`src/lib/leak-bound.test.ts`](../src/lib/leak-bound.test.ts); read it to
follow or change the reasoning. In short, for one lateral axis, a slice
of level k with near side a layers ahead, t layer spacings thick
(`thickness`, as voxels reach past their sites), centre plane at D,
is baked in perspective from its eye (section 3),
a site of N·FCC (N = 2^k) whose cell holds the camera:

1. A point the camera sees along tangent σ and the eye along τ, at depth z,
   is drawn (σ − τ)·(z − D) off at the plane.
2. In the slice |z − D| ≤ t/2, so every edge is off by at most |σ − τ|·t/2.
3. A hidden voxel shows past its occluder by at most both edges' errors:
   |σ − τ|·t (the one modelling assumption; checked end to end).
4. The camera is in the eye's cell, at most N layers along the axis
   and √2·N world units off at the cell's corners, so for the face's rays
   |σ − τ| ≤ 2N / d, d the point's depth from the eye in layers (`eyeError`).
5. As an angle the leak is at most 2N·t / (a·(a − N)),
   2N·t·focal / (a·(a − N)) pixels,
   and slices are at most a / detail thick.
6. With r = N / t, at most 64/65 (from level 6 on), that is at most
   2r / (detail·(detail − r)) (`leakAngle`), which `leakFreeDetail` makes ε,
   rounded up to a quarter octave:

```
detail = (r + √(r² + 8r / ε)) / 2  ≈  √(8r / ε)
```

Measured in world units instead of an angle, the bound would be the
same thickness at every distance and a slice count linear in the range.
What matters on screen is the angle, and far voxels are small, so slices
grow with distance. Slices are at least one layer (two spacings) thick, though,
so closer than 2 · `detail` layers the leak is bounded by two spacings
instead (an eye error of at most 1, below), which can be more than ε.

This holds straight along the stack's axis.
A turned view draws a plane larger towards the screen's edges
(by 1 / cos² of the angle from the screen centre),
so leaks there can be up to 2× larger in a 90° view.

Slices of one face for range 400, ε = 1/40 (1.4°): detail 9.5 and 78 slices.
Chunks baked along one direction each, nearest a grid of directions
spaced by how wide they are, needed detail 26.9 and 170 slices:
their error grew with the chunk's width, the eye's only with its spacing.

The bound is nearly tight. It cannot be pushed to zero by thinner slices:
even single layers leak, because the back faces and the inside of the slab
are flattened.

## 3. Chunks and rasterisation (`renderer.ts`, `chunk-baker.ts`, `bake/`)

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
so rows of bricks are offset by half a brick, like a wall's,
and each brick is the shadow of one column of cells of M·FCC,
with six neighbours like the hexagon around its site.
Brick (i, j) is row j's i-th, 2M·LAYER ≈ 11.3 · 2^k by M = 8 · 2^k
of the slice's centre plane, at most 16 times its thickness.
Since a slice's thickness is about its distance / detail,
every chunk covers about the same part of the screen. Only chunks whose
cell reaches the screen are rasterised; empty ones are cached too, so
their cells are not searched again, but not shown.

**Empty cells.** A chunk draws the voxels of its cell and a margin around it
(`cellMargin`), within a few units of their b.
The renderer looks them up in the layers the slice's sites span.
`LayerIndex` (`voxel-grid.ts`) holds, per face, layer c and block of 8 columns a,
the least and greatest b of its voxels.
A cell with no voxels it can draw is never baked
(on the forest, about 3 in 10 cells in view).
For the others, only the part of the cell within reach of those b
is checked against the screen and coverage (section 4),
so a treetop's chunk is not held to the sky above it.

**Baking from the eye.** The chunks of level k are baked in perspective
from its eye (`eyeOf`): the site of N·FCC (N = 2^k, the level's own
lattice) whose cell holds the camera, the same for every face.
Point p goes to where the line from the eye through p meets the centre
plane, clipped to the cell.
A cached chunk is keyed by its cell and its eye, so it stays valid while
the camera stays in that cell; when the camera leaves it,
the whole level is baked again (section 5).
The eye is at most N layers ahead of the camera, and slices keep their
voxels at least 2N layers past it (`NEAR_DEPTH` for level 0,
`leakFreeDetail` for the rest), so its rays are at most 1 off the
camera's (section 2): rays from the eye through the part of a chunk the
face shows are within 1 of the face's rays, which bounds the margin a
cell's surface needs (`cellMargin`).
Drawn are the open (and cut) faces facing the eye,
normal · (p − eye) < 0 (`chunk_faces`).
Shading, by the axes a face's normal leans towards, is shared with the
reference viewer (`faceShade`).

**Painting order.** Voxels by their site's distance from the eye,
farthest first.
Voxels are the cells of their sites' Voronoi diagram:
a ray from the eye passing from one into another crosses where their
sites are equally far, coming from the nearer site's side,
so along every ray a voxel met first is nearer the eye,
whatever the ray's direction.
A voxel's faces facing the eye never overlap.
`src/lib/bake.test.ts` checks chunks pixel by pixel against rays cast
from the eye through the slice's voxels.

A face, a rhombus, projects to a convex quadrilateral, filled exactly:
a pixel is filled when its centre is inside, with half-open edges,
each evaluated from its upper end,
so neighbours meet without gaps or overlap (`fill_faces`).
No antialiasing: a soft 1-pixel edge magnified 20× on screen is a blur.

**Filling gaps.** A slice draws a point at depth z′
that the camera sees along σ and the eye along τ,
(σ − τ)·(z′ − c) off (section 2).
Within a slice that keeps surfaces whole;
only where a surface goes on into another slice can it tear.
At the boundary between slices k (nearer) and k + 1,
the nearer one draws its far edge (σ − τ_k)·s_k/2 off
and the farther one its near edge −(σ − τ_{k+1})·s_{k+1}/2 off,
so on screen they are apart by
J = (σ − τ_k)·s_k/(2D_k) + (σ − τ_{k+1})·s_{k+1}/(2D_{k+1}).
(s is the slice's thickness.)
A surface running along z moves towards the screen centre
with depth: J opens a gap where σ is farther from the centre than τ,
and makes the slices overlap (the leak of section 2) where it is nearer.
A gap is |J| − t/D_{k+1}, as the cut of a solid t units thick covers t/D:
only thin things tear, e.g. a one-layer floor seen from high above.
So each slice closes its share:
the open faces of voxels with a neighbour in another slice
grow towards where they belong, by (σ − τ)·d for the σ that open a gap,
d the depth of the voxel's tips from the centre plane
(slices interlock, so where they meet is the voxel's own depth, at most s/2)
(`chunk_faces`; |σ − τ| is at most the eye error of the voxel's depth,
section 2).
Faces the eye doesn't face grow too where some camera in the eye's cell
faces them: normal · (p − eye) < N, the cell's reach towards them.
A grown face is its quadrilateral swept over the rectangle of its growth,
a convex polygon of up to 8 corners;
grown faces are painted nearest first into pixels the chunk left empty
(`fill_faces`). Cut faces and the inside of slices don't grow.
"Fill Gaps" turns it off.

**The baker** is Rust (`bake/`), compiled to WebAssembly
and inlined into the app, so it compiles synchronously
in a worker, on the main thread (for coverage, section 4) or in Node,
with no files to fetch.
`ChunkBaker` hands it a world once:
the grid, the voxel's faces in lattice coordinates and the palettes (`bake.ts`).
Then each chunk is one call,
and its pixels are copied out of the WebAssembly memory.
A chunk's voxels are those of its cell and a margin around it
(`cell_surface`, cached per cell, so other eyes reuse it).

**Resolution.** A chunk of a slice of thickness t, both in units of the
grid it reads (section 1), is drawn at `render_scale` pixels per such
unit: 40 / t, between 1 and 10 (2 to 20 pixels across a voxel).
Such a slice lies at distance ≥ t · detail
layers, where a unit is at most focal / (t · detail · LAYER),
about 34 / t screen pixels at 1280 × 800 and 90°,
so more pixels would not show there
(larger views and narrower fields of view magnify chunks, see `todo.md`).
The nearest slices get 10, so
magnified rhombi keep their slanted edges. Other settings scale the chunk
on screen.

Each chunk is cropped to what is drawn; `originX`/`originY` record
the world position of its top-left corner on the centre plane. The image
depends on the camera's position only through the eye, so a chunk is
drawn once when it first comes into view (from its eye) and is never
redrawn.

On the forest at 1280 × 800, 90°, looking along an axis (yaw 0):
316 slices of 5 faces, ~530 chunks drawn, ~26 Mpx in view;
a first render baking all of them takes ~4 s on one thread.
Looking diagonally (yaw 45°): 393 slices, ~490 chunks, ~20 Mpx, ~2.4 s.
render() with everything cached takes ~17 ms in Node.
Chunks baked along one direction each (section 2) took 676 slices,
~3200 chunks, ~62 Mpx and ~42 ms.
But moving bakes more:
walking 8 units forward, ~1.3 to 1.8 times as many chunks,
as the near levels' eyes, a unit or a few apart, move every few steps.

**Workers.** In the app, a frame never waits for chunks:
each render lists the missing chunks in view,
and up to 8 workers, each with a copy of the world, bake them
(`WorkerBakery`, `bake.worker.ts`, `ChunkBaker`).
Until a chunk arrives, the cell's latest bake (from an older eye) stands in;
if that one is covered (section 4), nothing is baked.
A cell never baked shows nothing yet, e.g. right after start.
Those cells are baked first, near to far across all faces;
then the re-tiled segments below, those missing fewest chunks first;
then the rest, near to far.
When the camera moves along a face's axis, its slices re-tile
where levels change.
`matchTilings` pairs the new slices with the ones drawn last:
slices in both stand for themselves,
and the new ones between two of them
for the old ones between the same two;
until a segment's new slices have a bake in every cell they show,
the old ones are drawn instead.
Moving fast, segments often re-tile again before they are baked;
finishing those missing fewest chunks first
shows about twice as many of the chunks baked meanwhile.
The list is split by cell between the workers,
so a cell's surface is cached in one worker
(rebaking it from a new eye reuses it).
Each render's list replaces a worker's previous one;
the worker bakes it a frame's worth (16 ms) per task, taking new lists in between,
and posts each batch at once,
without waiting for the main thread, which may be busy for hundreds of milliseconds.
Chunks posted but not yet taken when the list was made are skipped (`bakeHandler`).
Racing ahead, workers also bake far chunks that nearer ones turn out to cover:
on the forest, about a third more.
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
shaded by the way they face in the world (`shadedPalette`).

Each face draws the rays closer to its axis than to any other face's
(`inFace`): |x| + √2·|y| ≤ z, a rhombus of tangents reaching 1
towards the world axes (where 4 faces meet) and 1/√2 towards the cube's
diagonals (where 3 meet). The faces tile the screen;
the fragment shader discards the rest of their chunks,
deciding by the pixel's ray, the same for all layers of a face,
with a hair of overlap so no seam pixel falls to neither face.
Looking straight along a world axis, as the scenes' walls and streets
invite, puts a corner of 4 faces in the middle of the screen.
"Show Face Seams" draws the seams over the picture.

A chunk's eye depends on where the camera is, not where it looks,
and detail on the leak tolerance only, not the field of view or viewport.
So turning, zooming and resizing keep every slice and chunk;
only chunks coming into view are baked,
and faces coming into view are sliced.

**Placement.** `render()` returns a `Frame`:
the view (orientation, viewport W × H in CSS pixels,
`focal = max(W, H) / 2 / tan(fov/2)` in pixels)
and, face by face, for each slice near to far,
its distance d (to its centre plane), its chunks and its fog.
A chunk is a rectangle on the plane z = d in its face's coordinates,
its top-left corner at (originX − camera.x, originY − camera.y, d)
relative to the camera, chunk pixels / `render_scale` wide and high.
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
marks which 4 × 4 pixel sub-blocks of the screen
the face's layers drawn so far cover;
a cell whose box, plus 8 pixels around it, lies on covered sub-blocks
is neither drawn nor baked,
and a baked chunk is not drawn if each of its tiles
(a sixteenth of it) with something drawn lies on covered sub-blocks.
Each pixel shows one face, so coverage starts over for each face,
and sub-blocks outside the face count as covered.
A sub-block is covered once one layer's chunks draw all of the face's part of it,
but different sub-blocks may be covered by different layers:
leaves, trunks and the ground together close what none of them closes alone.
Bakers record which 4 × 4 pixel blocks of a chunk are all drawn and which partly (`solid_blocks`);
the blocks under a sub-block, plus a pixel around for smoothing, must all be drawn.
A chunk is handed to `Coverage` when first drawn or tested,
which keeps its tiles and a table of sums of its all-drawn blocks,
so testing the blocks under a rectangle takes four reads;
chunks unused for 16 renders are dropped, and handed over again if needed.
Only chunks already baked cover anything.
When the camera moves, cells coming out from behind nearer layers
show nothing until baked, a frame or two.
On the forest at 1280 × 800 a settled view bakes half the chunks it would without coverage,
and draws ~3200, of which ~1600 show a pixel.
Coverage adds ~17 ms to render() in Node,
most of it in `cover`, testing sub-blocks one by one;
the rays through each sub-block are kept per face while the camera only moves,
and turning, found only in the cells the face crosses or holds
(a face's side is a half-plane of the screen).

The `Compositor` draws a frame onto one opaque WebGL 2 canvas, one
textured rectangle per chunk, then the sky behind them. Chunk pixels are
opaque or empty, so it draws near to far with a depth test and discards
empty pixels: what nearer slices cover is rejected instead of blended
(the forest stacks ~40 screens of chunks). Depth is the layer's index,
not its distance: a face's slices are parallel planes, so every ray
meets them near to far, and faces share pixels only along seams
(the first face drawn wins there). A chunk's texture is uploaded
when it comes into view and deleted when it leaves. Magnified chunks are
sampled nearest, to keep voxel edges crisp; minified ones smoothly.

**Fog.** A slice at distance d keeps `exp(−d / F)` of its own colour;
the rest is the horizon colour, mixed in per pixel.
F is 0.7 × the render distance, so the far cut-off stays hazy.
d is measured along the face's axis, not the ray:
fog is the same on both sides of a seam,
but a little lighter towards a face's corners than true distance would make it.
The sky fades from the horizon colour at eye level (elevation 0) to blue
at 32° up, along each pixel's ray, and gets the fog of the render
distance (or of the last slice, if farther).

## 5. Caching

Slices are keyed by `face:level:slab:cutoff` and hold their chunks keyed by brick.
Insertion order of the slice `Map` is the LRU order. Slices and chunks that
leave the view lose their textures but stay cached so moving back
reuses them: up to `MAX_CACHED_SLICES` = 2048 slices, and hidden chunks
up to `MAX_HIDDEN_CHUNK_PIXELS` = 48 Mpx (oldest slices' first). Chunks
of cells in view are kept even if they draw nothing, or they would be
baked again on every render.

| Event | Effect |
|---|---|
| x/y movement | Chunks still in view reused unless their level's eye changes: a level is baked again when the camera leaves its eye's cell; chunks entering the view rasterised; placements update |
| turning | Slices and chunks reused; chunks entering the view baked |
| field of view change | Slices and chunks reused; only the compositor's focal length changes |
| leak tolerance change | Slices still needed are reused |
| small z movement | Slices reused but where levels change: there a few slices get a new cutoff or level, the ones they replace standing in until baked |
| resize | Slices and chunks reused; placements update |
| gap filling toggle | Chunks rebaked (keyed by whether they are grown) |
| world regeneration | Whole cache cleared |

Per-render stats (`UpdateStats`) report chunks reused vs shown for the
first time since they were baked, chunks still baking, voxel faces baked
since the previous render and the pixels of all cached chunks;
`SessionStats` accumulates them. `renderTime` is main-thread time, which
excludes baking in workers.

## 6. Worlds (`world.ts`, `forest.ts`, `city.ts`)

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

**Storage** (`voxel-grid.ts`). A world is a dense voxel grid per level of
detail (section 1) plus a spawn point: one material id byte per site of a
box, 0 for empty; the coarser grids add a seventh to the finest.
Sites are z-major, and a row (y, z) holds every other x,
so a slice is one contiguous range.
A worker reads its copy of the grid for every face (`Reader` in `bake/`),
without turned copies (forest: ~35 MB, city: ~50 MB).
A list of voxel objects is built only on request (tests).
Renderer distances are in world units: render distance 40–800, default 400.

**Scenes** describe shapes in metres and fill the sites inside them;
each world picks its own scale, the renderer assumes none.
The forest's `UNIT_SIZE` is 0.25 m per unit (neighbours 0.35 m apart),
so the default render distance is 100 m and the fog distance 70 m.
The forest is endless and can be generated in any box alone
([generators.md](generators.md));
the app generates one box of it, its extent, into a grid.
Its ground is a heightfield sampled per column of sites;
trunks are columns at least 2 units square,
crowns leaf clusters (balls) or a solid of revolution (spruce),
branches and fallen logs capsules, so thin parts stay one piece on the lattice
(a single column of sites only touches at corners).
The city is planned on a 0.25 m grid (`PLAN_CELL_SIZE`):
each plan cell is a box holding four sites, lattice units of 0.125 m
(render distance 50 m), so walls stay flat and frames one piece.
The toy worlds are built of blocks 2 units wide in the same way.
Columns of ground are filled a few units deep;
the renderer only draws faces that no neighbour covers.

## 7. Reference viewer (`src/viewer`)

`viewer.html` rebuilds the world from its seed
and draws every exposed voxel face, a rhombus, with three.js.
World (x, y, z) maps to three.js (x, y, −z),
so the default camera, looking down −z, looks along world +z.
The app passes its orientation, field of view and render distance in the URL,
so from the parallax camera's position both renderers should agree.

## Limitations

- Each slice is drawn at the scale of its centre, as seen from its
  level's eye, so it is only approximately in perspective; section 2
  bounds the error.
- Faces as seen from the eye: a face nearly parallel to the rays may be
  missing or slightly too wide.
- A level is baked again all at once when the camera leaves its eye's
  cell (section 3): the near levels every unit or few of movement.
- Near slices are heavily upscaled from `RENDER_SCALE`.
- Within `NEAR_DEPTH` of the camera along a face's axis nothing is
  drawn, and near voxels seen at a steep angle are the least accurate.
- Inside a voxel (the camera has no collisions), the cut faces of the
  nearest slices fill the screen.
- At a seam, the two faces slice the world differently: geometry
  crossing it can be off by up to the leak bound on each side.
  Looking along a world axis, four faces meet in the middle of the screen.
- More faces are in view at once than with a cube's six (four or more,
  against one to three), each with its own near slices: baking the view
  looking along an axis takes ~2.5× as long (section 3).
- Gap filling may leave a few pixels:
  20 of 181 on a one-layer floor seen from above, in one place.
- Slices are slabs of whole cells, drawn in order, but a voxel on a
  cell's boundary is half in the next layer of cells: where it overlaps a
  voxel of the next slice on screen, the nearer slice's one may show
  though the other is in front, by up to a voxel.
- Slabs of whole cells are ragged on both sides,
  (R + 1) / R times as deep as their R layers of atoms (section 1),
  1.02 for the 64 of the far levels,
  so they take about that many more slices than slabs of whole layers.
- Far slices draw coarse voxels (section 1):
  what fills less than a quarter of a coarse voxel's cell is lost
  (a lone voxel, a thin pole), and solids grow by up to half a coarse
  voxel.
- Rhombic dodecahedra have jagged outlines: a flat wall or floor is a
  surface of pyramids, and a silhouette along an axis is a zigzag.
- Four voxels per old cube, each showing up to six faces towards the
  camera instead of up to three: a first bake of the toy world in the tests takes
  ~1.4 s on one thread (0.26 s with cubes). The forest has ~3.1–3.5 M voxels
  (35 MB) and the city ~3 M (50 MB), held by the main thread and each
  worker; the reference viewer's mesh has ~10 M faces (~850 MB of buffers).
