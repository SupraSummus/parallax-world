# World generators

A generator (`worldgen/src/generator.rs`) describes a world;
the framework draws any box of it, alone, at any level of detail,
and gets the same voxels whichever boxes it drew before.
That lets a world be endless and generated in pieces around the camera,
coarser further away, for about the same cost per box at every level.
`worldgen/` is that engine, and `worlds/` the worlds on it,
which use only the engine's public API.
The forest (`worlds/src/forest.rs`) is written this way,
and so is the valley (`valley.rs`), the forest with towns and fields,
three domains composed as in "Composing";
a host starts either by name from `WORLDS` (`worlds/src/lib.rs`),
as a `World`: its generator, spawn, reach and heights;
the web lists and generates them all as WebAssembly (`src/lib/worldgen.ts`);
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
  each coarse site kept with the chance a ray crossing its cell is stopped:
  the share of its columns holding a fine site of the thing,
  on average over the axes,
  of those holding a fine site at all.
  Fine sites are two apart along a column,
  so a cell holds fewer columns than its shadow, and unevenly by axis;
  counted against its shadow, a solid cell came out 0.83 at sites twice apart,
  and one house in six vanished.
  A coarse site is of a kind in proportion to what rays along columns see:
  each column's first fine site from either end
  counts for the first kept cell the column crosses from that end.
  From the ends of every column's run in the cell instead,
  a town seen from above showed walls hidden inside its houses.
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
  `Shadows` measures it from sample things drawn alone at a level,
  slab by slab across an axis, each slab a medium of its own:
  seen across, what the slab adds to the shadow of the rest of the thing,
  from either side, as a tree's slabs overlap
  (the sum of 2 m slabs' own shadows from above was 2 to 3 times a tree's,
  and the canopy from above 87 % at level 4 where level 0 shows 79 %);
  seen along the other axes, its own shadow, as slabs side by side don't.
  The two are weighed as in a convex body's mean shadow over directions,
  half of what is seen across and ⅔ of what is seen along
  (π/4 for a cylinder, ½ for a ball):
  a medium is isotropic, and denser it hides more from above than the trees,
  thinner it lets rays among them hit the ground ("Media are isotropic").
- **Fields** are evaluated once per site or line of sites,
  and get the spacing, to smooth themselves to it.
- **Plans** they read hold numbers, read where the line is,
  or categories, which can't be averaged (`Plan::dithered`:
  what a town's column is, who owns the land).
  Where the sites are further apart than its scale,
  a line of sites reads a plan of categories at one point
  of the s × s columns it stands for,
  the same point for every such plan,
  so each category shows as often as it covers, its edges dithered.
  Along a row of lines the points are a Weyl sequence from a hashed phase,
  so a street along the row is read by as many lines as it covers, give or take one,
  not by chance.
  Read where each line is, the town's streets aliased:
  its fields from above were 0.03 off at level 6, under 0.015 dithered.
  Numbers aren't dithered: heights would roughen by their slope times the spacing.
  Things read plans where they ask,
  so a thing at a dithered edge may stand a site off what the fields show.
- **Faces.** Boxes and spans keep the sites in [min, max) along each axis.
  On a plane of a level's sites (y = 0 is one at every level)
  half the lines across a face have a site on it,
  all kept at a bottom face and dropped at a top one,
  so a ground and the houses on it would sink half a site, and more each level.
  A site on a face counts as above it with the chance ½ + 1/(2s)
  by a hash of its position and the axis (`Tie`):
  always at level 0, about half the time far away,
  so on average a face is where level 0 has it.
  Boxes and spans meeting on a face share its sites;
  a box one site thick keeps its bottom face whole, as at level 0,
  as halves of two faces would let rays through.
  Faces off those planes still round to the sites, up to half a site either way:
  even over many heights, not for one flat thing.

Build solids as one shape, not stacks of slices:
a stack of discs a unit thick, each grown to a site, draws its outline, not its shape.
And give a sheet over something else, a roof over its gable,
the depth of two sites of the levels it is measured at:
columns of sites step over a thinner one to what is under it.

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

`worldgen::contract` (`worldgen/src/contract.rs`) checks a world in a region:
`worlds/tests/contract.rs` runs it for each world in `WORLDS`
at seed 42 and three more, a thread each,
and `worldgen/tests/contract.rs` for test worlds of fog, caves,
and a village of houses with doors and tables
on y = 0, a plane of sites at every level ("Faces" in "Levels").
An entry's regions are chosen at seed 42 (`REGIONS_SEED`);
at other seeds each moves up or down with the world's surface over it
(`follow_surface`: the median height of what is seen from above),
as the ground lies higher or lower, while x and z stay.
A world whose regions only hold what they should at some seeds
names them in `worlds!` (`moor = [42]`).

