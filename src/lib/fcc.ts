/**
 * Geometry of a voxel on the face-centred cubic (FCC) lattice. See
 * docs/architecture.md, section 6.
 *
 * Sites are the integer points (x, y, z) with x + y + z even (voxel-grid.ts).
 * A voxel is a site's Voronoi cell, a rhombic dodecahedron: the points p
 * with |px| + |py|, |py| + |pz| and |px| + |pz| all ≤ 1 relative to the
 * site. Its 12 faces are rhombi, one towards each nearest neighbour,
 * (±1, ±1, 0) and permutations, √2 away. A voxel's volume is 2. The world
 * axes are the lattice's 4-fold axes, so walls and floors along them look
 * the same in every direction. The renderer looks along the neighbour
 * directions instead (the faces of view.ts), where the lattice is a stack
 * of flat layers.
 */

/**
 * The 12 faces of a voxel by their outward normals, which are also the
 * offsets of the neighbours across them; a face's index is its kind
 * (VOXEL_FACES in bake/src/tables.rs, in the same order).
 */
export const VOXEL_FACES: readonly (readonly [number, number, number])[] = [
  [-1, -1, 0], [-1, 1, 0], [1, -1, 0], [1, 1, 0],
  [0, -1, -1], [0, -1, 1], [0, 1, -1], [0, 1, 1],
  [-1, 0, -1], [-1, 0, 1], [1, 0, -1], [1, 0, 1],
]
