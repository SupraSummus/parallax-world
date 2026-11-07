# To do

Known problems, measured or derived, then open ideas.
Numbers are for the forest at 1280×800, as measured when written.

- **worldgen still decides some things with floats.**
  Shapes are kept and tested in fixed point,
  but they are given in floats, and their areas, fills and lathe radii are f64,
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
- **The native frontend is missing pieces.**
  Its renderer has no `matchTilings`:
  where slices re-tile as the camera moves along an axis,
  the new ones show nothing until baked.
  Its camera is f64, not fixed point (positions reach the GPU relative to it).
  Its compositor needs indirect draws counted on the GPU
  (`MULTI_DRAW_INDIRECT_COUNT`), and so Vulkan:
  Metal and GL lack them, and on DX12 wgpu's leave out the first vertex
  the chunk's slot comes as (`BACKENDS`).
  That its culling hides nothing that shows is checked only by hand
  (shots with `hidden` off match pixel for pixel).
  Its frames are bound by the GPU (`--stroll`):
  culling the clusters in view costs about a third of a frame,
  drawing those that show most of the rest.
  The first phase reads every cluster only to find those that showed;
  a list of them, kept from the last frame, would spare that.
  Walking in the window for minutes, a frame sometimes takes seconds
  until the driver resets the GPU (seen on Intel UHD with Mesa);
  the window then makes its device again (`window.rs`).
  Why is open: the same views and paths, drawn headless, take milliseconds.
  `native/` is not built in CI.
- **TS and Rust still say much the same twice.**
  The tiles around the camera (`world.ts`) are `native/src/world.rs`,
  slice selection (`bake/src/slicing.rs`) is `slice-selection.ts`,
  and faces and the renderer are in `native/src` too;
  `LayerIndex.range` (`voxel-grid.ts`) is `bake/src/layer_index.rs`'s.
  The renderer is the large step.
- **Tests take most of CI's time.**
  Most of it is the worlds' contract, bound by the CPU:
  pictures from above generate level 0 over all of a world's heights.
  CI runs the valley's in a job of its own;
  it and the rest of the Rust tests are CI's longest jobs.
  If CI is still too slow, the contract at seeds but `REGIONS_SEED`
  (`worlds/tests/contract.rs`) could run only on main.
  Caching Rust's builds would spare little: `libm` is their one dependency.
  Not tried: one Cargo target for `bake/`, `worldgen/` and `worlds/`,
  each of which now builds the ones before it again.
