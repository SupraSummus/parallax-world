/**
 * Voxel storage on the face-centred cubic (FCC) lattice: one material id
 * per site of a box (0 = empty), in bricks, only the bricks with voxels
 * stored. Sites are the integer points with x + y + z even; each is the
 * centre of a rhombic dodecahedron (fcc.ts).
 * Plain typed arrays and integer ids, the same layout in TS or Rust/WASM.
 */

import { VOXEL_FACES } from './fcc'
import { copyIn, copyOut, wasmInstance } from './wasm'
import type { Vec3 } from './world'

export interface Voxel {
  x: number
  y: number
  z: number
  color: string
}

export interface Material {
  /** '#rrggbb'; '' for id 0, empty. */
  color: string
}

/** A grid's fields without its methods: what a worker receives from postMessage. */
export type VoxelData = Pick<VoxelGrid, 'minX' | 'minY' | 'minZ' | 'nx' | 'ny' | 'nz' | 'bricks' | 'cells' | 'faces' | 'materials'>

/** Half-open integer box: x in [minX, maxX), y and z likewise. */
export interface Box {
  minX: number
  minY: number
  minZ: number
  maxX: number
  maxY: number
  maxZ: number
}

/** Material ids are bytes. */
const MAX_MATERIALS = 256

/**
 * Bricks are BRICK lattice units wide along each axis, BRICK_SITES sites.
 * In the forest about a fifth of them hold voxels (the ground, the trees),
 * so the grid takes about a fifth of the bytes it would densely.
 */
export const BRICK = 8
const BRICK_SITES = BRICK ** 3 / 2

/** Words per site in a grid's `faces`: its index in `cells`, then its faces' materials, four to a word (bake's FACE_WORDS). */
export const FACE_WORDS = 4

/**
 * Whether (x, y, z) is a site of the lattice.
 * @internal
 */
export const isSite = (x: number, y: number, z: number) => ((x + y + z) & 1) === 0

/**
 * The box is cut into bricks BRICK units wide, from (minX, minY, minZ),
 * laid out z-major, then y, then x. A brick with voxels has BRICK_SITES
 * bytes of `cells`, its sites z-major, then y, then x, a row (y, z)
 * holding every other x: those with x + y + z even. minX and nx are even,
 * so x − minX and its row's other sites share x div 2.
 *
 * A voxel has a material on each of its faces (VOXEL_FACES in fcc.ts):
 * what is seen of it looking at that face. Mostly they are one, its
 * site's in `cells`; `faces` lists the sites whose faces differ.
 */
export class VoxelGrid {
  readonly minX: number
  readonly minY: number
  readonly minZ: number
  readonly nx: number
  readonly ny: number
  readonly nz: number
  /** Per brick, where its sites start in `cells`, in BRICK_SITES; −1 if it is empty. */
  readonly bricks: Int32Array
  /** Material id per site of the bricks with voxels (0 is empty), and room for more bricks. */
  cells: Uint8Array
  /**
   * The sites whose faces are not all of their material in `cells`, by
   * index in `cells`, FACE_WORDS words each: the index, then the faces'
   * materials, four to a word, the first in the lowest byte.
   */
  faces: Uint32Array = new Uint32Array(0)
  /** Material of each id; entry 0 (empty) is unused. */
  readonly materials: readonly Material[]
  // Bricks along x and y.
  private readonly bx: number
  private readonly by: number
  // Bricks in `cells`.
  private used = 0

  /** A grid of the sites in `box`, widened by one in x if needed to start and end on an even x. */
  constructor(box: Box, materials: readonly Material[]) {
    if (materials.length > MAX_MATERIALS) throw new RangeError(`at most ${String(MAX_MATERIALS)} materials`)
    this.minX = Math.floor(box.minX / 2) * 2
    this.minY = box.minY
    this.minZ = box.minZ
    this.nx = Math.max(0, Math.ceil(box.maxX / 2) * 2 - this.minX)
    this.ny = Math.max(0, box.maxY - box.minY)
    this.nz = Math.max(0, box.maxZ - box.minZ)
    this.bx = Math.ceil(this.nx / BRICK)
    this.by = Math.ceil(this.ny / BRICK)
    this.bricks = new Int32Array(this.bx * this.by * Math.ceil(this.nz / BRICK)).fill(-1)
    this.cells = new Uint8Array(0)
    this.materials = materials
  }

