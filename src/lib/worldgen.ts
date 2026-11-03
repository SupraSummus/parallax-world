/**
 * The worlds of worldgen/ (Rust, worldgen/src/wasm.rs): the forest, and
 * the valley (the forest with a town and fields), generated in any box
 * alone, at any level of detail, in a WebAssembly instance of their own.
 * See docs/generators.md. No DOM.
 */
import { wasmInstance } from './wasm'
import type { Box, Material, VoxelData } from './voxel-grid'
import type { Generated } from './world'

/** worldgen's exports (worldgen/src/wasm.rs). */
interface Exports {
  memory: WebAssembly.Memory
  worldgen_start: (world: number, seed: number) => number
  worldgen_heights: (level: number) => number
  worldgen_generate: (level: number, minX: number, minY: number, minZ: number, maxX: number, maxY: number, maxZ: number) => number
}

/** The forest of `seed` (an integer). */
export const forest = (seed: number) => generated(0, seed)
/** The valley of `seed`: the forest, with a town on its path and fields around it. */
export const valley = (seed: number) => generated(1, seed)

function generated(world: number, seed: number): Generated {
  const exports = wasmInstance() as unknown as Exports
  // Before reading memory.buffer: each call may grow (and so replace) it.
  const result = (at: number, count: number) => new Float64Array(exports.memory.buffer, at, count)
  const [x, y, z, reach, count, colours] = result(exports.worldgen_start(world, seed), 6)
  const materials: Material[] = Array.from(new Uint32Array(exports.memory.buffer, colours, count), (rgb, id) => ({
    color: id === 0 ? '' : `#${rgb.toString(16).padStart(6, '0')}`,
  }))
  return {
    spawn: { x, y, z },
    reach,
    materials,
    heights: level => {
      const [minY, maxY] = result(exports.worldgen_heights(level), 2)
      return { minY, maxY }
    },
    generate: ({ minX, minY, minZ, maxX, maxY, maxZ }: Box, level: number): VoxelData => {
      const [gx, gy, gz, nx, ny, nz, bricks, brickCount, cells, cellCount] = result(exports.worldgen_generate(level, minX, minY, minZ, maxX, maxY, maxZ), 10)
      const { buffer } = exports.memory
      // Copied out: the next call reuses the buffers.
      return {
        minX: gx, minY: gy, minZ: gz, nx, ny, nz,
        bricks: new Int32Array(buffer, bricks, brickCount).slice(),
        cells: new Uint8Array(buffer, cells, cellCount).slice(),
        materials,
      }
    },
  }
}
