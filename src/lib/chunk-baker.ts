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
import type { VoxelData } from './voxel-grid'

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
  world_grid: (minX: number, minY: number, minZ: number, nx: number, ny: number, nz: number) => number
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
 * cells. `levels` are the world's grids by level of detail (World.levels),
 * all of the same materials.
 */
export class ChunkBaker {
  private baker: Baker

  constructor(levels: readonly VoxelData[]) {
    const { materials } = levels[0]
    const baker = bakerInstance() as unknown as Baker
    // Memory may grow on each call, detaching earlier views of it.
    const write = <T extends Uint8Array | Int32Array | Uint32Array | Float64Array>(View: new (buffer: ArrayBuffer, offset: number, length: number) => T, offset: number, values: T) => {
      new View(baker.memory.buffer, offset, values.length).set(values)
    }
    baker.world_start(materials.length)
    for (const { minX, minY, minZ, nx, ny, nz, cells } of levels) write(Uint8Array, baker.world_grid(minX, minY, minZ, nx, ny, nz), cells)
    write(Float64Array, baker.world_table(0), VOXEL_GEOMETRY)
    write(Int32Array, baker.world_table(1), Int32Array.from(FACES.flatMap(({ lattice: { right, up, forward } }) => [right, up, forward].flatMap(v => [v.x, v.y, v.z]))))
    const palettes = FACES.map(face => shadedPalette(materials, face.lattice))
    palettes.forEach((palette, f) => { write(Uint32Array, baker.world_table(2) + f * palette.byteLength, palette) })
    baker.world_ready()
    this.baker = baker
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
 * its results echo), or the chunks to bake, most wanted first, replacing
 * the previous list. `received` is the `seq` of the worker's last result
 * the main thread had taken when it listed them.
 */
export type BakeMessage =
  | { type: 'world'; generation: number; levels: VoxelData[] }
  | { type: 'want'; requests: Float64Array; received: number }

/** A bake worker's message: chunks baked, `seq` numbering the worker's messages. */
export interface BakeResult {
  generation: number
  seq: number
  baked: { request: ChunkRequest; image: ChunkImage }[]
}

/**
 * A bake worker's message handler: bakes the latest list in `schedule`d
 * steps, so a new list (handled between steps) replaces the rest of the
 * old one. A step bakes a chunk, and more until `budget` ms have passed,
 * and posts them at once, handing over their buffers.
 * Chunks posted after the list's `received` are on their way, and skipped.
 */
export function bakeHandler(
  post: (result: BakeResult, transfer: Transferable[]) => void,
  schedule: (step: () => void) => void,
  budget = 0,
): (message: BakeMessage) => void {
  let baker: ChunkBaker | null = null
  let generation = 0
  let queue: Float64Array = new Float64Array(0)
  let next = 0
  let seq = 0
  // Chunks posted, by key: the seq they were posted with.
  const posted = new Map<string, number>()
  let scheduled = false
  const remaining = () => next < queue.length / REQUEST
  const step = () => {
    scheduled = false
    const start = performance.now()
    const baked: BakeResult['baked'] = []
    while (baker && remaining() && (baked.length === 0 || performance.now() - start < budget)) {
      const request = decodeRequest(queue, next++)
      const key = requestKey(request)
      if (posted.has(key)) continue
      baked.push({ request, image: baker.bake(request) })
      posted.set(key, seq + 1)
    }
    if (baked.length > 0) post({ generation, seq: ++seq, baked }, baked.flatMap(({ image }) => [image.pixels.buffer, image.blocks.buffer]))
    wake()
  }
  const wake = () => {
    if (scheduled || !baker || !remaining()) return
    scheduled = true
    schedule(step)
  }
  return message => {
    if (message.type === 'world') {
      baker = new ChunkBaker(message.levels)
      generation = message.generation
      queue = new Float64Array(0)
      posted.clear()
    } else {
      // Taken by the main thread: listed again, they are wanted again.
      for (const [key, s] of posted) if (s <= message.received) posted.delete(key)
      queue = message.requests
      next = 0
      wake()
    }
  }
}