  /** Where site (x, y, z) is in `cells`: −1 outside the box or if it is not a site, −2 − its brick's index if that is empty. */
  private slot(x: number, y: number, z: number): number {
    const i = x - this.minX
    const j = y - this.minY
    const k = z - this.minZ
    if (i < 0 || i >= this.nx || j < 0 || j >= this.ny || k < 0 || k >= this.nz || !isSite(x, y, z)) return -1
    const brick = ((k >> 3) * this.by + (j >> 3)) * this.bx + (i >> 3)
    const at = this.bricks[brick]
    return at < 0 ? -2 - brick : at * BRICK_SITES + ((((k & 7) * BRICK + (j & 7)) * BRICK + (i & 7)) >> 1)
  }

  /** Material id at (x, y, z); 0 if empty, outside the box or not a site. */
  get(x: number, y: number, z: number): number {
    const i = this.slot(x, y, z)
    return i < 0 ? 0 : this.cells[i]
  }

  /** Where the site at `i` in `cells` is in `faces`, or −1 if its faces are its material. */
  private faceRecord(i: number): number {
    let [lo, hi] = [0, this.faces.length / FACE_WORDS]
    while (lo < hi) {
      const mid = (lo + hi) >> 1
      const at = this.faces[mid * FACE_WORDS]
      if (at === i) return mid * FACE_WORDS
      if (at < i) lo = mid + 1
      else hi = mid
    }
    return -1
  }

  /** The material of face `kind` (of VOXEL_FACES) of the voxel at (x, y, z); 0 if empty, outside the box or not a site. */
  face(x: number, y: number, z: number, kind: number): number {
    const i = this.slot(x, y, z)
    if (i < 0 || this.cells[i] === 0) return 0
    const at = this.faces.length > 0 ? this.faceRecord(i) : -1
    return at < 0 ? this.cells[i] : (this.faces[at + 1 + (kind >> 2)] >>> (8 * (kind & 3))) & 255
  }

  /** Whether the faces of the voxel at (x, y, z) are not all of its site's material. */
  facesDiffer(x: number, y: number, z: number): boolean {
    if (this.faces.length === 0) return false
    const i = this.slot(x, y, z)
    return i >= 0 && this.faceRecord(i) >= 0
  }

  /**
   * Sets the faces of the voxels `sites` holds, each its position and
   * its faces' materials (by VOXEL_FACES), as the only ones whose faces
   * differ from their site's material. Sites not set yet are left out.
   */
  setFaces(sites: Iterable<readonly [number, number, number, ArrayLike<number>]>) {
    const faces: Uint32Array[] = []
    for (const [x, y, z, materials] of sites) {
      const i = this.slot(x, y, z)
      if (i >= 0 && this.cells[i] !== 0) faces.push(faceWords(i, materials))
    }
    this.faces = facesOf(faces)
  }

  /** Sets site (x, y, z) to material `id` (0 clears), all its faces; outside the box or off the sites, nothing. */
  set(x: number, y: number, z: number, id: number) {
    let i = this.slot(x, y, z)
    const at = i >= 0 && this.faces.length > 0 ? this.faceRecord(i) : -1
    if (at >= 0) {
      const faces = new Uint32Array(this.faces.length - FACE_WORDS)
      faces.set(this.faces.subarray(0, at))
      faces.set(this.faces.subarray(at + FACE_WORDS), at)
      this.faces = faces
    }
    if (i === -1 || (i < 0 && id === 0)) return
    if (i < 0) {
      const brick = -2 - i
      if (this.used * BRICK_SITES === this.cells.length) {
        const grown = new Uint8Array(Math.max(16, Math.ceil(this.used * 1.25)) * BRICK_SITES)
        grown.set(this.cells)
        this.cells = grown
      }
      this.bricks[brick] = this.used++
      i = this.slot(x, y, z)
    }
    this.cells[i] = id
  }

