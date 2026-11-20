# Rendering Architecture

One frontend (`native/`) draws the worlds (section 5),
in a window and, built for browsers, in a page (`web/index.html`),
by engines it switches between (section 4):
rays through the voxels by default, or cells of the lattice baked into chunks.
The Rust is a workspace of crates (`Cargo.toml`):
`fcc/` under all of them
(the lattice, grids and tiles of them, the light from voxels' faces),
`bake/` (chunks baked) and `worldgen/` (the generator framework) on it,
`worlds/` on `worldgen/`
(the registry, and worlds in crates of their own inside it);
`native/` links them.
For browsers it is built as WebAssembly and bound to the page
(`web/bindgen/`):
WebGPU for its GPU, a canvas for its window,
and the voxels engine alone,
as the chunks' compositor draws with what WebGPU lacks.
A browser gives it no threads,
so its world makes a piece or more after each frame
(`World::work` in `native/src/world.rs`).

Voxels are rhombic dodecahedra on the face-centred cubic (FCC) lattice
(section 5).
The chunks engines look along the lattice's 12 nearest-neighbour directions,
one per face of a rhombic dodecahedron around the camera (section 4);
sections 1–3 describe one of them.
Seen along such a direction, the lattice is a stack of flat layers
LAYER = 1/√2 apart, and a voxel reaches from the layer before its own to
the one after, so neighbouring layers interlock.

## 1. Cells and chunks (`bake/src/cells.rs`, `bake/src/lod.rs`)

### Cells of coarser lattices

Every second site in each direction of the FCC lattice is again an FCC
lattice, twice as coarse, and so on: N·FCC for N = 1, 2, 4, …
(`bake/src/cells.rs`).
In a face's lattice coordinates (section 4) its sites are the (a, b, c)
with all three multiples of N and a/N, b/N, c/N of one parity,
so it has its own layers along every face's axis, N layers apart.
Each site belongs to the cell of the nearest site of N·FCC,
a rhombic dodecahedron N times a voxel's.
Sites exactly between cells, on a cell's face or corner,
go to the cell with the greatest c, then b, then a:
the minimal tie-break, and the same along every face.
A cell holds N³ sites, in layers c0 − N to c0 + N − 1 around its own
site's layer c0 (at N = 2, c0 − 2 to c0).
Cells are as round as rhombic dodecahedra:
no direction is preferred.

Cells of different N do not nest.
A cell of 2N·FCC cuts in half the 12 cells of N·FCC
centred on its faces, and in six the 6 at its four-fold corners;
a tree of nested cells would need cells of a preferred direction
(the README's design principles put isotropy first).
So each level is used on its own.

### Levels of detail and tiles

A world is voxels at each level of detail:
level ℓ holds the sites of 2^ℓ·FCC, in its own units,
each a voxel standing for its cell.
Every world is generated at each level (`docs/generators.md`),
each tile only as far up and down as it may show
(`shown`), its bricks buried in rock kept as one,
and what is buried under that, or under a block of its columns (`Floors`),
not made but one brick of rock for all
(`generate_shown`, in `worldgen/src/generator/generate.rs`).
A level's voxels are in tiles (`native/src/world.rs`):
cubes, alike along every axis, so no direction is up to how they are asked for,
and small (`TILE` in `worldgen/src/tiles.rs`),
as rays ask for little of a larger one
(`make_tiles` there: `generate_shown`;
those outside the world's heights are empty).
In a world with heights, a tile is generated with those above and below it,
as one box, and parted (`made_with`):
they share their fields' lines and the things reaching several.
An endless world's cost as much together, so each is made alone.
Lit, a tile's sites are made into the light, which reads the same tiles
(`Shared::make_tiles`): once, for what draws them and what lights them.
Tiles are made in the order the engine asks for them (`Wants`):
a cell is baked once the world has every tile it reads (`chunk_reads`),
and until then it asks for them, nearest first;
the voxels' rays ask for those they meet (section 4).
No other tile is made but with one asked for,
so turning asks for what comes into view
(a body walking asks for the ground around it, seen or not).
Tiles are kept as far around the camera as the engine may ask for them,
whichever way it looks (`Wants`), and dropped farther:
for the chunks, as far as each grid's chunks read
(`Lod::grid_reach`: their level's claims, to the render distance);
for the voxels engine, `voxels::READ`, as far as its rays read.
A tile never changes once made,
so no chunk or cell surface goes stale as tiles come.

### Chunks are whole cells

The chunks engines draw cells (`bake/src/lod.rs`, `native/src/engine/chunks/renderer.rs`).
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
A relief is off by an angle that detail bounds (section 2).
Or, drawn by the `splats` engine (`Renderer::relief` off),
its whole voxels a ray can enter are each one hexagon too:
a ray's first voxel is entered so and is in one cell,
so its hexagon, holding its outline, meets the ray;
no runs and no cut faces.
Or, drawn flat (the `flat` engine, an experiment;
`Flat` in `native/src/engine/chunks/compositor.rs`),
its relief is a picture of its columns' colours without their depths,
on a plane fixed in the world:
the camera's perspective moves and scales it.
A column is a brick of it, a row high, a row's bricks side by side,
the next row's half a brick on, so that they fill the plane;
the picture is drawn as its rows' runs of columns that show,
a quad each (`flatten`),
so that every pixel drawn shows and none works out its own depth,
which keeps the GPU testing depth before the pixel is drawn
(on Intel's, a pixel left out or its depth written by the shader does not);
the depth is the plane's, by w, ahead along the view (`flat_depth`),
and a quad holds to its face's rays by four planes through the eye.
Where two cells' pictures meet, each on its own plane,
a gap may open as wide as the rays' slant times
how far the edge columns' pillars reach from the planes,
widest along a surface parallel to the axis,
where a picture is a row or two of columns
and their pillars run through the cell.
So the plane is the one they reach least from,
the least steep of those
(any slope is as good for one column's pillar,
and a steep plane near the face's edge shows edge on),
and each frame each edge of a picture moves out by that reach
times the slant of the rays to it;
depth decides where neighbours overlap.
No gaps (`ground_along_a_face_shows_no_gaps`),
but what a deep pillar at an edge holds is smeared.
Its see-through voxels are pictures of its medium, slab by slab
(section 4, "See-through");
the relief takes them for empty.
Its tiles hold the sites' media too, with how much each stops
(`VoxelGrid::to_grid`):
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
those of whole voxels and of medium apart, each by its own Lod,
each with what it knows of it (`Seen`): its chunk, that it holds nothing,
or what it waits for.
A cell holds nothing if none of its level's voxels meets it (`Level::meets`),
or if they are buried, as far underground most cells are:
every brick of its grid holding them and their neighbours is whole
(`Level::buried`, by each tile's whole bricks),
so no ray from anywhere enters one;
the same for every face;
its medium holds nothing if no see-through voxel meets what it reads.
Each tile keeps which cells of 8·FCC and 32·FCC its voxels meet,
and its see-through ones, if it has any,
the voxels engine's cells (below);
the default cells are of those sizes, and smaller ones are read site by site.
It picks anew once the eye or view leaves its cells:
what it knew of cells still picked moves to the new pick,
of the rest it keeps a while.
A frame is every chunk the picks compose for an eye anywhere in them
(`Lod::composed`), found as a face is picked and added to as chunks come;
which of them the eye draws from where it is, the GPU finds (section 4).
So the eye moving within a pick finds its chunks baked,
and the renderer can run on a thread of its own
(`native/src/engine/chunks.rs`): a frame draws the last it rendered.
Its bakery's threads, as the world's, run behind the thread that draws.
What the eye composed when its face was picked is waited for;
the rest of the pick is baked after.
Nor is a cell baked that what is in front of it hides
(`Hiding` in `renderer.rs`, `occlusion.rs`):
from where the eye was when the face was picked,
a ray through each corner of a buffer's pixels
over the face goes through the voxels drawn along it,
the grid of the level claiming each layer,
half a voxel at a time, across empty bricks half a brick,
and stops at the first whole voxel.
So ground, walls and trunks hide what is behind them,
and leaves (grain, as often as they cover) what is deep enough in a wood;
see-through voxels hide nothing, as the GPU sees no mist in its depths.
A cell whose box (drawn flat, grown by how far its picture reaches)
lies behind where the rays round every pixel it covers stop is hidden.
A ray that met a tile not made yet goes on from there once it comes.
Between rays, or from where the eye has moved since, a hidden cell may show,
so its box is a probe in the frame (`Frame`'s `probes`):
if a pixel's ray reaches it past all the compositor drew (section 4),
the renderer bakes the cell after all
and hides it no more while picked: it may show a frame or few late.
It depends on nothing the camera does but where the eye was,
so a host could do it apart from what draws.
Every other chunk in view is baked,
and the compositor culls what is hidden (section 4).

## 2. Detail

Detail is viewing distance over a cell's thickness (`Lod::new`):
a cell 2N layers deep is drawn from Q · N layers ahead on,
Q at least twice the detail,
so far voxels are small on screen and cells grow with distance.
It depends on nothing else, not the field of view or viewport.
A cell's voxels are `shift` levels finer than the cell.
The voxels engine has no detail:
each level is read as far as its window holds (section 4).

## 3. Baking (`bake/`)

**Baking along the axis.** A chunk is baked as seen along its face's axis
(`Baker::bake_cell`):
each voxel at its site, its depth past the plane through the cell's site kept.
So it does not depend on the camera, and is baked once.
Voxels are read layer by layer from the level's tiles in the face's
lattice coordinates (section 4), those of the cell and around it
(`chunk_reads`), with a bitmask of their open faces and of those cut,
their neighbour in another cell; buried voxels are dropped (`surface`).
Which cell a site is in comes from a table over one period
of N·FCC, 2N along each of a, b and c (`bake/src/cells.rs`).
A baker per thread shares one copy of the tiles (`share_tiles`).

**Splats.** Seen along the axis, a face's columns (a, b) tile the plane.
A voxel is painted as one hexagon (`Splat`),
the voxel's outline seen along the face's axis
(√2 wide and 2 high, its corners the rhombic dodecahedron's),
across the axis at its site, its depth in half layers.
Seen along a ray slanting σ off the axis, the outline is 1 + |σx| times
as wide (the voxel's tips a layer before and after move sideways)
and, within the face's rays, no higher;
the compositor widens it so,
and the splats of a surface leave no gaps from any ray of the face.
A splat's colour is that of the voxel's face towards the camera, −F
(the face a chunk seen along F shows; `Reader` in `fcc/` reads it),
shaded as the mean of its open faces' shades
(of its cut faces' where it has none),
each by the axes its normal leans towards
(`face_shade`, `voxel_colour`):
a voxel is shaded by the way its surface faces,
and coloured by what is seen of it along the face.
Lit by computed light (`Lighting` in the panel, or `--light`),
the baker is handed instead
the light each face of a voxel it shows sends out
(`fcc/src/light.rs`, found by `worldgen/src/light.rs`),
so only what shows is lit:
a splat shows the mean of its open faces',
each as much as it faces the camera.

## 4. Faces, compositing and the voxels engine (`native/src/view.rs`, `native/src/engine/`)

**Faces.** The camera has a yaw and a pitch, no roll (`view_basis`).
To look all around, the chunks engines draw 12 stacks of layers,
one per face of a rhombic dodecahedron around the camera,
i.e. one per nearest-neighbour direction F of the lattice (`FACES`).
A face is a rotation, forward = F/√2, up = the world axis across F,
right = up × forward, and its stack is sections 1–3 in those coordinates.
The rotation maps sites to sites, and every face sees the same lattice:
site p is at lattice coordinates (a, b, c) = (p · √2·right, p · up, p · F),
all even or all odd, placed at (a·LAYER, b, c·LAYER);
layer c is a rectangular grid of sites, √2 by 2 apart,
and the next layer is the same grid shifted by half a cell.
The world is read in these coordinates (`Reader` in `fcc/`), with faces
shaded by the way they face in the world (`shades` in `bake/`).

Each face draws the rays closer to its axis than to any other face's
(`in_face`): |x| + √2·|y| ≤ z, a rhombus of tangents reaching 1
towards the world axes (where 4 faces meet) and 1/√2 towards the cube's
diagonals (where 3 meet). The faces tile the screen:
a pixel is the face's whose axis its ray is nearest,
by the ray's two largest world components, one face a pixel (`owns`).
Looking straight along a world axis, as the scenes' walls and streets
invite, puts a corner of 4 faces in the middle of the screen.
The panel draws the seams over the picture,
and can colour the splats by layer or by chunk (`Colours`).

Chunks depend on neither where the camera is nor where it looks,
and detail on nothing else, not the field of view or viewport.
So turning, zooming and resizing keep every chunk;
only chunks coming into view are baked.

**The compositor** (`native/src/engine/chunks/compositor.rs`) draws only what may show.
Its chunks are cells' reliefs (section 1),
and the bakery's threads put a chunk's columns in tiles of 4 × 4,
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
A probe, a hidden cell's box (section 1), is tested once all is drawn,
against the pyramid made of it anew:
it shows if a pixel within its quad on screen is as far as its box,
looked for down the pyramid only where the farthest is,
so only along the quad's edges, and a bounded number of texels (`sees`).
Those that show are read back a few frames later
and handed to the renderer (`Feedback`).
A phase draws only the tiles it keeps (an empty draw is not free),
listed in their order (`scan_draws`, `pack_draws`): roughly near to far,
so nearer tiles mostly hide what is behind them early,
and where depths tie, the same tile shows every frame, culled or not.
It draws into a picture of its own, which the window (or `--shot`) shows.
Where whole voxels are splats (section 1),
the bakery's threads put them in clusters of a few nearby on its plane instead,
each with the box of their outlines,
drawn and culled as the tiles are,
each splat at the depth of its site,
widened by the slant of the ray to it;
a chunk's slot in the table comes to the vertex shader
as its draws' first vertex,
so its splats are uploaded as the threads made them.
A chunk's record, where it is whatever the camera
and which clusters are its, is uploaded with it;
each frame the GPU places every chunk from the camera (`place`),
and keeps those whose level claims their cell (`Lod::layers`, mirrored there).
The culling goes over every cluster in the buffer,
those left where chunks were too, which their slot's record disowns,
so there is no list of the frame's clusters to keep.
A frame comes with its last changes, the chunks that came and went
(`Change` in `renderer.rs`):
the compositor applies those since the frame it prepared last,
at most `CHANGES` chunks a frame, those that came first,
or takes the frame whole at once if it missed some,
so that a change costs the main thread what changed, not the frame,
and a face picked anew, thousands of chunks, is spread over a few frames.
What a change writes, anywhere in a buffer,
goes up as one copy of its words, which the GPU scatters (`scatter`):
a copy command costs the GPU microseconds,
and a face picked anew writes thousands of places.
Otherwise a frame writes its globals alone.

**See-through.** A site's medium is drawn see-through
(`KindDef::medium`, as mist) where its pieces don't show;
those come whole.
Like a relief drawn flat, a cell's medium is pictures:
one per slab of its columns, as deep as its cells' spacing
(`bake::Medium`, `cell_medium`),
on the slab's middle plane, which every cell's slab there shares,
each column's colour and optical depth:
−ln of what each voxel lets through along its mean chord
(worldgen's `CHORD`), added up.
Its cells are its own (`Lod::for_medium`):
the same grid as far away as whole voxels, but two levels coarser,
so slabs four times as deep.
A ray costs a fragment per slab it crosses, 2Q near the eye and Q a level after
(of the medium's Q);
deeper slabs lose little along the axis, as optical depths add up in any order,
though within a slab colours mix without their order.
A ray takes the slab where it crosses that plane,
so where it slants it takes it a little aside,
which blurs where the medium ends by about a slab's depth times the slant:
where a face's axis is tilted, mist lying on the ground
shows that much above and below it,
and at a seam with a level face's rays, which take it where it is,
the difference shows as a V.
The compositor lays the pictures one over another, far to near,
each as much as it stops, fogged by its distance as whole voxels are,
into a picture of their own, in front of what the reliefs drew,
and lays that over the picture.
Every ray of a face meets the planes across its axis in the same order,
so the culling lists the pictures by their place
among the half slabs the face's rays cross (`veil_place`):
each level's medium lies between two planes, finer levels nearer,
and its halves, 2^(L − 1) layers of grid 0 deep, follow one another,
so they are numbered from the near plane on, level by level.
It counts the pictures at each place
and lists each after those at farther places
(a counting sort, `scan_veils` and `order_veils`).
A face's pictures at one place are on one plane, which they tile once,
so their order there makes no difference.
So media of different colours, smoke in fog say,
mix in their order along a ray (`a_nearer_medium_shows_over_one_behind`),
and far mist fades to the horizon.
Each slab of a column is one cell's, the one holding its middle site,
so a slab's pictures tile its plane once;
and a pixel takes the columns around it as a field,
each a tent over twice its brick, so that they sum to one,
from its picture, which holds its own columns' neighbours too,
whichever cells they are:
bricks alone showed their edges.
So that none counts twice,
each pixel is one face's only,
and a level draws the medium between two planes across its axis:
past the next finer level's reach, and up to its own (`place`).
Its slabs start a layer on, so that those planes fall between their halves,
and a column keeps an optical depth per half.
A slab of a column may be a cell's that holds none of it,
so a level picks a layer of cells more each way than it claims,
whose medium alone it draws (`Lod::layers`, `medium_claims`).
A cell's medium is a chunk of its own, apart from its whole voxels,
and no level finer than `MEDIUM_LEVEL` (`lod.rs`) has one:
its cells, 32 voxels across, draw it from the near plane on,
not the finest cells, a chunk per voxel.
So a picture of medium takes the depths of the level
whose whole voxels are drawn where it is (`level_ahead`),
or finer levels' voxels behind it would hide it.
A test holds a slab of medium to exp(−extinction · path),
thin and thick, along the axis and slanted
(`a_medium_of_splats_lets_through_as_much_as_its_extinction_says`).

**Fog.** What is drawn at distance d keeps `exp(−d / F)` of its own colour;
the rest is the horizon colour, mixed in per pixel;
the medium too, picture by picture (see-through, above).
F is a fixed share of the render distance (`Look`'s `fog`),
so the far cut-off stays hazy.
d is measured along the face's axis, not the ray:
fog is the same on both sides of a seam,
but a little lighter towards a face's corners than true distance would make it.
The sky fades from the horizon colour at eye level to blue
higher up, along each pixel's ray, and gets the fog of the render
distance.

### The engines, and rays through the voxels

The window switches at run time between engines (`Kind`),
each drawing the same tiles its own way, with its own shaders:
chunks above, their whole voxels reliefs or splats (`reliefs`, `splats`),
or rays through the tiles' voxels (`native/src/engine/voxels.rs`),
with no chunks, faces or baking, described here.

They share one interface (`native/src/engine.rs`), in two halves.
On the main thread an `Engine` makes a `Frame` of what a `Look` sees:
a `Picture`, and what it wants of the world (`Wants`);
what it makes of the tiles, on threads of its own, is its own business.
On the GPU its `Painter` keeps what it needs from frame to frame
and draws its pictures into one of its own,
which the window shows under egui's panel, or a shot saves.
A device holds one painter at a time (`Easel`, `native/src/gpu.rs`).
So the window, `--headless` and the tests
name no engine:
a view is complete when its engine lacks no tile and makes nothing more.

In the voxels engine a ray goes from rhombic dodecahedron to dodecahedron across their faces;
each level's voxels are read as far from the eye as its window holds,
then the next coarser level's, as far as the render distance.
The GPU holds of each level a ball of bricks around the eye's
in a window a little larger;
its table of the bricks' slots in a pool is a torus,
a brick's entry at its coordinates modulo the window's size,
so the ball moves a brick at a time
and only the bricks that came into it or left are touched.
A frame steps one window, each in turn:
its ball a brick along an axis toward the eye's,
and the tiles in it, gone or new, those made with a few at most (`STEP`),
so what a frame costs is bounded however fast the eye or many the tiles;
a window behind is read only as far as it holds around the eye,
and one holding nothing around it starts anew there.
The levels' balls share one radius, in each level's own bricks,
as large as the pool holds:
short of slots, all shrink, every level read less far,
and with room to spare they grow back.
The pool is bounded; what does not fit shows coarser, sooner.
Tiles never change once made, and are packed for the GPU where they are made,
so a brick is uploaded once,
and bricks of a tile holding the same sites (its buried rock) share a slot.
A brick is unknown, not empty, till its tile is made,
and, with voxels, till a ray asks for it (and, lit, it is lit: below):
rays take the next coarser level there,
so what is missing shows coarser, never as a hole,
and each such ray sets a bit for the brick, which the CPU reads back,
a frame or two later (`Asked` in `voxels.rs`).
A brick asked for whose tile is made is placed (`Window::ripe`);
the tile of one that is not is what the engine asks the world for,
those before any light, as no ray asks for the light of a tile not made,
each kind nearest first in its level's own sites, as rays read them.
So the pool holds what shows, and the world makes what shows,
not every brick of a window's ball: walls hide most of what is near.
A window is settled once a frame of its bricks as they are asks for nothing.
A voxel shows how it looks along the ray,
the mean of its looks through the faces turned most against it
(fcc's `facing`, the one rule every host and check reads by),
times the light on it (below):
a coarse voxel of a house may show its wall from the side and its roof from above.
So a brick whose voxels look unlike through some faces takes twelve more slots of the pool,
its sites' looks through one face each, each slot naming the next.
Empty space is crossed on the lattice too, not by boxes:
beside its bricks, a window counts for each cell of 8·FCC and of 32·FCC
(rhombic dodecahedra about every other corner of its bricks, or of blocks of 4³ of them)
the bricks around it whose voxels meet it, or that are unknown,
as is every brick out of its ball, which rays do not read:
counts start full, and only bricks of tiles change them,
so a ball moves through what is unknown at no cost to them.
A ray in a cell that none meets goes across it at once, face to face,
as it goes across a voxel: whatever it met in the cell would meet the cell.
A window also keeps the box, in the level's axes as its bricks are,
that the voxels of its ball and its unknown bricks are in:
out of it, a ray takes the next level, from where it would have,
so open sky costs a ray a few steps a level.
A ray that runs out of steps shows magenta: a march must not stick,
and if one does, it must not hide.
One ray is traced for every 4 × 4 pixels;
each pixel takes the nearest of the voxels its four nearest samples met
that its own ray enters, so edges stay sharp.
See-through voxels add up in the order the ray meets them,
each stopping `1 − (1 − alpha)^(path / chord)` of what is left,
its path the ray's through the voxel, until little is let through.
Each channel apart (`lets_through`), by what the voxel stops of it:
a site's medium holds that, and what it shows, channel by channel
("Kinds and things" in generators.md),
handed over beside the looks where the channels differ (fcc's `stops`),
a slot more for a brick that has any (`STOPS`).
Grey mist stops all alike; the sea stops red first,
so its floor turns green, then blue, with the water in front of it.
The chunks engines take the look's alpha for every channel:
that of the channel stopped least.
A pixel takes the medium in front of what it sees
from those of its samples that saw the same.
Tests hold the window's table to a new one's after any moves and tiles
(`a_window_moved_and_its_tiles_changed_holds_what_a_new_one_would`),
its box and its cells counted hollow, at its edge too, to ones no voxel it holds is out of or meets
(`no_voxel_the_window_holds_is_out_of_its_box_or_meets_a_cell_counted_hollow`),
its rays to their steps (`no_ray_of_the_voxels_runs_out_of_steps`),
a block's coarse voxels to the faces rays enter them by (`voxels_show_the_faces_rays_enter_by`),
the picture to the same one with the world and the eye moved by a vector of the lattice
that is not one of the bricks' (`voxels_look_the_same_wherever_the_bricks_fall`),
a slab of medium to exp(−extinction · path), as the splats' test does,
and each channel to its own
(`a_medium_of_voxels_stops_more_of_the_channels_its_colour_is_darker_in`).

The panel picks the lighting apart from the engine (`Lighting`):
voxels shaded by the way their faces face, or computed,
by worldgen's `Light`, one model and one cache (`Shared`) for every engine:
the world's tiles where the light reads, made if not yet,
each site as it is, what it stops a share,
where the grids the world draws show it whole or not at all.
It reads a few sites past what it lights, at each coarser level,
kept as far around the camera as the world keeps its tiles;
each engine only shows it its own way.
The splats' bakers light the voxels they show, face by face.
For the voxels, the world's threads light the bricks rays ask for
(above; `World::update`),
each thread, of the most wanted, the one beside its last:
each voxel of the brick a ray can meet gets the light on it,
by the way its surface faces (`Light::on_seen`),
and what it glows,
and shows its colour times the one, and the other,
as worldgen's lit pictures do (`Light::relight`);
shaded, the light is the mean of its open faces' shades.
The light goes to the GPU beside the bricks, two words a site (RGB9E5), in a pool of its own.
Till a brick with voxels is lit, it is not on the GPU, as if no ray had asked for it:
what is not lit yet shows coarser, lit, never shaded by another light.
A tile never changes but for its bricks' light, each found once.

## 5. Worlds (`fcc/`, `worldgen/`, `worlds/`)

**The lattice** (`fcc/`). Voxels sit on the face-centred cubic lattice:
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

**Storage** (`Grid` in `fcc/`). A world is tiles of voxels per level of
detail (section 1) plus a spawn point: a word per site of a tile's box,
what its voxel looks like (`Look` in `fcc/`):
its colour, sRGB, and how much of the rays through it it stops; 0 for empty.
Grids hold colours, not materials:
a coarse voxel holds the mean of what is in it.
A voxel may look different through each of its 12 faces (`VOXEL_FACES`),
what is seen of it looking at that face:
a coarse voxel stands for its cell, which may look different from each side.
Mostly it looks alike through all;
the sites that do not are listed apart (`views`).
What stops a walking body is a bit a site apart (`solid`).
Each reader shows the faces it sees:
the baker the face towards its chunks' camera (section 3),
rays (`worldgen/src/picture.rs`) those turned most against them.
The box is cut into bricks 8 units wide, 256 sites,
and only bricks with voxels are stored.
A baker reads the tiles for every face (`Reader` in `fcc/`),
without turned copies.
Distances are in world units.

**Scenes** describe shapes in metres and fill the sites inside them;
each world picks its own scale, the renderer assumes none.
The worlds have `UNIT_SIZE` metres per unit
(`worldgen/src/units.rs`).
They are endless and can be generated in any box alone
([generators.md](generators.md)),
by `worlds/` on the engine of `worldgen/`.
The forest's ground is a heightfield sampled per column of sites;
trunks are columns at least 2 units square,
crowns leaf clusters (balls) or a solid of revolution (spruce),
branches tubes and fallen logs capsules, so thin parts stay one piece on the lattice
(a single column of sites only touches at corners).
The city (`worlds/src/city.rs`) is planned on a 0.25 m grid:
each plan cell is a box holding four sites, so walls stay flat and frames one piece.

## Limitations

Of the chunks engines' splats:

- A voxel's hexagon holds its outline, not more:
  near a slant's extremes it may hide a neighbour in its layer that is in
  front (section 3).
- A voxel is painted as one hexagon of one colour holding its outline,
  that of its face towards the camera:
  its other faces' shapes, shades and looks are lost,
  and a silhouette is a row of hexagons.
- At a seam, the two faces cut the world differently.
  Looking along a world axis, four faces meet in the middle of the screen.
- More faces are in view at once than with a cube's six (four or more,
  against one to three), so more is baked.
