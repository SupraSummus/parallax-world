/**
 * Baking whole chunks: from a chunk request to its splats, by the baker in
 * bake/ (Rust, in the WebAssembly module of wasm.ts). Used by the
 * renderer directly (without workers) and inside bake workers, through the
 * message protocol below. No DOM.
 */
import type { SliceId } from './slice-selection'
import { FACES } from './view'
import { LayerIndex, writeGrid, type LayerIndexData, type Material, type VoxelData } from './voxel-grid'
import { copyIn, copyOut, wasmInstance } from './wasm'
import { generatePart, type GridPart, type WorldType } from './world'

/**
 * A chunk to bake: cell (i, j) (see chunkCell) of a slice (see SliceId) of
 * a face (by name, see FACES), from its level's grid of
 * World.version `grid`. The baker bakes from the grid it has;
 * `grid` only tells bakes apart.
 */
export interface ChunkRequest extends SliceId {
  face: string
  i: number
  j: number
  grid: number
}

/** Numbers per splat in ChunkSplats.splats. */
export const SPLAT = 5

/**
 * A baked chunk: its visible voxels, each a hexagon of one colour (a
 * splat) at its site, in relief over its slice's centre plane (see
 * chunk_splats and Splat in bake/src/splats.rs), those that may be drawn
 * in its cell.
 */
export interface ChunkSplats {
  /**
   * SPLAT numbers each, far to near: x, y of its centre relative to
   * (originX, originY), its width and height (the points (x + u, y + v)
   * with |u| ≤ width and |u| / √2 + |v| ≤ height), in world units, then its
   * colour as RGBA bytes (little-endian, as in ImageData) as the float's
   * bits: opaque, its alpha instead how many half layers of its grid past
   * the centre plane it is, plus 128.
   */
  splats: Float32Array
  /** x, y of the top-left corner of where it may draw, in face coordinates, and its size in world units. */
  originX: number
  originY: number
  extentWidth: number
  extentHeight: number
  mask: Mask
}

/**
 * What a chunk draws, width × height pixels from its top-left corner at
 * `scale` pixels per world unit: a byte per block of 4 × 4 pixels, in
 * rows, whether all its pixels are drawn whatever the camera, and whether
 * some may be (solid_blocks in bake/src/fill.rs), for finding what the
 * chunk hides and whether it is hidden (occlusion.ts).
 */
interface Mask {
  blocks: Uint8Array
  width: number
  height: number
  scale: number
}

/** The baker's exports (bake/src/lib.rs). */
interface Baker {
  memory: WebAssembly.Memory
  world_start: (materials: number) => number
  world_grid: (level: number, minX: number, minY: number, minZ: number, nx: number, ny: number, nz: number, used: number) => number
  world_cells: () => number
  world_faces: (sites: number) => number
  world_ready: () => void
  bake: Call
  sites: () => number
}
type Call = (face: number, level: number, layer: number, cutoff: number, i: number, j: number) => number

const faceIndex = new Map(FACES.map((face, index) => [face.name, index]))

/**
 * Bakes chunks of one world: a WebAssembly instance holding a copy of its
 * grids, the palette of each face (it shades the materials' colours per
 * face itself, bake/src/tables.rs). `levels` are the world's grids by level of detail so far
 * (World.level), all of `materials`; the rest come with addGrid.
 */
export class ChunkBaker {
  private baker: Baker
  /** How many splats the chunk baked last has. */
  private count = 0

  constructor(materials: readonly Material[], levels: readonly (VoxelData | undefined)[]) {
    const baker = wasmInstance() as unknown as Baker
    this.baker = baker
    copyIn(baker.memory, baker.world_start(materials.length), Uint32Array.from(materials, ({ color }) => parseInt(color.slice(1), 16) || 0))
    baker.world_ready()
    levels.forEach((grid, level) => { if (grid) this.addGrid(level, grid) })
  }

