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

export interface Material {
  /** '#rrggbb'; '' for id 0, empty. */
  color: string
  /** Whether a walking body goes through it, as through plants and water (walk.ts); scenes of voxels' are solid. */
  walkThrough?: boolean
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

  /**
   * Sets the faces of the voxels `sites` holds, each its position and
   * its faces' materials (by VOXEL_FACES), as the only ones whose faces
   * differ from their site's material. Sites not set yet are left out.
   * @internal
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

/** The sites of a grid's bricks in use, without the room `cells` has for more. */
function usedCells({ bricks, cells }: VoxelData): Uint8Array {
  return cells.subarray(0, bricks.reduce((n, at) => Math.max(n, at + 1), 0) * BRICK_SITES)
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
