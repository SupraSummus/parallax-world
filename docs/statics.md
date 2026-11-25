# Statics

An experiment (the crate `statics/`, not part of the engine yet):
does what stands in a world hold, and what falls when it does not?
A tree cut through at its foot should fall,
a house should come down with its columns,
nothing should hang on nothing,
and no beam should reach further than its material lets it.
The lattice stays where it is; matter moves between its sites.
Dynamics matters only as far as it brings things to rest,
and things should be cheap to simulate however large they are.
The experiments are `statics/examples/statics.rs`, one per argument;
here is what they found.

## The model

Rigid voxels joined by springs on the faces they share
(a rigid body–spring model, as used for fracture of concrete),
each face carrying a force and a moment
(`statics/src/model.rs`).
A voxel of FCC is the Voronoi cell of its site,
the cell such models are built on,
and every face is a rhombus whose area and second moments are known.
A face breaks once the stress at one of its corners,
or its shear, passes its material's strength;
what is no longer joined to held ground falls.
Strengths and stiffnesses are in pascals,
so voxels of any size are the same material.

Faces carry moments, so a line of single voxels is a beam, not a chain,
and how far a beam reaches comes out of its section and its material:
a stone beam out of a wall
held between 0.6 and 0.9 of what beam theory gives,
closer the more voxels across it
(two, four, eight across: about 0.67, 0.82, 0.90).
Masonry on mortar, half a metre thick, reaches about a metre.

## Coarser, by bodies, fails

The same scenes in voxels of level 1 or 2, each its cell's fill,
were wrong both ways:
a stone beam that holds at level 0 fell,
and so did a whole uncut tree,
as its trunk turned into a few voxels joined at their corners.

Rigid bodies made of level-0 voxels
(a cell of a coarser level, split into its connected pieces;
`statics/src/coarse.rs`),
with the fine faces between bodies kept as they are,
are exact wherever a weak section lies between two bodies.
But a cut inside a body is invisible:
a tree cut through 70 % at its foot, which falls at level 0,
stood as bodies of any coarser level.
Refining the bodies on either side of every face loaded
over a share θ of its strength
found the cut only with θ as low as a quarter:
the coarse bodies are stiffer around the cut, so they load it less.

## Loads gather, strength does not

What a structure weighs and where its weight is
adds up exactly from fine voxels to coarse ones,
as the levels' averages do.
What it holds does not:
it is the strength of its weakest section,
which a coarse cell averages away.
So the forces can come from far and coarse,
but the strength must be read where it is, fine.

Two rigid bodies, the tree above its cut and below,
joined by the faces left in the cut,
said whether the tree falls as level 0 does
in every case tried
(leaning 3° and 8°, cut from either side, 10 % to 95 % deep).
The body above took its weight from voxels as coarse as 1 m
with the same answers; from 2 m, for a crown 4 m across, they shifted.

Checking each coarse body alone at level 0
under the forces its boundary carries in the coarse solution
(global–local, as engineers check a detail of a whole model;
`local_check`)
finds a cut inside it, at a threshold of 0.8.
Each body's check is small and stands alone,
so they can run on threads, and only where forces changed.
Starting from bodies of 1 m and refining where faces are loaded,
a rock 6 m across under a cap of soil overhanging it
lost what it does at level 0,
refined only around the overhang:
a quarter as many bodies as voxels, in under half the time.

## Falling

Breaking, each round, every face over its strength
(rather than only those near the worst)
gave the same answers in a few rounds instead of tens:
how a structure comes apart can be rough, so long as it does.

## What it costs

A solve is conjugate gradients, preconditioned by each voxel's block:
hundreds of iterations for a few thousand voxels,
many more for long slender beams
and for soft soil on stiff rock.
An additive preconditioner of the coarser bodies
(a residual's resultants per body, a rigid motion back)
barely helped.

## Where it is to go

Bodies split where a structure is narrow,
not along the cells of a level:
then a few bodies answer as all its voxels do.
Finding the narrow places, and keeping a body's narrowest section
along each direction for places whose level 0 is not made
(as light wants sites' shares by direction, [light.md](light.md)),
is the next step;
and a structure's state from edits made to a world
that never changes once generated.
