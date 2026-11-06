/**
 * Walking on the ground (bake/src/walk.rs), on this thread's own
 * WebAssembly instance: a body the camera is the eye of, on its world's
 * grid of one level, copied in whenever the world replaces it.
 */
import { writeGrid, type VoxelGrid } from './voxel-grid'
import { copyIn, copyOut, wasmInstance } from './wasm'
import type { Vec3, World } from './world'

interface Exports {
  memory: WebAssembly.Memory
  walk_start: (x: number, y: number, z: number) => number
  walk_materials: (count: number) => number
  walk_grid: (minX: number, minY: number, minZ: number, nx: number, ny: number, nz: number, used: number) => number
  walk_cells: () => number
  walk_step: (forward: number, right: number, yaw: number, jump: boolean, sneak: boolean, dt: number) => number
}

/** Held keys: forward and right each −1, 0 or 1, along `yaw` (degrees, as view.ts's). */
export interface WalkInput {
  forward: number
  right: number
  yaw: number
  jump: boolean
  sneak: boolean
}

export class Walker {
  private readonly wasm = wasmInstance() as unknown as Exports
  private readonly level: number
  private grid: VoxelGrid | undefined
  /** Whether it stays put while nothing is held; false until stepped. */
  settled = false

  /** A body with its eye at `eye`, about to fall. */
  constructor(eye: Readonly<Vec3>) {
    this.level = this.wasm.walk_start(eye.x, eye.y, eye.z)
  }

  /** Its eye `dt` seconds on, holding `input`, on `world`'s ground. */
  step(input: WalkInput, dt: number, world: World): Vec3 {
    const { wasm } = this
    const grid = world.level(this.level)
    if (grid && grid !== this.grid) {
      this.grid = grid
      copyIn(wasm.memory, wasm.walk_materials(grid.materials.length) >>> 0, Uint8Array.from(grid.materials, m => (m.walkThrough ? 1 : 0)))
      writeGrid(wasm.memory, grid, (...box) => wasm.walk_grid(...box) >>> 0, () => wasm.walk_cells() >>> 0)
    }
    const { forward, right, yaw, jump, sneak } = input
    const [x, y, z, settled] = copyOut(wasm.memory, Float64Array, wasm.walk_step(forward, right, yaw, jump, sneak, dt) >>> 0, 4)
    this.settled = settled === 1
    return { x, y, z }
  }
}
