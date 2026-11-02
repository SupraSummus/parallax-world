/**
 * Voxel storage on the face-centred cubic (FCC) lattice: one material id
 * per site of a box (0 = empty), in bricks, only the bricks with voxels
 * stored. Sites are the integer points with x + y + z even; each is the
 * centre of a rhombic dodecahedron (fcc.ts).
 * Plain typed arrays and integer ids, the same layout in TS or Rust/WASM.
 */

import type { Vec3 } from './world'

export interface Voxel {
  x: number
  y: number
  z: number
  color: string
  type: number
}

export interface Material {
  /** '#rrggbb' */
  color: string
  type: number
}

/** A grid's fields without its methods: what a worker receives from postMessage. */
export type VoxelData = Pick<VoxelGrid, 'minX' | 'minY' | 'minZ' | 'nx' | 'ny' | 'nz' | 'bricks' | 'cells' | 'materials'>

/** Half-open integer box: x in [minX, maxX), y and z likewise. */
interface Box {
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
export const BRICK_SITES = BRICK ** 3 / 2

/** Whether (x, y, z) is a site of the lattice. */
export const isSite = (x: number, y: number, z: number) => ((x + y + z) & 1) === 0

/**
 * The box is cut into bricks BRICK units wide, from (minX, minY, minZ),
 * laid out z-major, then y, then x. A brick with voxels has BRICK_SITES
 * bytes of `cells`, its sites z-major, then y, then x, a row (y, z)
 * holding every other x: those with x + y + z even. minX and nx are even,
 * so x − minX and its row's other sites share x div 2.
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

  /** Sets site (x, y, z) to material `id` (0 clears); outside the box or off the sites, nothing. */
  set(x: number, y: number, z: number, id: number) {
    let i = this.slot(x, y, z)
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
      const { color, type } = this.materials[id]
      voxels.push({ x, y, z, color, type })
    })
    return voxels
  }

  /**
   * The grid of `parts` one after the other along z: of one box but for z,
   * each but the last a whole number of bricks deep, nearest z first.
   */
  static stack(parts: readonly VoxelData[]): VoxelGrid {
    const [first, last] = [parts[0], parts[parts.length - 1]]
    const grid = new VoxelGrid({ minX: first.minX, minY: first.minY, minZ: first.minZ, maxX: first.minX + first.nx, maxY: first.minY + first.ny, maxZ: last.minZ + last.nz }, first.materials)
    const used = parts.map(({ bricks }) => bricks.reduce((n, at) => Math.max(n, at + 1), 0))
    grid.cells = new Uint8Array(used.reduce((sum, n) => sum + n, 0) * BRICK_SITES)
    parts.forEach(({ minZ, bricks, cells }, k) => {
      const offset = ((minZ - first.minZ) / BRICK) * grid.bx * grid.by
      for (let b = 0; b < bricks.length; b++) if (bricks[b] >= 0) grid.bricks[offset + b] = bricks[b] + grid.used
      grid.cells.set(cells.subarray(0, used[k] * BRICK_SITES), grid.used * BRICK_SITES)
      grid.used += used[k]
    })
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

    const materials: Material[] = [{ color: '#000000', type: 0 }]
    const ids = new Map<string, number>()
    const idOf = ({ color, type }: Voxel) => {
      const key = `${color}|${String(type)}`
      let id = ids.get(key)
      if (id === undefined) {
        id = materials.push({ color, type }) - 1
        ids.set(key, id)
      }
      return id
    }
    const voxelIds = voxels.map(idOf)
    const grid = new VoxelGrid(box, materials)
    voxels.forEach(({ x, y, z }, n) => { grid.set(x, y, z, voxelIds[n]) })
    return grid
  }
}

/**
 * Calls `visit` with the corner (x, y, z) of each brick of `grid` with
 * voxels (its first site is x or x + 1) and where its sites start in `cells`.
 */
function forEachBrick({ minX, minY, minZ, nx, ny, bricks }: VoxelData, visit: (x: number, y: number, z: number, at: number) => void) {
  const [bx, by] = [Math.ceil(nx / BRICK), Math.ceil(ny / BRICK)]
  for (let b = 0; b < bricks.length; b++) {
    if (bricks[b] < 0) continue
    visit(minX + (b % bx) * BRICK, minY + (Math.floor(b / bx) % by) * BRICK, minZ + Math.floor(b / (bx * by)) * BRICK, bricks[b] * BRICK_SITES)
  }
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

const PARTS = Int8Array.from(COARSE_CELL.flat())

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
 * The grid one level of detail coarser: the sites of 2·FCC, in their own
 * units (site S stands for 2S of this grid). Each stands for its cell, and
 * is full if at least a quarter of the cell is (COARSE_FULL), with the
 * material most of it has; ties go to the lower id. With the same materials.
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
              for (let k = 0; k < PARTS.length; k += 4) {
                const id = grid.get(2 * x + PARTS[k], 2 * y + PARTS[k + 1], 2 * z + PARTS[k + 2])
                if (id === 0) continue
                if (weights[id] === 0) ids.push(id)
                weights[id] += PARTS[k + 3]
                full += PARTS[k + 3]
              }
              if (full >= COARSE_FULL) {
                let best = 0
                for (const id of ids) if (weights[id] > weights[best] || (weights[id] === weights[best] && id < best)) best = id
                coarse.set(x, y, z, best)
              }
              for (const id of ids) weights[id] = 0
              ids.length = 0
            }
          }
        }
      }
    }
  }
  return coarse
}

