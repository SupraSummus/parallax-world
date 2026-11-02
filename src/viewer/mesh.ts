import type { Vec3, World } from '@/lib/world'
import { faceShade } from '@/lib/bake'
import { VOXEL_FACES } from '@/lib/fcc'

export interface VoxelMesh {
  /** xyz per vertex, in three.js space: (x, y, -z), so the default -z view looks along world +z. */
  positions: Float32Array
  /** rgb per vertex (0–255), with fixed per-face shading baked in. */
  colors: Uint8Array
  indices: Uint32Array
  faceCount: number
}

// Each face's corners in three.js space (z negated), counter-clockwise seen
// from outside: negating z mirrors, so the world's order is reversed.
const FACES = VOXEL_FACES.map(({ normal: [a, b, c], corners }) => ({
  normal: [a, b, c],
  shade: faceShade({ x: a, y: b, z: c }),
  corners: [...corners].reverse().map(([x, y, z]) => [x, y, -z]),
}))

/**
 * Builds a triangle mesh of every voxel face not covered by a neighbour,
 * of the voxels of the world's level 0 within `reach` of `around` in x and z
 * (all of them by default).
 * Voxel (x, y, z) is the rhombic dodecahedron around site (x, y, z)
 * (see fcc.ts), as in the parallax renderer.
 */
export function buildVoxelMesh(world: World, around: Vec3 = { x: 0, y: 0, z: 0 }, reach = Infinity): VoxelMesh {
  const grid = world.grid
  // Each face not covered by a neighbour: f(x, y, z, id, face index).
  const eachFace = (f: (x: number, y: number, z: number, id: number, face: number) => void) => {
    grid.forEach((x, y, z, id) => {
      if (Math.abs(x - around.x) > reach || Math.abs(z - around.z) > reach) return
      FACES.forEach(({ normal: [a, b, c] }, k) => {
        if (grid.get(x + a, y + b, z + c) === 0) f(x, y, z, id, k)
      })
    })
  }
  let faceCount = 0
  eachFace(() => { faceCount++ })

  const positions = new Float32Array(faceCount * 12)
  const colors = new Uint8Array(faceCount * 12)
  const indices = new Uint32Array(faceCount * 6)
  const rgbs = grid.materials.map(({ color }) => [1, 3, 5].map(i => parseInt(color.slice(i, i + 2), 16)))
  let n = 0
  eachFace((x, y, z, id, f) => {
    const { shade, corners } = FACES[f]
    corners.forEach(([cx, cy, cz], k) => {
      const o = (n * 4 + k) * 3
      positions[o] = x + cx
      positions[o + 1] = y + cy
      positions[o + 2] = -z + cz
      for (let c = 0; c < 3; c++) colors[o + c] = Math.round(rgbs[id][c] * shade)
    })
    const base = n * 4
    indices.set([base, base + 1, base + 2, base, base + 2, base + 3], n * 6)
    n++
  })
  return { positions, colors, indices, faceCount }
}
