# To do

Known problems, measured or derived, then open ideas.
Numbers are for the forest at 1280×800, as measured when written.

- **Moving rebakes whole levels.**
  A level is baked again when the camera leaves its eye's cell,
  so walking rebakes 1.3–1.8× the chunks that one direction per chunk did,
  mostly of levels 0 and 1 (cells 1 and 2 units across).
  By the scaling of section 2, eyes on a lattice finer than the level's
  (N = 2^(k−1)) would cut both: detail ∝ √N,
  chunks in view ∝ detail³ and rebakes per unit moved ∝ chunks / N,
  so ∝ √N, with eyes between voxel sites.
- **Chunks are magnified on screen.**
  `render_scale` bakes ⌊40 / t⌋ pixels per unit of a slice t thick,
  but at a level's nearest slices a unit is focal / (t · detail · LAYER)
  pixels on screen, about 95 / t at 90° on 1280 px with the default leak,
  so chunks are upscaled up to about 3×
  (more where `RENDER_SCALE` caps them, half that at a level's far end).
  Slices ignore the field of view, and so does baking:
  zoomed in, chunks are upscaled more.
  `render_scale` could follow the focal length and detail,
  keyed in the chunk like `fill`, leaving the slices as they are.
- **Bricks on the edge of a face's rhombus are baked whole.**
  Near where a level starts, about 19 bricks meet the rhombus,
  and most of them cross its edge or the screen's.
- **Most baked pixels are hidden.**
  Over 95 % of opaque baked pixels are covered by nearer slices.
  The eye's cell bounds where the camera can see from,
  so content hidden from the whole cell need not be baked.
- **Flying, the main thread can't keep up with the workers.**
  Flying up (a trace in a desktop browser),
  workers bake about 4,000 chunks a second
  and the main thread is busy 97 % of the time, at 10–13 frames a second.
  Renders take 30–90 ms, longer the more new chunks they take:
  uploading textures, handing chunks to `Coverage` (`coverage_add`),
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
- **Detail around the camera, not the spawn point.**
  The forest's grids are generated once, around the spawn point:
  level 0's reaches 96 m from it, each coarser twice as far
  (section 1 of `docs/architecture.md`).
  Level 0 must hold the world 88 m to a face's sides, 8 m short of its grid:
  a little further from the spawn point,
  the nearest slices find nothing at the edges of the view;
  100 m away, nothing near at all.
  A grid could be generated again around the camera
  once it leaves the middle of the grid's box,
  and the chunks of the levels reading it dropped:
  level 0's takes ~6 s of one worker, in 8 parts.
- **Far voxels 5.6–11 pixels across.**
  With `ATOM_SHIFT` 6 they were 1.4–2.7 pixels,
  but level 0's grid would have to reach 4 times as far (from 230 m on):
  ~16 times its 22 MB, in each bake worker.
  Grids shared between the workers would allow it,
  or bricks sent to a worker only once a chunk it bakes needs them.
  (Shared memory needs cross-origin isolation,
  which GitHub Pages can only get through a service worker.)
- **Generating the forest's grids takes ~44 s of worker time**,
  ~30 s until all nine are in with 3 workers,
  4 to 7 s per level whatever its sites' spacing:
  mostly the ground's noise, which the terrain and the canopy
  compute about 14 times per column (with its neighbours, for slopes).

## Ideas

- Measure the seams between faces against ray-cast perspective.
- Make the 12 stacks cheaper:
  four or more faces are in view at once, each with its own near slices.
- Find covered chunks more exactly: many chunks drawn show no pixel.
- Measure the WebGL compositor on real GPUs (headless Chromium only has a
  software one); if draw calls matter, pack chunks into a texture atlas.
- Paint cut cross-sections with the colour of the surface above them: a 2D
  prototype allowed slices twice as thick at the same leak.
- Fill gaps in screen space (only pixels no layer covers) instead of
  growing faces.
- Describe the city in metres, like the forest, instead of boxes on its plan.
- Port the toy worlds to the forest's metric scale, or drop them.
