# Checks and cost

How the worlds of [generators.md](generators.md) are held to what they promise:
the contract every world keeps, the checks of places and directions,
and what generating costs, counted.
Which tests run them is in [testing.md](testing.md).

## Checks

`worldgen::checks::contract` (`worldgen/src/checks/contract.rs`) checks a world
in regions holding everything it is made of around spawn,
which it finds itself (`worldgen::checks::survey`, `survey.rs`), not from a list:
`worlds/tests/contract.rs` runs it for each world in `WORLDS`
at seed 42 and three more, a thread each,
and `worldgen/tests/contract.rs` for test worlds of fog, caves,
and a village of houses with doors and tables
on y = 0, a plane of sites at every level ("Faces" in "Levels", generators.md);
and knobs anchored along x on a slanting wall,
one per square 2N wide across x.
`worldgen/tests/shapes.rs` checks each solid,
as many of one kind, of sizes and places drawn at random, on a ground:
bright on dark, the palette that hides the least:
a world's colours can only hide what breaks there.

The survey asks every layer of the generator around spawn,
drawing no voxels;
one drawing nothing there is asked around each place the world names, in turn,
so a feature too rare to be near spawn at every seed
(the valley's tunnels) is checked where the world says it is,
and only a layer drawing nowhere is silent.
Things come apart into parts no wider than a region (`REGION`),
named by their children's names, numbers left out (`towns/block/house`);
a dense population is surveyed over a smaller square than a sparse one (`PARTS`),
and one of small cells over fewer metres (`HALF_CELLS`).
Fields are asked at columns of a coarse level, and at level 0 there,
all of them at a column before the next, as they share its plans:
they keep what is finer than their sites in proportion ("Plans" in "Levels", generators.md),
so a railway, a kerb or a pond shows;
a field's place is the site asked, at its surface, as a part's is its bounds.
A stratum is a part or a field, with a kind it draws;
the survey counts its places, and keeps those nearest spawn (`PLACES`).
Every stratum found gets a region: the rarest first,
each in the region it widens least, or in one of its own nearest spawn.
Its cells are those in its places there,
so a rare thing is checked without being listed,
and in itself, not averaged with the rest of a region;
but a part's variants drawing the same kinds are one stratum,
and a part's kinds share its places.
A region holds every stratum surveyed in it, not only those it was laid out for
(`survey_in`: the survey again, there, keeping every place),
and a stratum's sample is shared by the regions holding it:
as many cells and parts as in one region,
but a common stratum's from wherever the rare ones took regions,
on slopes, by houses, in a forest,
not from the one setting it was laid out in.
A layer that draws nothing surveyed breaks the contract;
the kinds nothing draws are only printed,
as worlds share tables of kinds (the moor's is the valley's).
`CONTRACT_ONLY='towns/church,railway'` checks only the regions
of strata so named, and surveys only their layers.
A world's author works on one part at a time:
`worlds/examples/check.rs` checks a component (`check_component`),
a name in the tree of the world's things (`towns/block/church`).
At the few places of it nearest spawn that the survey finds,
a region each, it holds the cells in its places to what is in them,
the world drawn all around, as `check` does a stratum's;
its baseline is a snapshot outside git, taken before a change.
`alone`, the world is cut to it first (`Generator::alone`).
A coarse site is shared, so where it meets the rest its cells look otherwise
(on the moor's hut, by as much as the margins themselves):
what breaks alone is its own,
what breaks only in place is how it meets the rest.
Alone, its parents draw it at every spacing, their far forms measured from it;
a carve has nothing to cut, a paint paints over nothing.
A world's heights are its own, a ground's sky and the rock under it,
and an endless world has none;
the checks look at it in cubes around places
(regions, the survey's, places' and directions' around spawn),
each within its heights if it has them (`World::within`),
so a cube as wide as a ground's heights holds all of them,
and an endless world's is a cube of it.
y stays up, but some of the engine still takes a ground
("Assume no ground" in `docs/todo.md`).

The contract says three things.
A world is a function of position:
a box generated in parts is the box generated whole.
What a layer says of itself holds:
what it skips, where it is whole, its things' reach, the world's heights.
And a coarse site stands for finer ones:
each level's cells look, along each of the lattice's 12 directions,
as what is in them does the level two finer (`checks::cells`; "Levels" in generators.md).
Not what a ray through many cells sees:
that differs by what is inherent to seeing each cell as a whole.
A cell is one site, its look coarse,
so it is checked on average:
each stratum's cells, near a surface in its places, a sample, over its regions at every seed,
each region weighed by its cells
(drawn by where each cell is, so the same in any region holding it),
as far apart as `TOLERANCE` (0.045) in how much they stop,
or in the colour of what they stop (so a thin medium's colour counts as a solid's),
and each cell as far from what is in it as `SPREAD` (0.1), on average.
Those are the defaults (`Accuracy::DEFAULT`).
A world's entry in `WORLDS` may declare others by stratum (`accuracy`),
as it declares to which level it keeps it (`levels`):
one for a thing or a layer covers its parts, unless a part has its own
(`Accuracy::of`), and each is checked as closely as declared;
a name that covers no stratum breaks it, so a misspelt one is not lost.
A mean settles as seeds are added; the largest of many only grows.
Where a world, a test world or a solid breaks it,
its baseline says by how much, as lint baselines do:
`worlds/data/contract/<world>.txt`, `worldgen/data/contract/`
(`contract::hold_to_baseline`), a line per stratum and level that breaks it,
how far apart and how spread on average.
A test fails on a break not in it,
and on one further apart or more spread, or nearer or less, by more than `SLACK`,
or kept now: the baseline goes with the code it measures.
`CONTRACT_BLESS=1` writes the baselines anew
(`cargo test --release --test contract` in `worlds/`, or `--test contract --test shapes` in `worldgen/`).
Breaks of the rules that hold exactly have no baseline.
The worst case is each solid's alone (`shapes.rs`),
though its scenes ask little of it (`docs/todo.md`).

Rays see each site by its alpha (`VoxelGrid`'s):
each stops its share of what gets to it, and shows that much of its colour,
and what it glows (`composited` in `picture.rs`).
A ray that starts or ends inside a voxel stops as a medium would
over the part of its chord it crosses, 1 − (1 − alpha)^part:
counted whole, the voxel a coarse level is first looked at in
made coarse levels look denser near.
What rays see, a `Look`, is how much of them stops,
and what shows: each site's colour, linear, by how much of it the site stops,
and what glows, as in compositing (premultiplied).
A medium stops each channel its own share (`VoxelGrid`'s `medium`),
so a ray keeps what gets through of each,
and how much of it stops is of the channel most gets through in.
Hosts, light and these rays all take what the site holds.
Linear, as a coarse site averages finer ones;
with no sky behind, as over any sky
it differs by no more than its colour and how much it stops do.
A cell's rays run along the rows of the finer level's sites,
each crossing of a finer site filled in part counted as `CROSSING` (3/2):
through a row's middles a ray crosses fewer sites, by longer chords, than one at any offset.
A site holds what it looks like, not what it is made of:
how much of a ray it stops, what that shows and what it glows,
each the sum of what is in it (`see` in `generate.rs`),
a coarse site's colour its kinds' mean shades;
so a coarse level may draw one kind for another,
as the moor's heather for its sprigs, if it looks alike.
Hosts take what a site's pieces look like, through each face where it differs,
whole as often as they are seen there, by a site's coin
(`VoxelGrid::show`): noisy, and a site too thin to show still stops light.
What stops a walking body is apart from how a site looks:
the share of its rays what stops one stops, wherever in it (`VoxelGrid`'s `solid`);
a host's site stops a body where that is over half.
Tests and stats that ask which kind a site is
read it from its colour (`kind_of`): a mix of kinds is none.
Margins quoted elsewhere in this document are older ones,
in shares of material types or in sRGB.

The first two of these share a region's levels, each generated once (`hold_region`):
- at each level, the region against parts of it 32 sites a side (`parts_hold`),
  each generated alone:
  for each stratum a sample of those meeting its places (`PARTS`),
  drawn by where each is and shared by its regions, as its cells are:
  as many at every level and for every stratum,
  not all of a region's, hundreds at level 0 and a few at the coarsest.
  A field's places reach above and under its surface,
  where it says it draws nothing, or all of a kind.
  A seam, order dependence or hidden state shows as a differing site;
  and with what is skipped where it says it may not be drawn all the same
  (`checking_skipped`): it draws nothing there,
  what a field fills unasked, as all of a kind, is what it would draw,
  and a part `whole` says is empty or buried is.
  This finds what drawing can't see,
  as a plan steeper between columns no layer reads together;
- at each level up to the world's, against the level two finer,
  a sample of cells near a surface in each stratum's places in each region
  (`checks::cells`), the region grown by a site of the level, and to at least 16 across,
  each along each of the 12 directions, inside it only;
- at each level up to one past the world's, in each region
  and a host tile's width around spawn,
  a slab above the world's `heights`, if it has them, stops no rays at all:
  hosts generate only within them, and cut whatever is outside
  (under them, the ground goes on, unseen);
- the things of each population the survey found, of all heights,
  keep within its `reach`, which drawing checks only where it draws;
  and where a thing's bounds leave the heights, nothing is drawn outside.

A world's layout must not follow the grids it is generated on:
a town placed once per region, kept inside it,
is never where two regions meet.
`worldgen::checks::places` (`places.rs`) checks that things prefer no places.
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
(towns along the valley's course),
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
`worldgen::checks::directions` (`directions.rs`) measures it
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

With `--nocapture` it prints each level's margins;
`contract::in_regions` returns them as a `Report` to print,
each margin's looks at both levels.
A broken mean (`contract::breaks`) names the stratum and level,
and each region it was averaged over, at its seed.
The engine's tests also check that a thing's carves cut only the thing.
A new world is a crate of its own in `worlds/`, as the cheese is,
an entry in `WORLDS`,
and its name in `worlds!` in `worlds/tests/contract.rs`,
whose tests fail without it.
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

`cost::report` generates boxes of a world at several levels
around some places, each by a fresh world, as a host's first is:
cubes of the lattice, four of native's tiles a side (`tiles::tile_box`),
as large as its tiles were.
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

`worlds/tests/cost.rs` holds each world in `WORLDS`,
at every level a host makes, odd ones too,
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
