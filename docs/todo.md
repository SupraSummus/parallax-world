# To do

Known problems, measured or derived, then open ideas.
Numbers are for the forest at 1280×800, as measured when written.

- **worldgen still decides some things with floats.**
  Shapes are kept and tested in fixed point,
  but they are given in floats, and their areas, fills and tubes' radii are f64,
  as is the forest's geometry (sin, cos, pow, exp, atan2, hypot, from `js.rs`).
  They should go to fixed point like the noise,
  the forest building each thing relative to its anchor.
  The fixed-point containment tests cost about 15% at level 0.
- **A flat thing's faces round to the sites.**
  Faces on a plane of a level's sites split its sites ("Faces" in
  `docs/generators.md`); off those planes a face rounds to the sites,
  up to half a site either way: even over a ground's many heights,
  but a town's floors at one height all go one way.
  Sites near a face kept with the share of their voxel inside it
  would be even for any height,
  but would leave holes in boxes under two sites thick
  and roughen every surface.
  On a gentle slope the rounding is steps a site high,
  which computed light shows as contour lines from level 2 on.
  Tried on spans' open ends past level 0, that share took the lines away,
  but each of the layers painting one ground (forest, mountains)
  had to cover the same part of its top site, not add up, by a flag on the share;
  a ground declared once (see "Fields get the spacing", below)
  would hold its top's share in one place.
