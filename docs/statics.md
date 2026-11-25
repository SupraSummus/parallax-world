# Statics

An experiment (the crate `statics/`, not part of the engine yet):
does what stands in a world hold, and what falls when it does not?
A tree cut through at its foot should fall,
a house should come down with its columns,
nothing should hang on nothing,
and no beam should reach further than its material lets it.
The lattice stays where it is; matter moves between its sites.
Dynamics matters only as far as it brings things to rest,
and things should be cheap to simulate however large they are,
in a world of voxels only ("A world without structures", below).
The experiments are `statics/examples/statics.rs`, one per argument;
here is what they found.

## The model

Every level is the same: a node is a face between two cells,
and a cell joins its faces by beams, one to a pair of them, up to 66
(`statics/src/model.rs`, `statics/src/lattice.rs`).
A beam is four bonds, springs at points of its section
that keep the section's area and second moment,
so it bears a moment as well as a pull,
and its stress is what it bears over its section.
Its share of its strength is the greatest of
tension, compression and shear, whichever way that stress lies.
A cell's weight hangs on its faces.
Strengths and stiffnesses are in pascals,
so cells of any size are the same material.

Level 0 is given, not found from anything finer:
a cell full of a material has a beam between every pair of its faces,
its section by the angle between them (`SECTION`), its strength the material's.
So level 0 needs nothing but what is in each cell;
it costs six nodes to a cell, where voxels were one.
Voxels joined on their faces, which level 0 was before,
held a stone beam out of a wall to 0.7 to 0.9 of beam theory's length;
a cell's faces joined by beams hold it to about 0.95.

What settles (`settle`):
the forces under weight;
every beam with a bond over its strength breaks, whole;
what no longer holds to the ground falls;
again, until nothing breaks.
A beam with only some of its bonds left
would be a ball joint, a mechanism statics cannot answer
(one cut tree took minutes to solve where it took a second);
broken whole, whatever is joined is joined rigidly.
Breaking everything over its strength at once,
rather than only what is near the worst,
gives the same answers in a few rounds instead of tens.

## The forces: all nodes, at once

Every node is in equilibrium under its weight and its bonds,
all at once (`statics/src/bodies.rs`):
conjugate gradients, preconditioned by V-cycles over bodies within bodies.
A body of level l is a connected piece of the nodes
whose cells lead to one site of 2^l·FCC,
each site leading to the nearest of the next level up, so bodies nest.
What a level leaves unbalanced goes up to its bodies, and comes back as their motions,
each body moving as a rigid one would,
smoothed by a step of what its neighbours ask
(smoothed aggregation, as algebraic multigrid has it);
a body's stiffness is what its parts' is under those motions;
the top level, a few bodies, is solved directly.
Rigid bodies alone are a poor guess at how a net of beams bends:
a tree of ten thousand cells took over 300 iterations, smoothed under 30,
and 4 s on four cores where it took 55 s.
Its verdicts were the same to a millionth of the load as to a trillionth.

## What was tried, and not kept

Each failed the same way: a coarse level holding forces,
not only finding them.
What a structure weighs, and where, adds up exactly from fine to coarse;
what it holds does not:
it is its weakest section's, which a coarse cell averages away.
These were measured with an earlier check of each face's corners.

Coarser voxels, each its cell's fill, were wrong both ways:
a stone beam that holds fell,
and so did an uncut tree, its trunk a few voxels joined at their corners.

Rigid bodies of level-0 voxels, with the faces between them kept,
are exact wherever the weak section lies between bodies:
two bodies split at a tree's cut said whether it falls
as all its voxels did in every case tried,
the upper one weighed from voxels as coarse as 1 m.
But a cut inside a body is invisible:
a tree cut 70 % through stood as bodies of any coarser level.
Refining where a face is loaded over a share of its strength
found it only at a quarter;
checking each body's voxels alone
under the forces on its boundary (global–local)
found it at 0.8, and refined only around what is loaded:
the soil overhanging a rock, a quarter as many bodies as voxels.
Both rest on a threshold, and break by approximate forces.

Forces from the top down, each body's parts rigid
and its boundary found before it,
leave every voxel in equilibrium:
so where nothing is over its strength, the structure stands
(a lower bound, as limit analysis has it),
which proved an uncut tree with a third of the work.
But bodies of a few voxels, star-shaped, are no rigid parts:
they bore what their joints do not,
and nearly everywhere else the structure had to be solved whole anyway.

## A world without structures

The engine knows no tree, tower or bridge:
voxels only, in a world as large as integers let it be,
and statics is to be had, as light is, around the camera,
fine near and coarse far,
where small collapses far off do not matter
and a tower bigger than the fine levels reach still falls on the coarse ones.

What holds is what lies deep in bulk (`hold_bulk`):
a voxel all of whose sites within a radius are filled
is ground that never moves.
A mountain holds what stands on it, a lattice tower never holds itself,
and nothing need say where the ground is.
Without it the world would fall whole:
soil gives way under its own weight a few metres down.

Each level is a statics of its own, by the same rules,
each cell keeping as many numbers as any other:
for each pair of its faces a beam's section and strength, up to 66 of each,
and what it weighs on each face.
Where a cell's numbers come from is the generator's to say,
as a coarse site's light is:
a forest may give its far trees as it likes,
or measure them from finer cells (`statics/src/gather.rs`),
as a scene does (`Far::measured`, docs/generators.md);
what weight loads them with, the engine finds.