- a box against eight parts generated alone, in reverse order,
  at levels 0, 2 and one past the last checked below:
  any seam, order dependence or hidden state shows up as a differing site;
- at each level up to the world's, 20,000 rays in random directions
  from where a camera could be, against the level two finer:
  of the rays that get 2 sites far, the share that gets 4 more,
  and of those that hit something, the materials they hit,
  must agree within 0.1.
  A camera could be at the empty sites of the finer level
  it could fly to from above,
  through empty sites from the top of the box:
  under a canopy (in the forest, all but 0.3 %), not under the ground's shell.
  Sites a level rougher and sheets a site thicker cost up to ~0.08;
  a town losing its roofs, 0.12;
- at each level up to the entry's `from_above`,
  what rays straight down hit first,
  over the region grown to at least 16 sites of the level across,
  against level 0, within 0.1:
  levels two apart miss what drifts a little at every level,
  like a town's walls taking over from its roofs (0.17 off at level 2).
  The valley, the moor and the learned town stop at 3:
  at level 4 a house's edges show their walls from above
  (see "Media are isotropic").
  The forest goes to 6, its canopy seen over 128 m there,
  which takes most of its test's time;
- in an entry's `far` regions, wide ones (only x and z count),
  what its fields alone show from above at levels 4 to 6, against level 0,
  within 0.025: fields come from position alone,
  and hundreds of lines of sites average out,
  so what is finer than the sites must show in proportion ("Plans" in "Levels").
  Things are left out, with their far fields,
  as their far forms keep what rays see around them, not from above.
  The valley's has its first town (the same at every seed),
  the moor's its stream and patches of heath,
  the learned town's its whole crop;
- at each level up to one past the world's, in each region
  and a host tile's width around spawn,
  a slab above and one below the world's `heights` are empty:
  hosts generate only within them, and cut whatever is outside;
- the things of each population in a few thousand columns of cells
  spread over kilometres around spawn, of all heights,
  keep within its `reach`, which drawing checks only where it draws;
  and where a thing's bounds leave the heights, nothing is drawn outside.

Parts and far regions are checked at seed 42 only:
they check how a world is drawn, not what a seed puts in it.
`reach` in `World` is how far around the camera hosts generate each level,
a rendering distance, not checked.
With `--nocapture` it prints each level's margins;
`contract::margins` returns them as a `Report` to print,
which breaks a margin down by material type, and each type by kind,
largest differences first (`{}` the broken ones, `{:#}` all),
as a broken contract's message does, naming the world, seed, region and level.
The engine's tests also check that a thing's carves cut only the thing.
A new world is an entry in `WORLDS`
with regions that have, between them, a bit of everything in it,
`far` ones where its fields have patterns finer than the far sites,
and its name in `worlds!` in `worlds/tests/contract.rs`,
whose tests fail without it;
the web app lists it by itself, under its name.
To see a world without the renderer,
`worlds/examples/map.rs` draws its layout from above, any level, any area,
and `worlds/examples/look.rs` takes a picture of it at one level
(`worldgen::picture`: rays through its voxels, from any eye or straight down),
with what each pixel hit and how far away, for checks to measure
(`cargo run --release --example map` or `look` in `worlds/`, arguments in the files).

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
Layers whose sites are coarser than a plan of categories
read it dithered ("Plans" in "Levels").
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
(`Span::medium`, `column.rs`).

### Kinds and things

A domain names its kinds with `kinds!`,
after those of the domains it builds on:
the forest's come first in any world it grows in,
so its things are a library others draw from
(the towns' trees are the forest's `make_tree`).
A kind's type (`kind_type`) is its class of material:
the contract compares what rays hit by type, not by kind,
so kinds that may stand in for each other far away share one.

### Learned domains

