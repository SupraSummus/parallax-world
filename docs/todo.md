# To do

Measured or derived, not yet done.
Numbers are for the forest at 1280×800 (`docs/architecture.md`).

- **Moving rebakes whole levels.**
  A level is baked again when the camera leaves its eye's cell,
  so walking rebakes 1.3–1.8× the chunks that one direction per chunk did,
  mostly of levels 0 and 1 (cells 1 and 2 units across).
  By the scaling of section 2, eyes on a lattice finer than the level's
  (N = 2^(k−1)) would cut both: detail ∝ √N,
  chunks in view ∝ detail³ and rebakes per unit moved ∝ chunks / N,
  so ∝ √N, with eyes between voxel sites.
- **Zooming in magnifies chunks.**
  Slices ignore the field of view, but so does baking:
  chunks are baked for 90° on about 1280 px (`render_scale`),
  and coarse voxels are sized for it (`ATOM_SHIFT`),
  so at 30° (3.7× zoom) far voxels are 5 to 10 px across and chunks upscaled.
  `render_scale` could follow the focal length,
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
