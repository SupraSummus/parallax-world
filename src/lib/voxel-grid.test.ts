import { describe, it, expect } from 'vitest'
import { FACES } from './view'
import { VOXEL_FACES } from './fcc'
import { coarsen, LayerIndex, VoxelGrid, isSite } from './voxel-grid'
import { hills } from './hills.test-helper'

const { grid, voxels } = hills(7)

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

  it('round-trips voxels, and refuses points that are not sites', () => {
    const list = [{ x: 0, y: 0, z: 0, color: '#ff0000' }, { x: 3, y: -1, z: 2, color: '#00ff00' }]
    expect(VoxelGrid.fromVoxels(list).toVoxels()).toEqual(list)
    expect(() => VoxelGrid.fromVoxels([{ x: 1, y: 0, z: 0, color: '#ff0000' }])).toThrow(RangeError)
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

describe('coarsen', () => {
  const RED = { color: '#ff0000' }
  const BLUE = { color: '#0000ff' }
  /** A grid of the sites in a box that `fill` gives a material (1 red, 2 blue) or 0. */
  const made = (fill: (x: number, y: number, z: number) => number) => {
    const grid = new VoxelGrid({ minX: -16, minY: -16, minZ: -16, maxX: 16, maxY: 16, maxZ: 16 }, [{ color: '' }, RED, BLUE])
    for (let z = -16; z < 16; z++) {
      for (let y = -16; y < 16; y++) {
        for (let x = -16; x < 16; x++) grid.set(x, y, z, fill(x, y, z))
      }
    }
    return grid
  }
  /**
   * The y of the full coarse sites around the vertical line (x, z), away
   * from the box's edges: in the columns (x, z) and (x + 1, z), the one
   * holding the sites of even y and the other those of odd y.
   */
  const column = (grid: VoxelGrid, x: number, z: number) =>
    Array.from({ length: 12 }, (_, k) => k - 6).filter(y => grid.get(x, y, z) !== 0 || grid.get(x + 1, y, z) !== 0)

  it('keeps the surface of a solid where it is, or half a coarse voxel up', () => {
    // Ground up to y = 0: coarse sites up to y = 0 (2y = 0); up to y = 1: up to 1 (2y = 2).
    expect(column(coarsen(made((_, y) => (y <= 0 ? 1 : 0))), 0, 0)).toEqual([-6, -5, -4, -3, -2, -1, 0])
    expect(column(coarsen(made((_, y) => (y <= 1 ? 1 : 0))), 0, 0)).toEqual([-6, -5, -4, -3, -2, -1, 0, 1])
  })

  it('keeps a plane one voxel thick, between layers of cells or through them', () => {
    for (const plane of [0, 1, 2, 3]) {
      const coarse = coarsen(made((_, y) => (y === plane ? 1 : 0)))
      for (const [x, z] of [[0, 0], [2, 1], [-3, 2], [1, 3]]) expect(column(coarse, x, z).length, `plane ${String(plane)}`).toBeGreaterThan(0)
    }
  })

  it('drops a lone voxel, and takes the material most of a cell has', () => {
    expect(coarsen(made((x, y, z) => (x === 0 && y === 0 && z === 0 ? 1 : 0))).toVoxels()).toEqual([])
    // Red below y = 0, blue above: the coarse site at the origin holds more red (its own site and the layer below).
    const coarse = coarsen(made((_, y) => (y <= 0 ? 1 : 2)))
    expect(coarse.get(0, 0, 0)).toBe(1)
    expect(coarse.get(1, 1, 0)).toBe(2)
  })

  it('makes each face that shows of the material of the open faces facing its way', () => {
    // A red block under a blue sheet one voxel thick, up to x = 8. From above, coarse voxels of the sheet
    // hide the block's; at its edge, the red one under the sheet is blue only on its faces up towards ±z,
    // which nothing coarse covers, and red on the rest, which look at the block's side or are covered.
    const coarse = coarsen(made((x, y) => (x > 8 ? 0 : y === 1 ? 2 : y <= 0 ? 1 : 0)))
    const faces = (grid: VoxelGrid, x: number) => VOXEL_FACES.map((_, k) => grid.face(x, 0, 0, k))
    const edge = VOXEL_FACES.map(([a, b]) => (a === 0 && b > 0 ? 2 : 1))
    expect([coarse.get(4, 0, 0), coarse.get(3, 1, 0)]).toEqual([1, 2])
    expect(faces(coarse, 4)).toEqual(edge)
    // A coarser level measures the finer one's faces.
    expect(faces(coarsen(coarse), 2)).toEqual(edge)
  })
})