  /** Fills the sites in the box [x0, x1) × [y0, y1) × [z0, z1) with material `id` (0 clears), clipped to the grid. */
  fillBox(x0: number, y0: number, z0: number, x1: number, y1: number, z1: number, id: number) {
    const [fromX, toX] = [Math.max(x0, this.minX), Math.min(x1, this.minX + this.nx)]
    for (let z = Math.max(z0, this.minZ); z < Math.min(z1, this.minZ + this.nz); z++) {
      for (let y = Math.max(y0, this.minY); y < Math.min(y1, this.minY + this.ny); y++) {
        for (let x = fromX + ((fromX + y + z) & 1); x < toX; x += 2) this.set(x, y, z, id)
      }
    }
  }

  /** Whether any brick meeting the box [x0, x1) × [y0, y1) × [z0, z1) holds voxels. */
  mayHold(x0: number, y0: number, z0: number, x1: number, y1: number, z1: number): boolean {
    const range = (lo: number, hi: number, min: number, n: number) =>
      [Math.max(0, Math.floor((lo - min) / BRICK)), Math.min(Math.ceil(n / BRICK), Math.ceil((hi - min) / BRICK))]
    const [i0, i1] = range(x0, x1, this.minX, this.nx)
    const [j0, j1] = range(y0, y1, this.minY, this.ny)
    const [k0, k1] = range(z0, z1, this.minZ, this.nz)
    for (let k = k0; k < k1; k++) {
      for (let j = j0; j < j1; j++) {
        for (let i = i0; i < i1; i++) if (this.bricks[(k * this.by + j) * this.bx + i] >= 0) return true
      }
    }
    return false
  }

  /** Calls `visit` with every voxel's position and material id, ordered by z, then y, then x. */
  forEach(visit: (x: number, y: number, z: number, id: number) => void) {
    const { minX, minY, minZ, nx, ny, nz } = this
    for (let z = minZ; z < minZ + nz; z++) {
      for (let y = minY; y < minY + ny; y++) {
        for (let x = minX + ((minX + y + z) & 1); x < minX + nx; x += 2) {
          const i = this.slot(x, y, z)
          if (i < 0) {
            // An empty brick: on to the next one's first site.
            const next = minX + ((x - minX) | (BRICK - 1)) + 1
            x = next + ((next + y + z) & 1) - 2
            continue
          }
          if (this.cells[i] !== 0) visit(x, y, z, this.cells[i])
        }
      }
    }
  }

  /** Every voxel, ordered by z, then y, then x. */
  toVoxels(): Voxel[] {
    const voxels: Voxel[] = []
    this.forEach((x, y, z, id) => {
      voxels.push({ x, y, z, color: this.materials[id].color })
    })
    return voxels
  }

  /**
   * The grid of `box`, its bricks copied from `parts`, grids of the same
   * materials: each a whole number of bricks from the box's corner, no two
   * with voxels in one brick of it, but only the bricks whose corner
   * (x, z) it `keeps`. The rest of the box is empty.
   */
  static compose(box: Box, materials: readonly Material[], parts: readonly VoxelData[], keeps: (x: number, z: number) => boolean = () => true): VoxelGrid {
    const grid = new VoxelGrid(box, materials)
    const bz = Math.ceil(grid.nz / BRICK)
    // Calls `visit` with each of a part's bricks with voxels in the box: its index in the grid, and where its sites are in the part.
    const inBox = ({ minX, minY, minZ, nx, ny, nz, bricks }: VoxelData, visit: (brick: number, at: number) => void) => {
      const [i0, j0, k0] = [(minX - grid.minX) / BRICK, (minY - grid.minY) / BRICK, (minZ - grid.minZ) / BRICK]
      if (![i0, j0, k0].every(Number.isInteger)) throw new RangeError('a part is not a whole number of bricks from the box')
      const [px, py, pz] = [Math.ceil(nx / BRICK), Math.ceil(ny / BRICK), Math.ceil(nz / BRICK)]
      for (let k = Math.max(0, -k0); k < Math.min(pz, bz - k0); k++) {
        for (let j = Math.max(0, -j0); j < Math.min(py, grid.by - j0); j++) {
          for (let i = Math.max(0, -i0); i < Math.min(px, grid.bx - i0); i++) {
            if (!keeps(minX + i * BRICK, minZ + k * BRICK)) continue
            const at = bricks[(k * py + j) * px + i]
            if (at >= 0) visit(((k + k0) * grid.by + j + j0) * grid.bx + i + i0, at)
          }
        }
      }
    }
    let count = 0
    for (const part of parts) inBox(part, () => { count++ })
    grid.cells = new Uint8Array(count * BRICK_SITES)
    const faces: Uint32Array[] = []
    for (const part of parts) {
      // Where each of the part's bricks of sites went, if it did.
      const moved = new Map<number, number>()
      inBox(part, (brick, at) => {
        moved.set(at, grid.used)
        grid.bricks[brick] = grid.used
        grid.cells.set(part.cells.subarray(at * BRICK_SITES, (at + 1) * BRICK_SITES), grid.used++ * BRICK_SITES)
      })
      for (let k = 0; k < part.faces.length; k += FACE_WORDS) {
        const to = moved.get(Math.floor(part.faces[k] / BRICK_SITES))
        if (to === undefined) continue
        const site = part.faces.slice(k, k + FACE_WORDS)
        site[0] = to * BRICK_SITES + (part.faces[k] % BRICK_SITES)
        faces.push(site)
      }
    }
    grid.faces = facesOf(faces)
    return grid
  }

