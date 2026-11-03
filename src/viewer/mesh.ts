import type { Vec3, World } from '@/lib/world'
import { VOXEL_FACES } from '@/lib/fcc'

export interface VoxelMesh {
  /** xyz per vertex, in three.js space: (x, y, -z), so the default -z view looks along world +z. */
  positions: Float32Array
  /** rgb per vertex (0–255), with fixed per-face shading baked in. */
  colors: Uint8Array
  indices: Uint32Array
  faceCount: number
}

/**
 * Brightness of a face by its outward normal: the axes' shades weighted by
 * the squared components, as the baker shades (bake/src/tables.rs).
 */
function faceShade({ x, y, z }: Vec3): number {
  const [front, back, left, right, bottom, top] = [0.9, 0.65, 0.7, 0.8, 0.55, 1]
  return (x * x * (x < 0 ? left : right) + y * y * (y < 0 ? bottom : top) + z * z * (z < 0 ? front : back)) / (x * x + y * y + z * z)
}

// Each face's corners in three.js space (z negated), counter-clockwise seen
// from outside: negating z mirrors, so the world's order is reversed.
const FACES = VOXEL_FACES.map(({ normal: [a, b, c], corners }) => ({
  normal: [a, b, c],
  shade: faceShade({ x: a, y: b, z: c }),
  corners: [...corners].reverse().map(([x, y, z]) => [x, y, -z]),
}))

/**
 * Builds a triangle mesh of the voxels of the world's `level` of detail
 * (World.levelAround) around `around` (level 0 units), in its ring:
 * level 0 within `reach` of it in x and z (all of it by default),
 * each next level in a ring twice as wide, from where the one before ends,
 * as the parallax renderer draws farther voxels coarser.
 * Voxel (x, y, z) of level ℓ is the rhombic dodecahedron around site
 * 2^ℓ · (x, y, z) (see fcc.ts).
 * A face is drawn unless a neighbour in the ring covers it,
 * so each ring is closed where it meets the next.
 */
export function buildVoxelMesh(world: World, around: Vec3 = { x: 0, y: 0, z: 0 }, reach = Infinity, level = 0): VoxelMesh {
  const grid = world.levelAround(level, around)
  const scale = 2 ** level
  const inRing = (x: number, z: number) => {
    const d = Math.max(Math.abs(x * scale - around.x), Math.abs(z * scale - around.z))
    return d <= reach * scale && (level === 0 || d > (reach * scale) / 2)
  }
  // Each face drawn: f(x, y, z, id, face index).
  const eachFace = (f: (x: number, y: number, z: number, id: number, face: number) => void) => {
    grid.forEach((x, y, z, id) => {
      if (!inRing(x, z)) return
      FACES.forEach(({ normal: [a, b, c] }, k) => {
        if (grid.get(x + a, y + b, z + c) === 0 || !inRing(x + a, z + c)) f(x, y, z, id, k)
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
      positions[o] = (x + cx) * scale
      positions[o + 1] = (y + cy) * scale
      positions[o + 2] = (-z + cz) * scale
      for (let c = 0; c < 3; c++) colors[o + c] = Math.round(rgbs[id][c] * shade)
    })
    const base = n * 4
    indices.set([base, base + 1, base + 2, base, base + 2, base + 3], n * 6)
    n++
  })
  return { positions, colors, indices, faceCount }
}
