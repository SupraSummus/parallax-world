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

type Triple = [number, number, number]

/**
 * A face of a voxel: its outward normal, which is also the offset of the
 * neighbour across it, and its four corners relative to the site,
 * counter-clockwise seen from outside. Corners 0 and 2 are on the voxel's
 * axes, 1 and 3 at (±½, ±½, ±½).
 */
interface VoxelFace {
  normal: Triple
  corners: [Triple, Triple, Triple, Triple]
}

const cross = (a: Triple, b: Triple): Triple => [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
const dot = (a: Triple, b: Triple) => a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
const minus = (a: Triple, b: Triple): Triple => [a[0] - b[0], a[1] - b[1], a[2] - b[2]]

function voxelFaces(): VoxelFace[] {
  const faces: VoxelFace[] = []
  // The face between axes i and j, towards (a, b) along them; k is the third axis.
  for (const [i, j, k] of [[0, 1, 2], [1, 2, 0], [0, 2, 1]]) {
    for (const a of [-1, 1]) {
      for (const b of [-1, 1]) {
        const at = (u: number, v: number, w: number): Triple => {
          const p: Triple = [0, 0, 0]
          p[i] = u
          p[j] = v
          p[k] = w
          return p
        }
        const normal = at(a, b, 0)
        let corners: VoxelFace['corners'] = [at(a, 0, 0), at(a / 2, b / 2, 0.5), at(0, b, 0), at(a / 2, b / 2, -0.5)]
        if (dot(cross(minus(corners[1], corners[0]), minus(corners[2], corners[0])), normal) < 0) {
          corners = [corners[0], corners[3], corners[2], corners[1]]
        }
        faces.push({ normal, corners })
      }
    }
  }
  return faces
}

/** The 12 faces of a voxel. */
export const VOXEL_FACES: readonly VoxelFace[] = voxelFaces()