  /** The smallest grid holding `voxels`, which must be sites; where two share a site, the later one wins. */
  static fromVoxels(voxels: readonly Voxel[]): VoxelGrid {
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

    const palette = new Palette()
    const grid = new VoxelGrid(box, palette.materials)
    for (const { x, y, z, color } of voxels) grid.set(x, y, z, palette.id(color))
    return grid
  }
}

/** Materials by colour, an id each, added as they are first used; id 0 is empty. */
class Palette {
  readonly materials: Material[] = [{ color: '' }]
  private readonly ids = new Map<string, number>()

  /** The id of `color` ('#rrggbb'). */
  id(color: string): number {
    let id = this.ids.get(color)
    if (id === undefined) {
      if (this.materials.length === MAX_MATERIALS) throw new RangeError(`at most ${String(MAX_MATERIALS)} materials`)
      id = this.materials.push({ color }) - 1
      this.ids.set(color, id)
    }
    return id
  }
}

/** A grid's `faces` of sites' words (FACE_WORDS each), ordered by index. */
function facesOf(sites: Uint32Array[]): Uint32Array {
  sites.sort((a, b) => a[0] - b[0])
  const faces = new Uint32Array(sites.length * FACE_WORDS)
  sites.forEach((site, k) => { faces.set(site, k * FACE_WORDS) })
  return faces
}

/** Site `index`'s words in `faces`, of its faces' materials. */
function faceWords(index: number, materials: ArrayLike<number>): Uint32Array {
  const site = new Uint32Array(FACE_WORDS)
  site[0] = index
  for (let k = 0; k < VOXEL_FACES.length; k++) site[1 + (k >> 2)] |= materials[k] << (8 * (k & 3))
  return site
}

/**
 * Hands a grid to a WebAssembly instance (bake/): `start` takes its box
 * and how many bricks of sites it uses and returns where to write its
 * bricks, then `cells` where to write those sites, and `faces`, if given,
 * where to write how many sites' faces.
 */
export function writeGrid(
  memory: WebAssembly.Memory,
  grid: VoxelData,
  start: (minX: number, minY: number, minZ: number, nx: number, ny: number, nz: number, used: number) => number,
  sites: () => number,
  faces?: (sites: number) => number,
) {
  const { minX, minY, minZ, nx, ny, nz, bricks } = grid
  const used = usedCells(grid)
  copyIn(memory, start(minX, minY, minZ, nx, ny, nz, used.length / BRICK_SITES), bricks)
  copyIn(memory, sites(), used)
  if (faces && grid.faces.length > 0) copyIn(memory, faces(grid.faces.length / FACE_WORDS), grid.faces)
}

