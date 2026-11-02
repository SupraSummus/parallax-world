import { describe, it, expect } from 'vitest'
import { FACES } from './view'
import { coarsen, LayerIndex, VoxelGrid, isSite } from './voxel-grid'
import { World } from './world'

const { grid, voxels } = new World(7, 'hills_with_trees')

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
    const list = [{ x: 0, y: 0, z: 0, color: '#ff0000', type: 1 }, { x: 3, y: -1, z: 2, color: '#00ff00', type: 2 }]
    expect(VoxelGrid.fromVoxels(list).toVoxels()).toEqual(list)
    expect(() => VoxelGrid.fromVoxels([{ x: 1, y: 0, z: 0, color: '#ff0000', type: 1 }])).toThrow(RangeError)
  })
})

const dot = (p: { x: number; y: number; z: number }, v: { x: number; y: number; z: number }) => p.x * v.x + p.y * v.y + p.z * v.z

describe('LayerIndex', () => {
  const index = new LayerIndex(grid, FACES.map(f => f.lattice))
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
})

describe('coarsen', () => {
  const RED = { color: '#ff0000', type: 1 }
  const BLUE = { color: '#0000ff', type: 1 }
  /** A grid of the sites in a box that `fill` gives a material (1 red, 2 blue) or 0. */
  const made = (fill: (x: number, y: number, z: number) => number) => {
    const grid = new VoxelGrid({ minX: -16, minY: -16, minZ: -16, maxX: 16, maxY: 16, maxZ: 16 }, [{ color: '#000000', type: 0 }, RED, BLUE])
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
})
