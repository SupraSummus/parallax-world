/**
 * Tests' scenes: worlds of hand-made voxels alone, whose coarser levels
 * are sampled from them. Generated worlds are generated at each level.
 */
import { GRID_LEVELS } from './slice-selection'
import { colorOf, lookOf } from './look.test-helper'
import { isSite, VoxelGrid } from './voxel-grid'
import { World, type Vec3 } from './world'

export interface Voxel {
  x: number
  y: number
  z: number
  color: string
}

/** The smallest grid holding `voxels`, which must be sites, whole and solid; where two share a site, the later one wins. */
export function gridOf(voxels: readonly Voxel[]): VoxelGrid {
  const box = voxels.length === 0
    ? { minX: 0, minY: 0, minZ: 0, maxX: 0, maxY: 0, maxZ: 0 }
    : { minX: Infinity, minY: Infinity, minZ: Infinity, maxX: -Infinity, maxY: -Infinity, maxZ: -Infinity }
  for (const { x, y, z } of voxels) {
    if (!isSite(x, y, z)) throw new RangeError(`(${String(x)}, ${String(y)}, ${String(z)}) is not a lattice site`)
    box.minX = Math.min(box.minX, x)
    box.minY = Math.min(box.minY, y)
    box.minZ = Math.min(box.minZ, z)
    box.maxX = Math.max(box.maxX, x + 1)
    box.maxY = Math.max(box.maxY, y + 1)
    box.maxZ = Math.max(box.maxZ, z + 1)
  }
  const grid = new VoxelGrid(box)
  for (const { x, y, z, color } of voxels) grid.set(x, y, z, lookOf(color))
  return grid
}

/** Every voxel of `grid`, ordered by z, then y, then x. */
export function voxelsOf(grid: VoxelGrid): Voxel[] {
  const voxels: Voxel[] = []
  grid.forEach((x, y, z, look) => { voxels.push({ x, y, z, color: colorOf(look) }) })
  return voxels
}

/**
 * A grid at each level the renderer reads (GRID_LEVELS): level k holds
 * the voxels of `grid` at the sites of 2^k·FCC, in the level's units
 * (site S of it is site 2^k·S of `grid`; `set` skips the other points).
 * Each looks alike through every face. What misses those sites, such as a
 * floor at odd y, is gone there: unlike a generated world's levels.
 */
export function levelsOf(grid: VoxelGrid): VoxelGrid[] {
  const { minX, minY, minZ, nx, ny, nz } = grid
  return Array.from({ length: GRID_LEVELS }, (_, level) => {
    const s = 2 ** level
    const up = (v: number) => Math.ceil(v / s)
    const coarse = new VoxelGrid({ minX: up(minX), minY: up(minY), minZ: up(minZ), maxX: up(minX + nx), maxY: up(minY + ny), maxZ: up(minZ + nz) })
    grid.forEach((x, y, z, look) => { if (x % s === 0 && y % s === 0 && z % s === 0) coarse.set(x / s, y / s, z / s, look) })
    return coarse
  })
}

/** A world of `voxels` alone, its levels levelsOf them. */
export function ofVoxels(voxels: readonly Voxel[], spawn: Readonly<Vec3> = { x: 0, y: 0, z: 0 }): World {
  return new World(0, 'voxels', { levels: levelsOf(gridOf(voxels)), spawn })
}
