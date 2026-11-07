import { describe, it, expect } from 'vitest'
import { ChunkBaker, SPLAT } from './chunk-baker'
import { VOXEL_FACES } from './fcc'
import { sliceCentre, holds, halfLayer, atomLevel, atomSpacing, ATOM_SHIFT } from './slice-selection'
import { FACES, LAYER, toLocal, type Face } from './view'
import { VoxelGrid } from './voxel-grid'
import { gridOf, levelsOf, voxelsOf, type Voxel } from './scene.test-helper'
import type { Vec3 } from './world'

const dot = (p: Vec3, v: Vec3) => p.x * v.x + p.y * v.y + p.z * v.z
/** A world point's lattice coordinates on a face (see Face in view.ts). */
const toLattice = ({ lattice: { right, up, forward } }: Face, p: Vec3) => ({ a: dot(p, right), b: dot(p, up), c: dot(p, forward) })
/** The world point at lattice coordinates (a, b, c) of a face. */
const toWorld = ({ lattice: { right: R, up: U, forward: F } }: Face, a: number, b: number, c: number): Vec3 => ({
  x: (a * R.x + c * F.x) / 2 + b * U.x,
  y: (a * R.y + c * F.y) / 2 + b * U.y,
  z: (a * R.z + c * F.z) / 2 + b * U.z,
})
/** The kind (of VOXEL_FACES) of a voxel's face towards a camera looking along a face's forward F: −F's. */
const frontOf = ({ lattice: { forward: { x, y, z } } }: Face) => VOXEL_FACES.findIndex(([a, b, c]) => a === -x && b === -y && c === -z)

/**
 * The first voxel of a slice, `size` times the lattice's, hit by the ray
 * along the face's axis through (px, py, centre): its index in `sites`, -1
 * if none. All in face coordinates. Null if rays a hair apart disagree (an
 * edge passes there).
 */
function rayCast(sites: Vec3[], size: number, centre: number, px: number, py: number): number | null {
  // Kinds' normals in face coordinates, the same on every face.
  const normals = VOXEL_FACES.map(([x, y, z]) => toLocal(FACES[0], { x, y, z }))
  // Along (0, 0, 1) from the plane, both ways: p = (ox, oy, centre + t).
  const one = (ox: number, oy: number) => {
    let best = { t: Infinity, site: -1 }
    sites.forEach((v, index) => {
      let enter = -Infinity
      let exit = Infinity
      // Inside the voxel: n · (p − site) ≤ size for every face normal n.
      normals.forEach(({ x: a, y: b, z: c }) => {
        const no = a * (ox - v.x) + b * (oy - v.y) + c * (centre - v.z)
        if (Math.abs(c) < 1e-12) {
          if (no > size) enter = Infinity
        } else if (c < 0) {
          enter = Math.max(enter, (size - no) / c)
        } else {
          exit = Math.min(exit, (size - no) / c)
        }
      })
      if (enter < exit && enter < best.t) best = { t: enter, site: index }
    })
    return best.site
  }
  const hits = [[0, 0], [0.01, 0.003], [-0.004, 0.01], [0.007, -0.009], [-0.01, -0.005]].map(([dx, dy]) => one(px + dx, py + dy))
  return hits.every(c => c === hits[0]) ? hits[0] : null
}

