import { describe, it, expect } from 'vitest'
import { FACES } from './view'
import { LayerIndex, VoxelGrid, isSite } from './voxel-grid'
import { hills } from './hills.test-helper'
import { voxelsOf } from './scene.test-helper'

const grid = hills(7).levelAround(0, { x: 0, y: 0, z: 0 })
const voxels = voxelsOf(grid)

describe('VoxelGrid', () => {
  it('stores the sites of its box, in bricks of those it has voxels in, and nothing else', () => {
    const grid = new VoxelGrid({ minX: -3, minY: 0, minZ: 0, maxX: 20, maxY: 3, maxZ: 2 }, [])
    // Widened to start and end on an even x.
    expect([grid.minX, grid.nx]).toEqual([-4, 24])
    const sites: [number, number, number][] = []
    for (let z = -1; z < 3; z++) {
      for (let y = -1; y < 4; y++) {
        for (let x = -5; x < 21; x++) sites.push([x, y, z])
      }
    }
    sites.forEach(([x, y, z], n) => { grid.set(x, y, z, 1 + (n % 200)) })
    sites.forEach(([x, y, z], n) => {
      const inside = x >= -4 && x < 20 && y >= 0 && y < 3 && z >= 0 && z < 2 && isSite(x, y, z)
      expect(grid.get(x, y, z)).toBe(inside ? 1 + (n % 200) : 0)
    })
    // Three bricks along x, all with voxels; clearing one site keeps its brick.
    expect([...grid.bricks]).toEqual([0, 1, 2])
    grid.set(0, 0, 0, 0)
    expect(grid.get(0, 0, 0)).toBe(0)
    expect(new VoxelGrid({ minX: 0, minY: 0, minZ: 0, maxX: 64, maxY: 64, maxZ: 64 }, []).bricks.every(b => b === -1)).toBe(true)
  })
})

const dot = (p: { x: number; y: number; z: number }, v: { x: number; y: number; z: number }) => p.x * v.x + p.y * v.y + p.z * v.z

describe('LayerIndex', () => {
  const index = LayerIndex.of(grid)
  const along = (p: { x: number; y: number; z: number }, face: (typeof FACES)[number]) =>
    ({ a: dot(p, face.lattice.right), b: dot(p, face.lattice.up), c: dot(p, face.lattice.forward) })

  it('bounds b of the voxels in a box of layers and columns, by whole blocks of columns', () => {
    let [some, none] = [0, 0]
    let seed = 3
    const random = () => (seed = (seed * 16807) % 2147483647) / 2147483647
    for (const face of FACES) {
      const placed = voxels.map(v => along(v, face))
      for (let n = 0; n < 40; n++) {
        const [c0, a0] = [Math.floor(random() * 300) - 150, Math.floor(random() * 300) - 150]
        const [c1, a1] = [c0 + 1 + Math.floor(random() * 40), a0 + 1 + Math.floor(random() * 60)]
        const range = index.range(face.lattice, c0, c1, a0, a1)
        const inBox = placed.filter(({ a, c }) => c >= c0 && c < c1 && a >= a0 && a < a1).map(p => p.b)
        // Whole blocks: some voxels a little outside [a0, a1) may count, never ones of other layers.
        const near = placed.filter(({ a, c }) => c >= c0 && c < c1 && a >= a0 - 8 && a < a1 + 8).map(p => p.b)
        if (inBox.length === 0 && !range) { none++; continue }
        expect(range).not.toBeNull()
        if (!range) continue
        some++
        expect(range[0]).toBeLessThanOrEqual(Math.min(...inBox))
        expect(range[1]).toBeGreaterThanOrEqual(Math.max(...inBox))
        expect(range[0]).toBeGreaterThanOrEqual(Math.min(...near))
        expect(range[1]).toBeLessThanOrEqual(Math.max(...near))
      }
    }
    expect(some).toBeGreaterThan(20)
    expect(none).toBeGreaterThan(20)
  })

  it('is the same wherever the grid is, far from the origin too', () => {
    // A lattice vector past 2¹⁵ along x and z, the faces' up for eight of them.
    const shift = { x: 40000, y: 0, z: -40000 }
    const moved = new VoxelGrid(
      { minX: grid.minX + shift.x, minY: grid.minY, minZ: grid.minZ + shift.z, maxX: grid.minX + grid.nx + shift.x, maxY: grid.minY + grid.ny, maxZ: grid.minZ + grid.nz + shift.z },
      grid.materials,
    )
    for (const v of voxels) moved.set(v.x + shift.x, v.y, v.z + shift.z, grid.get(v.x, v.y, v.z))
    const far = LayerIndex.of(moved)
    for (const face of FACES) {
      const [a, b, c] = [face.lattice.right, face.lattice.up, face.lattice.forward].map(v => dot(shift, v))
      for (let c0 = -150; c0 < 150; c0 += 7) {
        for (let a0 = -150; a0 < 150; a0 += 24) {
          const range = index.range(face.lattice, c0, c0 + 7, a0, a0 + 24)
          const moved = far.range(face.lattice, c0 + c, c0 + c + 7, a0 + a, a0 + a + 24)
          expect(moved).toEqual(range && [range[0] + b, range[1] + b])
        }
      }
    }
  })

  it('is as large as the voxels, not the box: tiles span the world heights', () => {
    const tables = (maxY: number) => {
      const grid = new VoxelGrid({ minX: 0, minY: 0, minZ: 0, maxX: 32, maxY, maxZ: 32 }, [{ color: '' }, { color: '#ff0000' }])
      for (let x = 0; x < 32; x++) {
        for (let z = 0; z < 32; z++) grid.set(x, (x + z) % 2, z, 1)
      }
      return LayerIndex.of(grid).tables
    }
    expect(tables(1024)).toEqual(tables(16))
  })
})