/** The sites of a grid's bricks in use, without the room `cells` has for more. @internal */
export function usedCells({ bricks, cells }: VoxelData): Uint8Array {
  return cells.subarray(0, bricks.reduce((n, at) => Math.max(n, at + 1), 0) * BRICK_SITES)
}

/**
 * The parts of the fine sites that make up the cell of 2·FCC around a site
 * of it, times 6 (cells.ts): the site itself, a half of each of its 12
 * neighbours (on the cell's faces) and a sixth of each of the 6 sites two
 * away along an axis (at its corners), 8 sites in all.
 */
const COARSE_CELL: readonly (readonly [number, number, number, number])[] = [
  [0, 0, 0, 6],
  ...[-1, 1].flatMap(s => [-1, 1].flatMap(t => [[s, t, 0, 3], [s, 0, t, 3], [0, s, t, 3]] as const)),
  ...[-2, 2].flatMap(s => [[s, 0, 0, 1], [0, s, 0, 1], [0, 0, s, 1]] as const),
]

/**
 * How much of a cell (of 48, COARSE_CELL's weights) must be full for its
 * coarse voxel to be: a quarter. A plane of voxels one thick holds a
 * quarter of each of the two layers of cells it runs between (or nearly
 * half of the one it runs through), so it stays a plane; a half would lose
 * it, and with it every thin wall and floor seen from afar. Solids grow by
 * about half a coarse voxel where their surface runs between layers.
 */
const COARSE_FULL = 12

/**
 * The fine sites a cell's faces are measured from, x, y, z each:
 * COARSE_CELL's parts, then the sites next to them across their faces.
 * ACROSS[kind][k] is the index of part k's neighbour across face `kind`.
 */
const SITES: number[] = COARSE_CELL.flatMap(([x, y, z]) => [x, y, z])
const ACROSS = VOXEL_FACES.map(({ normal: [a, b, c] }) =>
  Int8Array.from(COARSE_CELL, ([x, y, z]) => {
    const [u, v, w] = [x + a, y + b, z + c]
    for (let i = 0; i < SITES.length; i += 3) if (SITES[i] === u && SITES[i + 1] === v && SITES[i + 2] === w) return i / 3
    return SITES.push(u, v, w) / 3 - 1
  }))
/** COARSE_CELL as numbers: x, y, z and weight of each part; VOXEL_FACES' normals, x, y, z each. */
const PARTS = Int8Array.from(COARSE_CELL.flat())
const NORMALS = Int8Array.from(VOXEL_FACES.flatMap(({ normal }) => normal))

/**
 * The grid one level of detail coarser: the sites of 2·FCC, in their own
 * units (site S stands for 2S of this grid). Each stands for its cell, and
 * is full if at least a quarter of the cell is (COARSE_FULL), with the
 * material most of it has; ties go to the lower id. With the same materials.
 * Each face that shows, with no coarse voxel across it, is of the material
 * most of the cell's open faces facing its way have (those with no voxel
 * across them); a face with none, or hidden, is of the site's: a coarse
 * voxel at a roof's edge is roof seen from above, wall from the side.
 */
