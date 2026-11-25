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

Voxels are rigid, joined by springs on the faces they share
(a rigid body–spring model, as used for fracture of concrete;
`statics/src/model.rs`).
A voxel of FCC is the Voronoi cell of its site,
the cell such models are built on.
Each face is four bonds, springs at points
that keep the rhombus's area and second moments,
so a face bears a moment as it would whole,
and a line of voxels is a beam, not a chain.
Strengths and stiffnesses are in pascals,
so voxels of any size are the same material.

A voxel's stress is what its bonds' forces add up to over its volume.
At a bond, the stress is its voxels',
but on its own face what the bond bears itself,
which keeps the bending within a voxel
(a voxel's mean stress alone made beams two voxels thick
twice as strong as they are).
Its share of the strength is the greatest of
tension, compression and shear, whichever way the stress lies.
A check of each face's own load was anisotropic:
the lattice's faces lie at 45° to the cube's axes,
so along those a beam was twice as strong as across them.

What settles (`settle`):
the forces under weight;
every face with a bond over its strength breaks, whole;
what no longer holds to the ground falls;
again, until nothing breaks.
A face with only some of its bonds left
would be a ball joint, a mechanism statics cannot answer
(one cut tree took minutes to solve where it took a second);
broken whole, whatever is joined is joined rigidly.
Breaking everything over its strength at once,
rather than only what is near the worst,
gives the same answers in a few rounds instead of tens.

A stone or masonry beam out of a wall
holds 0.7 to 0.9 of the length beam theory gives,
more the thicker it is (the wall's corner bears more than theory says);
wood, whose shear strength limits it, about two thirds.
A leaning tree falls once cut deep enough,
the shallower the more it leans:
leaning 3°, cut 80 to 90 % through; leaning 8°, 60 to 65 %
(`statics tree`).

## The forces: all voxels, at once

Every voxel is in equilibrium under its weight and its bonds,
all at once (`statics/src/bodies.rs`):
conjugate gradients, preconditioned by V-cycles over bodies within bodies.
A body of level l is a connected piece of the voxels
whose sites lead to one site of 2^l·FCC,
each site leading to the nearest of the next level up, so bodies nest.
What a level leaves unbalanced goes up as resultants on its bodies,
force and moment, and comes back as their rigid motions;
the bonds between two parts sum to one joint, a stiffness of their relative motion;
the top level, a few bodies, is solved directly.
The bodies only find the forces sooner:
a few times fewer iterations than each voxel's block alone,
about twice as fast for a tree of ten thousand voxels.

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

Each level is a statics of its own,
its voxels gathered from the level below (`coarser`):
one for each connected piece of the matter in a cell,
weighing what that matter weighs, centred where it is;
on each face between two pieces, four bonds,
which keep the area, the centre and the spread of the matter crossing it,
as a face of level 0 keeps its rhombus's.
So a coarse voxel of a lattice tower is light but holds as its members do,
and an edit changes only the cells it touches, at every level.
A cell joining everything in it, not its pieces,
made the tower a shell: its members bore a tenth of their load.

A tower 30 m high, of members 0.3 m across, a leg cut (`statics tower`):
at level 0, 34 000 voxels, it falls in five minutes;
gathered into 1 m voxels, 1 100, it falls in two seconds.
Every level sees what the cut does,
the most the members bear rising by half
(`statics members`);
but what they bear differs between levels by up to twice,
so a structure near its strength stands at one level and falls at another:
with steel weakened to fall only when cut,
levels of 0.25, 0.5 and 2 m felled it whole too.
Cells about a member's size see its pieces' corners;
cells larger than a panel make its members stiffer than they are.

## Where it is to go

What a cell's pieces do inside, a coarse level can only guess
as long as each piece is rigid.
A cell can be solved alone at the level below,
once when it is made or edited:
how its faces move together (its stiffness, numerically coarsened),
and the most stress inside it for each way they may be loaded.
Then a coarse level bears and breaks as its cells' insides would,
and levels agree where they overlap.
And the levels are to be tied as light's are:
the finer where it is made, the coarser beyond,
a finer level taking its boundary's forces from the coarser.

The V-cycles' rigid motions are a coarse guess at how bodies bend,
so iterations still grow with a structure's size;
smoothing them, as smoothed aggregation does, is a step too.
Only cells whose faces or loads changed need solving,
from the forces they had.
What falls is taken off the lattice whole; moving it is not done.
