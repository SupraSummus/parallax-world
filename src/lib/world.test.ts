import { describe, it, expect } from 'vitest'
import { forest } from './forest'
import { generate } from './generator'
import { GRID_LEVELS } from './slice-selection'
import { generatePart, World, WORLD_TYPES } from './world'

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

  it("puts a generated world's grids together from parts generated alone, in any order", () => {
    // The coarsest level: the cheapest to generate.
    const level = GRID_LEVELS - 1
    const world = new World(7, 'forest')
    const parts = world.missingParts().filter(p => p.level === level)
    expect(parts.length).toBeGreaterThan(1)
    const received = [...parts].reverse().map(part => world.receive(part, generatePart(7, 'forest', part)))
    expect(world.missingParts().some(p => p.level === level)).toBe(false)
    const grid = received[received.length - 1]
    const { generator, box } = forest(7)
    const whole = generate(generator, box(level), level)
    let voxels = 0
    whole.forEach((x, y, z, id) => {
      voxels++
      expect(grid?.get(x, y, z)).toBe(id)
    })
    grid?.forEach(() => { voxels-- })
    expect(voxels).toBe(0)
  })

})
