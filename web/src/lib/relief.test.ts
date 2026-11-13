/**
 * Drawn in relief, slices show what perspective shows, but where a column
 * has a gap: a column keeps its nearest voxel alone, so thick slices leak
 * there (docs/architecture.md, section 2). The renderer against
 * true perspective, in 2D, in the plane b = 0 of a face's lattice (see Face
 * in view.ts): lateral a and depth c, voxels (a, 0, c) with a and c even.
 * The plane cuts each into the square |Δa|, |Δc| ≤ 1 (in lattice units; a
 * and c are both LAYER per unit, so tangents are the same as in face
 * coordinates), and these tile it. Slices, chunks and splats come from the
 * renderer's own functions. Only the compositing is restated here, along
 * the line b = 0 of each chunk. A leak is a run of screen where the
 * renderer shows a voxel that no perspective ray reaches, measured in
 * pixels of a SCREEN-pixel screen.
 */
import { describe, it, expect } from 'vitest'
import { selectSlices, halfLayer, chunkCell, sliceCentre, holds, atomSpacing, type SliceId } from './slice-selection'
import { ChunkBaker, SPLAT } from './chunk-baker'
import { MAX_DETAIL } from './renderer'
import { FACES, LAYER } from './view'
import { VoxelGrid } from './voxel-grid'
import { gridOf, levelsOf, voxelsOf } from './scene.test-helper'

/** Where level 0 begins, in layers ahead of the camera, as in the renderer (NEAR_DEPTH). */
const NEAR = 4

