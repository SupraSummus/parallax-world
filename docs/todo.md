# To do

Known problems, measured or derived, then open ideas.
Numbers are for the forest at 1280×800, as measured when written.

- **worldgen still decides some things with floats.**
  Shapes, their containment tests, areas and fills are f64,
  and so is the forest's geometry (sin, cos, pow, exp, atan2, hypot, from `js.rs`).
  They should go to fixed point like the noise,
  the forest building each thing relative to its anchor.
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
- **Every splat in view is drawn every frame.**
  The forest has millions in view;
  headless Chromium's software WebGL takes over a second per frame,
  and real GPUs are not measured yet.
  Splats a chunk's nearer ones hide from the whole eye's cell
  need not be in it.
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
- **One browser check for agents.**
  Agent sessions spend about a third of their command time
  in one-off Playwright scripts:
  each rebuilds, starts a server
  and waits fixed times for the scene to settle.
  A committed script that waits until nothing is left to bake would do.
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
- **Generating the forest's grids takes 7–20 s of worker time per level**,
  most at levels 0 to 4 (a moved grid, a seventh of that):
  mostly the ground's noise, which the terrain and the canopy
  compute about 14 times per column (with its neighbours, for slopes);
  at levels 0 and 1 also the undergrowth's many small shapes.
- **The reference viewer freezes for over a minute on the forest**:
  it generates all its levels on the main thread,
  then holds about 25 million faces, over 2 GB.
  Generating in the bake workers would unfreeze it.

## Ideas

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
- Show move speed and render distance in metres:
  they are lattice units, and the forest's unit is 0.125 m.
- Show the forest to 22 km again: `MAX_LEVEL` 13, one grid more.
