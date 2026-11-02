import { describe, it, expect } from 'vitest'
import { forest, UNIT_SIZE } from './forest'
import { generate } from './generator'
import { isSite, type Voxel } from './voxel-grid'

// 96 m × 160 m ahead of the spawn point, all heights. Generation takes a
// while; every test only reads the result.
const generated = (seed: number) => {
  const { generator, spawn, box } = forest(seed)
  const { minY, maxY } = box(0)
  // Grids start on an even x.
  const x = 2 * Math.floor((spawn.x - 48 / UNIT_SIZE) / 2)
  const area = { minX: x, minY, minZ: spawn.z - 1 / UNIT_SIZE, maxX: x + 96 / UNIT_SIZE, maxY, maxZ: spawn.z + 159 / UNIT_SIZE }
  return { grid: generate(generator, area), spawn }
}
const { grid, spawn } = generated(42)
const voxels = grid.toVoxels()
const key = (x: number, y: number, z: number) => `${String(x)},${String(y)},${String(z)}`

describe('forest', () => {
  // The 3D viewer rebuilds the app's world from its seed.
  it('is deterministic for a seed', () => {
    const again = generated(42)
    expect(again.spawn).toEqual(spawn)
    expect(again.grid.materials).toEqual(grid.materials)
    // toEqual on millions of sites is slow.
    expect(again.grid.cells.every((id, i) => id === grid.cells[i])).toBe(true)
  })

  it('has ground under every column of a 96 m × 160 m area', () => {
    const columns = new Set(voxels.map(v => `${String(v.x)},${String(v.z)}`))
    expect(columns.size).toBe((96 / UNIT_SIZE) * (160 / UNIT_SIZE))
  })

  it('grows trees between 10 m and 25 m tall', () => {
    // Longest vertical run of wood in any column: its sites are every other y.
    const wood = voxels.filter(v => v.type === 4).sort((a, b) => a.x - b.x || a.z - b.z || a.y - b.y)
    let longest = 0
    let run = 0
    wood.forEach((v, i) => {
      const prev = wood[i - 1] as Voxel | undefined
      run = prev?.x === v.x && prev.z === v.z && prev.y === v.y - 2 ? run + 1 : 1
      longest = Math.max(longest, run)
    })
    expect(2 * longest * UNIT_SIZE).toBeGreaterThan(10)
    expect(2 * longest * UNIT_SIZE).toBeLessThan(25)
  })

  it('starts the camera in open air with a clear view ahead', () => {
    const cells = new Set(voxels.map(v => key(v.x, v.y, v.z)))
    // The sites around the camera, 4 m ahead.
    for (let dz = 0; dz < 4 / UNIT_SIZE; dz++) {
      for (let dx = -1; dx <= 1; dx++) {
        for (let dy = -1; dy <= 1; dy++) {
          if (!isSite(spawn.x + dx, spawn.y + dy, spawn.z + dz)) continue
          expect(cells.has(key(spawn.x + dx, spawn.y + dy, spawn.z + dz))).toBe(false)
        }
      }
    }
  })
})
