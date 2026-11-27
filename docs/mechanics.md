# Mechanics

Matter that moves, bends and breaks, on the voxels:
the crate `mechanics/`, driven after a cut by `native/src/fall.rs`.
The lattice never moves; matter moves between its sites.
A tree standing, cut and toppling, and lying where it lands are one model,
and statics is its rest.
It is generic: it knows cells, their materials and what holds still,
and nothing of trees.
Where a world does not suit it, the world changes ("What it asks of worlds").

## The model

Each cell holding matter is a finite element:
a cube with a node at each corner, its stiffness from its material
(`mechanics/src/element.rs`).
Cells that share a face are welded there and share its corners,
so a block of cells is an elastic solid and a line of cells a beam.
What the world holds still holds the corners welded to it.
Each cell bends in a frame turned with its corners (co-rotation),
so turning costs nothing, however far.
A weld breaks where the stress across it passes its strength.

Cells that meet unwelded push apart, as balls a cell across,
and hold against sliding as far as friction lets them.

A step is backward Euler over all nodes at once,
so wood takes steps as long as its motion allows,
not the 10⁻⁵ s an explicit step would want.
Damping slows what bends, not what moves as a whole.
The solve is conjugate gradients with V-cycles over the octree
(`mechanics/src/solve.rs`):
a level up, the nodes of an octree cell move as one rigid body,
so a tree falling is found at the level it is large at.

A cell whose middle is more than 0.6 of a cell off its site
moves to the next site, if that is free;
only which site shows it changes, not where it is.

## In the engine

A cut (C) sets moving what bears and is joined to it across faces,
within 10 m across and 30 m above it.
What lies lower than the cut holds still,
as does what lies at the edge of what is looked at.
What bears is all wood; the worlds give no materials.
Loose matter (leaves) is not simulated:
it goes with the matter that bears nearest it.
Matter is simulated at the finest level at which it is few enough (`BUDGET`),
each cell carrying the level-0 sites of its cube.
It runs on a thread of its own, and the changes show when due.

## What it asks of worlds

- **Matter joined across faces.** What touches only at edges or corners falls apart.
- **What is built stands at its material's strength.**
  Nothing is made stronger for having been generated.
- **Coarser levels holding no more than level 0.**
  Where a coarser level holds more (a trunk thicker than it is),
  a cut that parts level 0 does not part it.
