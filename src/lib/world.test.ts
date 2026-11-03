import { describe, it, expect, vi } from 'vitest'
import { VoxelGrid } from './voxel-grid'
import { generatePart, World, WORLD_TYPES, type GridPart, type Vec3 } from './world'
import { forest } from './worldgen'

// A small stand-in for the forest: these tests are about generating in parts.
vi.mock('./worldgen', async () => ({ forest: (await import('./small-forest.test-helper')).smallForest }))

describe('World', () => {
  // The forest and the city are checked in their own tests; comparing them here is slow.
  const smallWorlds = WORLD_TYPES.map(t => t.value).filter(t => t !== 'forest' && t !== 'city')

  it('is deterministic for a given seed and type', () => {
    for (const value of smallWorlds) {
      const [a, b] = [new World(42, value).grid, new World(42, value).grid]
      // toEqual on a million sites is slow.
      expect(a.cells.length === b.cells.length && a.cells.every((id, i) => id === b.cells[i])).toBe(true)
    }
  })

  it('varies with the seed', () => {
    const [a, b] = [new World(42, 'hills_with_trees').grid, new World(99, 'hills_with_trees').grid]
    expect(a.cells.length === b.cells.length && a.cells.every((id, i) => id === b.cells[i])).toBe(false)
  })

  it.each(smallWorlds)('generates a %s world with a grass surface', worldType => {
    const world = new World(12345, worldType)
    expect(world.worldType).toBe(worldType)
    expect(world.voxels.some(v => v.type === 1)).toBe(true)
    // Columns of blocks (two units wide: sites 2b - 1 and 2b) are filled from
    // bedrock at block y = -5 within blocks x, z in [-40, 40); trees may
    // overhang the edge by their 2-block canopy radius.
    const block = (c: number) => Math.ceil(c / 2)
    let minY = Infinity
    let maxXZ = 0
    world.grid.forEach((x, y, z) => {
      minY = Math.min(minY, block(y))
      maxXZ = Math.max(maxXZ, Math.abs(block(x) + 0.5), Math.abs(block(z) + 0.5))
    })
    expect(minY).toBe(-5)
    expect(maxXZ).toBeLessThanOrEqual(42.5)
  })

  it('puts water around the island in the lake world', () => {
    const world = new World(12345, 'lake_with_island')
    // Top voxel of the column of sites at block (x, z).
    const surfaceAt = (x: number, z: number) =>
      world.voxels.filter(v => v.x === 2 * x && v.z === 2 * z).reduce((top, v) => (v.y > top.y ? v : top))
    expect(surfaceAt(15, 0).type).toBe(8) // water
    expect(surfaceAt(0, 0).type).not.toBe(8)
  })

  /** Whether `grid` holds what generating its box whole at `level` gives. */
  const generatedWhole = (grid: VoxelGrid | undefined, level: number) => {
    if (!grid) return false
    const { minX, minY, minZ, nx, ny, nz } = grid
    const box = { minX, minY, minZ, maxX: minX + nx, maxY: minY + ny, maxZ: minZ + nz }
    const whole = VoxelGrid.compose(box, grid.materials, [forest(7).generate(box, level)])
    let [voxels, wrong] = [0, 0]
    whole.forEach((x, y, z, id) => {
      voxels++
      if (grid.get(x, y, z) !== id) wrong++
    })
    grid.forEach(() => { voxels-- })
    return voxels === 0 && wrong === 0
  }
  /** Whether `grid` holds the box `reach` (smallForest's) around `camera` at `level`. */
  const holds = (grid: VoxelGrid | undefined, { x, z }: Vec3, level: number) => {
    const reach = 64
    const [cx, cz] = [x / 2 ** level, z / 2 ** level]
    return !!grid && grid.minX <= cx - reach && grid.minX + grid.nx >= cx + reach && grid.minZ <= cz - reach && grid.minZ + grid.nz >= cz + reach
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
    expect(generatedWhole(grid, level)).toBe(true)
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
    expect(generatedWhole(grid, 0)).toBe(true)
  })
})
