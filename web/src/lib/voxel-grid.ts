/**
 * Voxel storage on the face-centred cubic (FCC) lattice: what each site
 * of a box looks like (0 = empty), in bricks, only the bricks with voxels
 * stored. Sites are the integer points with x + y + z even; each is the
 * centre of a rhombic dodecahedron (fcc.ts).
 * Plain typed arrays, the same layout in TS or Rust/WASM (fcc's Grid).
 */

import { VOXEL_FACES } from './fcc'
import { copyIn, copyOut, wasmInstance } from './wasm'
import type { Vec3 } from './world'

/** A grid's fields without its methods: what a worker receives from postMessage. */
export type VoxelData = Pick<VoxelGrid, 'minX' | 'minY' | 'minZ' | 'nx' | 'ny' | 'nz' | 'bricks' | 'cells' | 'views' | 'solid'>

/** Half-open integer box: x in [minX, maxX), y and z likewise. */
export interface Box {
  minX: number
  minY: number
  minZ: number
  maxX: number
  maxY: number
  maxZ: number
}

/**
 * Bricks are BRICK lattice units wide along each axis, BRICK_SITES sites.
 * In the forest about a fifth of them hold voxels (the ground, the trees),
 * so the grid takes about a fifth of the bytes it would densely.
 */
export const BRICK = 8
const BRICK_SITES = BRICK ** 3 / 2
/** Words of `solid` per brick: a bit a site. */
const SOLID_WORDS = BRICK_SITES / 32

/** Words per site in a grid's `views`: its index in `cells`, then its look through each face (fcc's VIEW_WORDS). */
export const VIEW_WORDS = 1 + VOXEL_FACES.length

/**
 * Whether (x, y, z) is a site of the lattice.
 * @internal
 */
export const isSite = (x: number, y: number, z: number) => ((x + y + z) & 1) === 0

