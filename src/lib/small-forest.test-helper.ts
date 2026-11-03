/** Tests' stand-in for worldgen.ts's forest: generated at every level like it, in grids small enough to take moments. */
import { VoxelGrid, type Box, type Material } from './voxel-grid'
import type { Generated } from './world'

const MATERIALS: Material[] = [{ color: '', type: 0 }, { color: '#4f7a2a', type: 1 }, { color: '#5a8a30', type: 1 }]

/** Gentle hills of moss, in two shades, held 64 sites of each level around the camera. */
export function smallForest(seed: number): Generated {
  return {
    materials: MATERIALS,
    generate: (box: Box, level: number) => {
      const s = 2 ** level
      const grid = new VoxelGrid(box, MATERIALS)
      for (let z = box.minZ; z < box.maxZ; z++) {
        for (let x = grid.minX; x < grid.minX + grid.nx; x++) {
          // In lattice units: sites of the level are s apart.
          const top = 1 + Math.round(8 * Math.sin((x * s) / 40) * Math.sin((z * s) / 40))
          const shade = 1 + ((((x * s) >> 3) ^ ((z * s) >> 3) ^ seed) & 1)
          grid.fillBox(x, Math.ceil(-8 / s), z, x + 1, Math.ceil(top / s), z + 1, shade)
        }
      }
      return grid
    },
    spawn: { x: 0, y: 6, z: -4 },
    reach: 64,
    heights: level => ({ minY: Math.floor(-8 / 2 ** level), maxY: Math.ceil(10 / 2 ** level) }),
  }
}
