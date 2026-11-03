/**
 * The forest of worldgen/ (Rust, worldgen/src/wasm.rs): generated in any
 * box alone, at any level of detail, in a WebAssembly instance of its own.
 * See docs/generators.md. No DOM.
 */
import { wasmInstance } from './wasm'
import type { Box, Material, VoxelData } from './voxel-grid'
import type { Generated } from './world'

/** worldgen's exports (worldgen/src/wasm.rs). */
interface Exports {
  memory: WebAssembly.Memory
  forest_start: (seed: number) => number
  forest_heights: (level: number) => number
  forest_generate: (level: number, minX: number, minY: number, minZ: number, maxX: number, maxY: number, maxZ: number) => number
}

/** The forest of `seed` (an integer). */
export function forest(seed: number): Generated {
  const exports = wasmInstance() as unknown as Exports
  // Before reading memory.buffer: each call may grow (and so replace) it.
  const result = (at: number, count: number) => new Float64Array(exports.memory.buffer, at, count)
  const [x, y, z, reach, count, pairs] = result(exports.forest_start(seed), 6)
  const ids = new Uint32Array(exports.memory.buffer, pairs, count * 2)
  const materials: Material[] = Array.from({ length: count }, (_, id) => ({
    color: id === 0 ? '' : `#${ids[2 * id].toString(16).padStart(6, '0')}`,
    type: ids[2 * id + 1],
  }))
  return {
    spawn: { x, y, z },
    reach,
    materials,
    heights: level => {
      const [minY, maxY] = result(exports.forest_heights(level), 2)
      return { minY, maxY }
    },
    generate: ({ minX, minY, minZ, maxX, maxY, maxZ }: Box, level: number): VoxelData => {
      const [gx, gy, gz, nx, ny, nz, bricks, brickCount, cells, cellCount] = result(exports.forest_generate(level, minX, minY, minZ, maxX, maxY, maxZ), 10)
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
