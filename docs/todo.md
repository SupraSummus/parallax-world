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
  Turning, a frame costs its main thread over 10 ms in the forest,
  and walking (`--stroll`) about 15 ms, 40 ms at the 99th percentile:
  most of a frame then, the GPU and uploads far less.
  `native/` is not built in CI.
  Its `--shot` draws only a seed's spawn view, not the web's view query.
- **TS and Rust still say much the same twice.**
  The tiles around the camera (`world.ts`) are `native/src/world.rs`,
  and slice selection, faces and the renderer are in `native/src` too;
  `native/src/slicing.rs` restates parts of `bake/src/slicing.rs`.
  The renderer is the large step.
- **Moving rebakes whole levels.**
  A level is baked again when the camera leaves its eye's cell,
  so walking rebakes 1.3–1.8× the chunks that one direction per chunk did,
  mostly of levels 0 and 1 (cells 1 and 2 units across).
  By the scaling of section 2, eyes on a lattice finer than the level's
  (N = 2^(k−1)) would cut both: detail ∝ √N,
  chunks in view ∝ detail³ and rebakes per unit moved ∝ chunks / N,
  so ∝ √N, with eyes between voxel sites.
- **Coverage masks are coarser than the screen.**
  `mask_scale` marks ⌊40 / t⌋ pixels per unit of a slice t thick,
  but at a level's nearest slices a unit is focal / (t · detail · LAYER)
  pixels on screen, about 95 / t at 90° on 1280 px with the default leak.
  A mask pixel counts as drawn when its centre is in a splat,
  so a gap between splats narrower than a mask pixel can count as covered,
  and what is behind it is skipped.
  The mask could follow the focal length and detail,
  keyed in the chunk like `fill`, leaving the slices as they are.
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
  The native frame (about 9 ms on an Intel UHD, `--bench`)
  is mostly drawing the clusters that show,
  whose splats cover the screen several times over.
  About half of all splats are hidden by nearer ones of their chunk,
  the same from every camera (splats lie where the eye puts them),
  so the baker could drop them.
- **Bricks on the edge of a face's rhombus are baked whole.**
  Near where a level starts, about 19 bricks meet the rhombus,
  and most of them cross its edge or the screen's.
- **Most baked splats are hidden.**
  When chunks were bitmaps,
  over 95 % of opaque baked pixels were covered by nearer slices.
  The eye's cell bounds where the camera can see from,
  so content hidden from the whole cell need not be baked.
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
- Project splats from the eye in the vertex shader instead of in the baker:
  a chunk's splats would no longer depend on the eye, only its mask would,
  so moving would rebake masks, not splats.
- Paint cut cross-sections with the colour of the surface above them: a 2D
  prototype allowed slices twice as thick at the same leak.
- Fill gaps in screen space (only pixels no layer covers) instead of
  growing faces.
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
- Let the framework sample plans finer than the level's sites
  (`Plan::sampled` at the spacing), since plans declare their scale:
  coarse levels would skip fine bands' cost as they skip their things.
- Solve what a coarse cell decides (a block's lots, a room's furniture)
  with a small search whose boundaries the coarser band fixes:
  constraint solving stays local and needs no backtracking across cells.
