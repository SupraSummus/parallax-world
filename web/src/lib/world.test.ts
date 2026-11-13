import { describe, it, expect, vi } from 'vitest'
import { VoxelGrid } from './voxel-grid'
import { generatePart, World, type GridPart, type Vec3 } from './world'
import { generated } from './worldgen'
import { GRID_REACH } from './slice-selection'

// A small stand-in for the forest: these tests are about generating in parts.
vi.mock('./worldgen', async () => (await import('./small-forest.test-helper')).smallWorldgen)

describe('World', () => {
  /** Where (x, z) of `level` is within GRID_REACH of `camera`. */
  const near = ({ x, z }: Vec3, level: number) => (gx: number, gz: number) => Math.hypot(gx - x / 2 ** level, gz - z / 2 ** level) < GRID_REACH
  /** Whether `grid` holds what generating its box whole at `level` gives, within GRID_REACH of `camera`. */
  const generatedWhole = (grid: VoxelGrid | undefined, camera: Vec3, level: number) => {
    if (!grid) return false
    const { minX, minY, minZ, nx, ny, nz } = grid
    const box = { minX, minY, minZ, maxX: minX + nx, maxY: minY + ny, maxZ: minZ + nz }
    const whole = VoxelGrid.compose(box, grid.materials, [generated('forest', 7).generate(box, level)])
    const within = near(camera, level)
    let [voxels, wrong] = [0, 0]
    whole.forEach((x, y, z, id) => {
      if (!within(x, z)) return
      voxels++
      if (grid.get(x, y, z) !== id) wrong++
    })
    grid.forEach((x, _, z) => { if (within(x, z)) voxels-- })
    return voxels === 0 && wrong === 0
  }
  /** Whether `grid` holds the square GRID_REACH around `camera` at `level`. */
  const holds = (grid: VoxelGrid | undefined, { x, z }: Vec3, level: number) => {
    const [cx, cz] = [x / 2 ** level, z / 2 ** level]
    return !!grid && grid.minX <= cx - GRID_REACH && grid.minX + grid.nx >= cx + GRID_REACH && grid.minZ <= cz - GRID_REACH && grid.minZ + grid.nz >= cz + GRID_REACH
  }

  it("puts a generated world's grids together from parts generated alone, in any order", () => {
    const level = 0
    const world = new World(7, 'forest')
    const parts = world.missingParts(world.spawn).filter(p => p.level === level)
    expect(parts.length).toBeGreaterThan(1)
    const received = [...parts].reverse().map(part => world.receive(part, generatePart(7, 'forest', part)))
    expect(world.missingParts(world.spawn).some(p => p.level === level)).toBe(false)
    const grid = received[received.length - 1]
    expect(holds(grid, world.spawn, level)).toBe(true)
    expect(generatedWhole(grid, world.spawn, level)).toBe(true)
  })

  it('moves its grids with the camera, generating only the tiles they lack', () => {
    const world = new World(7, 'forest')
    const receive = (parts: GridPart[]) => parts.map(part => world.receive(part, generatePart(7, 'forest', part)))
    const first = world.missingParts(world.spawn)
    receive(first)
    const { x, y, z } = world.spawn
    // A little way: the grids stay.
    expect(world.missingParts({ x: x + 40, y, z: z - 40 })).toEqual([])
    // Farther: level 0's grid moves, the coarser ones' reach far enough.
    const camera = { x: x + 200, y, z }
    const parts = world.missingParts(camera)
    expect(parts.length).toBeGreaterThan(0)
    expect(parts.every(p => p.level === 0)).toBe(true)
    expect(parts.length).toBeLessThan(first.filter(p => p.level === 0).length / 2)
    // Asked for once.
    expect(world.missingParts(camera)).toEqual([])
    const grid = receive(parts).find(Boolean)
    expect(grid).toBe(world.level(0))
    expect(holds(grid, camera, 0)).toBe(true)
    expect(generatedWhole(grid, camera, 0)).toBe(true)
  })
})
