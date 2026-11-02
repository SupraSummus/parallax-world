import { describe, it, expect } from 'vitest'
import { shadedPalette, faceShade, FACE_SHADE } from './bake'
import { ChunkBaker, FACE } from './chunk-baker'
import { VOXEL_FACES } from './fcc'
import { sliceCentre, holds, atomLevel, atomSpacing } from './slice-selection'
import { FACES, LAYER, toLocal, type Face } from './view'
import { VoxelGrid, type Voxel } from './voxel-grid'
import { levelsOf, type Vec3 } from './world'

const dot = (p: Vec3, v: Vec3) => p.x * v.x + p.y * v.y + p.z * v.z
/** A world point's lattice coordinates on a face (see Face in view.ts). */
const toLattice = ({ lattice: { right, up, forward } }: Face, p: Vec3) => ({ a: dot(p, right), b: dot(p, up), c: dot(p, forward) })
/** The world point at lattice coordinates (a, b, c) of a face. */
const toWorld = ({ lattice: { right: R, up: U, forward: F } }: Face, a: number, b: number, c: number): Vec3 => ({
  x: (a * R.x + c * F.x) / 2 + b * U.x,
  y: (a * R.y + c * F.y) / 2 + b * U.y,
  z: (a * R.z + c * F.z) / 2 + b * U.z,
})
// Face kinds as bake.ts numbers them: VOXEL_FACES in the lattice coordinates of any face.
const kindOffsets = VOXEL_FACES.map(({ normal: [x, y, z] }) => toLattice(FACES[0], { x, y, z }))

describe('shadedPalette', () => {
  it('shades a face by the axes it faces along in the world', () => {
    const { front, back, left, right, bottom, top } = FACE_SHADE
    expect([[0, 0, -1], [0, 0, 1], [-1, 0, 0], [1, 0, 0], [0, -1, 0], [0, 1, 0]].map(([x, y, z]) => faceShade({ x, y, z })))
      .toEqual([front, back, left, right, bottom, top])
    expect(faceShade({ x: 0, y: 1, z: -1 })).toBeCloseTo((top + front) / 2)

    const white = [{ color: '#000000', type: 0 }, { color: '#ffffff', type: 1 }]
    const kind = (a: number, b: number, c: number) => kindOffsets.findIndex(o => o.a === a && o.b === b && o.c === c)
    // Seen along +x+z (up is +y, right is (1, 0, -1) / √2): the front kind
    // faces -x and -z; the kind a step up, right and towards the camera faces up and -z.
    const palette = shadedPalette(white, (FACES.find(f => f.name === '+x+z') as Face).lattice)
    const red = (k: number) => palette[VOXEL_FACES.length + k] & 255
    expect(red(kind(0, 0, -2))).toBe(Math.round(255 * (left + front) / 2))
    expect(red(kind(1, 1, -1))).toBe(Math.round(255 * (top + front) / 2))
  })
})

/**
 * The colour (an index into shadedPalette) that the first voxel of a slice,
 * `size` times the lattice's, hit by the ray from `eye` through
 * (px, py, centre) shows: the face it
 * enters through. All in face coordinates. Null if it hits nothing, or if
 * rays a hair apart disagree (an edge passes there).
 */
