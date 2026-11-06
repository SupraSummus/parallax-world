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
  A lost GPU device (a hang the driver resets, or memory running out)
  ends it with a panic in egui-wgpu, which makes its device once:
  to recover in the same window, it would need a winit loop of its own.
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
  Not tried: the Rust tests beside vitest
  (whose 20 s timeout may then be too short),
  Rust's builds cached in CI,
  and one Cargo target for `bake/`, `worldgen/` and `worlds/`,
  each of which now builds the ones before it again.
- **`coarsen` serves only tests.**
  Every world is generated at each level;
  only tests' worlds of voxels (`World.ofVoxels`) are coarsened, in TS.
  Tests on small generated worlds would let it go.
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
  (the city at 1280×660 with a 2.4° leak did, before `GRID_REACH`).
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
- **The reference viewer freezes for over a minute on the forest**:
  it generates all its levels on the main thread,
  then holds about 25 million faces, over 2 GB.
  Generating in the bake workers would unfreeze it.
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
- **Landmarks and the moor's steadings keep off their regions' edges.**
  A landmark keeps its cell and all it shapes inside its region (`landmarks.rs`),
  so a strip along every region's edge never holds one,
  and a town's farmland there no farmstead or windmill
  (rows of regions are staggered, so the strips differ from row to row).
  So do the moor's steadings and pools (`moor.rs`), one to a region,
  and its regions are not staggered.
  Claims with hashed priorities between neighbouring regions
  ("Open problems" in `docs/generators.md`) would let one stand anywhere.
- **The highest massifs end in broad rounded domes of snow.**
  Behind the crest the high country is one smooth noise,
  and wherever it stands over the snowline, all of it is snow.
  It wants crags and cirques of its own, or less snow where it is broad.

## Ideas

- Report cost at points the worlds name (a town's square, a pass),
  not at the spawn and the regions' centres (`cost::places`).
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
- Drop lab's superseded experiments (WFC rule-sets, templates),
  their findings kept in `lab/src/lib.rs`: the critic now says which model wins.
- Join lab's cubic and FCC samplers with one lattice type
  (sites, neighbours, parents): each pair now repeats its box bookkeeping.

Composing generators (section "Composing" of `docs/generators.md`), untried:

- Bid per quantity: a land per channel (use, ground, wetness),
  each domain bidding in those it cares about, as the towns' terrace needs.
- Measure a thing's far forms for all levels from one drawing:
  `Far::measured` draws a thing whole again for each level and box,
  most of a coarse tile's cost by a town (`worlds/examples/cost.rs`).
- Place the houses as things of their own, from the plan of lots,
  not as children of one thing per town:
  a box in a town now draws the whole town's carves aside, in a buffer
  as large as the box, where each house's would do.
- Let the framework filter plans of numbers finer than the level's sites
  (`Plan::sampled` at the spacing), since plans declare their scale:
  coarse levels would skip fine bands' cost as they skip their things.
  Plans of categories are read dithered already ("Plans" in "Levels").
- Solve what a coarse cell decides (a block's lots, a room's furniture)
  with a small search whose boundaries the coarser band fixes:
  constraint solving stays local and needs no backtracking across cells.
