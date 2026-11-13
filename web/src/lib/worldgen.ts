/**
 * The worlds of worlds/ (Rust, web/wasm/src/lib.rs), as WORLDS lists
 * them (the forest, the valley: the forest with a town and fields, ...),
 * generated in any box alone, at any level of detail, in a WebAssembly
 * instance of their own. See docs/generators.md. No DOM.
 */
import { wasmInstance } from './wasm'
import { FACE_WORDS, type Box, type Material, type VoxelData } from './voxel-grid'
import type { Generated } from './world'

/** The worlds' exports (web/wasm/src/lib.rs). */
interface Exports {
  memory: WebAssembly.Memory
  worldgen_worlds: () => number
  worldgen_start: (world: number, seed: number) => number
  worldgen_heights: (level: number) => number
  worldgen_generate: (level: number, minX: number, minY: number, minZ: number, maxX: number, maxY: number, maxZ: number) => number
}

let names: string[] | null = null

/** The worlds' names, in the order of WORLDS (worlds/src/lib.rs). */
export function worlds(): readonly string[] {
  if (!names) {
    const { memory, worldgen_worlds } = wasmInstance() as unknown as Exports
    // Before reading memory.buffer: the call may grow (and so replace) it.
    const result = worldgen_worlds()
    const [, at, length] = new Float64Array(memory.buffer, result, 3)
    // Each name ends with a NUL.
    names = new TextDecoder().decode(new Uint8Array(memory.buffer, at, length)).split('\0').slice(0, -1)
  }
  return names
}

/** The world named `name` (one of worlds()) of `seed` (an integer). */
export function generated(name: string, seed: number): Generated {
  const world = worlds().indexOf(name)
  if (world < 0) throw new RangeError(`no world named ${name}`)
  const exports = wasmInstance() as unknown as Exports
  // Before reading memory.buffer: each call may grow (and so replace) it.
  const result = (at: number, count: number) => new Float64Array(exports.memory.buffer, at, count)
  const [x, y, z, count, colours] = result(exports.worldgen_start(world, seed), 5)
  const materials: Material[] = Array.from(new Uint32Array(exports.memory.buffer, colours, count), (word, id) => ({
    color: id === 0 ? '' : `#${(word & 0xffffff).toString(16).padStart(6, '0')}`,
    walkThrough: (word & (1 << 24)) !== 0,
  }))
  return {
    spawn: { x, y, z },
    materials,
    heights: level => {
      const [minY, maxY] = result(exports.worldgen_heights(level), 2)
      return { minY, maxY }
    },
    generate: ({ minX, minY, minZ, maxX, maxY, maxZ }: Box, level: number): VoxelData => {
      const [gx, gy, gz, nx, ny, nz, bricks, brickCount, cells, cellCount, faces, faceCount] = result(exports.worldgen_generate(level, minX, minY, minZ, maxX, maxY, maxZ), 12)
      const { buffer } = exports.memory
      // Copied out: the next call reuses the buffers.
      return {
        minX: gx, minY: gy, minZ: gz, nx, ny, nz,
        bricks: new Int32Array(buffer, bricks, brickCount).slice(),
        cells: new Uint8Array(buffer, cells, cellCount).slice(),
        faces: new Uint32Array(buffer, faces, faceCount * FACE_WORDS).slice(),
        materials,
      }
    },
  }
}
