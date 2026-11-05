/**
 * The occlusion-leak bound, derived step by step and checked.
 *
 * Claim: with the slicing of slice-selection.ts, slices drawn flat on
 * their centre planes show geometry that true perspective hides by at most
 * ε screen pixels (ε = the renderer's max leak), except closer than
 * 2 · `detail` layer spacings, where slices cannot be thinner than one
 * layer of voxels (two layer spacings: a voxel reaches a layer past its
 * own, see fcc.ts) and the bound is two layer spacings.
 * The renderer draws them in relief instead, each voxel at its depth
 * (compositor.ts), and leaks only slivers (step 8): the bound is what the
 * slicing keeps, no longer what is seen.
 *
 * Each `describe` below is one step (step 1 aside, one line of algebra):
 * the claim in words, then a check on random samples (seeded, so failures
 * reproduce) using the same functions the renderer uses.
 * Steps 1, 2 and 4–6 are exact geometry and algebra.
 * Step 3 is the one modelling assumption: that a leak is at most the error
 * of the occluding edge plus that of the hidden one. It holds for single
 * intervals; for real scenes (several occluders, faces seen edge-on, the
 * cross-section of a cut slab) step 8 checks the whole chain against
 * ray-cast perspective. Where a step's check could pass vacuously, a
 * deliberately wrong variant is shown to fail.
 *
 * Each of these plausible mistakes fails the step in brackets (checked by
 * hand): centre plane at depth + s/2 [2], slice thickness measured from the
 * centre instead of the near face [5], detail rounded down [6].
 *
 * Depths are in face coordinates (view.ts): layers are LAYER apart. A
 * slice is a slab of layers of the voxels of its grid, N = 2^atomLevel
 * times the lattice's (slice-selection.ts); their sites are every N-th of
 * the slab's 2^level layers, and they reach N layers past those, so it is
 * t = thickness layer spacings thick.
 *
 * Notation, for one lateral axis: the camera is at lateral 0, depth 0 (so
 * z is relative to it). A slice of level k has its near face a layers
 * ahead, thickness t, and centre plane at distance D. Drawn flat, a point
 * is drawn at its own lateral position on that plane, which the camera
 * sees along tangent x / D where it sees the point along σ.
 * focal is the focal length in pixels, ε the leak tolerance in pixels.
 */
import { describe, it, expect } from 'vitest'
import { selectSlices, leakFreeDetail, leakAngle, halfLayer, chunkCell, sliceCentre, thickness, holds, atomLevel, atomSpacing, MAX_LEVEL, type SliceId } from './slice-selection'
import { ChunkBaker, SPLAT } from './chunk-baker'
import { FACES, LAYER, inFace } from './view'
import { VoxelGrid } from './voxel-grid'
import { levelsOf } from './world'

/** Seeded uniform samples in [lo, hi). */
function sampler(seed: number) {
  let state = seed
  return (lo: number, hi: number) => {
    state = (state * 1103515245 + 12345) % 2147483648
    return lo + ((hi - lo) * state) / 2147483648
  }
}

/** Runs `check` on `n` random cases; fails with the first counterexample. */
function forAll<T>(n: number, generate: (random: (lo: number, hi: number) => number) => T, check: (c: T) => boolean) {
  const random = sampler(n * 7919 + 1)
  for (let i = 0; i < n; i++) {
    const c = generate(random)
    if (!check(c)) expect.fail(`counterexample: ${JSON.stringify(c)}`)
  }
}

const EPSILON = 1e-9
/** Where level 0 begins, in layers ahead of the camera, as in the renderer (NEAR_DEPTH). */
const NEAR = 4

/** A random slice: level, layer, no cutoff. */
const randomSlice = (r: (lo: number, hi: number) => number, from: number, to: number): SliceId =>
  ({ level: Math.floor(r(0, MAX_LEVEL + 1)), layer: Math.floor(r(from, to)), cutoff: Infinity })
/** The layer of a slice's voxels' near tips: its atoms' sites are its slab's layers, every N-th, and its voxels reach N layers past them. */
const nearSide = ({ level, layer }: SliceId) => layer * 2 ** level - atomSpacing(level)

