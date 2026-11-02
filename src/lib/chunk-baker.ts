/**
 * Baking whole chunks: from a chunk request to its pixels, by the baker in
 * bake/ (Rust, compiled to WebAssembly and inlined here). Used by the
 * renderer directly (without workers) and inside bake workers, through the
 * message protocol below. No DOM.
 */
import wasm from '../../bake/target/wasm32-unknown-unknown/release/bake.wasm?inline'
import { shadedPalette, VOXEL_GEOMETRY } from './bake'
import type { SliceId } from './slice-selection'
import { FACES } from './view'
import { BRICK_SITES, LayerIndex, type LayerIndexData, type Material, type VoxelData } from './voxel-grid'
import { generatePart, type GridPart, type WorldType } from './world'

/**
 * A chunk to bake: cell (i, j) (see chunkCell) of a slice (see SliceId) of
 * a face (by name, see FACES), seen from `eye` (a site in the face's
 * lattice coordinates, see eyeOf), with faces grown to fill gaps if `fill`
 * (see docs/architecture.md, section 3).
 */
export interface ChunkRequest extends SliceId {
  face: string
  i: number
  j: number
  eye: readonly [number, number, number]
  fill: boolean
}

/** A baked chunk's pixels, cropped to what is drawn. */
export interface ChunkImage {
  /** Size in pixels; see originX/originY. */
  width: number
  height: number
  /** RGBA, rows top to bottom; transparent where nothing is drawn. */
  pixels: Uint32Array
  /**
   * A byte per block of 4 × 4 pixels, in rows: whether all its pixels are
   * drawn, and whether some are (solid_blocks in bake/src/fill.rs), for
   * finding what the chunk hides and whether it is hidden (occlusion.ts).
   */
  blocks: Uint8Array
  /** x, y of the top-left pixel's corner, in face coordinates. */
  originX: number
  originY: number
  /** Pixels per world unit (render_scale in bake/src/lib.rs). */
  scale: number
  /** Number of voxel faces drawn. */
  facesDrawn: number
}

/** The baker's exports (bake/src/lib.rs). */
interface Baker {
  memory: WebAssembly.Memory
  world_start: (materials: number) => void
  world_grid: (level: number, minX: number, minY: number, minZ: number, nx: number, ny: number, nz: number, used: number) => number
  world_cells: () => number
  world_table: (table: number) => number
  world_ready: () => void
  bake: Call
  faces: Call
}
type Call = (face: number, level: number, layer: number, cutoff: number, i: number, j: number, eyeA: number, eyeB: number, eyeC: number, fill: number) => number

/**
 * Numbers per face in ChunkBaker.faces: its four corners on the
 * centre plane (x0, y0, …, x3, y3), its colour (an index into
 * shadedPalette), 1 if a ray from the eye reaches it, its
 * growth (left, bottom, right, top) and its voxel's site (a, b, c). See
 * bake/src/faces.rs.
 * @internal
 */
export const FACE = 17

// Compiled on first use, once per thread, synchronously
// (browsers allow that for a module this small even on the main thread).
let compiled: WebAssembly.Module | null = null
const bakerModule = () => (compiled ??= new WebAssembly.Module(Uint8Array.from(atob(wasm.slice(wasm.indexOf(',') + 1)), c => c.charCodeAt(0))))

/** A new instance of the module, with its own memory: for baking a world, or for Coverage (occlusion.ts). */
export function bakerInstance(): WebAssembly.Exports {
  return new WebAssembly.Instance(bakerModule()).exports
}

const faceIndex = new Map(FACES.map((face, index) => [face.name, index]))

/**
 * Bakes chunks of one world: a WebAssembly instance holding a copy of its
 * grids, the palette of each face, and the surfaces of recently baked
 * cells. `levels` are the world's grids by level of detail so far
 * (World.level), all of `materials`; the rest come with addGrid.
 */
export class ChunkBaker {
  private baker: Baker

  constructor(materials: readonly Material[], levels: readonly (VoxelData | undefined)[]) {
    const baker = bakerInstance() as unknown as Baker
    this.baker = baker
    baker.world_start(materials.length)
    this.write(Float64Array, baker.world_table(0), VOXEL_GEOMETRY)
    this.write(Int32Array, baker.world_table(1), Int32Array.from(FACES.flatMap(({ lattice: { right, up, forward } }) => [right, up, forward].flatMap(v => [v.x, v.y, v.z]))))
    const palettes = FACES.map(face => shadedPalette(materials, face.lattice))
    palettes.forEach((palette, f) => { this.write(Uint32Array, baker.world_table(2) + f * palette.byteLength, palette) })
    baker.world_ready()
    levels.forEach((grid, level) => { if (grid) this.addGrid(level, grid) })
  }

