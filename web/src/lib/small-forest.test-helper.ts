/** Tests' stand-in for worldgen.ts's forest: generated at every level like it, in grids small enough to take moments. */
import { VoxelGrid, type Box, type Material } from './voxel-grid'
import type { Generated } from './world'

const MATERIALS: Material[] = [{ color: '' }, { color: '#4f7a2a' }, { color: '#5a8a30' }]

// Every test makes the same few worlds: each box is generated once, and shared.
// Callers only read it (VoxelGrid.compose copies it).
const generated = new Map<string, VoxelGrid>()

/** Gentle hills of moss, in two shades, held 64 sites of each level around the camera. */
export function smallForest(seed: number): Generated {
  // All a seed changes.
  const parity = seed & 1
  return {
    materials: MATERIALS,
    generate: (box: Box, level: number) => {
      const key = [parity, level, box.minX, box.minY, box.minZ, box.maxX, box.maxY, box.maxZ].join()
      const known = generated.get(key)
      if (known) return known
      const s = 2 ** level
      const grid = new VoxelGrid(box, MATERIALS)
      for (let z = box.minZ; z < box.maxZ; z++) {
        for (let x = grid.minX; x < grid.minX + grid.nx; x++) {
          // In lattice units: sites of the level are s apart.
          const top = 1 + Math.round(8 * Math.sin((x * s) / 40) * Math.sin((z * s) / 40))
          const shade = 1 + ((((x * s) >> 3) ^ ((z * s) >> 3) ^ parity) & 1)
          for (let y = Math.ceil(-8 / s); y < Math.ceil(top / s); y++) grid.set(x, y, z, shade)
        }
      }
      generated.set(key, grid)
      return grid
    },
    spawn: { x: 0, y: 6, z: -4 },
    heights: level => ({ minY: Math.floor(-8 / 2 ** level), maxY: Math.ceil(10 / 2 ** level) }),
  }
}

/** Tests' stand-in for worldgen.ts (vi.mock): the forest and the valley, each smallForest. */
export const smallWorldgen = {
  worlds: () => ['forest', 'valley'],
  generated: (_name: string, seed: number) => smallForest(seed),
}
