# World generators

A generator (`worldgen/src/generator.rs`) describes a world;
the framework draws any box of it, alone, at any level of detail,
and gets the same voxels whichever boxes it drew before.
That lets a world be endless and generated in pieces around the camera,
coarser further away, for about the same cost per box at every level
(where it is not, "Cost" shows).
`worldgen/` is that engine, and `worlds/` the worlds on it,
which use only the engine's public API.
The forest (`worlds/src/forest.rs`) is written this way,
and so is the valley (`valley.rs`),
the forest with towns, fields, landmarks, moorland and a railway
between mountains, six domains composed as in "Composing",
and the city (`city.rs`), boxes on a plan of quarter metres;
a host starts either by name from `WORLDS` (`worlds/src/lib.rs`),
as a `World`: its generator, spawn and heights,
and the places it names (`points`);
the web lists and generates them all as WebAssembly (`src/lib/worldgen.ts`).
Integers and fixed point (`noise.rs`) decide wherever they can,
so a world is the same on every machine and as fine far from the origin.

This is the second version of the contract (see "Open problems").

## Where it is going

The engine is moving towards worlds made of continuous parts only:
fields, points and shapes, each given at any position.
Only the engine would know the lattice, its cells, levels and caches,
and what the checks below measure
(levels that agree, cost, no preferred places or directions)
would follow from how the parts are built, not be tested.
Points are a first step:
a far form can read the things that stand, by construction
(the forest's canopy at levels 4 and 5).

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
  An anchor picked evenly within the cell's reach (a cube 2N wide)
  gives things as even and as dense as one per cube 2N wide would:
  every point is in one cell, which picks it as often as any.
  Or the population gives an anchor per **line** of cells along an `axis`
  (`anchor_along`): the cells whose centres differ only along it, every 2N,
  which lie within a square 2N wide across it.
  The framework asks each line a box reaches once,
  and the cell of the line the anchor is in owns it;
  an anchor in another line's cell is no one's.
  Every point is in one cell, so in one line along each axis:
  a point picked evenly in each line's square,
  on a surface each line crosses once (`Line::pick`, a ground under columns),
  gives things as even as one per square 2N wide,
  for one pick however long the line.
  Cells and lines give named random sequences (`cell.random("species")`)
  that depend on them and the name only.
  Or a population makes a thing at each point of `Points`
  (`anchor_points`; see "Points").

A layer may say **where it is**, about a plan of heights
such as a world's ground (`plan::Sloped`):
a field's spans along y lie from `below` under to `above` over it
at their column (`Field::about`),
a population's things stand on it (`Population::on`);
the ground's field may say what it is under all that (`Field::under`),
solid down without end.
Each is a rule (below), checked wherever it draws;
so the framework asks it to draw only where it may be:
a field line by line in blocks (`LINES`), a population box by box,
from the plan's range over their columns (`Sloped::range`),
which its slope bounds;
a plan promising no slope (`Sloped::any`) leaves them drawn everywhere.
So a host may ask for any box, in any order,
and a field about the ground costs little in the air above it
or the rock below it, where it is not asked at all.
And what a box holds throughout follows from what they say (`whole`):
nothing, or ground so deep that none of it shows.
The last field drawing all of a box decides it;
a layer drawing in it after keeps only that:
a carve what is empty, a fill or paint what is buried
(a thing's carves cut only it).
A host need not make either (`shown`):
it generates a box less its whole bottom and top.
A layer that says nothing is drawn everywhere.

A thing is a **node**: its **shapes**, and **children**,
finer things it makes only for the area asked for (`Children::Reaching`).
Shapes are analytic: `ball`, `ellipsoid`, `tube` (along a curve, below),
`capsule` and `lathe` (straight tubes, their ends round or flat:
a twig, a crown), `cuboid`.
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

**Curves** (`curve.rs`) are lines through space: a path, a river, a tunnel.
A curve runs between two ends and is made top-down:
each level puts a point between each two of the level before,
at most its `wander` from their middle,
reading only them and what is coarser.
So the curve between two neighbours of a level
stays within the finer levels' wanders, summed, of the segment between them:
its corridor.
`near` finds where a curve passes nearest a point, if within a distance:
how far, how far along, and the point.
It looks only into segments whose corridor reaches that near,
so it costs little but close to the curve.
One rule serves every piece of a curve too long to make whole,
between ends the world finds near a point.
The forest's path (`worlds/src/path.rs`) is one, in pieces 8 m long:
what is on it or keeps off it measures from the curve;
the valley is laid out across its centre line, along x.
The railway's line is another, its points chosen by cost ("Railway").

A **tube** is the points within a radius of a curve: a branch, a root.
At sites s apart it is drawn through the points of the coarsest level
whose corridor is s / 2 or less,
so far away it is the same curve, coarser, within half a site of where it is.
Its radius is a parameter, as fill is:
the same all along, or a function of how far along
(`Radius`: a trunk narrowing, a crown's whorls).
The forest's branches bend a little, along random curves (`Curve::wandering`);
its oaks' trunks lean and narrow, and their limbs fork twice (`Crown`).

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
  (from `detail` on):
  a cluster's ball for its sprays of leaves on twigs, a site thick at level 0:
  the whole ball they reach, as dense as they are on average,
  so see-through near its edge, as they are.
  Clumped, the sprays stop as many rays as half their density spread evenly.
  A ball as wide as their shadow and all but opaque kept the shadow too,
  but darkened the crowns inside.
  Or the far form is the near form measured (`Far::measured`):
  drawn whole two levels finer than it is drawn at,
  but no finer than the coarsest level its near form is drawn at,
  then coarsened.
  Seen along an axis, a voxel is a square turned 45° round its column,
  twice a column's area:
  the even columns' voxels cover the plane once, the odd ones' once more,
  so a ray along an axis meets the voxels of two neighbouring columns (`pairs`).
  A ray crossing a coarse site's cell is stopped
  as often as the pairs holding a fine site of the cell hold one of the thing,
  on average over the axes.
  Counted by columns, a sheet one site thick stopped half what it does,
  and sparse content a third less
  (0.18 against 0.27, traced through the voxels, in a cell four sites wide).
  Each cell is drawn with its share, as alpha (see "Kinds and things").
  Shown as often as covered, a house's walls come out ragged;
  all held to one threshold, each house came out fatter or thinner by a site,
  and the few a camera sees often all one way
  (the valley's first town lost a third of its walls at level 4, 0.099 off);
  each kept where more than half covered,
  its houses lost half their roofs at level 6.
  Each face of a coarse site is of a kind
  in proportion to what rays through it see
  (a site's `Look`: a kind, and one per face where they differ,
  so a house's eaves are roof from above and wall from the side).
  Along the face's normal, a line of nearest neighbours,
  each column's first fine site from that end
  counts for that face of every cell the column crosses:
  a ray that gets to a cell, through the cells before it, sees that.
  The site's own kind is of what all its faces see,
  for whatever does not look at faces.
  From the ends of every column's run in the cell instead,
  a town seen from above showed walls hidden inside its houses;
  credited to the first cell only,
  through holes in its roofs.
  Two levels finer, a cell holds enough fine sites to measure,
  and a thing costs an eighth as much a level coarser,
  where a tile holds four times as many:
  measured at its finest, the valley's town tiles at levels 4 and 6
  were over the budget ("Cost").
  But a thing only a few fine sites big loses what is finer
  (the town's houses at level 6, docs/todo.md).
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
  their shadows by pairs of columns, as above,
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
  The forest's medium is as dense as the trees that stand ("Points").
  Things reach past their cells, crowns over a clearing:
  `Shadows::spread` measures where around a cell its things' shadow falls,
  and at levels 4 and 5 the forest's canopy over a column hides, from above,
  at least what the crown of the tree whose room it is in would
  (one crown, as they keep to their rooms: "Rooms").
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
And build what a sheet covers of the sheet's kind, then paint what shows:
columns of sites step over a sheet thinner than two sites to what is under it,
at whatever level it is measured.
A roof is solid down to its eaves, its gables walls only at its ends
(`gable_ends` in `town.rs`), so it is roof from above, not the walls under it.
A sheet over nothing, such as a windmill's stage, is a child of its own,
drawn as sheets are, not measured.

## Points

`Points` (`points.rs`) say where things are, top down,
so a cell of any size knows how many it holds without making them.
Its cells are squares on the axes it spans,
from leaves to roots, each twice as wide as the one below.
It is given, by mark (a tree's species),
the chance a leaf at a point holds a thing of it,
read where asked, as a plan's columns are made:
read dithered, a cell's count would hang on the layer asking.
A root holds a Poisson number of each mark;
a cell's points go to its children one by one,
by the chances at their leaves' corners times their leaves still free;
a leaf's points stand where their mark has a chance.
So a cell's counts are its children's added, by construction,
and a far form reading them shows the things that stand.
A `Blur` weighs the counts of the cells around a column by their offset,
reading a block of cells at a time:
the forest's canopy at levels 4 and 5
is the trees standing in the cells a column meets,
and the crown of the tree whose room it is in ("Rooms", below).
A cell costs the chances at its leaves' corners,
sites of the levels at least as fine as the leaves,
so leaves should be no wider than the chances vary over.
A cell as wide as the roots (the forest's are 8 m wide)
costs a column a Poisson draw and those corners,
and wider ones their roots', sites the level doesn't read:
from level 6 the forest's canopy is the trees expected,
whose shadow there falls mostly on their own column.
Weighing cells by their chances filtered to their size doesn't avoid it:
the trees keep to chance only with chances read every 16 m or closer,
so a wide cell still reads sites its level doesn't
(the forest's level-8 tiles took 6.5 times the instructions),
and counting the trees that stand at levels 6 and 7
did little for the windmill there (0.099 to 0.089 over 16 seeds).
Points keep no distance between things, as `Claims` do:
a hard core would make a cell's count hang on its neighbours' points.
But a thing at a point can see those around it,
as points are a function of position.

### Rooms

`Rooms` (`points.rs`) split the plane among the things at a process's points:
its power diagram.
A place is the room of the thing whose power there,
squared distance less weight, is least, if that thing reaches it;
each wall loses half a gap to either side.
Weights are of a point alone and the framework draws each pair's gap,
so both sides of a wall agree on it:
rooms never meet where gaps are positive,
by construction, and counts stay as they are.
Whose room a place is (`owner`) is exact, in whole units,
and needs only the points within √(reach² + most weight) of it.
So a thing's near form keeps to its `room`,
and a far form reads the same rule:
the forest's canopy over a column is its owner's crown,
not the overlap of the crowns around
(summed as if they overlapped, level 4 showed 0.26 more canopy from above).
Its crowns keep to their rooms (crown shyness):
a taller tree weighs more, a crowded crown is squeezed,
a quarter of pairs meet a little, by their gaps,
and a tree with no room, under a taller one, stands bare.
What crowded trees hide from the side
is measured in sample stands, `Points` of constant chance.

## Rules

Broken rules panic, naming the thing and pointing here:

- **Populations.** A thing reaches at most `reach` past its cell's reach,
  N from its centre (one number or per axis), whichever way it is anchored;
  only things that get made are checked, so derive `reach` from the largest possible.
- **Things.** A node with children gives `bounds` holding them;
  a child stays inside its parent's bounds;
  siblings have unique names;
  a measured far form starts at a `detail` of a unit or more.
- **Plans.** A plan reads only plans at least as coarse (see "Bands").
- **Curves.** A curve's point stays within its level's `wander`;
  only points a query looks at are checked,
  so derive `wander` from the most a rule can stray.
- **Kinds.** A world has at most 255 materials: ids are bytes.
- **Slopes.** A `Sloped` plan's column differs from a neighbour's
  by at most its `slope`, whenever one is read and the other is kept.
- **About.** A field's span lies within `below` under and `above` over
  what it is `about` at its column,
  give or take two sites (a span is at least two sites tall)
  and twice the slope times the spacing
  (a line reaches down past the eight columns around it, `sides`);
  under that, if it says, all its `under`, a solid span from `DEEP`.
- **On.** A thing's anchor lies at most N (its cell) off
  the plan its population stands `on`.
- **Rooms.** A thing's weight lies from 0 to the rooms' `most`.

Not checked, but caught by the tests:
draw from position, seeds and named random sequences only
(no shared `Rng`, no global state),
and fields from position alone.

## Checks

`worldgen::contract` (`worldgen/src/contract.rs`) checks a world
in regions holding everything it is made of around spawn,
which it finds itself (`worldgen::survey`, `survey.rs`), not from a list:
`worlds/tests/contract.rs` runs it for each world in `WORLDS`
at seed 42 and three more, a thread each,
and `worldgen/tests/contract.rs` for test worlds of fog, caves,
and a village of houses with doors and tables
on y = 0, a plane of sites at every level ("Faces" in "Levels");
and knobs anchored along x on a slanting wall,
one per square 2N wide across x.
`worldgen/tests/shapes.rs` holds each solid to the levels it keeps,
as many of one kind, of sizes and places drawn at random, on a ground:
bright on dark, the palette that hides the least:
a world's colours can only hide what breaks there.

The survey asks every layer of the generator around spawn,
drawing no voxels.
Things come apart into parts no wider than a region (`REGION`),
named by their children's names, numbers left out (`towns › block › house`);
a dense population is surveyed over a smaller square than a sparse one (`PARTS`).
Fields are asked at columns of a coarse level, and at level 0 there:
they keep what is finer than their sites in proportion ("Plans" in "Levels"),
so a railway, a kerb or a pond shows.
A stratum is a part or a field, with a kind it draws.
Every stratum found gets a region: the rarest first,
each in the region it widens least, or in one of its own nearest spawn.
So a rare thing is checked without being listed,
and not lost among the rest of a wide region;
but a part's variants drawing the same kinds are one stratum.
A layer that draws nothing surveyed breaks the contract;
the kinds nothing draws are only printed,
as worlds share tables of kinds (the moor's is the valley's).
`CONTRACT_ONLY='towns › church,railway'` checks only the regions
of strata so named, and surveys only their layers.
Where a world is known to break the contract at a seed,
in the regions holding a stratum (`KNOWN_BREAKS` in `worlds/tests/contract.rs`),
they must still break, so the list only shrinks;
each is planned work in `docs/todo.md`.
The checks do take y as up, as hosts do:
a world's heights are along it, and "from above" looks down it
(see `docs/todo.md`).

Rays see each site by its alpha (`VoxelGrid`'s):
each stops its share of what gets to it, and shows that much of its colour,
and what it glows (`composited` in `picture.rs`).
What they see is compared as under a white light all round (`colour`),
in the sRGB channel that differs the most:
colours, alpha and glow average as a coarse site stands for finer ones;
materials don't (by type, the canopy's oaks could all turn orange).
The voxels a site's coin shows (`VoxelGrid::show`) are as often, but noisy,
and a site too thin to show one still stops light.
Margins quoted elsewhere in this document are older ones,
in shares of material types.

- a box against eight parts generated alone, in reverse order,
  at levels 0, 2 and one past the last checked below:
  any seam, order dependence or hidden state shows up as a differing site,
  in its voxels, its alpha, its faces' materials or its glow;
- at each level up to the world's, 20,000 rays in random directions
  from where a camera could be, against the level two finer:
  of what of the rays gets 2 sites far, the share that gets 4 more,
  and what they see past those 2, must agree within 0.1.
  A camera could be at the sites of the finer level that show no voxel
  it could fly to from above,
  through such sites from the top of the box (it collides with voxels shown):
  under a canopy (in the forest, all but 0.3 %), not under the ground's shell.
- at each level up to the entry's `from_above`,
  what rays straight down see,
  over the region grown to at least 16 sites of the level across,
  against level 0, within 0.1:
  levels two apart miss what drifts a little at every level,
  like a town's walls taking over from its roofs.
  Rays see the faces they enter by, from above the ones up.
  Each world goes as far as it keeps what a camera sees;
  the forest to 6, its canopy seen over 128 m there,
  which takes most of its test's time;
- in an entry's `far` regions, wide ones (only x and z count),
  what its fields alone show from above at levels 4 to 6, against level 0,
  within 0.025: fields come from position alone,
  and hundreds of lines of sites average out,
  so what is finer than the sites must show in proportion ("Plans" in "Levels").
  Things are left out, with their far fields,
  as their far forms keep what rays see around them, not from above.
  The valley's has its first town,
  the moor's its stream and patches of heath,
  the learned town's its whole crop;
- at each level up to one past the world's, in each region
  and a host tile's width around spawn,
  a slab above the world's `heights` stops no rays at all:
  hosts generate only within them, and cut whatever is outside
  (under them, the ground goes on, unseen);
- the things of each population the survey found, of all heights,
  keep within its `reach`, which drawing checks only where it draws;
  and where a thing's bounds leave the heights, nothing is drawn outside;
- at levels 0, 2 and one past the last checked, in each region
  cut into boxes 32 sites a side (`skips_hold`),
  what is skipped where it says it may not be draws nothing there:
  it is drawn all the same, to see (`checking_skipped`),
  and what a field fills under its ground unasked is what it would draw;
  and a box `whole` says is empty or buried is.
  The rules above hold wherever something is drawn;
  this finds what they can't see,
  as a plan steeper between columns no layer reads together.

A world's layout must not follow the grids it is generated on:
a town placed once per region, kept inside it,
is never where two regions meet.
`worldgen::places` (`places.rs`) checks that things prefer no places.
It gathers each population's things over 16 km around spawn at eight seeds,
from columns of cells in patches anywhere
(a cell's anchor finds the thing of the cell it is in,
so one thing per region of a plan is found from any of its cells).
For each axis and period, from four cells to an eighth of the area,
it sums the things' phases 2π·x/P over stretches of whole periods;
a preference is a mean of those sums away from zero
against how much they vary from stretch to stretch
(Hotelling's T², its chance over all the tests under 10⁻³).
Stretches, not things, are the samples:
things that come in clusters, or bunch where a world has a feature
(towns along a path at x = 0 at every seed),
are a bias in one stretch, a place;
the same bias in every stretch is a period.
All seeds' things are tested together,
to find few things that prefer the same places at every seed,
and each seed's alone:
a grid whose phase a seed draws still keeps towns off its regions' edges.
Only things are checked: claims a field draws (the moor's pools) are not,
and things as rare as the valley's haystacks are too few to tell.
`worlds/tests/places.rs` checks each world in `WORLDS`,
but for the populations it knows prefer places (`KNOWN`, docs/todo.md).

Nor must a world's directions follow the lattice's.
Its own may be anything: up, a heading, a valley along z.
But about the vertical the lattice, square noise and tiles are four-fold,
and leave a four-fold harmonic of horizontal directions
at the lattice's phase, the same at every seed.
`worldgen::directions` (`directions.rs`) measures it
in pairs of points a few metres apart, in random directions,
from how often the two differ in kind;
seeds are the samples (Student's t, its chance over all the tests under 10⁻³).
A world's own direction makes a four-fold harmonic too,
but none larger than its two-fold one, so only the excess counts:
a strong direction of the world's own hides as much of the lattice's.
Each place a world names is tested apart, in a box around it at each seed,
as is the rest of the world, in boxes anywhere.
`worlds/tests/directions.rs` checks each world in `WORLDS`,
but for the places it knows follow the lattice (`KNOWN`, docs/todo.md).

Parts and far regions are checked at seed 42 only:
they check how a world is drawn, not what a seed puts in it.
With `--nocapture` it prints each level's margins;
`contract::margins` returns them as a `Report` to print,
which breaks a margin of colours down by material type, and each type by kind
(what changed, not what is checked),
largest differences first (`{}` the broken ones, `{:#}` all),
as a broken contract's message does, naming the world, seed, region and level.
The engine's tests also check that a thing's carves cut only the thing.
A new world is an entry in `WORLDS`
with `far` regions where its fields have patterns finer than the far sites,
and its name in `worlds!` in `worlds/tests/contract.rs`,
whose tests fail without it;
the web app lists it by itself, under its name.
To see a world without the renderer,
`worlds/examples/map.rs` draws its layout from above, any level, any area,
and `worlds/examples/look.rs` takes a picture of it at one level
(`worldgen::picture`: rays through its voxels, from any eye or straight down),
with what each pixel hit and how far away, for checks to measure
(`cargo run --release --example map` or `look` in `worlds/`, arguments in the files).
`look` takes the places a world names for its eye, target or area,
so a feature can be seen at any seed (`at windmill`);
to find one more, give its world a point for it, not a scratch search.

## Cost

Timings are noisy on a shared machine, so what generating costs is counted
(`worldgen/src/cost.rs`):
while `cost::counted` runs, `generate` counts its work
by layer and by plan (`Work`),
plans' lookups (`Plan::at`) among it,
counted to the layer or plan looking them up:
a layer reading a plan at many points around each column shows,
as does a plan keyed finer than a level's columns,
made afresh for nearly every one.
Counts are the same on every machine;
weighed by rough times per count (`NS`),
they show what takes the time.

`cost::report` generates a world's host tiles at several levels
around some places, each tile by a fresh world, as a host's first is.
It prints the estimate by layer and level, largest first,
the time each tile took here,
and flags what looks costly:
a tile over budget,
a layer growing fourfold from one level to the next,
a population asking many more cells for anchors than own one,
a field read at every site,
one huge thing,
things drawn whole to be measured,
a plan making each column over `cost::REMADE` times,
as a failing cache does (each plan's row shows its times made).
`cargo run --release --example cost -- <world>` in `worlds/` prints it
(arguments in the file).

`worlds/tests/cost.rs` holds each world in `WORLDS`
to its baseline, `worlds/data/cost/<world>.txt`:
it fails when a count rises or falls by more than a quarter
and the change weighs a hundredth of its tile or more.
When the change is meant,
`COST_BLESS=1 cargo test --release --test cost` in `worlds/`
writes the baselines anew, and their diff shows it.
It also fails when a tile's estimate is over the budget (`cost::BUDGET`),
unless the tile is listed in `OVER_BUDGET` there,
and when a listed one is not, to take it off:
the list only shrinks, and blessing does not move it.
It fails, too, when a plan makes a tile's columns over `REMADE` times each,
so a failing cache can't be blessed in.

Counts are an estimate and miss things.
When speed matters, time real tiles with `worlds/examples/bench.rs`;
its hashes of what they hold, before and after,
show a change leaves generation as it was.

## Composing

A world as large as the valley is **domains**
(the forest, `town.rs`, `fields.rs`, `landmarks.rs`, `moor.rs`, `railway.rs`):
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

Domains share land by **bidding** for each column (`Land`).
A bid is a function of the column,
read once a column by the land's plan, so not kept apart:
a domain bidding nothing somewhere costs nothing there.
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
Relief is not a domain:
the valley's mountains (`mountains.rs`) raise the ground all of them stand on,
massifs and passes along it, crests, spurs and side valleys across,
and end the forest by its share at a treeline,
where a field paints meadows, scree, rock and snow by height, slope and hollows.
Quantities want blends of their own, though.
A town levels the ground 120 m past its streets,
far beyond the metres in which it gives way to fields,
so the ground blends by the town's own terrace weight, not by shares.

A domain can't place what it builds whole, a hut or a pool, by the shares:
they are a finer band than its places,
and they read the ground (the fields bid by its slope), which a hut levels.
So the moor bids from position alone,
and places its steadings and pools only where it surely owns the land:
where its bid leads the most any other may bid there
by more than the land's width.

### Claims

Things placed whole at a coarse band, a town, a steading or a landmark,
must keep apart, yet a plan of one per region,
kept inside it so a column asks only its region's,
never puts one where regions meet ("Checks").
`Claims` (`plan.rs`) places them anywhere instead:
each cell proposes spots evenly in it, a Poisson number,
the world's `claim` makes each a thing there or turns it down,
and a thing is kept unless one of higher priority stands closer than `apart`,
priorities hashed from the spots
(one round of Luby's algorithm, Matérn's second hard-core process).
That is local, order-free and from position and seed alone,
and no spot is likelier anywhere, so no place is preferred.
A column asks `near` for the thing kept nearest it, within `apart`;
the keys of the few kept near each stretch of columns `apart` / 2 wide
are kept, so a column doesn't read the cells around afresh.
A thing may stand up to a cell from its spot:
the valley's towns turn down spots over half a cell from the path,
and stand on it.
A thing that reaches round its point,
a steading's hut, path and levelled ground,
a landmark's footprint and clearing,
or a pool, which a field draws,
is asked for within its reach, at most half of `apart`,
so a column is reached by one at most.
A `first` spot, kept whatever is near, puts one where a world needs it:
the one place preferred.

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
A world of fewer domains may still have all their kinds:
the moor alone has the valley's (`valley::kinds`), its own last,
so its ids are the same in both,
and it reuses the forest's stone and the town's thatch.
A kind's type (`kind_type`) is its class of material,
by which the contract's reports break down what rays see.
A kind marked `walk_through` is one a walking body goes through
(plants, water; see `bake/src/walk.rs`), the rest it stands on;
so ground and what stands on it are different kinds,
even of one colour (the moor's `heather` and `heatherSprigs`).
A kind marked `glows` gives light of its colour, as lamps do
(`worldgen/src/light.rs`, for now only in look and native's computed lighting).
Each site holds as much as what is in it stops rays
(`VoxelGrid`'s `pieces`, `medium` and `glow`), each thing its share,
and glows as its glowing things, in proportion to their shares.
Things are in a site in two ways, and it keeps a share for each:
pieces smaller than the site, as leaves are,
stop a share of its rays whole;
a kind marked `medium`, as mist is,
stops a share of every ray the pieces let through.
So a coarse site of a crown in mist holds both, each as it is.
The pieces show as grain, a whole voxel as often (`VoxelGrid::show`);
where they don't, a host that draws alpha (native)
shows the medium as see-through as it is.
Drawn see-through, leaves are smoke; as grain, mist is snow.
The web draws no alpha, and shows no medium.
So a lamp too small for a coarse site
is in it as a little glow, and stops a little light.
A far form without its near form's lamps goes dark,
as does a child too small to be drawn at a level (give lamps `ANY_SIZE`):
lamps light what is far from them only from coarse levels.
Fields do not glow.

### Landmarks

Structures that stand alone (`worlds/src/landmarks.rs`),
a ruined keep, a watchtower, chapels, ponds, farmsteads and windmills,
are the valley's fourth domain:
a new structure joins it as a landmark, or as a thing of a domain.
Sites are claimed ("Claims"):
each spot tries the kinds in a square around it,
and a column asks for the one site within its reach.
What suits a kind is read off the valley's ground
before landmarks shape it (`valley::Terrain`):
how far the place is from a town and from the path,
whether it is up on a rise or down in a hollow,
how even the ground under the footprint is.
A site levels the ground plan under it (a pond digs a bowl),
bids for its footprint and a clearing, which trees and crops then keep off,
and paints its ground;
a population anchored at the sites builds each in its own frame.
A pond's water stops under the rim its bowl leaves,
so it never stands over the ground round it.
The first is a chapel beside the path, in the camera's view at spawn.

### Railway

The railway (`worlds/src/railway.rs`) is a domain built on curves.
Its main line runs along one side of the valley,
in pieces 4 km long whose ends stand off the middle by noise.
Each piece is a curve in x, y and z, made top-down:
at each level a point between two neighbours,
its x the one of a few within the level's wander
whose track leaves the ground least,
its y the ground there as far as the grade lets it from both neighbours.
So no stretch is steeper than the grade,
and each wander is a quarter of the one before,
which bounds how sharply it turns at each point.
The turns are then smoothed away:
the line's x is the mean over 12 points either side,
across the pieces' ends too
(a piece reads its neighbours' unsmoothed points, a plan of their own),
so seen from above it bends by a fifth of a degree per point at most.
The line reads the ground as the towns level it (`valley::Terrain`),
not as it cuts and fills it;
it stands far enough across the valley
that it never reaches a town, by construction,
and the landmarks and the moor keep off it.

What carries the track comes from comparing it with the ground under it,
the same everywhere:
ground far above it is tunnelled, ground far below it bridged,
and nearer it is cut or filled, sloping from the formation's edges.
That is decided at each of the line's points, 8 m apart,
and a bridge or tunnel shorter than four points (32 m) is cut or filled
instead, so none is a stub where the ground just crosses a threshold.
Under a bridge's ends the fill rounds off as its sides slope,
and near the edge of the railway's reach every side steepens to meet the ground:
no earthwork ends in a wall but a tunnel's portal.
A station stands by each town the line passes on the ground.
The railway bids for its formation and under its bridges, not over its tunnels.
Seen from above, the line keeps within a corridor of the segment
between its piece's ends,
so it turns away columns further across before reading any plan.

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

## Scenes

A world can be a scene made elsewhere:
`scripts/voxelize.py` turns a glTF scene into voxels at level 0,
each the mean colour of the surfaces through it, of 255;
`worlds/src/scene.rs` draws them as a field,
each coarser level made from the one below.
A scene is its surfaces only, hollow inside.
`--scale` makes it bigger, so finer than the lattice's unit:
Sponza is drawn twice its size.

Scenes' licences keep them out of the repository:
their files go to `worlds/data/scenes/`, which git ignores,
and each world is built only with its feature.
The tests run without them,
as no scene keeps `places` or `directions`.

Sponza is the `glTF/` folder of
[Khronos' sample](https://github.com/KhronosGroup/glTF-Sample-Assets/tree/main/Models/Sponza);
then, from the repository's root:

```bash
python3 scripts/voxelize.py Sponza/glTF/Sponza.gltf worlds/data/scenes/sponza.bin --scale 2
```

```bash
pnpm wasm --features sponza && pnpm exec vite
```

```bash
cargo run --release --config native/.cargo/config.toml --manifest-path native/Cargo.toml --features worlds/sponza -- --view 'world=sponza&look=90,5' --shot out.png
```

## Writing a world

The moor (`worlds/src/moor.rs`) is the shortest whole world to start from.
`build.rs` has the short forms:
`Field::columns` and `Field::at`,
`Population::new(…).reach(…).anchor(…)`, `.anchor_along(axis, …)` or `.anchor_points(…)`, `.make(…)`,
`Node::group`, `Child::new`, and `Shape::ball` and the like,
solid and filling unless told to `paint` or `carve`;
`.about(ground, below, above)` on a field, `.on(ground)` on a population,
says where it is ("Model");
the ground's field `.under(kind)`, its columns ending `Column::under(kind)`:
a world is solid under its ground, not a shell over a hollow.
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
A heightfield is a shell, hollow under its sides:
`.sides(kind, lowest_around(ground, x, z, spacing), depth)` reaches each column
down past the lowest of the eight around it, so rays can't slip under a slope.
The diagonals too: a voxel shares faces with their sites at its own height,
and without them rays slipped in at a cliff's corners.

### Pitfalls

- **Thin skins vanish.** A skin of one kind over a slightly smaller solid
  (thatch over a stone gable, plaster over a wall) is thinner than coarse sites,
  which then show what is under it. Make it one solid, or paint it.
- **Fields can't see things.** A field that has to know where things are
  (a path to a steading) reads a plan of them,
  and the population anchors each where the plan says:
  each cell asks for the thing near it (`Claims::near`),
  and the cell it stands in owns it.
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
- **Things on a surface.** Anchor them along the axis across it
  (`anchor_along(1, …)` with `Line::pick` for a ground under columns),
  or at points of the axes across it (`anchor_points`), not per cell:
  a host's tiles span the world's heights (the web's)
  or are cubes over one another (native's),
  and asked per cell, a population reads the ground in every cell over each column,
  most of them in the air or under it;
  under the valley's mountains that was most of a tile's time.
- **Seeds.** The contract checks four seeds; make the world, its spawn
  and the places it names, for any: the spawn is a site (x + y + z even) in the air,
  found without assuming a feature a seed may lack,
  and a place is searched for until found (the valley's first windmill
  is kilometres away at some seeds).
- **The bottom.** Spans at least two spacings tall grow past a fixed bottom
  at coarse levels, out of the heights:
  fill to the world's bottom with `fill_below_exactly`.

## Open problems

- **The forest's canopy from above drifts with the level.**
  Leaves seen from above over 128 m round the spawn, at seed 42:
  80 % at level 0, 81 to 83 % at levels 1 to 3, 77, 80 and 81 % at levels 4 to 6;
  at seed 6, 75 % at level 0, 78 to 79 % at levels 1 to 3 and 5 to 6, 75 % at level 4.
  A uniform medium over flat or sloping ground draws what its spans predict.
  Likely: a ray down crosses the sites of two columns in turn,
  so it averages the density over a site,
  and where that varies (paths, clearings) it covers more.
  Not checked yet.
- **The forest's canopy shows its trunks from above.**
  Each slab of it is its trees' bark as often as their sites in it are wood
  (`Shadows::share`), else their foliage;
  all leaves, rays among the trees hit wood up to 0.10 less at level 4
  (the learned town at seed 1).
  But a site looks the same from every side,
  so from above, where the crowns are thin, rays reach bark that crowns hide:
  over the forest's spawn at seed 4, 0.03 of what level 4 shows from above,
  0.007 at level 0.
- **Media are isotropic.** A site stops rays from every direction alike;
  vertical trunks seen from above and from the side cannot both be right.
  (Opacity per direction, tried twice, did not pay: see "Sheets and rods" in `docs/todo.md`.)
  What they show needn't be: measured far forms give each face its kind,
  and at level 4 (sites 2 m apart) the valley's town edge from above
  came from 0.105 to 0.071 off, the learned town's from 0.099 to 0.062 (seed 42),
  while the materials rays hit from anywhere stayed within 0.08.
  At level 5 its houses and church cover more of its streets
  (rays hit streets 0.08 less often, 0.13 off),
  as a cell half covered by a wall stops every ray across the wall
  and is mostly kept;
  so the valley keeps what a camera sees to level 4.
  The forest's canopy is caught the same way ("Populations" in "Levels"):
  from above, rays want what each slab adds seen across;
  among the trees and obliquely, what it hides from the side.
  Slabs as dense as what they add from above alone
  put the materials rays hit at level 5 0.155 off;
  as Cauchy has it for a cylinder (½ and π/4),
  0.05 off, but from above 0.09 off at level 6.
- **A hollow thing measured keeps only what its walls cover.**
  A thing keeps as many cells as its shares add up to,
  and a hollow one's walls fill theirs only partly:
  the city's buildings, hollow all through, are measured
  from their solid outline instead (`INSIDE` in `city.rs`),
  and the moor's hut is drawn as one from afar.
  Filling what rays from outside can't reach through gaps finer than a cell
  (a closing) before measuring would keep them whole,
  but grew the valley's houses over the streets when tried.
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
  exclude each other.
  `Claims` keep things of one plan apart ("Claims" in "Composing"),
  and rooms things of one `Points` ("Rooms");
  populations' anchors could claim the same way, against each other's.