A domain's plan can be learned instead of written.
The learned town (`worlds/src/learned.rs`) is the forest and fields
with settlements whose layout is a split grammar
learned in `lab/` from the valley's towns seen from above at a metre:
rectangles of 32 kinds, each cut in two or a leaf.
A settlement's crop is drawn top down from its site's seed:
each rectangle takes a rule of a rectangle of its kind and about its size
seen in training, and of its parent's kind and side,
the one whose cross streets fit those passed down from the cuts above,
and cuts where it did, from the nearer end;
rectangles a leaf long (16 m) stop there,
as their ground and the box of what was built on them.
That is a function of the site alone, integers only,
so it is drawn whole, once, and kept.
What it says of each column is a dithered plan;
the domains bid by it outright:
streets, paving and yards are the town's,
fields the fields', canopy and what the grammar left as texture the forest's,
and the town's houses stand on the leaves' boxes, facing the street.
The rule table (`worlds/data/town-grammar.bin`) is a sample of the rules,
in proportion to how often each kind and size was seen, without pixels.
`cargo run --release --example export` in `lab/` learns it again and writes it;
with `-- --check` it says whether the committed one is what it learns.

## Writing a world

The moor (`worlds/src/moor.rs`) is the shortest whole world to start from.
`build.rs` has the short forms:
`Field::columns` and `Field::at`, `Population::new(…).reach(…).make(…)`,
`Node::group`, `Child::new`, and `Shape::ball` and the like,
solid and filling unless told to `paint` or `carve`.
A shape kept with a chance is made with the long form (`ball(…, fill, op)`,
`Part::cells(…, fill, op)`):
how many rays it stops is worked out from its fill when it is made,
so its fill is given only then, and can't be set afterwards.

### Units

| What | Type | Unit |
|------|------|------|
| `Point`, shapes' sizes | `f64` | lattice units |
| Heights, distances: `Units::m(metres)` | `Units` | whole lattice units, `UNIT_SIZE` m each |
| `m(metres)`, for `i64` arguments and const generics | `i64` | whole lattice units |
| Cells, reach, `Child` and population `size` | `i64` | lattice units |
| `Plan` scale, plan and field arguments | `i64` | lattice units |
| Noise (`at`), fractions, `smoothstep`, `Rng::chance`, `Land` | `Fx` | `Fx::ONE` is 1 |
| Lengths with their fraction: `Fx::m(metres)`, a path's centre line | `Fx` | lattice units: `fx(2.0)` is 2 |
| `Far` detail, a shape's `extents` | `Fx` | lattice units |
| A span's `extinction` | `f64` | per lattice unit of a ray's path |

`Fx` is fixed point, 16 bits after the point:
a fraction, or a length in units with its fraction kept.
It meets `Units` only as `Fx * Units` (exact, an `Fx`)
and `Fx / Units` (a fraction),
and becomes units only by `floor_units` or `round_units`;
so a fraction can't pass for units, as it could as an `i64`.
Products of `Fx` are floored, quotients truncated towards zero;
times or over a whole number (`* 3 / 4`) it is scaled.
`Units(x)` takes a coordinate as units, `.0` gives units to an `i64` argument.
Fields get the coordinates of sites in lattice units
(already times the spacing) and the spacing.

### Spans that stay