/*
 * Step 1: a baked, drawn point is off by σ·(z − D). A point at lateral x,
 * depth z is seen along σ = x / z. Baked along the axis, it stays at x on
 * the centre plane (chunk-baker.test.ts), drawn at distance D, where the
 * camera's ray through the point crosses the plane at x − σ·(z − D).
 * One line of algebra, so not checked here.
 */

describe('Step 2: within a slice |z − D| ≤ t/2', () => {
  /*
   * A voxel N wide in layer c reaches from layer c − N to c + N, so a
   * slice whose voxels' sites are every N-th of the layers [l, l + 2^k)
   * (l its slab's first layer) spans [l − N, l + 2^k], t = thickness layer
   * spacings thick, and sliceCentre is its middle. With step 1, every point
   * of a slice is drawn at most |σ|·t/2 layer spacings from where it
   * belongs.
   */
  it('holds for every point of the voxels of random slices', () => {
    forAll(2000, r => {
      const slice = randomSlice(r, -10, 10)
      const slab = 2 ** slice.level
      const n = atomSpacing(slice.level)
      // A site of its grid near the slab, kept if the slice holds it.
      const c = Math.floor((slice.layer * slab + r(-slab, 2 * slab)) / n)
      const site: [number, number, number] = [n * (2 * Math.floor(r(-4, 4)) + (c & 1)), n * (2 * Math.floor(r(-4, 4)) + (c & 1)), n * c]
      return { slice, site, z: (site[2] + r(-n, n)) * LAYER }
    }, ({ slice, site, z }) => {
      if (!holds(slice, site)) return true
      return Math.abs(z - sliceCentre(slice)) <= (thickness(slice.level) / 2) * LAYER + EPSILON
    })
  })
})

describe('Step 3 (assumption): a leak is at most the sum of two edge errors', () => {
  /*
   * In perspective, a hidden interval B lies inside an occluding interval O
   * (for one occluder). Drawn, each end of O moves by at most e_O and each
   * end of B by at most e_B, so at most e_O + e_B of B shows past O on
   * either side. With step 2 both are |σ|·t/2, so a leak is at most
   * |σ|·t layer spacings at the slice. Several occluders, faces seen
   * edge-on and the cut faces of slabs are what step 8 checks.
   */
  it('holds for random nested intervals and displacements', () => {
    forAll(5000, r => {
      const o0 = r(-10, 0)
      const o1 = r(0, 10)
      const b0 = r(o0, 0)
      const b1 = r(0, o1)
      const eO = r(0, 2)
      const eB = r(0, 2)
      return { o: [o0 + r(-eO, eO), o1 + r(-eO, eO)], b: [b0 + r(-eB, eB), b1 + r(-eB, eB)], eO, eB }
    }, ({ o, b, eO, eB }) => {
      const shownLeft = Math.max(0, Math.min(b[1], o[0]) - b[0])
      const shownRight = Math.max(0, b[1] - Math.max(b[0], o[1]))
      return Math.max(shownLeft, shownRight) <= eO + eB + EPSILON
    })
  })
})

describe("Step 4: |σ| ≤ 1 for the face's rays", () => {
  /*
   * A face draws the rays |x| + √2·|y| ≤ z (inFace): a rhombus of
   * tangents reaching 1 across (towards the world axes) and 1/√2 up,
   * convex, so |σ| ≤ 1, reached at its corners.
   */
  it("holds at the corners of the face's rhombus, and no further", () => {
    for (const [x, y] of [[1, 0], [-1, 0], [0, LAYER], [0, -LAYER]]) {
      expect(inFace(0.999 * x, 0.999 * y, 1)).toBe(true)
      expect(inFace(1.001 * x, 1.001 * y, 1)).toBe(false)
    }
  })
})

describe('Step 5: in pixels, a leak is at most t·focal / (a + t/2)', () => {
  /*
   * A leak of w layer spacings on the plane at distance D is w·focal/D
   * pixels. With steps 3 and 4, w ≤ t, and D = a + t/2. This checks the
   * slicing keeps thickness ≤ max(2, a / detail), in layers, a measured to
   * the slice's near side (single layers, level 0, are 2 thick), and the
   * centre plane t/2 past it.
   */
  it('selectSlices keeps every slice at most max(2, a / detail) thick', () => {
    forAll(300, r => ({ cameraZ: r(-300, 300), detail: 2 ** r(0, 8), range: r(10, 1000) }), ({ cameraZ, detail, range }) =>
      selectSlices(cameraZ, NEAR, range, detail).every(s => {
        const a = nearSide(s) - cameraZ
        const t = thickness(s.level)
        return t <= Math.max(2, a / detail) && Math.abs(sliceCentre(s) / LAYER - cameraZ - (a + t / 2)) < 1e-6
      }))
  })
})