  /** Sets the grid of `level`; until then, its chunks are empty. */
  addGrid(level: number, grid: VoxelData) {
    writeGrid(this.baker.memory, grid, (...box) => this.baker.world_grid(level, ...box), this.baker.world_cells, this.baker.world_faces)
  }

  /**
   * Bakes one chunk: the voxels of its slice the face's rays may reach,
   * as splats at their sites (docs/architecture.md, section 3), those that
   * may be drawn in its cell.
   * A chunk with no splats draws nothing.
   */
  bake(request: Omit<ChunkRequest, 'grid'>): ChunkSplats {
    const { memory } = this.baker
    const [width, height, originX, originY, extentWidth, extentHeight, scale, count, splats, blocks, blockCount] = this.call(request)
    this.count = count
    return {
      // Copied out: the baker reuses its buffers.
      splats: copyOut(memory, Float32Array, splats, count * SPLAT),
      originX,
      originY,
      extentWidth,
      extentHeight,
      mask: { blocks: copyOut(memory, Uint8Array, blocks, blockCount), width, height, scale },
    }
  }

  /** The sites (a, b, c in the face's lattice coordinates) of the splats of the chunk baked last, in their order. @internal */
  sites(): Int32Array {
    return copyOut(this.baker.memory, Int32Array, this.baker.sites(), this.count * 3)
  }

  /** Calls `bake` and returns its result's numbers. */
  private call({ face, level, layer, cutoff, i, j }: Omit<ChunkRequest, 'grid'>): Float64Array {
    const index = faceIndex.get(face)
    if (index === undefined) throw new Error(`no face ${face}`)
    const at = this.baker.bake(index, level, layer, cutoff, i, j)
    if (at === 0) throw new Error('the baker has no world')
    return copyOut(this.baker.memory, Float64Array, at, 11)
  }
}

/** Numbers per request in a `want` message (see encodeRequests). */
const REQUEST = 7

/** Requests packed into numbers, REQUEST each, to post to a worker cheaply. */
export function encodeRequests(requests: readonly ChunkRequest[]): Float64Array {
  const packed = new Float64Array(requests.length * REQUEST)
  requests.forEach(({ face, level, layer, cutoff, i, j, grid }, n) => {
    const index = faceIndex.get(face)
    if (index === undefined) throw new Error(`no face ${face}`)
    packed.set([index, level, layer, cutoff, i, j, grid], n * REQUEST)
  })
  return packed
}

/** The n-th request of encodeRequests' numbers. */
function decodeRequest(packed: Float64Array, n: number): ChunkRequest {
  const [face, level, layer, cutoff, i, j, grid] = packed.subarray(n * REQUEST, (n + 1) * REQUEST)
  return { face: FACES[face].name, level, layer, cutoff, i, j, grid }
}

/** A key per chunk request. */
const requestKey = ({ face, level, layer, cutoff, i, j, grid }: ChunkRequest) => [face, level, layer, cutoff, i, j, grid].join()

/**
 * Messages to a bake worker: a new world (numbered by `generation`, which
 * its results echo) with its grids so far, a grid of it (new, or
 * replacing the level's), parts of its grids to generate
 * (World.missingParts), or the chunks to bake, most
 * wanted first, replacing the previous list. `received` is the `seq` of
 * the worker's last result the main thread had taken when it listed them.
 */
export type BakeMessage =
  | { type: 'world'; generation: number; materials: readonly Material[]; levels: readonly (VoxelData | undefined)[] }
  | { type: 'grid'; generation: number; level: number; grid: VoxelData }
  | { type: 'generate'; generation: number; seed: number; worldType: WorldType; parts: GridPart[] }
  | { type: 'want'; requests: Float64Array; received: number }

/**
 * A bake worker's message: chunks baked, `seq` numbering the worker's
 * messages of chunks; or a generated part of a grid.
 */
