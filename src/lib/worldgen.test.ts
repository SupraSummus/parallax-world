import { describe, it, expect } from 'vitest'
import { VoxelGrid } from './voxel-grid'
import { generated, worlds } from './worldgen'

// worlds/tests check the worlds themselves; these check what crosses from their WebAssembly.
describe('worlds', () => {
  it('lists WORLDS, each started by its name and generated around its spawn point', () => {
    expect(worlds()).toEqual(expect.arrayContaining(['forest', 'valley']))
    for (const name of worlds()) {
      const { spawn, heights, generate } = generated(name, 7)
      // Coarse: moments for any world.
      const level = 6
      const [x, z] = [Math.floor(spawn.x / 2 ** level), Math.floor(spawn.z / 2 ** level)]
      const box = { ...heights(level), minX: x - 8, minZ: z - 8, maxX: x + 8, maxZ: z + 8 }
      expect(generate(box, level).cells.some(id => id !== 0), name).toBe(true)
    }
  })

  it('has no world of another name', () => {
    expect(() => generated('nowhere', 7)).toThrow(RangeError)
  })
})

describe('forest', () => {
  const forest = (seed: number) => generated('forest', seed)
  const { spawn, materials, heights, generate } = forest(7)
  // A coarse level: the ground around the spawn point in moments.
  const level = 4
  const s = 2 ** level
  const { minY, maxY } = heights(level)
  const [x, z] = [Math.floor(spawn.x / s), Math.floor(spawn.z / s)]
  const box = { minX: x - 16, minY, minZ: z - 16, maxX: x + 16, maxY, maxZ: z + 16 }

  it('gives its materials as colours, and the spawn point on a site above its ground', () => {
    expect(materials[0]).toEqual({ color: '' })
    for (const { color } of materials.slice(1)) expect(color).toMatch(/^#[0-9a-f]{6}$/)
    expect((spawn.x + spawn.y + spawn.z) & 1).toBe(0)
    let top = -Infinity
    VoxelGrid.compose(box, materials, [generate(box, level)]).forEach((x, y, z) => {
      if (Math.abs(x * s - spawn.x) <= s && Math.abs(z * s - spawn.z) <= s) top = Math.max(top, y * s)
    })
    expect(top).toBeGreaterThan(-Infinity)
    expect(spawn.y).toBeGreaterThan(top)
  })

  it('generates by its seed alone', () => {
    const cells = Array.from(generate(box, level).cells)
    expect(Array.from(forest(7).generate(box, level).cells)).toEqual(cells)
    expect(Array.from(forest(8).generate(box, level).cells)).not.toEqual(cells)
  })
})