describe('Step 6: leakFreeDetail closes the loop', () => {
  /*
   * Step 5 with a ≥ t · detail: a leak is at most focal / (detail + 1/2)
   * pixels (leakAngle), decreasing in detail. leakFreeDetail solves it for
   * ε and rounds up.
   *
   * Closer than 2 · `detail` layers, a / detail < 2 but slices are
   * at least one layer (two spacings) thick, so step 5's bound becomes
   * 2 layer spacings there.
   */
  const leakBound = (focal: number, detail: number) => focal * leakAngle(detail)

  it('keeps the bound within ε for random settings', () => {
    // Up to ε = 2/3 radian, where detail reaches its least, 1.
    forAll(5000, r => ({ focal: 2 ** r(4, 14), share: r(0.001, 2 / 3) }), ({ focal, share }) =>
      leakBound(focal, leakFreeDetail(share)) <= share * focal + EPSILON)
  })

  it('half the detail would break it', () => {
    const focal = 640
    expect(leakBound(focal, leakFreeDetail(16 / focal) / 2)).toBeGreaterThan(16)
  })
})

/**
 * Step 8 (end to end): the renderer against true perspective, in 2D, in the
 * plane b = 0 of a face's lattice (see Face in view.ts): lateral a and
 * depth c, voxels (a, 0, c) with a and c even. The plane cuts each into the
 * square |Δa|, |Δc| ≤ 1 (in lattice units; a and c are both LAYER per
 * unit, so tangents are the same as in face coordinates), and these tile
 * it. Slices, chunks and splats come from the renderer's own functions.
 * Only the compositing is restated here, along the line b = 0 of each
 * chunk. A leak is a run of screen where the renderer shows a voxel that
 * no perspective ray reaches, measured in pixels of a SCREEN-pixel screen.
 */