export type BakeResult =
  | { type: 'chunks'; generation: number; seq: number; baked: { request: ChunkRequest; chunk: ChunkSplats }[] }
  | { type: 'part'; generation: number; part: GridPart; grid: VoxelData; index: LayerIndexData }

/**
 * A bake worker's message handler: bakes the latest list in `schedule`d
 * steps, so a new list (handled between steps) replaces the rest of the
 * old one. A step bakes a chunk, and more until `budget` ms have passed,
 * and posts them at once, handing over their buffers.
 * Chunks posted after the list's `received` are on their way, and skipped.
 * Parts to generate take steps of their own, as much time in all as
 * baking while there are chunks to bake, finest level first: the near
 * grids, moving with the camera, go before the far ones.
 */
export function bakeHandler(
  post: (result: BakeResult, transfer: Transferable[]) => void,
  schedule: (step: () => void) => void,
  budget: number,
): (message: BakeMessage) => void {
  let baker: ChunkBaker | null = null
  let generation = 0
  let queue: Float64Array = new Float64Array(0)
  let next = 0
  let seq = 0
  // Chunks posted, by key: the seq they were posted with.
  const posted = new Map<string, number>()
  // Parts to generate, and of what world.
  let jobs: { seed: number; worldType: WorldType; parts: GridPart[] } | null = null
  // Milliseconds spent baking, less those spent generating.
  let ahead = 0
  let scheduled = false
  const remaining = () => next < queue.length / REQUEST
  const bake = () => {
    const start = performance.now()
    const baked: { request: ChunkRequest; chunk: ChunkSplats }[] = []
    while (baker && remaining() && (baked.length === 0 || performance.now() - start < budget)) {
      const request = decodeRequest(queue, next++)
      const key = requestKey(request)
      if (posted.has(key)) continue
      baked.push({ request, chunk: baker.bake(request) })
      posted.set(key, seq + 1)
    }
    if (baked.length > 0) post({ type: 'chunks', generation, seq: ++seq, baked }, baked.flatMap(({ chunk }) => [chunk.splats.buffer, chunk.mask.blocks.buffer]))
  }
  const generate = () => {
    if (!jobs) return
    const part = jobs.parts.shift() as GridPart
    const grid = generatePart(jobs.seed, jobs.worldType, part)
    if (jobs.parts.length === 0) jobs = null
    // Where its voxels are, so the main thread need not find out.
    const { keys, tables } = LayerIndex.of(grid)
    // Just the fields: a class's methods would not survive posting.
    const { minX, minY, minZ, nx, ny, nz, bricks, cells, faces, materials } = grid
    post({ type: 'part', generation, part, grid: { minX, minY, minZ, nx, ny, nz, bricks, cells, faces, materials }, index: { keys, tables } }, [bricks.buffer, cells.buffer, faces.buffer, ...tables.flatMap(t => [t.low.buffer, t.high.buffer])])
  }
  const step = () => {
    scheduled = false
    const start = performance.now()
    const baking = !!baker && remaining() && (!jobs || ahead <= 0)
    if (baking) bake()
    else generate()
    ahead += (baking ? 1 : -1) * (performance.now() - start)
    wake()
  }
  const wake = () => {
    if (scheduled || ((!baker || !remaining()) && !jobs)) return
    scheduled = true
    schedule(step)
  }
  return message => {
    if (message.type === 'world') {
      baker = new ChunkBaker(message.materials, message.levels)
      generation = message.generation
      queue = new Float64Array(0)
      posted.clear()
      jobs = null
      ahead = 0
    } else if (message.type === 'grid') {
      if (message.generation === generation) baker?.addGrid(message.level, message.grid)
    } else if (message.type === 'generate') {
      if (message.generation !== generation) return
      jobs = { seed: message.seed, worldType: message.worldType, parts: [...(jobs?.parts ?? []), ...message.parts].sort((a, b) => a.level - b.level) }
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