  /** Sets the grid of `level`; until then, its chunks are empty. */
  addGrid(level: number, { minX, minY, minZ, nx, ny, nz, bricks, cells }: VoxelData) {
    // Only the bricks in use: `cells` has room for more.
    const used = bricks.reduce((n, at) => Math.max(n, at + 1), 0)
    this.write(Int32Array, this.baker.world_grid(level, minX, minY, minZ, nx, ny, nz, used), bricks)
    this.write(Uint8Array, this.baker.world_cells(), cells.subarray(0, used * BRICK_SITES))
  }

  /** Copies `values` into the baker's memory at `offset`. Memory may grow on each call, detaching earlier views of it. */
  private write<T extends Uint8Array | Int32Array | Uint32Array | Float64Array>(View: new (buffer: ArrayBuffer, offset: number, length: number) => T, offset: number, values: T) {
    new View(this.baker.memory.buffer, offset, values.length).set(values)
  }

  /**
   * Rasterises one chunk: the slice seen from its eye, projected onto its
   * centre plane (docs/architecture.md, section 3), clipped to the chunk's
   * cell and cropped to what is drawn there. A chunk no face reaches has no
   * pixels.
   */
  bake(request: ChunkRequest): ChunkImage {
    const { memory } = this.baker
    const result = this.call(this.baker.bake, request, 9)
    const [width, height, originX, originY, scale, facesDrawn, pixels, blocks, blockCount] = result
    return {
      width,
      height,
      // Copied out: the baker reuses its buffers.
      pixels: new Uint32Array(memory.buffer, pixels, width * height).slice(),
      blocks: new Uint8Array(memory.buffer, blocks, blockCount).slice(),
      originX,
      originY,
      scale,
      facesDrawn,
    }
  }

  /** The faces of a chunk before filling, FACE numbers each, far to near. @internal */
  faces(request: ChunkRequest): Float64Array {
    const [count, at] = this.call(this.baker.faces, request, 2)
    return new Float64Array(this.baker.memory.buffer, at, count * FACE).slice()
  }

  /** Calls `bake` or `faces` and returns the first `length` numbers of its result. */
  private call(baker: Call, { face, level, layer, cutoff, i, j, eye, fill }: ChunkRequest, length: number): Float64Array {
    const index = faceIndex.get(face)
    if (index === undefined) throw new Error(`no face ${face}`)
    // Before reading memory.buffer: the call may grow (and so replace) it.
    const at = baker(index, level, layer, cutoff, i, j, ...eye, fill ? 1 : 0)
    if (at === 0) throw new Error('the baker has no world')
    return new Float64Array(this.baker.memory.buffer, at, length)
  }
}

/** Numbers per request in a `want` message (see encodeRequests). */
const REQUEST = 10

/** Requests packed into numbers, REQUEST each, to post to a worker cheaply. */
export function encodeRequests(requests: readonly ChunkRequest[]): Float64Array {
  const packed = new Float64Array(requests.length * REQUEST)
  requests.forEach(({ face, level, layer, cutoff, i, j, eye, fill }, n) => {
    const index = faceIndex.get(face)
    if (index === undefined) throw new Error(`no face ${face}`)
    packed.set([index, level, layer, cutoff, i, j, ...eye, fill ? 1 : 0], n * REQUEST)
  })
  return packed
}

/** The n-th request of encodeRequests' numbers. */
function decodeRequest(packed: Float64Array, n: number): ChunkRequest {
  const [face, level, layer, cutoff, i, j, a, b, c, fill] = packed.subarray(n * REQUEST, (n + 1) * REQUEST)
  return { face: FACES[face].name, level, layer, cutoff, i, j, eye: [a, b, c], fill: fill === 1 }
}

/** A key per chunk request. */
const requestKey = ({ face, level, layer, cutoff, i, j, eye, fill }: ChunkRequest) => [face, level, layer, cutoff, i, j, ...eye, fill].join()