export function coarsen(grid: VoxelGrid): VoxelGrid {
  const { minX, minY, minZ, nx, ny, nz } = grid
  // The coarse sites whose cells meet the box.
  const coarse = new VoxelGrid({
    minX: Math.floor((minX - 2) / 2), minY: Math.floor((minY - 2) / 2), minZ: Math.floor((minZ - 2) / 2),
    maxX: Math.ceil((minX + nx + 1) / 2), maxY: Math.ceil((minY + ny + 1) / 2), maxZ: Math.ceil((minZ + nz + 1) / 2),
  }, grid.materials)
  const weights = new Uint8Array(grid.materials.length)
  const ids: number[] = []
  // Adds a part's weight to its material's; the material of most weight, then empties them.
  const weigh = (id: number, weight: number) => {
    if (weights[id] === 0) ids.push(id)
    weights[id] += weight
  }
  const heaviest = () => {
    let best = 0
    for (const id of ids) if (weights[id] > weights[best] || (weights[id] === weights[best] && id < best)) best = id
    for (const id of ids) weights[id] = 0
    ids.length = 0
    return best
  }
  // The full sites whose faces may differ, x, y, z each: their cells hold more than one material, or faces that do.
  const mixed: number[] = []
  const hasFaces = grid.faces.length > 0
  const [endX, endY, endZ] = [coarse.minX + coarse.nx, coarse.minY + coarse.ny, coarse.minZ + coarse.nz]
  // Brick by coarse brick, skipping those whose cells meet no fine voxels.
  for (let bz = coarse.minZ; bz < endZ; bz += BRICK) {
    for (let by = coarse.minY; by < endY; by += BRICK) {
      for (let bx = coarse.minX; bx < endX; bx += BRICK) {
        if (!grid.mayHold(2 * bx - 2, 2 * by - 2, 2 * bz - 2, 2 * (bx + BRICK) + 2, 2 * (by + BRICK) + 2, 2 * (bz + BRICK) + 2)) continue
        for (let z = bz; z < Math.min(bz + BRICK, endZ); z++) {
          for (let y = by; y < Math.min(by + BRICK, endY); y++) {
            for (let x = bx + ((bx + y + z) & 1); x < Math.min(bx + BRICK, endX); x += 2) {
              let full = 0
              let differ = false
              for (let k = 0; k < PARTS.length; k += 4) {
                const id = grid.get(2 * x + PARTS[k], 2 * y + PARTS[k + 1], 2 * z + PARTS[k + 2])
                if (id === 0) continue
                weigh(id, PARTS[k + 3])
                full += PARTS[k + 3]
                differ ||= hasFaces && grid.facesDiffer(2 * x + PARTS[k], 2 * y + PARTS[k + 1], 2 * z + PARTS[k + 2])
              }
              differ ||= ids.length > 1
              const id = heaviest()
              if (full < COARSE_FULL) continue
              coarse.set(x, y, z, id)
              if (differ) mixed.push(x, y, z)
            }
          }
        }
      }
    }
  }

  // Their faces that show, with no coarse voxel across them.
  const near = new Uint8Array(SITES.length / 3)
  const materials = new Uint8Array(VOXEL_FACES.length)
  const faces: [number, number, number, Uint8Array][] = []
  let [x, y, z] = [0, 0, 0]
  // The material of SITES' j-th around (x, y, z), read once needed.
  const at = (j: number) => {
    if (near[j] === 255) near[j] = grid.get(2 * x + SITES[3 * j], 2 * y + SITES[3 * j + 1], 2 * z + SITES[3 * j + 2])
    return near[j]
  }
  for (let m = 0; m < mixed.length; m += 3) {
    x = mixed[m]
    y = mixed[m + 1]
    z = mixed[m + 2]
    const id = coarse.get(x, y, z)
    near.fill(255)
    let differ = false
    for (let kind = 0; kind < VOXEL_FACES.length; kind++) {
      materials[kind] = id
      if (coarse.get(x + NORMALS[3 * kind], y + NORMALS[3 * kind + 1], z + NORMALS[3 * kind + 2]) !== 0) continue
      for (let k = 0; k < COARSE_CELL.length; k++) {
        if (at(k) === 0 || at(ACROSS[kind][k]) !== 0) continue
        weigh(hasFaces ? grid.face(2 * x + SITES[3 * k], 2 * y + SITES[3 * k + 1], 2 * z + SITES[3 * k + 2], kind) : at(k), PARTS[4 * k + 3])
      }
      if (ids.length > 0) materials[kind] = heaviest()
      differ ||= materials[kind] !== id
    }
    if (differ) faces.push([x, y, z, materials.slice()])
  }
  coarse.setFaces(faces)
  return coarse
}

/** Width of a column block of LayerIndex, in lattice units along a. */
const EXTENT_BLOCK = 8

/** Per face whose forward has its first non-zero component positive. */
interface Table {
  cMin: number
  aMin: number
  layers: number
  blocks: number
  /** By (c − cMin, block of a − aMin): the least and greatest b, or least > greatest if there is no voxel. */
  low: Int16Array
  high: Int16Array
}

/** A LayerIndex's fields, as posted to or from a worker (a class's methods do not survive posting). */
export type LayerIndexData = Pick<LayerIndex, 'keys' | 'tables'>

