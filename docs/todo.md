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