describe('ChunkBaker', () => {
  // A seeded blob of voxels in the lattice of a face: solid at the bottom
  // (b), sparse above, in layers 0 to 7.
  let state = 12345
  const random = () => {
    state = (state * 1103515245 + 12345) % 2147483648
    return state / 2147483648
  }
  const face = FACES[0]
  const colors = ['#ff0000', '#00ff00', '#0000ff', '#ffff00']
  const voxels: (Voxel & { a: number; b: number; c: number })[] = []
  for (let c = 0; c < 8; c++) {
    for (let b = -6 + ((c + 6) & 1); b < 30; b += 2) {
      for (let a = -6 + ((c + 6) & 1); a < 40; a += 2) {
        if (b < 4 || random() < 0.25) voxels.push({ ...toWorld(face, a, b, c), a, b, c, color: colors[Math.floor(random() * 4)] })
      }
    }
  }
  const grid = gridOf(voxels)
  const levels = levelsOf(grid)
  const baker = new ChunkBaker(grid.materials, levels)

  // Slices with sites above the blob's bottom show cut faces; the blob is
  // within one slab of level 3, of level 4, and of level 5, made of the
  // voxels of the grid one level coarser (sites of 2·FCC).
  it.each([
    [0, 0, Infinity], [0, 3, Infinity], [1, 1, Infinity], [1, 2, Infinity],
    [2, 1, Infinity], [3, 0, 4], [4, 0, Infinity], [5, 0, Infinity], [5, 0, 8], [0, 5, Infinity],
  ] as const)('paints every voxel a ray along the axis hits first, far to near, at its site and depth, in the colour of its face towards the camera, its splat holding where the ray meets the plane (level %s, layer %s, cutoff %s)', (level, layer, cutoff) => {
    const slice = { level, atoms: atomLevel(level, ATOM_SHIFT), layer, cutoff }
    const centre = sliceCentre(slice)
    // The voxels of the grid it reads, N times the lattice's, in the colour of their faces towards the camera.
    const n = atomSpacing(slice)
    const read = levels[slice.atoms]
    const voxels = voxelsOf(read)
      .map(v => ({ ...toLattice(face, { x: n * v.x, y: n * v.y, z: n * v.z }), color: read.materials[read.face(v.x, v.y, v.z, frontOf(face))].color }))
      .filter(v => holds(slice, [v.a, v.b, v.c]))
    const sites = voxels.map(v => ({ x: v.a * LAYER, y: v.b, z: v.c * LAYER }))
    expect(sites.length).toBeGreaterThan(20)
    const chunk = baker.bake({ face: face.name, ...slice, i: 0, j: 0 })
    const { splats, originX, originY, extentWidth, extentHeight } = chunk

    // Each splat: centred on its site, its depth past the centre plane in
    // half layers of the grid in its alpha, its voxel's colour shaded (so
    // the channels it lacks stay 0).
    const lattice = baker.sites()
    const colorOf = new Map(voxels.map(v => [[v.a, v.b, v.c].join(), parseInt(v.color.slice(1), 16)]))
    const rgba = new Uint32Array(splats.buffer)
    const channels = (rgb: number, ...shifts: number[]) => shifts.map(s => ((rgb >> s) & 255) > 0)
    const splatOf = new Map<string, number>()
    for (let k = 0; k < splats.length / SPLAT; k++) {
      const [a, b, c] = lattice.subarray(3 * k, 3 * k + 3)
      splatOf.set([a, b, c].join(), k)
      expect(channels(rgba[k * SPLAT + 4], 0, 8, 16)).toEqual(channels(colorOf.get([a, b, c].join()) ?? 0, 16, 8, 0))
      expect(originX + splats[k * SPLAT]).toBeCloseTo(a * LAYER, 3)
      expect(originY + splats[k * SPLAT + 1]).toBeCloseTo(b, 3)
      expect(centre + ((rgba[k * SPLAT + 4] >>> 24) - 128) * halfLayer(slice)).toBeCloseTo(c * LAYER, 9)
      if (k > 0) expect(c).toBeLessThanOrEqual(lattice[3 * k - 1])
    }

    let compared = 0
    const missing: string[] = []
    const uncovered: string[] = []
    // About 1000 samples: ray casting is slow.
    for (let row = 0.5; row < 24; row++) {
      for (let col = 0.5; col < 36; col++) {
        const px = originX + (col / 36) * extentWidth
        const py = originY - (row / 24) * extentHeight
        const hit = rayCast(sites, n, centre, px, py)
        if (hit === null || hit < 0) continue
        compared++
        const { a, b, c } = voxels[hit]
        const k = splatOf.get([a, b, c].join())
        if (k === undefined) {
          missing.push([a, b, c].join())
          continue
        }
        const [u, v] = [Math.abs(px - originX - splats[k * SPLAT]), Math.abs(py - originY - splats[k * SPLAT + 1])]
        if (u > splats[k * SPLAT + 2] + 1e-4 || u * Math.SQRT1_2 + v > splats[k * SPLAT + 2] * Math.SQRT2 + 1e-4) uncovered.push([a, b, c].join())
      }
    }
    expect(compared).toBeGreaterThan(150)
    expect(missing).toEqual([])
    expect(uncovered).toEqual([])
  }, 30_000)

  it('knows the faces as view.ts orders them: the blob turned to face each bakes the same splats', () => {
    const sitesOn = (face: Face) => {
      const turned = gridOf(voxels.map(({ a, b, c, color }) => ({ ...toWorld(face, a, b, c), color })))
      const turnedBaker = new ChunkBaker(turned.materials, [turned])
      turnedBaker.bake({ face: face.name, level: 0, atoms: 0, layer: 3, cutoff: Infinity, i: 0, j: 0 })
      return turnedBaker.sites()
    }
    const sites = sitesOn(face)
    expect(sites.length).toBeGreaterThan(0)
    for (const other of FACES) expect(sitesOn(other)).toEqual(sites)
  })

  it('paints a voxel in the material of its face towards the camera', () => {
    for (const face of FACES) {
      // A voxel at the origin, red but for its face towards the camera, which is blue.
      const one = new VoxelGrid({ minX: -2, minY: -2, minZ: -2, maxX: 2, maxY: 2, maxZ: 2 }, [{ color: '' }, { color: '#ff0000' }, { color: '#0000ff' }])
      one.set(0, 0, 0, 1)
      one.setFaces([[0, 0, 0, VOXEL_FACES.map((_, k) => (k === frontOf(face) ? 2 : 1))]])
      const { splats } = new ChunkBaker(one.materials, [one]).bake({ face: face.name, level: 0, atoms: 0, layer: 0, cutoff: Infinity, i: 0, j: 0 })
      expect(splats.length).toBe(SPLAT)
      const rgb = new Uint32Array(splats.buffer)[4]
      expect([rgb & 255, (rgb >> 16) & 255].map(c => c > 0), face.name).toEqual([false, true])
    }
  })
})