- **Slicing keeps a leak bound slices need less.**
  Drawn in relief, slices of the bound leak nothing,
  and much thicker ones only where a column has a gap
  (step 8 of `src/lib/leak-bound.test.ts`),
  but they are still as thin as drawn flat they would need to be
  (section 2 of `docs/architecture.md`),
  and "Max Occlusion Leak" sets both their thickness
  and their level of detail (atoms of 2^(k−s) at level k,
  s as large as the grids' reach allows).
  Thicker is faster but coarser: in the forest, a first render takes
  about 6.5 s at detail 9.5 (6°), 3 s at 2.4 and 2.7 s at 1
  (voxels 4 and 10 times as large), against 2.5 s drawn flat at 9.5
  (Node, 1280×800).
  Thickness could follow what culling and the slice count want,
  and the level of detail the screen's pixels;
  the bound's derivation would go.
- **Coverage is weaker in relief.**
  A splat moves on its centre plane with the camera,
  so a chunk's mask holds what it covers from the axis and the corners
  of the rhombus of rays, and what it may reach from any:
  in the forest at 6°, twice the chunks of flat slices are drawn.
  Nor is that exact: a block covered from each of those may still open
  between them, which no picture has shown yet.
  The camera's slant is known where coverage is tested;
  masks for a few slants, picked per block, would cover nearly as much
  as flat ones, and between nearer slants.
- **Coverage masks are coarser than the screen.**
  `mask_scale` marks ⌊40 / t⌋ pixels per unit of a slice t thick,
  but at a level's nearest slices a unit is focal / (t · detail · LAYER)
  pixels on screen, about 95 / t at 90° on 1280 px with the default leak.
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
  Natively, the painting order is the draw order at one depth per layer,
  which rounding breaks: the same misordered about 6 % of the pixels.
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
  mostly testing sub-blocks one by one in `cover` (`bake/src/coverage.rs`):
  about 270,000 tests a frame, three in four failing,
  as the forest covers the screen in fine pieces.
  Testing quarters of a cell first would not help (two in three are mixed),
  nor would clipping without allocating in `shape`.
  Natively coverage runs on a thread of its own (`native/src/occlusion.rs`);
  the web could run it in a worker, a frame behind.
- **The native compositor's buffers stop at the device's limit.**
  `Arena::grow` caps a buffer at what the device allows
  (128 MB of splats on lavapipe), and `allocate` then hands out room past it:
  wgpu panics in `Compositor::prepare`
  (the city at 1280×660 with a 2.4° leak did, before `GRID_REACH`;
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
  (the WebAssembly of `worlds/`, in Node; the TS forest took twice that),
  most at levels 0 to 4 (a moved grid, a seventh of that).
  In the TS forest it was mostly the ground's noise,
  which the terrain and the canopy compute about 14 times per column
  (with its neighbours, for slopes),
  and at levels 0 and 1 the undergrowth's many small shapes;
  worldgen's, counted ("Cost" in `docs/generators.md`),
  goes mostly to the trees at levels 0 to 2
  and to measuring their shadows from level 4.
- **The learned town's rules are from the old towns.**
  `worlds/data/town-grammar.bin` was learned when the valley's towns
  were all market towns alike, so lab's `export -- --check` says it differs.
  Learned again from today's, mostly hamlets and villages,
  it draws far less town: 235 houses in `learned.rs`'s test, not over 600.
  Learn it from the market towns alone, or let the learned town be sparser.
- **The forest on steep slopes breaks the contract at level 4.**
  A region on a wooded mountainside of the valley,
  where the trees thin out to the treeline,
  came 0.10 to 0.15 apart at level 4, in materials hit and from above:
  leaves in place of trunks and of the slope's scree and rock.
- **The highest massifs end in broad rounded domes of snow.**
  Behind the crest the high country is one smooth noise,
  and wherever it stands over the snowline, all of it is snow.
  It wants crags and cirques of its own, or less snow where it is broad.
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
- **Measured houses show their walls from above at level 6**,
  which the contract does not check.
  Around the valley's first town's centre (seed 42),
  level 6 shows 46 % houses from above against 25 % at level 0,
  and 16 % roofs against 24 %: the rest is walls, timber and stone
  (0.31 off; 0.18 measured from level 1, 0.22 from level 3).
  Measured from level 4, a house is a few fine sites
  ("Levels" in `docs/generators.md`):
  its eaves, thinner than one, leave its walls' tops to show,
  and its gable ends (`gable_ends` in `town.rs`), grown to one,
  paint its roof's ends.
  And a cell's share counts rays the cells behind it stop too,
  so a thing keeps more cells than it stops rays.
- **A field reads every column of a tile, even where it draws nothing:**
  the railway's two, away from its line.
  A field could say which boxes it may draw in, and be skipped elsewhere.
- **Cost weighs every plan lookup alike** (`NS` in `worldgen/src/cost.rs`),
  though one of a column just read costs a fraction
  of one among claims making their keys.
- **The places check finds few things of sparse populations**
  (`worldgen/src/places.rs`):
  it asks COLUMNS cells at most,
  and finds a claimed thing only from those within its reach.
  The valley's landmarks: 118 at eight seeds (not 686 as one per region),
  under ENOUGH a seed, so no seed is tested alone;
  its towns 57, its haystacks 2.
- **The railway's line turns at points, and its bridges and tunnels can be short.**
  A curve's rule reads only a point's two neighbours,
  so a wander bounds the turn at the point it puts, not between points:
  seen from above, the line is straight between them and turns at each,
  up to 2.5°.
  A rule reading the points beyond the neighbours too
  (a four-point scheme) would make it smooth by construction.
  What carries the track is decided point by point,
  so where the ground just crosses a threshold
  a bridge or a tunnel can be one segment long (8 m).
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
- **Light is a prototype, in look's pictures and native's `--light`**
  (`worldgen/src/light.rs`); the web does not light its tiles.
  Light comes along the 12 neighbours' directions,
  so light from low over the horizon is too weak or too strong.
  Native's bakers each light what they show with a `Light` of their own
  (a generator is not `Send`):
  they share tiles (`Grids`) with the world's threads,
  but each finds the coarse levels' light again
  and makes a generator, whose plans' caches take tens of MB.
  Lit, the first chunks wait for the coarsest levels' tiles,
  which the renderer does not read.
  Grids hold materials only, but for glow:
  the generator shares each glowing shape's among the sites around it
  (`glow_shares`), which is light's business;
  handing over the shapes alone, light would share it,
  reading the tiles around a site for the shapes near it.
  The sun is one of the 12 directions.
  `light::around` reads the nearest site and its 12 neighbours,
  so near a corner where six voxels meet it misses one within its reach,
  and glow and light jump a little there.

## Ideas

- Report cost at the places worlds name (`World::points`),
  not at the spawn and the regions' centres (`cost::places`),
  once the forest, the moor and the city name some:
  they would be measured at their spawn alone.
- Drop y as up from hosts and checks.
  A world is bounded along y by its `heights`,
  hosts tile x and z over them,
  and the checks look from above down y;
  an endless holey cheese wants tiles along y too, and no heights.
- Lay out towns at any angle?
  Walls along the lattice's axes look crisp on FCC;
  at other angles they jag.
  But every town and the city runs north–south and east–west,
  which at the scale of a layout reads as a grid.
  A middle way: streets and plots at any angle, the buildings on the axes.
  The buildings' walls would still show to `directions.rs`.
- Keep the fit of `cost::NS` in the repo, to refit it on a quiet machine.
- Generate the web's tiles as the native spike does:
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
- Trunks as tubes, bent where the birch's boxes step:
  they want a radius changing along the tube, as a lathe's,
  and bark marks on round faces, not on boxes' faces.
- Solve what a coarse cell decides (a block's lots, a room's furniture)
  with a small search whose boundaries the coarser band fixes:
  constraint solving stays local and needs no backtracking across cells.
  Lab's WFC sampler did so on squares (`lab/src/synth.rs` at 114c077),
  in rounds: corners from their four parents, then edges, then insides.
  On the FCC lattice, a fine site in six coarse cells, two or one
  would be a corner, an edge or an inside.