Measured, a cell's beams are what its matter does between its faces:
each face held as one and moved alone along each axis,
the matter inside comes to rest where that leaves it
(one elimination of what is inside the cell),
what holds the faces is the cell's stiffness between them,
and the beams are those whose stiffness is nearest it.
Each beam pulled alone, its two faces apart,
takes as much of the finer matter's strength as it shows in its own,
so a cut inside a cell weakens the beams across it, and only those.
Matter that only lies near other matter in a cell does not join it,
so a truss stays a truss;
a beam is only between faces the cell's matter joins.
A cell is held if all in it is, a face if anything on it is
(a cell holding anything held, held, grew ground a cell a level,
up a column past a cut).

Faces held rigid stiffen a cell,
and leave the matter where two of them meet carrying all they move:
measured as it is, a full cell of stone was twice as stiff a level up
and half as strong, and so again the next level.
So a cell is measured against a full cell of its material,
beam by beam, by the angle between the faces it joins:
a full cell is its material at every level,
within a fifth two levels up,
and a generator may give a full cell as its material alone.

## What it may get wrong, and what not

Statics will be off from level 0, more so at coarser levels;
what matters is which errors are let in.
At each level, at its own resolution, these hold:
mass is kept (it may move, it is never lost);
a cell full of a material is that material;
every level follows the same rules, and gives the same answer twice;
taking matter away never makes a thing stronger,
and loading it more never helps it hold;
what a level sees apart stays apart,
but what is smaller than its sites may be taken into what is next to it,
a voxel hanging in the air held up by its supported neighbour;
and what a finer level decides is an edit, which no coarser level undoes:
near the camera that voxel falls, far off it may stay, unseen.
So too a cut thinner than a cell:
a face is one node, and bridges what is cut across it,
so a coarse level sees a cut only as far as its cells are cut;
a finer level sees it, and decides.
Taking a small thing into a large one is let in;
joining two large things that only lie side by side in a cell is not,
as it holds up what should fall:
a rigid cell does both, beams between a cell's faces only the first.

What should be right as it is, not to the number:
what is overloaded several times over falls,
what bears a fraction of its strength stands,
long thin cantilevers break,
a braced frame is much stiffer than one without braces,
and cutting the only support of something brings it down.
What may be off: the threshold itself, within two or three times,
so that a structure near its strength
may stand at one level and fall at another;
where and into how many pieces a thing breaks;
and small things far off.

A world as generated stands, at every level:
a coarse level's errors are in a structure before an edit as after it,
so what decides is how much an edit changes what it bears.
A leg cut from a lattice tower raised the most its members bear
by half at levels 0 to 1 m alike (level 0 voxels then),
though what they bore differed between levels up to four times;
at 2 m, where the cut is smaller than a cell, not at all.
So a beam's strength is to be no less than its state as generated needs,
with a margin;
then something generated near its strength
holds more than it would, before an edit brings it down.

## Edits, at every level

What an edit brings down, against level 0,
each level's strength no less than its world as generated needs, twice
(`as_generated`, `statics edits`);
the most of their strength anything uses at level 0 after the edit,
and the levels that let it fall:

| Edit | Used at 0 | 0 | 0.25 m | 0.5 m | 1 m | 2 m |
|---|---|---|---|---|---|---|
| a stone beam made 4 m | 0.55 | stands | stands | stands | stands | stands |
| made 6 m | 1.27 | stands | falls | falls | falls | falls |
| a slab, one column cut | 1.09 | stands | stands | stands | falls | stands |
| two columns cut | 8.4 | falls | falls | falls | falls | stands |
| a lattice tower, a leg cut | 0.46 | stands | stands | stands | stands | stands |
| two legs cut | 0.55 | stands | stands | half falls | stands | half falls |
| a tree cut 30 % | 0.39 | stands | stands | stands | stands | stands |
| cut 70 % | 2.9 | falls | stands | stands | stands | stands |
| cut through | | falls | falls | falls | stands | stands |
| a block hanging let go | | falls | stands | stands | stands | stands |

Where what is used is far from 1 and the thing is a few cells across, levels agree;
a steel tower stands on two legs, its members 0.3 m across,
and at cells larger than them half of it may fall.
What they miss is a cut thinner than a cell:
a face is one node, held as one when its cell is measured,
so a kerf across a face is bridged by it.
A trunk 0.6 m across cut 70 % stands from 0.25 m up,
its beams across the cut no weaker than the rest
(each beam pulled alone takes the strength of its matter as the work done in it,
mostly where the matter narrows; the most any finer bond used
was the edges where two rigid faces meet);
cut through, it stands from 1 m, where the trunk is smaller than a cell;
the block hangs on from 0.25 m, the gap under it an eighth of a metre.

## Where it is to go

What holds is still whatever is deep in bulk within a radius in metres;
by the same rule at every level, it would be in sites.
And the levels are to be tied as light's are:
the finer where it is made, the coarser beyond,
a finer level taking its boundary's forces from the coarser.

Level 0 as faces and beams costs six nodes to a cell:
a tree of ten thousand cells takes 4 s where its voxels took 2,
and as long to measure its cells a level up (`statics speed`).
Blocks in single precision, half as much to read, saved a thirtieth.
A coarser level measured from a finer one is also how to carry
a finer level's error to it and back (its cells' responses),
a multigrid the levels already are.
Only cells whose faces or loads changed need solving,
from the forces they had.
What falls is taken off the lattice whole; moving it is not done.