A line of sites along an axis has a site only every two spacings
(x + y + z is even at a level's sites),
so a span shorter than that may hold none, and vanishes at that level:
a pond, a layer of turf, a hedge.
`Column` (`column.rs`) builds a column's spans from its ground's top,
each at least two spacings tall:
`Column::new(height, spacing).surface(grass, m(0.5)).fill_below(soil, bottom)`,
`.medium(kind, height, extinction)` on top, thickened but as dense along a ray,
`.water_to(kind, level)` where the ground is lower.
Carves along a line want the same margin.

### Pitfalls

- **Thin skins vanish.** A skin of one kind over a slightly smaller solid
  (thatch over a stone gable, plaster over a wall) is thinner than coarse sites,
  which then show what is under it. Make it one solid, or paint it.
- **Fields can't see things.** A field that has to know where things are
  (a path to a steading) reads a plan of them,
  and the population anchors each where the plan says,
  in a region of the plan with a margin so the cell owning it is there too.
- **Categories finer than the sites.** A field that decides kinds
  on a pattern aligned with the sites (a checker, blocks and streets)
  aliases far away: decide them in a `Plan::dithered`.
  Don't widen them to a site either:
  the moor's stream, drawn at least a site wide, covered five times its share
  at level 6; decided in a dithered plan, it shows as often as it is.
- **Thickened media spread.** A medium thickened to two sites
  stands over the columns beside it, and takes over their edges from above
  (the moor's grass over its heather, 0.03 at level 6),
  and stops rays skimming over it.
  Give a thin medium only while sites are close.
- **Anchors on the ground.** `Cell::on_ground` picks a spot evenly under a cell;
  things anchored there are as even as one per square 2N wide.
- **Seeds.** The contract checks four seeds; make the world, and its spawn,
  for any: the spawn is a site (x + y + z even) in the air,
  found without assuming a feature a seed may lack.
- **The bottom.** Spans at least two spacings tall grow past a fixed bottom
  at coarse levels, out of the heights:
  fill to the world's bottom with `fill_below_exactly`.

## Open problems

- **The forest's canopy thickens from level 6** (sites 8 m apart),
  whatever its slabs' density:
  from above, over 128 m at seed 42, 83, 84 and 87 % at levels 4 to 6
  (79 % at level 0, 85 % at levels 1 to 3, whose trees the canopy is measured from),
  and up to 0.05 fewer rays through at level 6 than at 4.
  The canopy's spans predict 81 % at level 6, the same as at 4,
  and a uniform medium over flat or sloping ground draws what it predicts.
  Likely: a ray down crosses the sites of two columns in turn,
  so it averages the density over a site,
  and where that varies over 8 m (paths, clearings) it covers more.
  Not checked yet.
- **The forest's canopy is all leaves, and only over the trees' feet.**
  Its trunks are leaves too: rays among the trees hit wood 0.04 less
  at levels 4 and 5.
  Each slab's kind drawn as bark in proportion to its wood seen from the side
  overshot (materials hit 0.09 off).
  Crowns reach over the path, the canopy doesn't:
  spread over 2.5 m around each column, the materials rays hit came closer,
  but from above at level 6 went to 0.096.
- **Media are isotropic.** A site stops rays from every direction alike;
  vertical trunks seen from above and from the side cannot both be right.
  Measured far forms show it too: a site is of one kind from every side,
  so at a house's edge, roof from above and wall from the side,
  it shows walls from above some of the time:
  at level 4 (sites 2 m apart) the valley's town is 0.11 off from above.
  At level 5 its houses and church cover more of its streets
  (rays hit streets 0.08 less often, 0.13 off),
  as a cell half covered by a wall stops every ray across the wall
  and is mostly kept;
  so the valley keeps what a camera sees to level 4.
  Kinds credited by direction can't settle it:
  from above only, level 4 from above came to 0.05,
  but the materials rays hit at levels 3 and 4 to 0.12 and 0.16;
  each axis's columns weighted by the thing's shadow along it,
  or credited to the cell their first fine site is in,
  moved neither by more than 0.01.
  The forest's canopy is caught the same way ("Populations" in "Levels"):
  from above, rays want what each slab adds seen across;
  among the trees and obliquely, what it hides from the side.
  Slabs as dense as what they add from above alone
  put the materials rays hit at level 5 0.155 off;
  as Cauchy has it for a cylinder (½ and π/4),
  0.05 off, but from above 0.09 off at level 6.
- **A lone measured thing is kept or not almost whole.**
  Its cells are held to one threshold,
  and a hollow house's are only partly full:
  the moor's hut, measured, drew to its threshold of 0.96 (seed 42),
  so only wholly covered cells stayed,
  and its thatch went from 21 % of the view from above to 15 % and 3 %
  at levels 2 and 3; hence its outline.
  Filling what rays from outside can't reach through gaps finer than a cell
  (a closing) before measuring kept the thatch,
  and brought the valley's town's materials hit at level 3 from 0.064 to 0.026,
  but grew the houses over the streets (0.039 to 0.057 from above).
- **Numbers and fields' own categories are point-sampled.**
  A plan of numbers finer than the sites is read where each line is;
  filtered to the spacing it would cost coarse levels less (see docs/todo.md).
  Categories a field decides from position, not from a dithered plan
  (the fields' crops, the forest's columns, which hold its heights too),
  are read where each line is as well:
  even where their edges are irregular, aliased where they follow the sites' grid,
  as the town's square's checker of metres did (all light from level 4).
  Over the forest at level 6 such reads of its floor came within 0.042 (seed 42)
  of level 0, likely by chance: too close to the far check's tolerance to add it.
- **Things of one band can't see each other.** Bands order what decides where,
  but two populations of one scale ("no tree on a boulder") still can't
  exclude each other. Claims with hashed priorities would be local and
  order-free: a thing is kept unless a neighbour's claim of higher priority
  overlaps it (one round of Luby's algorithm), from anchors alone.