function rayCast(sites: (Vec3 & { id: number })[], size: number, centre: number, px: number, py: number, eye: Vec3): number | null {
  // Kinds' normals in face coordinates, the same on every face.
  const normals = VOXEL_FACES.map(({ normal: [x, y, z] }) => toLocal(FACES[0], { x, y, z }))
  const one = (ox: number, oy: number) => {
    // Along (tanX, tanY, 1) from the plane, both ways.
    const [tanX, tanY] = [(ox - eye.x) / (centre - eye.z), (oy - eye.y) / (centre - eye.z)]
    let best = { t: Infinity, colour: -1 }
    for (const v of sites) {
      let enter = -Infinity
      let exit = Infinity
      let through = -1
      // Inside the voxel: n · (p − site) ≤ size for every face normal n.
      normals.forEach(({ x: a, y: b, z: c }, k) => {
        const nd = a * tanX + b * tanY + c
        const no = a * (ox - v.x) + b * (oy - v.y) + c * (centre - v.z)
        if (Math.abs(nd) < 1e-12) {
          if (no > size) enter = Infinity
        } else if (nd < 0) {
          const t = (size - no) / nd
          if (t > enter) [enter, through] = [t, k]
        } else {
          exit = Math.min(exit, (size - no) / nd)
        }
      })
      if (enter < exit && enter < best.t) best = { t: enter, colour: v.id * VOXEL_FACES.length + through }
    }
    return best.colour
  }
  const colours = [[0, 0], [0.01, 0.003], [-0.004, 0.01], [0.007, -0.009], [-0.01, -0.005]].map(([dx, dy]) => one(px + dx, py + dy))
  return colours.every(c => c === colours[0]) ? colours[0] : null
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
        if (b < 4 || random() < 0.25) voxels.push({ ...toWorld(face, a, b, c), a, b, c, color: colors[Math.floor(random() * 4)], type: 1 })
      }
    }
  }
  const grid = VoxelGrid.fromVoxels(voxels)
  const ids = new Map(grid.materials.map((m, id) => [m.color, id]))
  const levels = levelsOf(grid)
  const baker = new ChunkBaker(grid.materials, levels)
  const palette = shadedPalette(grid.materials, face.lattice)

  // Slices with sites above the blob's bottom show cut faces; the blob is
  // within one slab of level 3, of level 4, and of level 5, made of the
  // voxels of the grid one level coarser (sites of 2·FCC).
  // Eyes (lattice coordinates, sites of the slice's grid) ahead and to the sides.
  it.each([
    [0, 0, Infinity, [0, 0, -12]], [0, 3, Infinity, [8, -4, -8]], [1, 1, Infinity, [-16, 0, -10]], [1, 2, Infinity, [4, 8, -6]],
    [2, 1, Infinity, [8, 4, -8]], [3, 0, 4, [8, 6, -14]], [4, 0, Infinity, [-8, -6, -20]], [5, 0, Infinity, [-8, -4, -20]], [5, 0, 8, [8, 4, -16]], [0, 5, Infinity, [0, -8, -4]],
  ] as const)('paints what a ray from the eye hits first (level %s, layer %s, cutoff %s, eye %j)', (level, layer, cutoff, eye) => {
    const slice = { level, layer, cutoff }
    const centre = sliceCentre(slice)
    // The voxels of the grid it reads, N times the lattice's.
    const n = atomSpacing(level)
    const sites = levels[atomLevel(level)].toVoxels().map(v => ({ ...v, ...toLattice(face, { x: n * v.x, y: n * v.y, z: n * v.z }) }))
      .filter(v => holds(slice, [v.a, v.b, v.c]))
      .map(v => ({ x: v.a * LAYER, y: v.b, z: v.c * LAYER, id: ids.get(v.color) ?? 0 }))
    expect(sites.length).toBeGreaterThan(20)
    const { width, height, pixels, originX, originY, scale } = baker.bake({ face: face.name, ...slice, i: 0, j: 0, eye, fill: false })
    const from = { x: eye[0] * LAYER, y: eye[1], z: eye[2] * LAYER }

    let compared = 0
    const wrong: string[] = []
    // About 1000 samples: ray casting is slow.
    for (let row = 0; row < height; row += height / 24) {
      for (let col = 0; col < width; col += width / 36) {
        const [i, j] = [Math.floor(col), Math.floor(row)]
        const px = originX + (i + 0.5) / scale
        const py = originY - (j + 0.5) / scale
        const expected = rayCast(sites, n, centre, px, py, from)
        if (expected === null) continue
        compared++
        const colour = expected < 0 ? 0 : palette[expected]
        if (pixels[j * width + i] !== colour) wrong.push(`(${String(px)}, ${String(py)}): ${String(pixels[j * width + i])} ≠ ${String(colour)}`)
      }
    }
    expect(compared).toBeGreaterThan(300)
    expect(wrong).toEqual([])
  }, 30_000)

  it('grows only the open faces of voxels going on into another slice', () => {
    // A floor two units thick through layers 0 to 7, seen from straight
    // ahead, level with it, so its sides facing up show only grown.
    const floor = VoxelGrid.fromVoxels(voxels.filter(v => v.b >= 0 && v.b < 2))
    const slice = { level: 1, layer: 1, cutoff: Infinity }
    const data = new ChunkBaker(floor.materials, [floor]).faces({ face: face.name, ...slice, i: 0, j: 0, eye: [0, 0, -20], fill: true })
    const filled = (a: number, b: number, c: number) => {
      const p = toWorld(face, a, b, c)
      return floor.get(p.x, p.y, p.z) !== 0
    }
    const grown = Array.from({ length: data.length / FACE }, (_, f) => f * FACE)
      .filter(o => data.subarray(o + 10, o + 14).some(g => g > 0))
      .map(o => {
        const [a, b, c] = data.subarray(o + 14, o + 17)
        const { a: da, b: db, c: dc } = kindOffsets[data[o + 8] % VOXEL_FACES.length]
        const crossing = kindOffsets.some(k => filled(a + k.a, b + k.b, c + k.c) && !holds(slice, [a + k.a, b + k.b, c + k.c]))
        return { open: !filled(a + da, b + db, c + dc), crossing, reached: data[o + 9] }
      })
    expect(grown.length).toBeGreaterThan(0)
    expect(grown.every(({ open, crossing }) => open && crossing)).toBe(true)
    expect(grown.some(({ reached }) => reached === 0)).toBe(true)
  })
})
