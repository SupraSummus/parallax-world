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
- **Coarsening the city takes twice as long with faces.**
  `coarsen` measures the faces of every coarse voxel at the surface
  whose cell holds more than one material,
  and the city's walls and floors hold several almost everywhere:
  about 0.7 s more for its level 1, on the main thread, at first use.
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
  `native/` is not built in CI.
  Its `--shot` draws only a seed's spawn view, not the web's view query.
- **TS and Rust still say much the same twice.**
  The tiles around the camera (`world.ts`) are `native/src/world.rs`,
  slice selection (`bake/src/slicing.rs`) is `slice-selection.ts`,
  and faces and the renderer are in `native/src` too.
  The renderer is the large step.
- **Slicing keeps a leak bound slices no longer need.**
  Drawn in relief, slices leak only slivers however thick they are
  (step 8 of `src/lib/leak-bound.test.ts`),
  but they are still as thin as drawn flat they would need to be
  (section 2 of `docs/architecture.md`),
  and "Max Occlusion Leak" sets both their thickness
  and their level of detail (atoms of 2^(k−4) at level k).
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
- **The web draws every splat in view every frame.**
  The forest has millions in view, and only about 1.5 % show a pixel;
  headless Chromium's software WebGL takes over a second per frame,
  and WebGL on real GPUs is not measured yet.
  The native compositor culls clusters of them on the GPU
  (section 4 of `docs/architecture.md`), which WebGL 2 cannot
  (no compute, no indirect draws).
  Its painting order is the draw order at one depth per layer,
  which rounding breaks: natively, the same misordered about 6 % of the pixels.
- **Most splats drawn natively are hidden within their own chunk.**
  The native frame (about 12 ms on an Intel UHD, `--bench`)
  is mostly drawing the clusters that show,
  whose splats cover the screen several times over.
  Keeping only each column's nearest voxel keeps all the axis sees
  ("One Voxel per Column", section 3 of `docs/architecture.md`).
  In the forest that halved the splats,
  but changed about one pixel in eight,
  mostly towards a face's corners, where rays slant most:
  the ground's steps flatten, trunks show seams, crowns fray.
  Dropping only those hidden from the axis and the rhombus's corners
  (as masks are filled) may keep most of the saving; not measured yet.
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
- **Moved grids are posted whole.**
  When a grid moves with the camera (section 1 of `docs/architecture.md`),
  the main thread posts all of it to every worker, though they have most of it:
  at level 0, ~40 MB a worker every 12 m walked.
  Workers could keep the tiles and put the grid together themselves.
- **Far voxels 5.6–11 pixels across.**
  With `ATOM_SHIFT` 6 they were 1.4–2.7 pixels,
  but level 0's grid would have to reach 4 times as far (from 115 m on):
  ~16 times its 40 MB, in each bake worker.
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
  worldgen's is not profiled yet.
- **The reference viewer freezes for over a minute on the forest**:
  it generates all its levels on the main thread,
  then holds about 25 million faces, over 2 GB.
  Generating in the bake workers would unfreeze it.

## Ideas

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
- Describe the city in metres, like the forest, instead of boxes on its plan.
- Port the toy worlds to the forest's metric scale, or drop them.
- Show render distance in metres:
  they are lattice units, and the forest's unit is 0.125 m.
- Show the forest to 22 km again: `MAX_LEVEL` 13, one grid more.

Composing generators (section "Composing" of `docs/generators.md`), untried:

- Bid per quantity: a land per channel (use, ground, wetness),
  each domain bidding in those it cares about, as the towns' terrace needs.
- Measure a thing's far forms for all levels from one drawing:
  `Far::measured` draws a thing whole again for each level and box.
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