/** The indexing exports (bake/src/layer_index.rs). */
interface Indexer {
  memory: WebAssembly.Memory
  index_grid: (minX: number, minY: number, minZ: number, nx: number, ny: number, nz: number, used: number) => number
  index_cells: () => number
  index_build: () => number
}

/** Numbers per table in index_build's result, and tables: a face and its opposite share one. */
const TABLE = 9
const TABLES = 6

// An instance per thread, made on first use.
let indexer: Indexer | null = null

/**
 * Where a grid's voxels are along each face's layers (in a face's lattice
 * coordinates (a, b, c), see Face in view.ts): for each layer c and block
 * of EXTENT_BLOCK columns a, the range of b of its voxels. Made by
 * bake/src/layer_index.rs, so the renderer can tell which chunk cells are
 * empty and how high their content reaches, without walking or baking them.
 */
export class LayerIndex {
  readonly keys: string[]
  readonly tables: Table[]

  constructor({ keys, tables }: LayerIndexData) {
    this.keys = keys
    this.tables = tables
  }

  /** The index of `grid`, for the faces of view.ts. */
  static of(grid: VoxelData): LayerIndex {
    const wasm = (indexer ??= wasmInstance() as unknown as Indexer)
    writeGrid(wasm.memory, grid, wasm.index_grid, wasm.index_cells)
    const result = copyOut(wasm.memory, Float64Array, wasm.index_build(), TABLES * TABLE)
    const keys: string[] = []
    const tables = Array.from({ length: TABLES }, (_, t) => {
      const [x, y, z, cMin, aMin, layers, blocks, low, high] = result.subarray(t * TABLE, (t + 1) * TABLE)
      keys.push(LayerIndex.key({ x, y, z }))
      // Copied out: the next grid frees them.
      const table = (from: number) => copyOut(wasm.memory, Int16Array, from, layers * blocks)
      return { cMin, aMin, layers, blocks, low: table(low), high: table(high) }
    })
    return new LayerIndex({ keys, tables })
  }

  /** Whether a face's forward is the opposite of one with its first non-zero component positive. */
  private static mirrored({ x, y, z }: Vec3): boolean {
    return (x || y || z) < 0
  }

  private static key({ x, y, z }: Vec3): string {
    const s = (x || y || z) < 0 ? -1 : 1
    return `${String(s * x)},${String(s * y)},${String(s * z)}`
  }

  /**
   * The face's table, and [c0, c1) and [a0, a1) in its coordinates: the
   * opposite face has the same up, and a and c negated, so c in [c0, c1)
   * is −c in [1 − c1, 1 − c0).
   */
  private table({ forward }: { forward: Vec3 }, c0: number, c1: number, a0: number, a1: number): [Table, number, number, number, number] {
    const table = this.tables[this.keys.indexOf(LayerIndex.key(forward))]
    return LayerIndex.mirrored(forward) ? [table, 1 - c1, 1 - c0, 1 - a1, 1 - a0] : [table, c0, c1, a0, a1]
  }

  /**
   * The least and greatest b of the voxels in a face's layers [c0, c1)
   * with a in [a0, a1) (or a little wider, by whole blocks), or null if
   * there are none.
   */
  range(lattice: { forward: Vec3 }, c0: number, c1: number, a0: number, a1: number): [number, number] | null {
    const [{ cMin, aMin, layers, blocks, low, high }, cFrom, cTo, aFrom, aTo] = this.table(lattice, c0, c1, a0, a1)
    const k0 = Math.max(0, Math.floor((aFrom - aMin) / EXTENT_BLOCK))
    const k1 = Math.min(blocks - 1, Math.floor((aTo - 1 - aMin) / EXTENT_BLOCK))
    let [lo, hi] = [Infinity, -Infinity]
    for (let c = Math.max(cFrom - cMin, 0); c < Math.min(cTo - cMin, layers); c++) {
      for (let k = c * blocks + k0; k <= c * blocks + k1; k++) {
        if (low[k] < lo) lo = low[k]
        if (high[k] > hi) hi = high[k]
      }
    }
    return lo <= hi ? [lo, hi] : null
  }
}
