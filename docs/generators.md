# World generators

A generator (`worldgen/src/generator.rs`) describes a world;
the framework draws any box of it, alone, at any level of detail,
and gets the same voxels whichever boxes it drew before.
That lets a world be endless and generated in pieces around the camera,
coarser further away, for about the same cost per box at every level.
The forest (`worldgen/src/forest.rs`) is written this way,
and so is the valley (`valley.rs`), the forest with towns and fields,
three domains composed as in "Composing";
the web generates both as WebAssembly (`src/lib/worldgen.ts`);
the TS city and the toy worlds (`src/lib`) are not on it.
Integers and fixed point (`noise.rs`) decide wherever they can,
so a world is the same on every machine and as fine far from the origin.

This is the second version of the contract (see "Open problems").

## Model

A world is **layers** of two kinds, drawn in `order`:

- **Fields:** something everywhere, from position alone: the ground, caves, fog.
  A field gives a kind (or a medium) at each site (`At`),
  or `Spans` along each line of sites parallel to an `axis` (heightfields).
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
  A cell gives named random sequences (`cell.random("species")`)
  that depend on the cell and the name only.

A thing is a **node**: its **shapes**, and **children**,
finer things it makes only for the area asked for (`Children::Reaching`).
Shapes are analytic: `ball`, `ellipsoid`, `capsule`, `cuboid`, `lathe`
(a solid of revolution, e.g. a crown).
Each `fill`s empty sites, `paint`s over them, or `carve`s them empty,
and may keep each site only with a chance (`fill` < 1: foliage, fog).
Authors never write sites; the framework draws shapes and fields at the level asked.
A thing is drawn whole, then over the world:
its carves cut only it, never the ground or other things
(dig the ground with a field),
and each of its sites meets the world as it was last drawn, filling or painting.

Things are built in **frames** (`frame.rs`):
a `Part` adds, paints and cuts boxes in coordinates of its own, u, v and w,
placed in its parent's by quarter turns about v, mirrors and an offset,
so boxes stay boxes and a part's sites land on its parent's.
`walls` gives a footprint's four walls as frames, u along each and w into it,
so one window serves every wall of a house, a nave or a tower.

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
  which drawn one by one would stop rays twice where they overlap;
  a cluster's ball for its sprays of leaves on twigs, a site thick at level 0.
  Or the far form is the near form measured (`Far::measured`):
  drawn whole at the coarsest level it is drawn at, then coarsened,
  each coarse site kept with the chance a ray crossing its cell is stopped
  (the share of its columns holding a site, on average over the axes)
  and of a kind in proportion to what shows on its sides.
  The chances are held to one threshold for the whole thing:
  with one per site a house's outline came out ragged, a cloud of its cells;
  so a porous thing is kept or not whole, right only over many of them.
  That is exact where an outline is a guess, at a cost:
  each box draws the things it reaches whole.
  Give such parts' children `bounds` and make their shapes only once read
  (`Shapes::lazy`): coarse boxes hold many things drawn only from afar.
  Parts a thing hides, like branches inside a crown, are children with a small `size`:
  drawn on their own, far away they would show.
- **Populations** make things while sites are at most `size` apart,
  so a box visits a bounded number of cells at any level;
  beyond, they draw their `far` field instead:
  a medium as dense as their things, by the theory of Boolean models
  (extinction = things per volume · their mean shadow).
  `Shadows` measures it from sample things drawn alone at a level:
  what each slab of them across an axis hides, seen along each axis.
- **Fields** are evaluated once per site or line of sites,
  and get the spacing, to smooth themselves to it.

Build solids as one shape, not stacks of slices:
a stack of discs a unit thick, each grown to a site, draws its outline, not its shape.

## Rules

Broken rules panic, naming the thing and pointing here:

- **Populations.** A thing reaches at most `reach` past its cell's reach,
  N from its centre (one number or per axis);
  only things that get made are checked, so derive `reach` from the largest possible.
- **Things.** A node with children gives `bounds` holding them;
  a child stays inside its parent's bounds;
  siblings have unique names;
  a measured far form starts at a `detail` of a unit or more.
- **Plans.** A plan reads only plans at least as coarse (see "Bands").
- **Kinds.** A world has at most 255 materials: ids are bytes.

Not checked, but caught by the tests:
draw from position, seeds and named random sequences only
(no shared `Rng`, no global state),
and fields from position alone.

