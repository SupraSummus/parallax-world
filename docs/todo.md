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
  `render_scale` bakes 40 / t pixels per unit of a slice t thick,
  but a unit is up to focal / (t · detail · LAYER) pixels on screen,
  about 95 / t at 90° on 1280 px with the default leak,
  so chunks are upscaled up to about 2.4×.
  Slices ignore the field of view, and so does baking:
  zoomed in, chunks are upscaled more,
  and coarse voxels, sized for 90° (`ATOM_SHIFT`), grow on screen.
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
  Turning, `coverage_cover` and `coverage_face` take most of a render.
- **One browser check for agents.**
  Agent sessions spend about a third of their command time
  in one-off Playwright scripts:
  each rebuilds, starts a server
  and waits fixed times for the scene to settle.
  A committed script that waits until nothing is left to bake would do.
- **A world larger than its finest grid.**
  Far slices read coarse grids (section 1 of `docs/architecture.md`),
  but every grid still covers the same box, so the world ends at the
  render distance, at most 200 m (`MAX_RENDER_DISTANCE`):
  today the coarser grids cost about a second of `coarsen` at load
  and show nowhere.
  Level 0's grid need only reach where level 7 begins (about 230 m in the
  forest), each coarser grid twice as far,
  so seven grids of about the same size could show 15 km.
  The forest can generate any box at any level (`generators.md`);
  at levels 4 and 5, a 2 km square takes about a minute (trees one by
  one, each smaller than a voxel), at level 6 under two seconds.
  Generating on demand, in a worker, would also free the main thread at start.

## Ideas

- Measure the seams between faces against ray-cast perspective.
- Hold less of the world: every worker has its own copy;
  a shared buffer or only the voxels with an open face would do.
- Make the 12 stacks cheaper:
  four or more faces are in view at once, each with its own near slices.
- Find covered chunks more exactly: many chunks drawn show no pixel.
- Build `LayerIndex` off the main thread or in Rust.
- Measure the WebGL compositor on real GPUs (headless Chromium only has a
  software one); if draw calls matter, pack chunks into a texture atlas.
- Paint cut cross-sections with the colour of the surface above them: a 2D
  prototype allowed slices twice as thick at the same leak.
- Fill gaps in screen space (only pixels no layer covers) instead of
  growing faces.
- Describe the city in metres, like the forest, instead of boxes on its plan.
- Port the toy worlds to the forest's metric scale, or drop them.