/** Width of a column block of LayerIndex, in lattice units along a. */
const EXTENT_BLOCK = 8

/** A face's lattice vectors R, U, F (see Face in view.ts). */
interface Lattice {
  right: Vec3
  up: Vec3
  forward: Vec3
}

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

/** A LayerIndex's fields. */
export type LayerIndexData = Pick<LayerIndex, 'keys' | 'tables'>

/**
 * Where a grid's voxels are along each face's layers (in a face's lattice
 * coordinates (a, b, c), see Face in view.ts): for each layer c and block
 * of EXTENT_BLOCK columns a, the range of b of its voxels. From one pass
 * over the grid, so the renderer can tell which chunk cells are empty and
 * how high their content reaches, without walking or baking them.
 */
export class LayerIndex {
  readonly keys: string[]
  readonly tables: Table[]

  constructor(grid: VoxelData, lattices: readonly Lattice[]) {
    const { minX, minY, minZ, nx, ny, nz, cells } = grid
    // Bounds of v · p over the grid's box.
    const along = (v: Vec3) => {
      const ends = [0, 1].flatMap(i => [0, 1].flatMap(j => [0, 1].map(k =>
        v.x * (minX + i * Math.max(0, nx - 1)) + v.y * (minY + j * Math.max(0, ny - 1)) + v.z * (minZ + k * Math.max(0, nz - 1)))))
      return [Math.min(...ends), Math.max(...ends)]
    }
    // A table per pair of opposite faces (see table()).
    const canonical = lattices.filter(l => !LayerIndex.mirrored(l.forward))
    const shapes = canonical.filter((l, k) => canonical.findIndex(m => LayerIndex.key(m.forward) === LayerIndex.key(l.forward)) === k).map(lattice => {
      const [aMin, aMax] = along(lattice.right)
      const [cMin, cMax] = along(lattice.forward)
      return { lattice, aMin, cMin, layers: cMax - cMin + 1, blocks: Math.floor((aMax - aMin) / EXTENT_BLOCK) + 1 }
    })
    // All tables' arrays in one, for one pass over the grid.
    const n = shapes.length
    const lowAt = new Int32Array(n + 1)
    shapes.forEach(({ layers, blocks }, k) => { lowAt[k + 1] = lowAt[k] + layers * blocks })
    const low = new Int16Array(lowAt[n]).fill(32767)
    const high = new Int16Array(lowAt[n]).fill(-32768)
    const blocks = Int32Array.from(shapes, s => s.blocks)
    // Per table: a, b and c of site p are v · p − offset, for v = R, U, F.
    const vectors = Int32Array.from(shapes.flatMap(({ lattice: { right: R, up: U, forward: F } }) => [R, U, F].flatMap(v => [v.x, v.y, v.z])))
    const offsets = Int32Array.from(shapes.flatMap(s => [s.aMin, 0, s.cMin]))
    // a, b and c of the row's first site, per table.
    const row = new Int32Array(3 * n)
    const visit = (x0: number, y0: number, z0: number, at: number) => {
      for (let z = z0; z < z0 + BRICK; z++) {
        for (let y = y0; y < y0 + BRICK; y++) {
          const x1 = x0 + ((x0 + y + z) & 1)
          for (let m = 0; m < 3 * n; m++) row[m] = vectors[3 * m] * x1 + vectors[3 * m + 1] * y + vectors[3 * m + 2] * z - offsets[m]
          for (let dx = 0; dx < BRICK; dx += 2, at++) {
            if (cells[at] === 0) continue
            for (let k = 0; k < n; k++) {
              const v = 9 * k
              const a = row[3 * k] + vectors[v] * dx
              const b = row[3 * k + 1] + vectors[v + 3] * dx
              const c = row[3 * k + 2] + vectors[v + 6] * dx
              // a ≥ 0 here, and EXTENT_BLOCK is 8.
              const i = lowAt[k] + c * blocks[k] + (a >> 3)
              if (b < low[i]) low[i] = b
              if (b > high[i]) high[i] = b
            }
          }
        }
      }
    }
    forEachBrick(grid, visit)
    this.tables = shapes.map((shape, k) => ({ ...shape, low: low.subarray(lowAt[k], lowAt[k + 1]), high: high.subarray(lowAt[k], lowAt[k + 1]) }))
    this.keys = shapes.map(s => LayerIndex.key(s.lattice.forward))
  }

  /** A LayerIndex of its fields, as posted to or from a worker (a class's methods do not survive posting). */
  static from({ keys, tables }: LayerIndexData): LayerIndex {
    return Object.assign(Object.create(LayerIndex.prototype) as LayerIndex, { keys, tables })
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