/**
 * The box is cut into bricks BRICK units wide, from (minX, minY, minZ),
 * laid out z-major, then y, then x. A brick with voxels has BRICK_SITES
 * words of `cells`, its sites z-major, then y, then x, a row (y, z)
 * holding every other x: those with x + y + z even. minX and nx are even,
 * so x − minX and its row's other sites share x div 2.
 *
 * A voxel looks as its site's word in `cells` says, unless `views` gives
 * it a look of its own through a face (VOXEL_FACES in fcc.ts): what is
 * seen of it looking at that face. Walking is stopped by those `solid` marks.
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
  /** The look of each site of the bricks with voxels (0 is empty), and room for more bricks. */
  cells: Uint32Array
  /** The sites that look unlike through some faces, by index in `cells`, VIEW_WORDS words each: the index, then the look through each face. */
  views: Uint32Array = new Uint32Array(0)
  /** Bit i % 32 of word i / 32: whether the voxel at index i of `cells` stops a walking body (walk.ts). */
  solid: Uint32Array
  // Bricks along x and y.
  private readonly bx: number
  private readonly by: number
  // Bricks in `cells`.
  private used = 0

  /** A grid of the sites in `box`, widened by one in x if needed to start and end on an even x. */
  constructor(box: Box) {
    this.minX = Math.floor(box.minX / 2) * 2
    this.minY = box.minY
    this.minZ = box.minZ
    this.nx = Math.max(0, Math.ceil(box.maxX / 2) * 2 - this.minX)
    this.ny = Math.max(0, box.maxY - box.minY)
    this.nz = Math.max(0, box.maxZ - box.minZ)
    this.bx = Math.ceil(this.nx / BRICK)
    this.by = Math.ceil(this.ny / BRICK)
    this.bricks = new Int32Array(this.bx * this.by * Math.ceil(this.nz / BRICK)).fill(-1)
    this.cells = new Uint32Array(0)
    this.solid = new Uint32Array(0)
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

  /** The look at (x, y, z); 0 if empty, outside the box or not a site. */
  get(x: number, y: number, z: number): number {
    const i = this.slot(x, y, z)
    return i < 0 ? 0 : this.cells[i]
  }

  /** Whether the voxel at (x, y, z) stops a walking body. */
  isSolid(x: number, y: number, z: number): boolean {
    const i = this.slot(x, y, z)
    return i >= 0 && ((this.solid[i >> 5] >>> (i & 31)) & 1) === 1
  }

  /** Where the site at `i` in `cells` is in `views`, or −1 if it looks alike through every face. */
  private viewRecord(i: number): number {
    let [lo, hi] = [0, this.views.length / VIEW_WORDS]
    while (lo < hi) {
      const mid = (lo + hi) >> 1
      const at = this.views[mid * VIEW_WORDS]
      if (at === i) return mid * VIEW_WORDS
      if (at < i) lo = mid + 1
      else hi = mid
    }
    return -1
  }

  /** The look through face `kind` (of VOXEL_FACES) of the voxel at (x, y, z); 0 if empty, outside the box or not a site. */
  face(x: number, y: number, z: number, kind: number): number {
    const i = this.slot(x, y, z)
    if (i < 0 || this.cells[i] === 0) return 0
    const at = this.views.length > 0 ? this.viewRecord(i) : -1
    return at < 0 ? this.cells[i] : this.views[at + 1 + kind]
  }

  /**
   * Sets the views of the voxels `sites` holds, each its position and
   * its look through each face (by VOXEL_FACES), as the only ones that
   * look unlike through some faces. Sites not set yet are left out.
   * @internal
   */
  setViews(sites: Iterable<readonly [number, number, number, ArrayLike<number>]>) {
    const views: Uint32Array[] = []
    for (const [x, y, z, looks] of sites) {
      const i = this.slot(x, y, z)
      if (i >= 0 && this.cells[i] !== 0) views.push(Uint32Array.of(i, ...Array.from(looks)))
    }
    this.views = viewsOf(views)
  }

  /**
   * Sets site (x, y, z) to look `look` (0 clears) through every face,
   * stopping a walking body if `solid`; outside the box or off the sites, nothing.
   */
  set(x: number, y: number, z: number, look: number, solid = look !== 0) {
    let i = this.slot(x, y, z)
    const at = i >= 0 && this.views.length > 0 ? this.viewRecord(i) : -1
    if (at >= 0) {
      const views = new Uint32Array(this.views.length - VIEW_WORDS)
      views.set(this.views.subarray(0, at))
      views.set(this.views.subarray(at + VIEW_WORDS), at)
      this.views = views
    }
    if (i === -1 || (i < 0 && look === 0)) return
    if (i < 0) {
      const brick = -2 - i
      if (this.used * BRICK_SITES === this.cells.length) {
        const bricks = Math.max(16, Math.ceil(this.used * 1.25))
        const grown = new Uint32Array(bricks * BRICK_SITES)
        grown.set(this.cells)
        this.cells = grown
        const solid = new Uint32Array(bricks * SOLID_WORDS)
        solid.set(this.solid)
        this.solid = solid
      }
      this.bricks[brick] = this.used++
      i = this.slot(x, y, z)
    }
    this.cells[i] = look
    const bit = 1 << (i & 31)
    this.solid[i >> 5] = solid && look !== 0 ? this.solid[i >> 5] | bit : this.solid[i >> 5] & ~bit
  }

  /** Calls `visit` with every voxel's position and look, ordered by z, then y, then x. */
  forEach(visit: (x: number, y: number, z: number, look: number) => void) {
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
   * The grid of `box`, its bricks copied from `parts`: each a whole number
   * of bricks from the box's corner, no two with voxels in one brick of
   * it, but only the bricks whose corner (x, z) it `keeps`. The rest of
   * the box is empty.
   */
  static compose(box: Box, parts: readonly VoxelData[], keeps: (x: number, z: number) => boolean = () => true): VoxelGrid {
    const grid = new VoxelGrid(box)
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
    grid.cells = new Uint32Array(count * BRICK_SITES)
    grid.solid = new Uint32Array(count * SOLID_WORDS)
    const views: Uint32Array[] = []
    for (const part of parts) {
      // Where each of the part's bricks of sites went, if it did.
      const moved = new Map<number, number>()
      inBox(part, (brick, at) => {
        moved.set(at, grid.used)
        grid.bricks[brick] = grid.used
        grid.cells.set(part.cells.subarray(at * BRICK_SITES, (at + 1) * BRICK_SITES), grid.used * BRICK_SITES)
        grid.solid.set(part.solid.subarray(at * SOLID_WORDS, (at + 1) * SOLID_WORDS), grid.used++ * SOLID_WORDS)
      })
      for (let k = 0; k < part.views.length; k += VIEW_WORDS) {
        const to = moved.get(Math.floor(part.views[k] / BRICK_SITES))
        if (to === undefined) continue
        const site = part.views.slice(k, k + VIEW_WORDS)
        site[0] = to * BRICK_SITES + (part.views[k] % BRICK_SITES)
        views.push(site)
      }
    }
    grid.views = viewsOf(views)
    return grid
  }
}

/** A grid's `views` of sites' words (VIEW_WORDS each), ordered by index. */
function viewsOf(sites: Uint32Array[]): Uint32Array {
  sites.sort((a, b) => a[0] - b[0])
  const views = new Uint32Array(sites.length * VIEW_WORDS)
  sites.forEach((site, k) => { views.set(site, k * VIEW_WORDS) })
  return views
}

/**
 * Hands a grid to a WebAssembly instance (bake/): `start` takes its box
 * and how many bricks of sites it uses and returns where to write its
 * bricks, then `cells` where to write those sites, `views`, if given,
 * where to write how many sites' views, and `solid`, if given, where to
 * write its solid bits.
 */
export function writeGrid(
  memory: WebAssembly.Memory,
  grid: VoxelData,
  start: (minX: number, minY: number, minZ: number, nx: number, ny: number, nz: number, used: number) => number,
  cells: () => number,
  views?: (sites: number) => number,
  solid?: () => number,
) {
  const { minX, minY, minZ, nx, ny, nz, bricks } = grid
  const used = bricks.reduce((n, at) => Math.max(n, at + 1), 0)
  copyIn(memory, start(minX, minY, minZ, nx, ny, nz, used), bricks)
  copyIn(memory, cells(), grid.cells.subarray(0, used * BRICK_SITES))
  if (views && grid.views.length > 0) copyIn(memory, views(grid.views.length / VIEW_WORDS), grid.views)
  if (solid) copyIn(memory, solid(), grid.solid.subarray(0, used * SOLID_WORDS))
}

/** Width of a column block of LayerIndex, in lattice units along a. */
const EXTENT_BLOCK = 8

/** Per face whose forward has its first non-zero component positive. */
interface Table {
  cMin: number
  aMin: number
  bMin: number
  layers: number
  blocks: number
  /** By (c − cMin, block of a − aMin): the least and greatest b − bMin, or least > greatest if there is no voxel. */
  low: Int16Array
  high: Int16Array
}

/** A LayerIndex's fields, as posted to or from a worker (a class's methods do not survive posting). */
export type LayerIndexData = Pick<LayerIndex, 'keys' | 'tables'>

/** The indexing exports (web/wasm/src/index.rs). */
interface Indexer {
  memory: WebAssembly.Memory
  index_grid: (minX: number, minY: number, minZ: number, nx: number, ny: number, nz: number, used: number) => number
  index_cells: () => number
  index_build: () => number
}

/** Numbers per table in index_build's result, and tables: a face and its opposite share one. */
const TABLE = 10
const TABLES = 6

// An instance per thread, made on first use.
let indexer: Indexer | null = null

/**
 * Where a grid's voxels are along each face's layers (in a face's lattice
 * coordinates (a, b, c), see Face in view.ts): for each layer c and block
 * of EXTENT_BLOCK columns a, the range of b of its voxels. Made by
 * web/wasm/src/index.rs, so the renderer can tell which chunk cells are
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
      const [x, y, z, cMin, aMin, bMin, layers, blocks, low, high] = result.subarray(t * TABLE, (t + 1) * TABLE)
      keys.push(LayerIndex.key({ x, y, z }))
      // Copied out: the next grid frees them.
      const table = (from: number) => copyOut(wasm.memory, Int16Array, from, layers * blocks)
      return { cMin, aMin, bMin, layers, blocks, low: table(low), high: table(high) }
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
    const [{ cMin, aMin, bMin, layers, blocks, low, high }, cFrom, cTo, aFrom, aTo] = this.table(lattice, c0, c1, a0, a1)
    const k0 = Math.max(0, Math.floor((aFrom - aMin) / EXTENT_BLOCK))
    const k1 = Math.min(blocks - 1, Math.floor((aTo - 1 - aMin) / EXTENT_BLOCK))
    let [lo, hi] = [Infinity, -Infinity]
    for (let c = Math.max(cFrom - cMin, 0); c < Math.min(cTo - cMin, layers); c++) {
      for (let k = c * blocks + k0; k <= c * blocks + k1; k++) {
        if (low[k] < lo) lo = low[k]
        if (high[k] > hi) hi = high[k]
      }
    }
    return lo <= hi ? [bMin + lo, bMin + hi] : null
  }
}