## Checks

`worldgen/tests/contract.rs` runs, for each world in its list
(the forest, the valley, and test worlds of fog, caves,
and a village of houses with doors and tables):

- a box against eight parts generated alone, in reverse order,
  at levels 0, 2 and one past the last checked below:
  any seam, order dependence or hidden state shows up as a differing site;
- at each level up to the world's, rays from empty sites in random directions,
  against the level two finer:
  of the rays that get 2 sites far, the share that gets 4 more,
  and the materials they stop on, must agree within 0.1.
  Sites a level rougher and sheets a site thicker cost up to ~0.06;
  losing content, or a wrong fill law, 0.14 and more.

With `--nocapture` it prints each level's margins.
It also checks that a thing's carves cut only the thing.
Add a new world to that list with a region that has a bit of everything in it.
To see a world's layout without the renderer,
`worldgen/examples/map.rs` draws it from above, any level, any area.

## Composing

A world as large as the valley is **domains**
(the forest, `town.rs`, `fields.rs`):
modules of plans and layers that never call each other.
They meet only through plans coarser than their own,
through the land they bid for, and through things they share.
Each part below is an experiment in how far that goes.

### Bands

A **plan** (`plan.rs`) is a decision for any column (x, z),
made on demand and kept: the path's centre line, the ground, where towns are.
Its **scale** is the size of the smallest features it decides,
and it reads only plans at least as coarse; reading a finer one panics.
Decisions then flow from coarse to fine, like bands of a frequency pyramid:
one at a place depends on a neighbourhood no wider than the bands it reads,
and a coarse decision never wobbles with a fine one.
The rule shapes designs.
A town can't sit on the path's bends, which are finer than a town:
it sits on the path's mean, and the path, a finer band, bends to its main street.
Its height is the mean of the valley floor under it, not one sample of its bumps.
Sampling is the exception (`Plan::sampled`):
it is the filter that makes a coarse band of fine ones,
so a sampled plan may read finer ones (the fields' slope, every 2 m).
Plans are built in order from those before them, so they can't form cycles;
layers read any plan.

### Land

Domains share land by **bidding** for each column (`Land`; bids are plans).
A domain's share eases from none to all
as its bid goes from `width` behind the best of the rest to `width` ahead,
and the domain with the most owns the column.
What the domains propose blends by their shares:
the forest's chances of trees, rocks and plants are times its share,
so it thins towards the fields by itself,
and the towns and fields draw only the columns they own.
The forest bids nothing: it has what no one outbids it for.
A domain added later is seen by the others only as a smaller share:
adding the fields changed neither the forest nor the towns.
Quantities want blends of their own, though.
A town levels the ground 120 m past its streets,
far beyond the metres in which it gives way to fields,
so the ground blends by the town's own terrace weight, not by shares.

### Far forms keep statistics

What is drawn far away keeps what is near on average, not its outline:
the towns' things and the forest's canopy are measured ("Levels"),
patterns finer than the sites draw their parts in proportion (`stripe`),
and media too thin for the sites are thickened, as dense along a ray
(`medium` in `fields.rs`).

### Kinds and things

A domain names its kinds with `kinds!`,
after those of the domains it builds on:
the forest's come first in any world it grows in,
so its things are a library others draw from
(the towns' trees are the forest's `make_tree`).

## Open problems

- **The forest's coarse levels stop ~0.05 more rays than two finer**
  (levels 6 and 7, sites 8 and 16 m apart).
  Measuring the canopy from sample trees drawn at level 2,
  instead of from their shapes' areas, brought level 4 from 0.024 off to 0.007,
  but left levels 6 and 7 about where they were (0.05 and 0.06):
  what is left is likely not the canopy's density but its band,
  26 m tall, two or three sites there. Not checked yet.
- **Media are isotropic.** A site stops rays from every direction alike;
  vertical trunks seen from above and from the side cannot both be right.
  Measured far forms show it too: a town far away shows its walls from above
  as much as its roofs, since a site's kind is what shows on all its sides.
- **Things of one band can't see each other.** Bands order what decides where,
  but two populations of one scale ("no tree on a boulder") still can't
  exclude each other. Claims with hashed priorities would be local and
  order-free: a thing is kept unless a neighbour's claim of higher priority
  overlaps it (one round of Luby's algorithm), from anchors alone.