describe.each([1, 0.5, 0.25])('ray-cast leaks with tan(fov / 2) = %s', TAN_HALF_FOV => {
  /** A voxel (u, 0, z), n times the lattice's (its grid's, see levelsOf). */
  interface Cell { u: number; z: number; n: number }
  const DEPTH = 300
  const SCREEN = 128
  const focal = SCREEN / 2 / TAN_HALF_FOV
  const RAYS = 3000
  const rays = Array.from({ length: RAYS }, (_, i) => -TAN_HALF_FOV + (2 * TAN_HALF_FOV * i) / (RAYS - 1))
  const step = rays[1] - rays[0]
  const face = FACES[0]

  /** Deterministic heightfield with steps and cliffs, solid down to u = -12. */
  function terrain(seed: number): Cell[] {
    let state = seed
    const random = () => {
      state = (state * 1103515245 + 12345) % 2147483648
      return state / 2147483648
    }
    const cells: Cell[] = []
    let height = 0
    for (let z = 2; z < DEPTH; z += 2) {
      height += 2 * (random() < 0.7 ? Math.round(random() * 2 - 1) : Math.round(random() * 4 - 2))
      height = Math.max(-12, Math.min(12, height))
      for (let u = -12; u <= height; u += 2) cells.push({ u, z, n: 1 })
    }
    return cells
  }

  function perspectiveVisible(cells: Cell[], cu: number, cz: number): Set<number> {
    const nearest = rays.map(() => ({ distance: Infinity, cell: -1 }))
    cells.forEach(({ u, z, n }, cell) => {
      // The ray (cu + t·d, cz + d) is in the cell while |cz + d − z| ≤ n and |cu + t·d − u| ≤ n.
      rays.forEach((t, i) => {
        let enter = Math.max(0, z - n - cz)
        let exit = z + n - cz
        if (t === 0) {
          if (Math.abs(cu - u) > n) return
        } else {
          const [d0, d1] = [(u - n - cu) / t, (u + n - cu) / t]
          enter = Math.max(enter, Math.min(d0, d1))
          exit = Math.min(exit, Math.max(d0, d1))
        }
        if (enter <= exit && enter < nearest[i].distance) nearest[i] = { distance: enter, cell }
      })
    })
    return new Set(nearest.map(n => n.cell))
  }

  /**
   * What the slices draw of a terrain: the voxels (u, 0, z) of the grid
   * each reads (levelsOf) that it holds, and those grids.
   */
  function drawn(terrain: Cell[], slices: SliceId[]): { cells: Cell[]; levels: VoxelGrid[] } {
    const { right: R, up: U, forward: F } = face.lattice
    const levels = levelsOf(gridOf(terrain.map(({ u, z }) => ({
      x: (u * R.x + z * F.x) / 2, y: (u * R.y + z * F.y) / 2, z: (u * R.z + z * F.z) / 2, color: '#808080',
    }))))
    const dot = (p: { x: number; y: number; z: number }, v: { x: number; y: number; z: number }) => p.x * v.x + p.y * v.y + p.z * v.z
    const inPlane = levels.map((grid, level) => voxelsOf(grid).map(p => ({ u: dot(p, R) << level, b: dot(p, U), z: dot(p, F) << level, n: 2 ** level })).filter(c => c.b === 0))
    const cells = new Map<string, Cell>()
    for (const slice of slices) {
      for (const { u, z, n } of inPlane[slice.atoms]) if (holds(slice, [u, 0, z])) cells.set(`${String(n)}:${String(u)},${String(z)}`, { u, z, n })
    }
    return { cells: [...cells.values()], levels }
  }

  /**
   * Which voxel each ray shows: the renderer's own face logic, on voxels
   * (u, 0, z) of the face's lattice, its splats drawn in relief as the
   * compositor does, each a pillar over its run, the one a ray enters
   * first showing (the later painted where they tie).
   */
  function parallaxOwners({ cells, levels }: { cells: Cell[]; levels: VoxelGrid[] }, cu: number, cz: number, slices: SliceId[]): number[] {
    const baker = new ChunkBaker(levels[0].materials, levels)
    const cellAt = new Map(cells.map(({ u, z, n }, cell) => [`${String(n)}:${String(u)},${String(z)}`, cell]))
    const owner = rays.map(() => -1)
    for (const slice of [...slices].reverse()) {
      // Within the slice, where each ray enters what it shows.
      const entered = rays.map(() => Infinity)
      const centre = sliceCentre(slice)
      const distance = centre - cz * LAYER
      // Bricks of row 0, which holds b = 0: brick i spans [x + i · w, x + (i + 1) · w).
      const { x, width: w } = chunkCell(slice.level, 0, 0)
      const n = atomSpacing(slice)
      const inSlice = (cell: Cell) => cell.n === n && holds(slice, [cell.u, 0, cell.z])
      const chunkCells = new Set(cells.filter(inSlice).flatMap(({ u }) => {
        const i = Math.floor((u * LAYER - x) / w)
        return [i - 1, i, i + 1]
      }))
      for (const i of chunkCells) {
        const chunk = baker.bake({ face: face.name, ...slice, i, j: 0 })
        const sites = baker.sites()
        const alphas = new Uint32Array(chunk.splats.buffer)
        // Tangents of the rays through what the chunk draws on the centre plane.
        const [left, right] = [chunk.originX, chunk.originX + chunk.extentWidth].map(x => (x - cu * LAYER) / distance)
        for (let k = 0; k < chunk.splats.length / SPLAT; k++) {
          const [sx, sy, sw, run] = chunk.splats.subarray(k * SPLAT, k * SPLAT + 4)
          const sh = sw * Math.SQRT2
          // Its site, camera-relative, at its depth (compositor.ts).
          const x = chunk.originX + sx - cu * LAYER
          const z = distance + ((alphas[k * SPLAT + 4] >>> 24) - 128) * halfLayer(slice)
          // Where its hexagon, widened by the slant, meets the line b = 0.
          const width = sw * (1 + Math.abs(x / z))
          const half = Math.min(width, (sh - Math.abs(chunk.originY + sy)) * Math.SQRT2)
          if (half <= 0) continue
          const voxel = cellAt.get(`${String(n)}:${String(sites[3 * k])},${String(sites[3 * k + 2])}`) ?? -1
          // Its pillar's far end; the rays that meet it are those through either end's hexagon.
          const back = z + run * halfLayer(slice)
          const first = Math.ceil((Math.max(left, Math.min((x - half) / z, (x - half) / back)) + TAN_HALF_FOV) / step)
          const last = Math.floor((Math.min(right, Math.max((x + half) / z, (x + half) / back)) + TAN_HALF_FOV) / step)
          for (let k = Math.max(0, first); k <= Math.min(RAYS - 1, last); k++) {
            // Where the ray (t·d, d) is within `half` of x, between the pillar's ends.
            const t = rays[k]
            let [enter, exit] = [z, back]
            if (t !== 0) {
              const [d0, d1] = [(x - half) / t, (x + half) / t]
              enter = Math.max(enter, Math.min(d0, d1))
              exit = Math.min(exit, Math.max(d0, d1))
            } else if (Math.abs(x) > half) continue
            if (enter > exit || enter > entered[k]) continue
            entered[k] = enter
            owner[k] = voxel
          }
        }
      }
    }
    return owner
  }

  /** Every leak's width in screen pixels. */
  function leaks(terrain: Cell[], cu: number, cz: number, detail: number): number[] {
    const slices = selectSlices(cz, NEAR, DEPTH, detail)
    // Against the world the slices stand for, coarser where they are.
    const world = drawn(terrain, slices)
    const visible = perspectiveVisible(world.cells, cu, cz)
    const runs: number[] = []
    let run = 0
    for (const cell of [...parallaxOwners(world, cu, cz, slices), -1]) {
      if (cell >= 0 && !visible.has(cell)) run++
      else if (run > 0) {
        runs.push(run)
        run = 0
      }
    }
    return runs.map(rays => rays * step * focal)
  }

  // Camera above the terrain looking over it.
  const scenes = [1, 2, 3, 4].map(seed => ({ cells: terrain(seed), cu: 12 + seed * 7.4, cz: -0.2 - seed * 0.4 }))

  // Each column's nearest voxel drawn where it is, over its run, nothing
  // hidden shows but a ray's sliver where hexagons stand in for outlines.
  it('leaks nothing from slices of the most detail', () => {
    for (const { cells, cu, cz } of scenes) {
      expect(leaks(cells, cu, cz, MAX_DETAIL).filter(pixels => pixels > 0.1)).toEqual([])
    }
  })

  // A column along the axis through a hill, a valley and another hill
  // keeps the first alone: a ray looking down into the valley meets the
  // slices behind, not the second hill.
  it('leaks from slices as thick as their distance', () => {
    const visible = scenes.flatMap(({ cells, cu, cz }) => leaks(cells, cu, cz, 1).filter(pixels => pixels > 2))
    expect(visible.length).toBeGreaterThan(0)
  })
})
