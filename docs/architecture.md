# Rendering Architecture

One frontend (`native/`) draws the worlds (section 4),
in a window and, built for browsers, in a page (`web/index.html`),
by engines it switches between:
rays through the voxels by default (section 2),
or cells of the octree baked into flat pictures (section 3).
The Rust is a workspace of crates (`Cargo.toml`):
`lattice/` under all of them
(the lattice, grids and tiles of them, rays across its voxels,
the light from voxels' faces),
`bake/` (chunks baked for the flat engine) and `worldgen/` (the generator framework, land) on it,
in which no direction is up,
`ground/` on `worldgen/` (worlds on a ground: plans by column, heights, columns,
land shared by column; up is +y),
`light/` (light worked out across a world, [light.md](light.md)),
`mechanics/` on `lattice/` (matter that moves and breaks, [mechanics.md](mechanics.md))
and `worlds/` on `worldgen/` and `ground/`
(the registry, and worlds in crates of their own inside it);
`native/` links them.
For browsers it is built as WebAssembly and bound to the page
(`web/bindgen/`):
WebGPU for its GPU, a canvas for its window,
and the voxels engine alone,
as the flat engine's compositor draws with what WebGPU lacks.
Its threads are there too, Web Workers on its memory
(`native/src/threads.rs`, `web/worker.js`; std built with atomics, by a nightly).
On either, the thread that draws waits for none of them,
as a browser's main thread may not:
it posts them work and reads what they did by channels,
and takes no lock they take.
A page shares memory only isolated (`web/vite.config.js`),
on GitHub Pages by a service worker (`web/public/isolated.js`).

Voxels are cubes of the cubic lattice Z³ (section 4),
and the levels of detail an octree of them (section 1).

## 1. Levels of detail and tiles

A world is voxels at each level of detail:
level ℓ holds the sites of 2^ℓ·Z³, in its own units,
site p the cube [2^ℓ p, 2^ℓ (p + 1)),
each a voxel standing for the eight of level ℓ − 1 in it: a classic octree.
A coarser site is read where it stands for its finer ones
(`sites::middle` in worldgen, [generators.md](generators.md), "Levels").
Every world is generated at each level,
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
the rays ask for those they meet (section 2),
a chunk for those it reads, till it has them all (section 3).
No other tile is made but with one asked for,
so turning asks for what comes into view
(a body walking asks for the ground around it, seen or not).
Tiles are kept as far around the camera as the engine may ask for them,
whichever way it looks (`Wants`), and dropped farther:
`voxels::most`, as far as its rays read;
`Lod::grid_reach`, as far as each grid's chunks read.
A tile never changes once made; it may be replaced.
A world's edits (`worldgen/src/edits.rs`) are what its level-0 sites hold
since it was generated: nothing where something was cut, matter where it fell,
and across which faces that holds on ([mechanics.md](mechanics.md)).
A coarser site holds what the level-0 site it is read at holds,
and every level's sites are made with the edits in (`make_sites`),
for what draws them and what lights them alike.
An edit has the tiles it meets made again, at every level,
shown as they were till they are, then replaced
(`World::change` in `native/src/world.rs`);
those a tile or less from it are lit again.
The chunks engine, which bakes a chunk once, does not see edits.
Those that read a level's tiles, on their own threads,
each take them as they were (`Level`, `lattice`'s `Tiles`):
tables in blocks of tiles, shared between one taking and the next (`Blocks`),
so that a frame in which tiles come or go
costs as those do, not as all the world holds.

## 2. The voxels engine (`native/src/engine/`)

The default engine draws the tiles by rays through their voxels
(`native/src/engine/voxels.rs`);
engines share one interface (`native/src/engine.rs`), in two halves.
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

A ray goes from cube to cube across their faces;
each level's voxels are read as far from the eye as its window holds,
then the next coarser level's, as far as the render distance.
The GPU holds of each level a ball of bricks around the eye's
in a window a little larger;
its table of the bricks' slots in a pool is a torus,
a brick's entry at its coordinates modulo the window's size,
so the ball moves a brick at a time
and only the bricks that came into it or left are touched.
A step moves one window, each in turn:
its ball a brick along an axis toward the eye's,
and the tiles in it, gone or new, those made with a few at most (`STEP`),
so what a step costs is bounded however fast the eye or many the tiles;
a window behind is read only as far as it holds around the eye,
and one holding nothing around it starts anew there.
The windows are kept on a thread of their own (`Keeper`):
a frame hands it the tiles, the eye and what rays asked for (a `Task`),
and takes what an earlier step changed (a `Patch`: bricks, tables, counts),
all of it copied to the GPU in that frame or none,
so a frame waits for no step, and the GPU holds the windows a frame or two late.
A picture drawn till settled (headless, and the tests) waits for each patch.
The levels' balls share one radius, in each level's own bricks,
as large as the pool holds and rays need (below):
short of slots, all shrink, every level read less far,
and with room to spare they grow back.
The pool is bounded; what does not fit shows coarser, sooner.
Tiles never change once made (an edit replaces them), and are packed for the GPU where they are made,
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
(lattice's `facing`, the one rule every host and check reads by),
times the light on it (below):
a coarse voxel of a house may show its wall from the side and its roof from above.
A site's look is read as every host reads it (`whole_or_none`, lattice's):
pieces held in part whole or not at all, by a coin of their site.
So rays, pixels and shades meet only voxels empty, whole or a medium.
So a brick whose voxels look unlike through some faces takes more slots of the pool,
each naming the next:
one for which sight each face of a site sees, then one a sight,
each look once, as worldgen holds them (`sights`);
a face that sees one more than they hold sees its site's own look.
Empty space is crossed by the octree's cells above the bricks:
beside its bricks, a window counts for each cube of 4³ bricks and of 16³
the bricks of its ball in it whose voxels are not all empty, or that are unknown.
A brick out of the ball counts in none, as rays do not read it:
a cell across the ball's edge is crossed as far as the level is read,
not stepped through voxel by voxel to its end.
A ray in a cell none is in goes across it at once, as across a voxel.
A window also keeps the box, in the level's axes as its bricks are,
that the voxels of its ball and its unknown bricks are in:
out of it, a ray takes the next level, from where it would have,
so open sky costs a ray a few steps a level.
A ray that runs out of steps shows magenta: a march must not stick,
and if one does, it must not hide.
One ray is traced for every few pixels each way
(`--apart`; a window's pixels are its points),
and a level is read no farther than its voxels show as wide as rays are apart,
so some ray meets each voxel that shows (`most`):
the farther apart, the fewer the rays and the tiles, and the coarser the voxels.
The render distance comes first:
no level takes over from the coarsest, so the balls are as large as it needs.
Each pixel takes the nearest of the voxels its four nearest samples met
that its own ray enters, so edges stay sharp.
The pointer's ray is marched in a pass of its own:
a march in the resolving pass kept it from running as many pixels at once.
See-through voxels add up in the order the ray meets them,
each stopping `1 − (1 − alpha)^path` of what is left,
its path the ray's through the voxel, in the level's units,
alpha what it stops of a ray crossing it along an axis,
until little is let through.
Each channel apart (`lets_through`), by what the voxel stops of it:
a site's medium holds that, and what it shows, channel by channel
("Kinds and things" in generators.md),
handed over beside the looks where the channels differ (lattice's `stops`),
a slot more for a brick that has any (`STOPS`).
Grey mist stops all alike; the sea stops red first,
so its floor turns green, then blue, with the water in front of it.
A pixel takes the medium in front of what it sees
from those of its samples that saw the same.
Tests hold the window's table to a new one's after any moves and tiles
(`a_window_moved_and_its_tiles_changed_holds_what_a_new_one_would`),
its box and its cells counted hollow, at its edge too, to ones no voxel it holds is out of or in
(`no_voxel_the_window_holds_is_out_of_its_box_or_in_a_cell_counted_hollow`),
its rays to their steps (`no_ray_of_the_voxels_runs_out_of_steps`),
a block's coarse voxels to the faces rays enter them by (`voxels_show_the_faces_rays_enter_by`),
the picture to the same one with the world and the eye moved by a vector of the lattice
that is not one of the bricks' (`voxels_look_the_same_wherever_the_bricks_fall`),
a slab of medium to exp(−extinction · path),
and each channel to its own
(`a_medium_stops_more_of_the_channels_its_colour_is_darker_in`, either engine).

The panel picks the lighting apart from the engine (`Lighting`):
voxels shaded by the way their faces face, or computed,
by `Light` (`light/`), one model and one cache (`Shared`):
the world's tiles where the light reads, made if not yet,
each site as it is, what it stops a share,
where the grids the world draws show it whole or not at all.
It reads a few sites past what it lights, at each coarser level,
kept as far around the camera as the world keeps its tiles.
The world's threads light the bricks rays ask for
(above; `World::update`),
each thread, of the most wanted, the one beside its last:
each voxel of the brick a ray can meet gets the light on it,
by the way its surface faces (`Light::on_seen`),
and what it glows,
and shows its colour times the one, and the other,
as look's lit pictures do (`Light::relight`);
shaded, the light is the mean of its open faces' shades.
The light goes to the GPU beside the bricks, two words a site (RGB9E5), in a pool of its own.
Till a brick with voxels is lit, it is not on the GPU, as if no ray had asked for it:
what is not lit yet shows coarser, lit, never shaded by another light.
A tile never changes but for its bricks' light, each found once.
Settled (`--settle`), one thread lights them instead (`native/src/settler.rs`),
by light kept settled as it changes (light's `Settling`, [light.md](light.md)):
a brick once each of its voxels' light was found,
and again as that light changes,
its tile replaced by one with the new light, which the engine places anew.
Another light (the sliders) settles where it is: no tile is lit anew.

Lit, the voxels show light as an eye adapting to it would (`native/src/eye.rs`):
the samples meter the luminance their rays meet,
read back with what they asked for.

With them comes what the ray through the pointer's pixel meets
(`native/src/inspect.rs`):
the resolving pass, at that pixel, marches it as the samples' rays are,
writing down the see-through voxels it crosses,
then the voxel that stops it and the light that voxel is drawn under;
the window finds their looks in their tiles.

## 3. The flat engine (`bake/`, `native/src/engine/chunks/`)

The flat engine draws cells of the octree,
each a picture on a plane fixed in the world,
which the camera's perspective moves and scales:
it bakes a cell once, whatever the camera, and draws it as quads.

**Faces.** A cell is seen along one of 18 directions (`bake/src/face.rs`):
the cube's 6 axes and its 12 edges' diagonals.
Each draws the rays nearer its axis than any other's:
an axis face those within tan 22.5° of it across and up,
an edge face those between, up to 1/√2 up;
so a ray is at most 35° off the axis it is drawn along,
and looking along a world axis, as streets and walls invite,
puts one face in the middle of the screen.
A face is a rotation, its lattice vectors R, U, F,
F an axis (k = 1) or an edge (k = 2, |F|² = 2), up a world axis across it.
Site p is at (a, b, c) = (p · R, p · U, p · F):
layer c is the sites with a ≡ c (mod k), 1/√k apart along the axis,
and column (a, b) the sites k layers apart.
A pixel is drawn by its ray's face alone (`face_of` in `compositor.wgsl`):
quads are cut by the planes that bound the face's rays.
The panel draws the seams over the picture,
and can colour the voxels by layer or by chunk (`Colours`).

**Cells and levels** (`bake/src/lod.rs`).
A chunk is a cell of level L, the cube of 2^L sites of grid 0 a side,
drawn as the voxels of grid max(0, L − S) (`--shift S`), M = 2^S across at most.
Along a face its cells are at (I, J, K) as sites are, K its layer of cells.
The camera picks the cells a face composes by their layer alone:
level L claims those with K · 2^L at most 2Q · 2^L layers ahead of the eye's,
and draws those of which some child is past the finer level's claims,
so a site is drawn at the finest level claiming its cell;
where a cell overlaps finer ones, both draw it, and the finer is in front.
A ray of a face goes along each world axis its F goes along,
so the cells it crosses come layer after layer,
and once it leaves a level's claims it is never back:
no ray meets a finer level after a coarser
(`no_ray_meets_a_finer_level_after_a_coarser_from_anywhere`).
Detail is viewing distance over a cell's thickness (`Lod::new`): Q at least that.
A face's cells are picked for the eye anywhere near where it is
and the view turned a little (`Lod::pick`),
those of whole voxels and of medium apart, each by its own Lod,
each with what is known of it (`Seen` in `renderer.rs`):
its chunk, that it holds nothing, that it is hidden, or what it waits for.
A cell holds nothing if none of its grid's voxels is in it,
or all are buried (`Level::meets`, `Level::buried`, by each tile's bricks).
A frame is every chunk the picks compose for an eye anywhere in them;
which of them the eye draws from where it is, the GPU finds.
So the renderer runs on a thread of its own (`chunks.rs`),
its bakers on others, as the world's, behind the thread that draws.

**Baking** (`bake/src/lib.rs`, `pillars.rs`).
A chunk reads its cell's sites and around them
(`chunk_reads`: a cell deeper along the axis, half a cell nearer),
all of whose tiles it waits for, so it is baked once.
Of each column, its first whole voxel a ray of the face can enter
(by a face of it turned towards the camera, or beside the axis)
stands for its pillar:
as deep as a ray leaving its side may see it,
till the column beside it is whole there, in the cell or not,
and no deeper than its own solid runs on.
Its colour is the voxel's look through its faces turned towards the camera
(lattice's `facing`), shaded by its open faces (`voxel_colour`),
or lit, the light those faces send (`lit_voxel_colour`, from `light/`).
The picture is the columns' colours on a plane (`flatten` in `compositor.rs`),
a column a brick of it, a layer wide and a site high:
the plane the edge pillars, beside an empty column, reach least from,
no steeper than the face's rays.
Where two cells' pictures meet, each on its own plane,
a gap may open as wide as the rays' slant times that reach,
so each frame each edge of a picture moves out by so much (`place`),
and depth decides where neighbours overlap
(`ground_along_a_face_shows_no_gaps`).
A surface along the axis is a row or two of columns
whose pillars run through the cell, and is smeared over its depth.
See-through voxels are pictures of their cell's medium, slab by slab of its columns
(`cell_medium`, `veils`).
A half of a slab of a column is a see-through voxel of its own:
what a ray along the axis gets of its voxels, near to far,
each channel's colour and optical depth (`Half`).
So it adds up as they do, whatever its depth,
and a level's halves show what a finer level's do
(`halves_of_any_depth_add_up_as_their_voxels_do`).
The halves are laid on the picture far to near,
each sending its colour and letting through the rest, channel by channel,
as the voxels engine's rays take a medium
(dual-source blending, `veil_fragment`;
`a_flat_medium_lets_through_as_much_as_its_extinction_says`).

**The compositor** (`native/src/engine/chunks/compositor.rs`) draws only what may show.
A chunk's picture is runs of its rows' columns, a quad each,
in clusters, with its record (where it is, wherever the camera),
uploaded when it comes and dropped when it goes, as the frame's changes say:
a change costs what changed, its writes one copy the GPU scatters.
Each frame the GPU places every chunk from the camera (`place`),
keeps those whose level claims their cell from where the eye is,
draws the clusters that showed in the last frame,
reduces their depth to a pyramid of farthest depths,
then tests every cluster in view against it, draws those newly showing
and keeps which show for the next (two-phase occlusion culling).
A pixel's depth is its plane's, by w, finer levels in front (`flat_depth`):
no pixel works out its own, so the GPU tests depth before drawing it.
Where neighbours' pictures overlap at a depth, the one drawn last shows,
so pixels here and there fall by the order chunks came in.
Before a cell is baked, its tiles even made,
rays cast on the CPU from where the eye was through the voxels drawn
hide it if all it may draw is behind what they meet
(`Hiding` in `renderer.rs`, `occlusion.rs`);
its box is a probe the GPU tests once all is drawn,
and one that shows is baked after all (`hiding_leaves_the_picture_unchanged`).
Fog, the sky and the medium's pictures are as the voxels engine's.
Lit, chunks show the light as it is, past white white.

## 4. Worlds (`lattice/`, `worldgen/`, `ground/`, `worlds/`)

**The lattice** (`lattice/`). Voxels are cubes of the cubic lattice Z³:
site (x, y, z) the cube [x, x + 1) × [y, y + 1) × [z, z + 1),
with 6 faces, one towards each neighbour along an axis.
So levels nest, a coarser cube the eight finer in it (section 1),
and walls and floors along the axes are flat;
the price is that directions are not alike:
a ray along an axis crosses a cube by a chord of 1, along a diagonal by less,
and shapes drawn on the cubes show the axes ([checks.md](checks.md), "Directions").

**Storage** (`Grid` in `lattice/`). A world is tiles of voxels per level of
detail (section 1) plus a spawn point: a word per site of a tile's box,
what its voxel looks like (`Look` in `lattice/`):
its colour, sRGB, and how much of the rays through it it stops; 0 for empty.
Under all of them, it is of pieces held in part, which stop a ray whole or not at all,
or of a medium, which stops some of each (`in_part`, `medium`).
Grids hold colours, not materials:
a coarse voxel holds the mean of what is in it.
A voxel, whole or held in part,
may look different through each of its 6 faces (`VOXEL_FACES`),
what is seen of it looking at that face:
a coarse voxel stands for its cell, which may look different from each side.
Mostly it looks alike through all;
the sites that do not are listed apart (`views`).
What stops a walking body is a bit a site apart (`solid`).
Rays (`lattice/src/ray.rs`) and the flat engine's pictures show the faces turned most against them (`facing`).
The box is cut into bricks 8 units wide, 512 sites,
and only bricks with voxels are stored.
Distances are in world units.

**Scenes** describe shapes in metres and fill the sites inside them;
each world picks its own scale, the renderer assumes none.
The worlds have `UNIT_SIZE` metres per unit
(`worldgen/src/units.rs`).
They are endless and can be generated in any box alone
([generators.md](generators.md)),
by `worlds/` on the engine of `worldgen/`.
The engine has no up: a field's spans lie along any axis,
shapes and things stand any way, the contract checks every direction.
Hosts show a world up along one axis, +y (`world::UP`):
its heights lie along it, its tiles are made together along it,
and the rock under them is found and kept along it (`generate_shown`);
pictures and the checks look along it and across it (`generator::across`).
What only a world on a ground needs, plans of its columns (x, z),
heights, layers down from a top, land shared column by column,
is `ground/`, on the engine's public API;
the cheese needs none of it, its parts sharing space point by point.
The forest's ground is a heightfield sampled per column of sites;
trunks are columns at least 2 units square,
crowns leaf clusters (balls) or a solid of revolution (spruce),
branches tubes and fallen logs capsules, so thin parts stay one piece at every level.
The city (`worlds/src/city.rs`) is planned on a 0.25 m grid:
each plan cell is a box holding four sites, so walls stay flat and frames one piece.