- **Sheets and rods stop too much seen along them.**
  Grown to a site and opaque ("Levels" in `docs/generators.md`),
  a wall stops every ray across it, as it should, and every ray down it:
  from above, walls a unit thick cover 8 times what they do at level 0
  at levels 3 and 4, posts 4 to 6 times from level 1 (`worldgen/tests/shapes.rs`).
  A site stops rays alike from every direction:
  at levels 2 and 3 the windmills' sails, measured, are seen through,
  rays from all round seeing about half as much of them as at level 0,
  yet from above they take 0.02 to 0.05 of what rays straight down see,
  against 0.002 to 0.02 at level 0, and the tower's plaster 0.03 to 0.04,
  against none.
  Measured far forms count what a thing hides along each axis
  by pairs of neighbouring columns, as rays meet voxels (`pairs` in `generator/forms.rs`);
  counted by columns, walls and rails hid a half to two thirds as much,
  which offset this: since, at level 4 against level 2,
  rays get through about 0.005 less often around houses
  and 0.01 less around the railway, on average.
  An opacity per line of nearest neighbours
  (a share per pair of a voxel's opposite faces, measured as rays meet columns along it,
  each ray taking the share of the line it enters a voxel across)
  was tried and not kept
  ([SupraSummus/parallax-world#205](https://github.com/SupraSummus/parallax-world/pull/205)
  and its branch `claude/measured-by-line`).
  Over random directions it is nearer what a thin thing stops:
  in a cell four fine sites wide,
  0.15 off for a wall a fine site thick, against 0.23 with one share,
  0.06 for a rod, against 0.12.
  But the contract barely moved
  (of 4104 margins, 68 closer and 31 farther, none by more than 0.01),
  as measured houses fill their cells but at their edges;
  and along an axis, as from above, a ray enters by the faces of four lines,
  two of them slanted to a wall across another axis,
  so it sees as much of the wall as with one share, and no six shares do better.
  What would: a voxel holding where in it its content lies,
  between two planes (contours, as in Laine and Karras' sparse voxel octrees),
  so that a wall is thin from every side and need not grow to a site.
- **Small things on a ground sink into it at coarse levels.**
  A solid under the sites is drawn at the site nearest it,
  which a few units over a ground on a plane of sites is the ground's,
  full already: what the solid stops is lost.
  From level 4, small blocks, balls and specks on the ground
  keep 0.4 to 0.6 of what they stop from above; lifted clear of it, 0.8.
- **A thin ellipsoid between two planes of sites can vanish.**
  Its thickness grows to half a site either way of its centre,
  so centred between two planes it holds their sites only at a point:
  discs 2 units thick, each 10 units over the ground, were gone at level 2.
  Boxes keep the sites on their faces by `Tie`.
- **Fields get the spacing, and mostly use it for one rule.**
  Of the 15 fields in `worlds/`, 9 hand it to `Column`,
  and 7 write its rule out (`m(0.5).max(2 * spacing)`):
  a layer at least two sites thick.
  The rest leave detail out by level, each by a rule of its own
  (the moor's heather, the ponds' reeds, the rails),
  dither by hand (the railway's ballast, in shares of iron and timber),
  or make their own far form (the forest's canopy, a scene's levels).
  A stack declared once (layers down from a height, media up,
  which end of each is fixed), sizes on spans as children have,
  and patterns read through `Plan::dithered`
  would leave the spacing to the engine but in the canopy and scenes:
  a field's levels would be the engine's to keep, not each world's.
  Found on the way: the fields' crops are thickened at every level
  ("Fields stand taller at level 4", below);
  the town's kerbs (a unit) and the railway's tunnel lining never are.
- **The native frontend is missing pieces.**
  Its camera is f64, not fixed point (positions reach the GPU relative to it).
  Its compositor needs indirect draws counted on the GPU
  (`MULTI_DRAW_INDIRECT_COUNT`), and so Vulkan:
  Metal and GL lack them, and on DX12 wgpu's leave out the first vertex
  the chunk's slot comes as (`BACKENDS`).
  Its frames are bound by the GPU (`--stroll`):
  culling the clusters in view costs about a third of a frame,
  drawing those that show most of the rest.
  The first phase reads every cluster only to find those that showed;
  a list of them, kept from the last frame, would spare that.
  Walking in the window for minutes, a frame sometimes takes seconds
  until the driver resets the GPU (seen on Intel UHD with Mesa);
  the window then makes its device again (`window.rs`).
  Why is open: the same views and paths, drawn headless, take milliseconds.
- **Native's cells could be simpler and cheaper**
  (section 1 of `docs/architecture.md`).
  Which cells may hold voxels is asked of `LayerIndex`, per face, layer and block of columns;
  the cells of each level are the same lattice for every face,
  so a set of the cells holding voxels, per level, would do for all twelve.
  Nothing skips cells behind nearer ones: every cell in view is baked,
  and the compositor's culling hides them;
  the depth pyramid it culls clusters with could cull cells not yet baked too.
  Cells straddling where a face's levels meet are drawn at both.
  Each render tests every cell a face composes against the screen,
  though a face keeps its cells picked (`Lod::pick`);
  the render that picks them anew, every few units moved,
  takes a few times longer.
  `surface` still finds a cell's cut faces, which `cell_relief` drops.
  Detail sets Q = max(4, ⌈2·detail⌉), so below 2 it changes nothing;
  Q itself, a whole number, would say what it does.
  The web still draws slices, with runs and cut faces:
  its compositor would need depth by distance too to draw cells.

- **TS and Rust still say much the same twice.**
  The tiles around the camera (`world.ts`) are `native/src/world.rs`,
  and faces are in `native/src` too, though its renderer draws cells, not slices;
  `LayerIndex.range` (`voxel-grid.ts`) is `bake/src/layer_index.rs`'s.
  The renderer is the large step.
  Tests too: cells' and view's are in both;
  relief's, pillars', the renderer's and its regressions' only in TS.
- **Tests take most of CI's time.**
  Most of it is the worlds' contract, bound by the CPU:
  pictures from above generate level 0 over all of a world's heights.
  CI runs the valley's in a job of its own;
  it and the rest of the Rust tests are CI's longest jobs.
  If CI is still too slow, the contract at seeds but `REGIONS_SEED`
  (`worlds/tests/contract.rs`) could run only on main.
  The contract's survey asks every cell of a sparse population's square:
  the valley's haystacks' anchors cost most of it.
  Caching Rust's builds would spare little: `libm` is their one dependency.
- **Slice detail sets both thickness and level of detail.**
  "Slice Detail" sets how thick slices are,
  and so their level of detail (atoms of 2^(k−s) at level k,
  s as large as the grids' reach allows).
  Thicker is faster but coarser: in the forest, a first render takes
  about 6.5 s at detail 9.5, 3 s at 2.4 and 2.7 s at 1
  (voxels 4 and 10 times as large), against 2.5 s drawn flat at 9.5
  (Node, 1280×800).
  Thickness could follow what culling and the slice count want,
  and the level of detail the screen's pixels:
  coarser atoms than detail gives are a smaller s,
  finer ones need more than 2^4 layers of atoms a slab,
  so splats' depths in more than a byte (`ATOM_SHIFT`).
- **Coverage is weaker in relief.**
  A splat moves on its centre plane with the camera,
  so a chunk's mask holds what it covers from the axis and the corners
  of the rhombus of rays, and what it may reach from any:
  in the forest at the default detail, twice the chunks of flat slices are drawn.
  Nor is that exact: a block covered from each of those may still open
  between them, which no picture has shown yet.
  The camera's slant is known where coverage is tested;
  masks for a few slants, picked per block, would cover nearly as much
  as flat ones, and between nearer slants.
- **Coverage masks are coarser than the screen.**
  `mask_scale` marks ⌊40 / t⌋ pixels per unit of a slice t thick,
  but at a level's nearest slices a unit is focal / (t · detail · LAYER)
  pixels on screen, about 95 / t at 90° on 1280 px at the default detail.
  A mask pixel counts as drawn when its centre is in a splat,
  so a gap between splats narrower than a mask pixel can count as covered,
  and what is behind it is skipped.
  The mask could follow the focal length and detail,
  keyed in the chunk like its grid, leaving the slices as they are.
- **The web culls only whole clusters, on the main thread.**
  The forest has millions of splats in view, and only about 1.5 % show a pixel;
  coverage leaves out the clusters hidden whole,
  and the rest are drawn.
  The native compositor culls finer, on the GPU
  (section 4 of `docs/architecture.md`), which WebGL 2 cannot
  (no compute, no indirect draws);
  WebGPU can, where a browser has it.
- **Native pillars more than double the splats.**
  Natively a run is a row of splats, one a voxel
  (section 4 of `docs/architecture.md`),
  which in the forest adds more than half to the GPU's frame.
  Splats of a row that no ray of the face can see,
  hidden by neighbouring columns with runs as long,
  need not be made.
  Fewer, larger splats cost more, not less:
  a splat every other voxel, grown to leave no gaps
  and with its depth varying along the axis, culled worse;
  one splat cut to the hull of a pillar's ends drew large rectangles,
  mostly discarded.
- **A chunk as an image was slower, on an Intel GPU.**
  The branch `relief-chunks` draws a chunk's columns as tiles of
  colour, depth and run, a rectangle each whose fragment shader
  follows each ray through them, or traced in compute:
  right to the pixel, runs included, a tenth of the vertices,
  but slower than splats in the forest.
  There (Mesa's driver), a fragment shader that discards
  or writes its depth, even conservatively, is depth tested after it runs,
  so it runs for several times the fragments that lie before what shows.
  GPUs that test early despite discarding may differ;
  fewer, thicker slices, a column holding several runs,
  would cut the layers a ray crosses.
- **The depth within a layer needs a fine depth buffer.**
  A layer's splats share 1 / (layers + 1) of the depth range;
  WebGL promises only 16 bits, which leaves a few dozen steps for them.
- **Bricks on the edge of a face's rhombus are baked whole.**
  Near where a level starts, about 19 bricks meet the rhombus,
  and most of them cross its edge or the screen's.
- **Most baked splats are hidden.**
  When chunks were bitmaps,
  over 95 % of opaque baked pixels were covered by nearer slices.
  Content that no ray of the face reaches within its slice
  need not be baked.
- **Flying, the main thread can't keep up with the workers.**
  Flying up (a trace in a desktop browser),
  workers bake about 4,000 chunks a second
  and the main thread is busy 97 % of the time, at 10–13 frames a second.
  Renders take 30–90 ms, longer the more new chunks they take:
  uploading textures (now splats), handing chunks to `Coverage` (`coverage_add`),
  and covering with them (`coverage_cover`).
- **Turning, finding covered chunks is slow.**
  About 11 ms of a render (forest, 1920×1080, Node),
  mostly testing sub-blocks one by one in `cover` (`web/wasm/src/coverage.rs`):
  about 270,000 tests a frame, three in four failing,
  as the forest covers the screen in fine pieces.
  Testing quarters of a cell first would not help (two in three are mixed),
  nor would clipping without allocating in `shape`.
  The web could run it in a worker, a frame behind.
- **Walking, native keeps more than a core busy**
  (the valley, `--stroll 20 --on-ground`, outside the GPU driver),
  measured when it drew slices: new tiles took about a third.
  Lit, about 1.6 times that: the light itself (`Light::light` most),
  and new tiles half as much again, why is open.
- **Native's medium costs a fragment per slab a ray crosses**
  (architecture.md, "See-through"), as far as the view goes:
  pictures added up in any order, nothing after a ray is all but stopped is left out.
  Only media are (`KindDef::medium`): the moor's mist.
  - Where a medium's colour changes along a ray, its colours mix without their order.
  - The medium starts at the first half slab past the near plane:
    right by the eye, mist is a little thin.
  - A site's pieces, where shown, hide its medium and what is behind:
    right on average, but a coarse site of a crown in mist
    is a whole voxel or mist, never the crown's grain seen through mist.
- **Native's other engines are short of the chunks in places**
  (architecture.md, "Native: another engine").
  - Voxels: lit, every voxel of a tile a ray can meet is lit,
    not only those that show:
    lighting a view's tiles takes six times as long as making them,
    much of it generating the tiles around them that the light reads,
    and the light takes twice the bricks' memory, so the pool holds a third as many.
    A voxel's light is one colour, its faces' mean, whichever face a ray meets.
  - Slabs are baked around the eye and baked again as it moves,
    hundreds of MB every few seconds: walking, they stutter.
    Once the eye moves, they are more than lavapipe binds (128 MB).
    A column keeps only its first voxel, so a medium lets through
    as much as that voxel does: 1‰ of extinction lets all through, not 93 %
    (the voxels' medium test, run on slabs).
    They read the compositor's globals, the faces' views.
- **`culling_on_the_gpu_leaves_the_picture_unchanged` fails now and then**
  on an Intel UHD (Mesa), at `b4c79ff` too; why is open.
  Drawn flat, every time: the forest from its spawn.
  Maybe as it failed on lavapipe by the moor's mist:
  the depth pyramid's shorter side lost its last levels,
  and clusters over the whole view were hidden (fixed since).
- **Native's flat pictures** (`--engine flat`, architecture.md, section 1).
  - Their depth is by w, ahead along the view (`flat_depth`),
    the other engines' by distance from the eye (`depth_of`),
    so culling and the pictures of medium ask which the frame draws.
- **Native's chunk engines while the eye moves.**
  - Their renderer composes a frame behind (`chunks.rs`),
    so `--stroll` counts nearly every frame as one whose content is still coming:
    it no longer tells holes from that.
  - A pick moved composes its every chunk again, on the renderer's thread,
    and thousands of chunks come and go at once,
    most of them the finest cells', a voxel or two each:
    the compositor spreads them over frames (`CHANGES`),
    a few microseconds each on the main thread.
    Fewer, larger chunks near the eye
    (a chunk holding a cell's finer cells' pictures too) would cost less.
  - A shot differs from run to run in about 0.3 % of its pixels (flat, the valley):
    ties between chunks fall in the order they came, which timing decides.
- **Native's splats leave a hole by a hut.**
  `--engine splats --view 'seed=42&world=valley&camera=1200,100,36700&look=20,-12'`:
  sky shows through the ground in the bottom right, with or without culling,
  in straight edges like a cell's. Not looked into.
- **Native's reliefs.**
  - Thin things seen at a slant break at the default detail:
    the city's road markings into combs, whole at twice the detail.
  - Marched per pixel, a tile per quad, they are slower on lavapipe
    than whole splats (`--engine splats`), which lavapipe tests no earlier;
    a real GPU is unmeasured.
  - Tiles hold their empty columns: about half of those uploaded.
  - A few pixels show what is under the ground at cells' edges
    (7 in the forest at `seed=12345&camera=-14,40,-200&look=-25,6`):
    columns' cells are not their voxels' outlines.
  - Where pillars of two cells meet a voxel at one site,
    the one whose front is nearer it shows (`shown` in `compositor.wgsl`);
    as near, they tie, and the one drawn last shows, its front's colour.
  - `chunk_reads` reads a cell deeper, for the solid behind a relief.
- **The native compositor's buffers stop at the device's limit.**
  `Arena::grow` caps a buffer at what the device allows
  (128 MB on lavapipe), and `allocate` then hands out room past it:
  wgpu panics in `Compositor::prepare`
  (the city at 1280×660 at detail 27 did, before `GRID_REACH`;
  the forest at its defaults does, `seed=42&world=forest&look=200,10`).
  Past the limit it could drop the farthest chunks, or split the buffer.
- **After a long jump, the workers' frame culls more than the main thread's.**
  Once the grids of three or more levels move at once,
  a renderer baking in workers counts a few more chunks covered
  than one baking on the main thread at the same view
  (`worker-bakery.test.ts` keeps its move short of that).
  Whether it shows on screen is not checked.
- **Moved grids are posted whole.**
  When a grid moves with the camera (section 1 of `docs/architecture.md`),
  the main thread posts all of it to every worker, though they have most of it:
  at level 0, ~40 MB a worker every 12 m walked.
  Workers could keep the tiles and put the grid together themselves.
- **Far voxels 5.6–11 pixels across.**
  Atoms 4 times as fine (1.4–2.7 pixels)
  would need `GRID_REACH` 4 times as large:
  ~16 times the grids' bytes, in each bake worker.
  Grids shared between the workers would allow it,
  or bricks sent to a worker only once a chunk it bakes needs them.
  (Shared memory needs cross-origin isolation,
  which GitHub Pages can only get through a service worker.)
- **Generating the forest's grids takes 3–8 s of worker time per level**
  (the web's WebAssembly, in Node; the TS forest took twice that),
  most at levels 0 to 4 (a moved grid, a seventh of that).
  In the TS forest it was mostly the ground's noise,
  which the terrain and the canopy compute about 14 times per column
  (with its neighbours, for slopes),
  and at levels 0 and 1 the undergrowth's many small shapes;
  worldgen's, counted ("Cost" in `docs/generators.md`),
  goes mostly to the trees at levels 0 to 2
  and to measuring their shadows from level 4.
- **Some of the valley's tiles are over the cost budget**
  (`OVER_BUDGET` in `worlds/tests/cost.rs`), each by a thing done one by one
  where a tile holds four times as many each level:
  at level 3 the towns' things drawn whole to be measured;
  at level 8 the moor's pools, each claimed by reading the ground around it.
- **Plans are made at every column read, whatever their scale.**
  The valley's land, decided at 4 m, makes its six domains' bids
  (the ground among them) at each column of a tile at level 0.
  Bids made every scale's width and interpolated would cost a column
  a share of a sample, up to the level whose sites are that far apart;
  past it, dithered reads would want their offsets on the samples.
  Domains' edges would move a little.
- **Hashing is the forest's largest single cost.**
  Each site hashes three times (shade, pick, `shown`'s coin),
  each `hash64` mixing three times, as do noise corners.
  A cheaper hash would change every world.
- **The learned town's rules are from the old towns.**
  `worlds/data/town-grammar.bin` was learned when the valley's towns
  were all market towns alike, so lab's `export -- --check` says it differs.
  Learned again from today's, mostly hamlets and villages,
  it draws far less town: 235 houses in `learned.rs`'s test, not over 600.
  Learn it from the market towns alone, or let the learned town be sparser.
- **A domain can't stand in a world twice.**
  Its populations seed their cells by layer name and the generator's seed
  (`Cell::random`), not its `Scope`: two forests would grow the same trees.
  And its kinds' ids are its place in a table every world concatenates
  in one order (`kinds!`'s `base`); named, the world would number them.
- **Worlds share tables of kinds**, so the contract only prints
  the kinds nothing surveyed draws, and fails on silent layers alone:
  the moor's, the learned town's and the city's are the valley's.
  With only the kinds it draws, a world could fail on a kind
  drawn only far from spawn, or never.
- **Far regions and cost's places are still listed by hand**
  (`Entry` in `worlds/src/lib.rs`).
  The survey could give both: wide boxes around fields' strata,
  the places of the costliest strata.
  Then `Region` would go.
- **Things' variants of one part, drawing the same kinds, are one stratum**
  (`worldgen::checks::survey`): a ruin and a watchtower, both of stone,
  are checked in one region between them.
  Naming a landmark's node by its variant, as towns name their children,
  would part them; or the survey could count which code drew each part.
- **The solids' scenes ask little against the level two finer.**
  In `worldgen/tests/shapes.rs` every solid keeps the contract there
  to the last level checked: a `Look` is a mean over a region's rays,
  so a solid that thins changes it only by its share of the view.
  Scenes the solid fills, the same share for every case, would ask more.
- **The forest's canopy measures crowded trees in sample stands.**
  From above it reads each column's owner (`Rooms`),
  but what the trees standing in a column hide from the side
  comes from stands at four densities, read by the trees' chance.
  A crown alone measured by offset and slab (`Shadows`),
  read for each column's owner, would leave rooms the one rule.
- **The forest on steep slopes breaks the contract at level 4.**
  A region on a wooded mountainside of the valley,
  where the trees thin out to the treeline,
  came 0.10 to 0.15 apart at level 4, in materials hit and from above:
  leaves in place of trunks and of the slope's scree and rock.
- **Towns and the city line up with the lattice.**
  `worlds/tests/directions.rs` lists them (`KNOWN`).
  The valley's first town lays out streets and blocks along x and z,
  and builds in frames of quarter turns (`frame.rs`):
  a four-fold harmonic of 10 %.
  The city is boxes on a plan, all along the axes: 4 %.
  The learned town's settlements are such boxes too (2.5 %),
  but their streets' own direction (2 %) hides them from the check.
  Square noise (`ValueNoise`, `Fbm`) draws under 1 %, too little to tell.
  `mountains.rs` turns its noise by 37° (`turned`),
  which leaves its four-fold harmonic mostly on the diagonals;
  22.6°, (12, 5)/13, would leave none there.
- **Fields stand taller at level 4.**
  At the valley's first town's edge, rays hit fields about 0.04 more often
  at level 4 than at level 2 (seed 42), whatever the houses' far form:
  a crop thinner than two sites is made two sites tall (`Span::medium`),
  4 m of wheat in front of the houses' walls.
  Sunk into the ground, its top where level 0 has it,
  the wheat showed less but the stubble under it more.
  It breaks the contract: averaged over seeds 42 and 1 to 3,
  what rays from cameras see changes 0.05 to 0.07 at level 4
  in the learned town's and the valley's crops, wheat and stubble most.
- **Worlds are checked to lower levels till what follows is fixed**
  (`Entry` in `worlds/src/lib.rs`):
  the valley to 3, from above to 2, the learned town to 3,
  the moor's far region to 5; then raise them back.
- **Things of the valley break the contract at level 4**, averaged over seeds:
  rays from cameras see 0.054 to 0.061 more of the towns' walls, towers and gates,
  and of a station at one seed;
  the forest's boletes 0.05 (likely "Sheets and rods", above).
- **The valley's market from above at level 3**:
  its stalls and paving change what rays straight down see 0.069 on average.
- **The moor's far region keeps its fields' look to 0.014 at level 6**,
  over a quarter of `TOLERANCE` (`check_far`), its heather drawn for its sprigs.
- **Measured houses show their walls from above at level 6**,
  which the contract does not check, and at level 4 in a region of houses alone
  (seed 2: plaster 0.19 against 0.01 at level 0, 0.106 apart;
  within the contract since the oaks among them are round, likely by chance).
  Around the valley's first town's centre (seed 42),
  level 6 shows 46 % houses from above against 25 % at level 0,
  and 16 % roofs against 24 %: the rest is walls, timber and stone
  (0.31 off; 0.18 measured from level 1, 0.22 from level 3).
  The engine's test village (`worldgen/tests/contract.rs`) is checked
  from above only to level 2: at 3 its walls take over from the grass
  (0.07 in a linear channel).
  Measured from level 4, a house is a few fine sites
  ("Levels" in `docs/generators.md`):
  its eaves, thinner than one, leave its walls' tops to show,
  and its gable ends (`gable_ends` in `town.rs`), grown to one,
  paint its roof's ends.
  And a cell's share counts rays the cells behind it stop too,
  so a thing keeps more cells than it stops rays.
- **Layers that don't say where they are are drawn everywhere**
  (`Field::about`, `Population::on`):
  the railway's track (a plan of heights too, as its tunnels are about),
  the sea, the moor's water and mist,
  the city's buildings, and the towns', landmarks', settlements' and steadings' things.
  Native's tiles are cubes, so what a column needs is made again
  in each cube the ground passes through (a plan's columns, per thread);
  its views take up to twice as long to complete
  as with tiles of whole columns,
  the voxels engine's most, as it asks for every cube around the eye.
  Once every layer says where it is, the world's `heights` can go:
  hosts would ask `whole` alone.
- **Cost weighs every plan lookup alike** (`NS` in `worldgen/src/cost.rs`),
  though one of a column just read costs a fraction
  of one among claims making their keys.
- **The places check finds few things of sparse populations**
  (`worldgen/src/checks/places.rs`):
  it asks COLUMNS cells at most,
  and finds a claimed thing only from those within its reach.
  The valley's landmarks: 118 at eight seeds (not 686 as one per region),
  under ENOUGH a seed, so no seed is tested alone;
  its towns 57, its haystacks 2.
- **The valley is still laid out along x, across the path's centre line.**
  The path is a curve (`worlds/src/path.rs`),
  but the ground's walls and dip, the ranges, the fields' plots
  and the landmarks measure across its centre line.
  A curve's distance folds past a bend's radius
  (16 m at the tightest, where the path bends onto a street),
  which would crease the walls and the ranges,
  and costs a search far from the curve;
  what is far would read a coarser band of the curve, its mean.
  The moor's stream (`stream_distance` in `moor.rs`) is measured along x too;
  from a curve its banks would move where it bends, so it is left as it is.
- **Light is a prototype, in look's pictures and native's computed lighting**
  (`worldgen/src/light.rs`); the web does not light its tiles.
  Light comes along the 12 neighbours' directions only,
  and each level takes one step (`STEPS`) before the coarser one takes over,
  or more where the coarser fills what it leaves empty.
  So light from D away comes from a level whose sites are about D across,
  which keeps what a region stops on average (the contract),
  not whether it is closed:
  walls thinner than the space they close let light in,
  openings smaller than it let none (the door and the window below).
  A railway tunnel, lined 0.5 m with the ground's hollow behind,
  is lit so from that hollow.
  Rock filled round the tunnels kept them dark,
  but bent a world to the light: not kept.
  Against the reference (`light/reference.rs`, test `against_the_reference`;
  the ignored test `maps` draws the scenes from above):
  a lamp lights the ground in four lobes, along the upward directions,
  and up to ten times too little right under it;
  a roof shades as much as it should only along the lattice,
  twice too little turned 45°;
  a wall's shadow in the sun is short and blotchy,
  as the levels that carry it get coarser;
  past a door from a yard, where only bounced light comes,
  a room is several times too bright, and the yard a third as bright.
  A coarse site holding part of a door (as `Far::measured` draws one)
  lets light through in every direction alike:
  drawn so, the yard's door made the room tens of times too bright.
  A window smaller than the coarser levels' sites lets no sky in at all
  (they close the room), so a room lit by its windows is black.
  Native's bakers each light what they show with a `Light` of their own
  (a generator is not `Send`):
  they share tiles (`Shared`) with each other and the world's threads,
  but each finds light again where another found it,
  and makes a generator, whose plans' caches take tens of MB;
  each of the world's threads makes two, its own and its Light's.
  Lit, the world's threads make each level's tiles near the camera first,
  the coarsest's before all,
  but a tile of the coarsest levels takes several times a fine one's work
  (the plans over its whole area),
  so the first chunks come seconds later than unlit.
  Native's brightness sliders light the world anew (`World::relight`),
  as slowly as at the start.
  Light adds up, so it is the sum of the sky's, the sun's and the glow's;
  kept apart, the sliders could only mix them,
  but lighting and keeping that light would cost three times as much.
  The sun is one of the 12 directions.
  `Light::coarser` reads the nearest site and its 12 neighbours,
  so near a corner where six voxels meet it misses one within its reach,
  and light jumps a little there;
  every site within reach would leak light through walls one site thick.
- **Light sees each site as hosts draw it (`Grid`), and what glows, a prototype**:
  - a site's pieces show a voxel where a hash is under their alpha
    (`VoxelGrid::show`), as leaves should,
    but measured houses' walls come out ragged;
    at the coarse levels light reads few sites,
    so a voxel there shown or not moves a lamp's light (the city at night);
  - a site's pieces' colour is one of theirs, picked as often as it is seen,
    not the mean of what its faces see;
    where they show, its medium is not seen;
    a glowing thing's glow is hidden by its own, pieces or medium,
    not by the other;
  - measured things are measured from fine voxels kept by chance,
    so their small lamps come and go;
  - a lamp glows from one site at each level:
    further off, that site may be as low as the ground's,
    its light comes flat, and none falls on the ground;
    two or three steps a level (`STEPS`), not one,
    would read each level only where its sites are small next to the distance,
    but against the reference they sharpen the wall's shadow
    and leave less light under the roofs and right under the lamp:
    the coarser levels' blur no longer hides how few the directions are.
    Light entering a site by each of the five faces it crosses
    (weighed as `cosines`), not by one,
    did better on the flat scenes with four or five steps,
    but darkened the valley, as its side lines ran into uneven ground,
    did not help the yard, and cost 2.5 to 4 times the time;
    mixing each direction's light into its neighbours' blurred more every level;
  - things in a site add up as if apart, and alpha has 255 steps,
    so at the coarsest levels a lamp stops nothing;
  - generating costs more, lit or not: every covered site is put for light.
- **LLVM 21 compiles AVX-512 code wrong (lavapipe).**
  It may rematerialise a vector of all ones into a register past 15
  in an encoding that cannot name one: the ones land in another register,
  and that one keeps what it held
  ([llvm/llvm-project#169009](https://github.com/llvm/llvm-project/pull/169009),
  fixed in LLVM 22).
  Where it strikes moves with the CPU LLVM tunes for and the shader's shape.
  Ubuntu 26.04's Mesa 26.0.8 on LLVM 21.1.8, tuned for Zen 4
  (CI's EPYC 9V74 runners), reads ymm29 in `voxels.wgsl`'s march unwritten,
  and most rays run out of steps.
  So CI's `native-gpu` runs without AVX-512, until its Mesa is on LLVM 22.
  To reproduce on a CPU with Zen 4's AVX-512:
  that Mesa (in a chroot will do),
  `llvm::sys::getHostCPUName` made to return `znver4` by `LD_PRELOAD`,
  and `MESA_SHADER_CACHE_DISABLE=true`,
  else lavapipe reuses what it compiled for another CPU.

## Ideas

- The valley's geography from water, not along one line:
  rivers that drain the ranges to the sea, lakes in basins,
  and the valley one of them;
  then paths as curves between towns (`curve.rs`), not `path_x(z)`,
  which towns, the railway, landmarks and the moor all read.
  Now the ranges, the sea and the upland run along z for ever.
- More scenes (`worlds/src/scene.rs`): Bistro, San Miguel.
  Not tried in them: walking, on walls a site thick.
  Lit (`--light 1,1,0`), Sponza's atrium is all but black; not looked into.
- Light along more directions at coarser levels, as radiance cascades do:
  near, light wants fine places and few directions; far, coarse places and many.
  Light keeps 12 at every level, too few far off (a lamp's lobes, blurred shadows).
- Slices' splats that show alpha (the web's show no medium,
  so the moor has no mist there):
  a chunk keeps a column's voxels till their alpha adds up to whole,
  and splats that overlap in a slice may add up to an opaque one,
  the rest drawn see-through.
  Alpha in steps of optical depth (−ln of what gets through),
  which add up where things are apart, might take fewer bits.
- Report cost at the places worlds name (`World::points`),
  not at the spawn and the regions' centres (`cost::places`),
  once the forest, the moor and the city name some:
  they would be measured at their spawn alone.
- Drop y as up from the web and the checks.
  A world is bounded along y by its `heights`,
  the web tiles x and z over them (native's tiles are cubes),
  and the checks look from above down y;
  an endless holey cheese wants no heights.
- Lay out towns at any angle?
  Walls along the lattice's axes look crisp on FCC;
  at other angles they jag.
  But every town and the city runs north–south and east–west,
  which at the scale of a layout reads as a grid.
  A middle way: streets and plots at any angle, the buildings on the axes.
  The buildings' walls would still show to `directions.rs`.
- Keep the fit of `cost::NS` in the repo, to refit it on a quiet machine.
- Generate the web's tiles as the native frontend does:
  those the chunks in view read, nearest first,
  instead of every tile within `GRID_REACH` of the camera.
- Keep the view in the address bar as the camera moves,
  so a view can be copied (the page reads it already).
- Measure the seams between faces against ray-cast perspective.
- Make the 12 stacks cheaper:
  four or more faces are in view at once, each with its own near slices.
- Find covered chunks more exactly: many chunks drawn show no pixel.
- Measure the WebGL compositor on real GPUs (headless Chromium only has a
  software one); if draw calls matter, pack chunks' splats into one buffer.
- Drop a splat's size: every splat is a voxel's outline,
  so a splat could be its site, depth and colour.
- Show render distance in metres:
  they are lattice units, and the forest's unit is 0.125 m.
- Show the forest to 22 km again: `MAX_LEVEL` 13, one grid more.
- Learn a world from examples instead of the split grammar.
  Two models in lab draw any box alone, flat or on the FCC lattice.
  The masked model (`lab/src/masked.rs`, `masked_fcc.rs`)
  learns the statistics and new layouts,
  but breaks long thin things (lanes, tunnels) into pieces.
  Synthesis by coordinates (`lab/src/exemplar.rs`, `exemplar_fcc.rs`)
  copies pieces of the examples, so it keeps lanes whole,
  but cuts houses where pieces of two towns meet.
  The critic (`lab/src/critic.rs`) says what tells them from real maps
  and what they copy;
  profiles (`lab/src/profile.rs`) how their parts are laid out.
  Drawn to a plan learned from real places (`lab/src/plan.rs`),
  shares with main roads and rivers as lines,
  both blend where the plan says, and keep the lines whole;
  but the streets they draw themselves fall into pieces
  (the masked model's, to a few stray draws: `lab/src/lib.rs`),
  and both draw paving in specks.
  Worth trying: a draw that settles neighbouring cells together,
  not each alone (cells drawn in an order that keeps neighbours apart early);
  a plan composed for a world worth walking through
  (landmarks, views, contrasts), not an average one.
- The critic judges flat maps only;
  3D scenes are still scored by pooled statistics (`lab/src/fcc_stats.rs`).
  Its windows as volumes would judge the FCC models too.
- Join lab's cubic and FCC samplers with one lattice type
  (sites, neighbours, parents): each pair now repeats its box bookkeeping.

Composing generators (section "Composing" of `docs/generators.md`), untried:

- More populations at `Points`, their far forms from what stands:
  the forest's boulders, logs and plants, the valley's haystacks.
- Bid per quantity: a land per channel (use, ground, wetness),
  each domain bidding in those it cares about, as the towns' terrace needs.
- Place the houses as things of their own, from the plan of lots,
  not as children of one thing per town:
  a box in a town now draws the whole town's carves aside, in a buffer
  as large as the box, where each house's would do.
  Counted as `Points` by kind, they could be drawn far away from their counts,
  not each drawn whole to be measured.
- Let the framework filter plans of numbers finer than the level's sites
  (`Plan::sampled` at the spacing), since plans declare their scale:
  coarse levels would skip fine bands' cost as they skip their things.
  Plans of categories are read dithered already ("Plans" in "Levels").
- More of the railway (`worlds/src/railway.rs`):
  a graph of lines between stations, made locally, and branch lines;
  catenary poles; a road from each station to its town;
  water in the route's cost, once the ground the line reads has any.
  Curves still lack arc length from an end (`Near::along` counts segments):
  the line's sleepers and piers stand by its segments along z.
- Rivers and roads along curves, in pieces and from `near`, as the path is,
  not as tubes:
  a tube makes all its points when made,
  and a shape cuts only its own thing, where a river digs the ground.
- Test only the sites of a row a shape can hold:
  a shape tests every site of its box,
  mostly empty for a slanting capsule,
  and a tube's against each piece of the level drawn
  (bent, the forest's branches cost a few percent more at level 0).
  A span per row from the pieces' boxes would keep the test exact.
- More trees as the oak (`Bole` and `Crown` in `worlds/src/forest.rs`):
  the birch's trunk round, bent where its boxes step, its branches forking;
  the spruce's trunk round.
- Solve what a coarse cell decides (a block's lots, a room's furniture)
  with a small search whose boundaries the coarser band fixes:
  constraint solving stays local and needs no backtracking across cells.
  Lab's WFC sampler did so on squares (`lab/src/synth.rs` at 114c077),
  in rounds: corners from their four parents, then edges, then insides.
  On the FCC lattice, a fine site in six coarse cells, two or one
  would be a corner, an edge or an inside.
