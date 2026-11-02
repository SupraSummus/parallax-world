# World generators

A generator (`src/lib/generator.ts`) describes a world;
the framework draws any box of it, alone, at any level of detail,
and gets the same voxels whichever boxes it drew before.
That lets a world be endless and generated in pieces around the camera,
coarser further away, for about the same cost per box at every level.
The forest (`src/lib/forest.ts`) is written this way;
the city and the toy worlds are not yet.

This is the second version of the contract (see "Open problems").

## Model

A world is **layers** of two kinds, drawn in `order`:

- **Fields:** something everywhere, from position alone: the ground, caves, fog.
  A field gives a kind (or a medium) `at` each site,
  or `spans` along each line of sites parallel to an `axis` (heightfields).
- **Populations:** things in cells, at most one per cell.
  A population's cells are those of the lattice N times coarser, N·FCC
  (as the renderer's, `cells.ts`, when N is a power of two):
  the rhombic dodecahedra of the points nearest to each of its sites,
  reaching N from their centre along each axis.
  The framework asks each cell for an `anchor`;
  the cell owns a thing only if its anchor is in it
  (a tree belongs to the cell its foot is in),
  then asks it to `make` the thing.
  An anchor picked evenly within the cell's reach
  (in a cube 2N wide, or on the ground under it)
  gives things as even and as dense as one per cube 2N wide would:
  every point is in one cell, which picks it as often as any.
  A cell gives named random sequences (`cell.random('species')`)
  that depend on the cell and the name only.

A thing is a **node**: its **shapes**, and **children**,
finer things it makes only for the area asked for (`children(clip)`).
Shapes are analytic: `ball`, `ellipsoid`, `capsule`, `box`, `lathe`
(a solid of revolution, e.g. a crown).
Each `fill`s empty sites, `paint`s over them, or `carve`s them empty,
and may keep each site only with a chance (`fill` < 1: foliage, fog).
Authors never write sites; the framework draws shapes and fields at the level asked.

No direction is special;
a world with a ground says so in its fields and anchors.

## Levels

At level k the sites are those of the lattice scaled by 2^k,
which is part of the lattice:
site (x, y, z) of the level is at 2^k · (x, y, z).
What has to stay the same across levels is what a camera sees:
how many rays get how far, and what they stop on.

- **Shapes** are drawn by how many of their sizes are under the sites' spacing s.
  None: the sites inside, each kept with the chance one of the s sites
  of level 0 it stands for along a ray is kept, 1 − (1 − fill)^s.
  One, a sheet such as a wall: grown to one site thick and kept opaque,
  since a ray crossing it crosses it once; it looks up to half a site thicker.
  Two or three, a rod or a speck: the sites near it, kept with a chance
  that stops as many rays as it does on average over directions
  (Cauchy: a convex body's mean shadow is a quarter of its area).
  Chances are compared with a hash of the position in lattice units,
  so a site kept at one level stays kept where coarser levels give it as high a chance.
- **Nodes** whose parts get smaller than the sites draw their `far` shapes instead
  (from `detail` on): a crown's outline for its leaf clusters,
  which drawn one by one would stop rays twice where they overlap.
  Parts a thing hides, like branches inside a crown, are children with a small `size`:
  drawn on their own, far away they would show.
- **Populations** make things while sites are at most `size` apart,
  so a box visits a bounded number of cells at any level;
  beyond, they draw their `far` field instead:
  a medium as dense as their things, by the theory of Boolean models
  (extinction = things per volume · their mean shadow).
  `areaProfile` measures their area along an axis from sample things.
- **Fields** are evaluated once per site or line of sites,
  and get the spacing, to smooth themselves to it.

Build solids as one shape, not stacks of slices:
a stack of discs a unit thick, each grown to a site, draws its outline, not its shape.

## Rules

Broken rules throw, naming the thing and pointing here:

- **Populations.** A thing reaches at most `reach` past its cell's reach,
  N from its centre (one number or per axis);
  only things that get made are checked, so derive `reach` from the largest possible.
- **Things.** A node with children gives `bounds` holding them;
  a child stays inside its parent's bounds;
  siblings have unique names.

Not checked, but caught by the tests:
draw from position, seeds and named random sequences only
(no shared `rng`, no module state),
and fields from position alone.

## Checks

`src/lib/generator.test.ts` runs, for each world in its list
(the forest, and test worlds of fog, caves, and a village of houses with doors and tables):

- a box against eight parts generated alone, in reverse order,
  at levels 0, 2 and one past the last checked below:
  any seam, order dependence or hidden state shows up as a differing site;
- at each level up to the world's, rays from empty sites in random directions,
  against the level two finer:
  of the rays that get 2 sites far, the share that gets 4 more,
  and the materials they stop on, must agree within 0.1.
  Sites a level rougher and sheets a site thicker cost up to ~0.06;
  losing content, or a wrong fill law, 0.14 and more.

Add a new world to that list with a region that has a bit of everything in it.

## Open problems

- **Far forms are calibrated from analytic areas.**
  The forest's canopy (its trees from level 3 on) stops 0.05–0.1 more rays
  at levels 4 to 6 (sites 4 to 16 m apart) than two levels finer:
  porous, uneven crowns stop fewer rays than their outlines' area says,
  while balls of sites stop ~1.5× more than a ball's area says.
  Far forms should be measured from sample things drawn at level 0:
  a site of level k stands for the 8^k sites of its cell (`cells.ts`),
  so counting them is exact where an area is an estimate.
- **Media are isotropic.** A site stops rays from every direction alike;
  vertical trunks seen from above and from the side cannot both be right.
- **Layers can't see each other.** "No tree on a boulder"
  needs a layer to ask another for its things in an area.
- **The renderer only reads level 0.**