describe.each([1, 0.5, 0.25])('Step 8: ray-cast leaks with tan(fov / 2) = %s', TAN_HALF_FOV => {
  /** A voxel (u, 0, z), n times the lattice's (its grid's, see levelsOf). */
  interface Cell { u: number; z: number; n: number }
  const DEPTH = 300
  const SCREEN = 128
  const MAX_LEAK = 2
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
    const levels = levelsOf(VoxelGrid.fromVoxels(terrain.map(({ u, z }) => ({
      x: (u * R.x + z * F.x) / 2, y: (u * R.y + z * F.y) / 2, z: (u * R.z + z * F.z) / 2, color: '#808080',
    }))))
    const dot = (p: { x: number; y: number; z: number }, v: { x: number; y: number; z: number }) => p.x * v.x + p.y * v.y + p.z * v.z
    const inPlane = levels.map((grid, level) => grid.toVoxels().map(p => ({ u: dot(p, R) << level, b: dot(p, U), z: dot(p, F) << level, n: 2 ** level })).filter(c => c.b === 0))
    const cells = new Map<string, Cell>()
    for (const slice of slices) {
      for (const { u, z, n } of inPlane[atomLevel(slice.level)]) if (holds(slice, [u, 0, z])) cells.set(`${String(n)}:${String(u)},${String(z)}`, { u, z, n })
    }
    return { cells: [...cells.values()], levels }
  }

  /**
   * Which voxel each ray shows: the renderer's own face logic, on voxels
   * (u, 0, z) of the face's lattice, its splats drawn in relief, as the
   * compositor does, or `flat` on their slices' centre planes.
   */
  function parallaxOwners({ cells, levels }: { cells: Cell[]; levels: VoxelGrid[] }, cu: number, cz: number, slices: SliceId[], flat: boolean): number[] {
    const baker = new ChunkBaker(levels[0].materials, levels)
    const cellAt = new Map(cells.map(({ u, z, n }, cell) => [`${String(n)}:${String(u)},${String(z)}`, cell]))
    const owner = rays.map(() => -1)
    for (const slice of [...slices].reverse()) {
      const centre = sliceCentre(slice)
      const distance = centre - cz * LAYER
      // Bricks of row 0, which holds b = 0: brick i spans [x + i · w, x + (i + 1) · w).
      const { x, width: w } = chunkCell(slice.level, 0, 0)
      const n = atomSpacing(slice.level)
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
          const [sx, sy, sw] = chunk.splats.subarray(k * SPLAT, k * SPLAT + 3)
          const sh = sw * Math.SQRT2
          // Its site, camera-relative, at its depth (compositor.ts).
          const x = chunk.originX + sx - cu * LAYER
          const z = flat ? distance : distance + ((alphas[k * SPLAT + 4] >>> 24) - 128) * halfLayer(slice.level)
          // Where its hexagon, widened by the slant, meets the line b = 0.
          const width = flat ? sw : sw * (1 + Math.abs(x / z))
          const half = Math.min(width, (sh - Math.abs(chunk.originY + sy)) * Math.SQRT2)
          if (half <= 0) continue
          const voxel = cellAt.get(`${String(n)}:${String(sites[3 * k])},${String(sites[3 * k + 2])}`) ?? -1
          const first = Math.ceil((Math.max(left, (x - half) / z) + TAN_HALF_FOV) / step)
          const last = Math.floor((Math.min(right, (x + half) / z) + TAN_HALF_FOV) / step)
          for (let k = Math.max(0, first); k <= Math.min(RAYS - 1, last); k++) owner[k] = voxel
        }
      }
    }
    return owner
  }

  /** Every leak: its width in screen pixels and in lattice units at its nearest leaked voxel. */
  function leaks(terrain: Cell[], cu: number, cz: number, detail: number, flat: boolean): { pixels: number; units: number }[] {
    const slices = selectSlices(cz, NEAR, DEPTH, detail)
    // Against the world the slices stand for, coarser where they are.
    const world = drawn(terrain, slices)
    const { cells } = world
    const visible = perspectiveVisible(cells, cu, cz)
    const owner = parallaxOwners(world, cu, cz, slices, flat)
    const runs: { rays: number; distance: number }[] = []
    let run: { rays: number; distance: number } | null = null
    for (const cell of owner) {
      if (cell >= 0 && !visible.has(cell)) {
        run ??= { rays: 0, distance: Infinity }
        run.rays++
        run.distance = Math.min(run.distance, cells[cell].z - cells[cell].n - cz)
      } else if (run) {
        runs.push(run)
        run = null
      }
    }
    if (run) runs.push(run)
    return runs.map(r => ({ pixels: r.rays * step * focal, units: r.rays * step * r.distance }))
  }

  // Camera above the terrain looking over it.
  const scenes = [1, 2, 3, 4].map(seed => ({ cells: terrain(seed), cu: 12 + seed * 7.4, cz: -0.2 - seed * 0.4 }))
  const detail = leakFreeDetail(MAX_LEAK / focal)
  // Within the leak bound, give or take one ray. Slices are at least one
  // layer (two spacings) thick, so near the camera (closer than 2 · detail)
  // the bound is 2 units instead, which is more pixels there.
  const withinBound = ({ pixels, units }: { pixels: number; units: number }) =>
    pixels <= MAX_LEAK + step * focal || units <= 2 + step * DEPTH

  it('drawn flat, keeps every leak within max(max leak pixels, 2 units) with leakFreeDetail', () => {
    for (const { cells, cu, cz } of scenes) {
      expect(leaks(cells, cu, cz, detail, true).filter(l => !withinBound(l))).toEqual([])
    }
  })

  // Not vacuously: slices 16 times as thick leak past it. (The bound is
  // loose here, at the face's edge only and for the worst occluders:
  // 4 times as thick leak past it at tan(fov / 2) = 1, not at 0.25.)
  it('drawn flat, leaks more from slices 16 times as thick', () => {
    const outside = scenes.flatMap(({ cells, cu, cz }) => leaks(cells, cu, cz, detail / 16, true).filter(l => !withinBound(l)))
    expect(outside.length).toBeGreaterThan(0)
  })

  // Each voxel drawn where it is, nothing hidden shows but a ray's sliver
  // where hexagons stand in for outlines.
  it('in relief, leaks nothing even from slices as thick as their distance', () => {
    for (const { cells, cu, cz } of scenes) {
      expect(leaks(cells, cu, cz, 1, false).filter(l => l.pixels > 0.1)).toEqual([])
    }
  })
})