/**
 * Messages to a bake worker: a new world (numbered by `generation`, which
 * its results echo) with its grids so far, a grid of it, parts of its
 * grids to generate (World.missingParts), or the chunks to bake, most
 * wanted first, replacing the previous list. `received` is the `seq` of
 * the worker's last result the main thread had taken when it listed them.
 */
export type BakeMessage =
  | { type: 'world'; generation: number; materials: readonly Material[]; levels: readonly (VoxelData | undefined)[] }
  | { type: 'grid'; generation: number; level: number; grid: VoxelData }
  | { type: 'generate'; generation: number; seed: number; worldType: WorldType; parts: GridPart[] }
  | { type: 'want'; requests: Float64Array; received: number }

/** A bake worker's result: a baked chunk, `seq` numbering the worker's chunks; or a generated part. */
export type BakeResult =
  | { type: 'chunk'; generation: number; seq: number; request: ChunkRequest; image: ChunkImage }
  | { type: 'part'; generation: number; part: GridPart; grid: VoxelData; index: LayerIndexData }

/**
 * A bake worker's message handler: bakes the latest list one chunk per
 * `schedule`d step, so a new list (handled between steps) replaces the rest
 * of the old one, and posts each chunk at once, handing over its buffers.
 * Chunks posted after the list's `received` are on their way, and skipped.
 * Parts to generate take steps of their own, as much time as baking while
 * there are chunks to bake.
 */
export function bakeHandler(
  post: (result: BakeResult, transfer: Transferable[]) => void,
  schedule: (step: () => void) => void,
): (message: BakeMessage) => void {
  let baker: ChunkBaker | null = null
  let generation = 0
  let queue: Float64Array = new Float64Array(0)
  let next = 0
  let seq = 0
  // Chunks posted, by key: the seq they were posted with.
  const posted = new Map<string, number>()
  let parts: { seed: number; worldType: WorldType; parts: GridPart[] } | null = null
  // Milliseconds spent baking, less those spent generating.
  let ahead = 0
  let scheduled = false
  const remaining = () => next < queue.length / REQUEST
  const bake = () => {
    while (baker && remaining()) {
      const request = decodeRequest(queue, next++)
      const key = requestKey(request)
      if (posted.has(key)) continue
      const image = baker.bake(request)
      posted.set(key, ++seq)
      post({ type: 'chunk', generation, seq, request, image }, [image.pixels.buffer, image.blocks.buffer])
      return
    }
  }
  const generate = () => {
    if (!parts) return
    const part = parts.parts.shift() as GridPart
    const grid = generatePart(parts.seed, parts.worldType, part)
    if (parts.parts.length === 0) parts = null
    // Where its voxels are, so the main thread need not find out.
    const { keys, tables } = new LayerIndex(grid, FACES.map(f => f.lattice))
    // Just the fields: a class's methods would not survive posting.
    const { minX, minY, minZ, nx, ny, nz, bricks, cells, materials } = grid
    post({ type: 'part', generation, part, grid: { minX, minY, minZ, nx, ny, nz, bricks, cells, materials }, index: { keys, tables } }, [bricks.buffer, cells.buffer, tables[0].low.buffer, tables[0].high.buffer])
  }
  const step = () => {
    scheduled = false
    const start = performance.now()
    const baking = !!baker && remaining() && (!parts || ahead <= 0)
    if (baking) bake()
    else generate()
    ahead += (baking ? 1 : -1) * (performance.now() - start)
    wake()
  }
  const wake = () => {
    if (scheduled || ((!baker || !remaining()) && !parts)) return
    scheduled = true
    schedule(step)
  }
  return message => {
    if (message.type === 'world') {
      baker = new ChunkBaker(message.materials, message.levels)
      generation = message.generation
      queue = new Float64Array(0)
      posted.clear()
      parts = null
      ahead = 0
    } else if (message.type === 'grid') {
      if (message.generation === generation) baker?.addGrid(message.level, message.grid)
    } else if (message.type === 'generate') {
      if (message.generation !== generation) return
      parts = { seed: message.seed, worldType: message.worldType, parts: [...(parts?.parts ?? []), ...message.parts] }
      wake()
    } else {
      // Taken by the main thread: listed again, they are wanted again.
      for (const [key, s] of posted) if (s <= message.received) posted.delete(key)
      queue = message.requests
      next = 0
      wake()
    }
  }
}
