# Mechanics

Matter that moves, bends and breaks, on the voxels:
the crate `mechanics/`, driven by `native/src/mechanics.rs`.
The lattice never moves; matter moves between its sites.
A tree standing, cut and toppling, and lying where it lands are one model,
and statics is its rest.
It is generic: it knows sites, their materials,
how much of each face holds on, and nothing of trees, nor of bodies.
The world is one lattice, and only the part of it awake is computed.
Where a world does not suit it, the world changes ("What it asks of worlds").

## The model

A site awake that holds matter is a cell, a finite element:
a cube with a node at each corner (`mechanics/src/element.rs`).
Two sites sharing a face are welded there by as much of the face as holds on
(`Lattice::joined`), and share its corners,
so a block of cells is an elastic solid and a line of cells a beam.
A site asleep that bears holds still, and holds the corners welded to it.
Each cell bends in a frame turned with its corners (co-rotation),
so turning costs nothing, however far.
A weld breaks where the stress across it,
over the share of it that holds on, passes its strength.
Pulled or pushed, the stress is the most at a corner, as bending has it;
slid, the mean over the face,
as a trilinear cube's shear at its corners in bending is its own, not the matter's.

Cells that meet unwelded push apart, as balls a cell across,
and hold against sliding as far as friction lets them.
A contact pushes no harder than its matter bears over a face (`CRUSH`):
pushed harder, it gives, as what crushes does,
rather than break what it is welded to.

A step is backward Euler over all nodes at once,
so wood takes steps as long as its motion allows,
not the 10⁻⁵ s an explicit step would want.
Damping slows what bends, not what moves as a whole.
The solve is conjugate gradients with V-cycles over the octree
(`mechanics/src/solve.rs`):
a level up, the nodes of an octree cell move as one rigid body,
so a tree falling is found at the level it is large at.
The levels are kept from step to step (`Kept`)
while the nodes and contacts are those they were made for,
none let go, and they solve about as fast as when new.

The nodes then move by their new speeds,
each piece's turn as a whole exactly, about its middle
or about where it is held,
and only what bends it along a line (`advance`).
Moved along a line, all that turns would be stretched each step,
by its turn in the step squared,
and damping lets that go only slowly:
a post toppling off an edge would be strained to breaking before it landed.

A cell whose middle is more than 0.6 of a cell off its site
moves to the next site, if that is free;
only which site shows it changes, not where it is.

## Awake and asleep

A cell slow for a while (`REST`), as are all about it, falls asleep:
it is written back to the world, which holds it still.
A site asleep wakes where what bears on it,
welded or touching, nears what it bears (`WAKE`).
A cut wakes what bears touching it across faces, held on or not,
over it and near it (`native/src/mechanics.rs`, `Around`):
a cut bears what is over it, so what lies lower stays asleep.
It wakes the nearest `BUDGET` cells,
at the finest level at which that is all of it, or the coarsest;
while something is awake, at its level.

## Every level alike

Level 0 is what is so; a coarser level reads it by the same rules.
A site of level ℓ is the cube of 2^ℓ level-0 sites a side it is (`View`):
it bears if any of it bears, is as heavy as what it holds,
and as stiff as the share of it that bears.
It holds on across a face by the share of the face's level-0 pairs joined;
at level 0 that is all or none.
What moves is written back to level 0, its sites moved by its cell's size,
letting go where its cell let go, and never holding on anew:
so a crack across part of a coarser face stays,
and a cut that parts level 0 parts every level.

Two things cannot be alike across levels:
a cell meets others as a ball a cell across, however little of it bears,
and a cube partly full bends as a full one of its stiffness,
so a beam thinner than its cells breaks later a level up.

## Holding on

What holds on to what is part of the matter,
so that a stack of blocks is not a trunk,
nor sand a rock, nor two stones that land on each other one.
Each level-0 site holds six bits, one per face (`Holds` in `lattice/`):
whether its matter holds on across that face.
Two sites are joined across a face where both hold on across it.
So each face's state is held twice, once by either side,
and a site's state is all its own:
it moves with its matter, as its colours do,
and no site ever writes another's.
A bit left over, pointing at something that came later, joins nothing,
unless what came holds on back.
The lattice never turns matter, so a face stays the face it was.

A cell holds on across the faces its welds lie across,
as its cells lie (`hold_on`),
so what turned holds on to whichever neighbours its welds now face.
What breaks lets go; what only touches never holds on.

Generated matter holds on across every face:
the worlds give no bodies yet.
Edits keep the bits of the sites they change (`worldgen/src/edits.rs`).

## In the engine

A cut (C) wakes what it sets moving,
each cell carrying the level-0 sites of its cube.
What bears is all wood; the worlds give no materials.
Loose matter (leaves) is not simulated:
it goes with the matter that bears nearest it,
and is crushed where it is carried onto matter that bears.
It runs on a thread of its own, with the world as that thread has it,
and the changes show when due.

## What it asks of worlds

- **Matter joined across faces.** What touches only at edges or corners falls apart.
- **What is built stands at its material's strength.**
  Nothing is made stronger for having been generated.

## By local rules

`local/` has this model's interface (`native/src/mechanics.rs` takes it by changing one import),
and is to take its place once it is fast enough.
In it, a cell's matter moves by rules that read it and its neighbours only.
Its tests take minutes, so pnpm leaves them out (`cargo test --release -p local`).
Of `native/src/mechanics.rs`'s tests it fails that of two cuts in a row;
that of a tree of the forest is too slow to run on it.

Every site holding matter is a cell: a rigid cube,
where its matter lies and how it is turned its own.
A cell asleep is as heavy as the world and holds still where its matter lies,
so falling asleep and waking change nothing its neighbours feel.
Between two cells is a spring.
Two that hold on across a face are bonded by a beam through it, a cell long,
its section as large as the share of the face that holds on,
anchored in each cell's own frame:
so a line of cells bends as beam theory has it, a block as its material,
and what turns as a whole strains nothing.
Two that do not hold on push apart as balls a cell across, with friction,
no faster than `PUSH_APART`: contacts are not elastic.
A contact acts where the balls meet, half way between the cells, as a bond does,
so what slides on something is turned by it.
Every spring is damped alike and has a strength:
past it, a bond breaks and a contact gives, as what crushes does.

What it gets wrong comes from those contacts and that damping.
A flat face stands on the tops of its balls:
a nudge slides it half a cell into the hollows between them,
a third of a cell lower, tilted where what it stands on is no wider.
Friction only drags against sliding, so what is pushed creeps.
Damped as a cell's swing is, what is many cells long rings for seconds.
So a post dropped onto its stump bounces and comes to rest leaning,
and, written back so, falls when the next cut wakes it.

A step is explicit, in rounds of two phases, on threads (`local/src/crew.rs`):
each spring works out what it does from its cells as they are,
then each cell moves by its springs.
The motion is the same on any number of threads.
Explicit, a round carries a push only a cell, at the speed of sound,
so it is as short as sound takes to cross a cell, microseconds in wood:
on four cores, a post cut through takes 45 times as long as with this model
([todo.md](todo.md), "Ideas").
Positional steps (XPBD), each spring put right in turn, take long steps,
but a beam out of a wall sagged a hundred times too far.
