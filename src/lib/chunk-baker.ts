/**
 * Baking whole chunks: from a chunk request to its splats, by the baker in
 * bake/ (Rust, in the WebAssembly module of wasm.ts). Used by the
 * renderer directly (without workers) and inside bake workers, through the
 * message protocol below. No DOM.
 */
import type { SliceId } from './slice-selection'
import { FACES } from './view'
import { BRICK_SITES, LayerIndex, usedCells, type LayerIndexData, type Material, type VoxelData } from './voxel-grid'
import { wasmInstance } from './wasm'
import { generatePart, type GridPart, type WorldType } from './world'

/**
 * A chunk to bake: cell (i, j) (see chunkCell) of a slice (see SliceId) of
 * a face (by name, see FACES), seen from `eye` (a site in the face's
 * lattice coordinates, see eyeOf), with faces grown to fill gaps if `fill`
 * (see docs/architecture.md, section 3), from its level's grid of
 * World.version `grid`. The baker bakes from the grid it has;
 * `grid` only tells bakes apart.
 */
export interface ChunkRequest extends SliceId {
  face: string
  i: number
  j: number
  eye: readonly [number, number, number]
  fill: boolean
  grid: number
}

/** Numbers per splat in ChunkSplats.splats. */
export const SPLAT = 5

/**
 * A baked chunk: its visible voxels, each a hexagon of one colour (a
 * splat) on its slice's centre plane (see chunk_splats and Splat in
 * bake/src/splats.rs), clipped to its cell.
 */
export interface ChunkSplats {
  /**
   * SPLAT numbers each: x, y of its centre relative to (originX, originY),
   * its width and height (the points (x + u, y + v) with |u| ≤ width and
   * |u| / √2 + |v| ≤ height), in world units, then its colour, opaque RGBA bytes
   * (little-endian, as in ImageData) as the float's bits. First the
   * `painted` ones, far to near, then the grown ones, far to near, drawn
   * only where the chunk paints nothing.
   */
  splats: Float32Array
  painted: number
  /** x, y of the top-left corner of what is drawn, in face coordinates, and its size in world units. */
  originX: number
  originY: number
  extentWidth: number
  extentHeight: number
  mask: Mask
}

/**
 * What a chunk draws, width × height pixels from its top-left corner at
 * `scale` pixels per world unit: a byte per block of 4 × 4 pixels, in
 * rows, whether all its pixels are drawn, and whether some are
 * (solid_blocks in bake/src/fill.rs), for finding what the chunk hides
 * and whether it is hidden (occlusion.ts).
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
  world_start: (materials: number) => void
  world_grid: (level: number, minX: number, minY: number, minZ: number, nx: number, ny: number, nz: number, used: number) => number
  world_cells: () => number
  world_table: (table: number) => number
  world_ready: () => void
  bake: Call
  sites: () => number
}
type Call = (face: number, level: number, layer: number, cutoff: number, i: number, j: number, eyeA: number, eyeB: number, eyeC: number, fill: number) => number

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
    const baker = wasmInstance() as unknown as Baker
    this.baker = baker
    // It shades the materials' colours per face itself (bake/src/tables.rs).
    baker.world_start(materials.length)
    this.write(Int32Array, baker.world_table(0), Int32Array.from(FACES.flatMap(({ lattice: { right, up, forward } }) => [right, up, forward].flatMap(v => [v.x, v.y, v.z]))))
    this.write(Uint32Array, baker.world_table(1), Uint32Array.from(materials, ({ color }) => parseInt(color.slice(1), 16) || 0))
    baker.world_ready()
    levels.forEach((grid, level) => { if (grid) this.addGrid(level, grid) })
  }

  /** Sets the grid of `level`; until then, its chunks are empty. */
  addGrid(level: number, grid: VoxelData) {
    const { minX, minY, minZ, nx, ny, nz, bricks } = grid
    const cells = usedCells(grid)
    this.write(Int32Array, this.baker.world_grid(level, minX, minY, minZ, nx, ny, nz, cells.length / BRICK_SITES), bricks)
    this.write(Uint8Array, this.baker.world_cells(), cells)
  }

  /** Copies `values` into the baker's memory at `offset`. Memory may grow on each call, detaching earlier views of it. */
  private write<T extends Uint8Array | Int32Array | Uint32Array>(View: new (buffer: ArrayBuffer, offset: number, length: number) => T, offset: number, values: T) {
    new View(this.baker.memory.buffer, offset, values.length).set(values)
  }

  /**
   * Bakes one chunk: the voxels of its slice a ray from its eye reaches,
   * as splats projected onto its centre plane
   * (docs/architecture.md, section 3), those that reach into its cell.
   * A chunk with no splats draws nothing.
   */
  bake(request: Omit<ChunkRequest, 'grid'>): ChunkSplats {
    const { memory } = this.baker
    const [width, height, originX, originY, extentWidth, extentHeight, scale, painted, grown, splats, blocks, blockCount] = this.call(request)
    return {
      // Copied out: the baker reuses its buffers.
      splats: new Float32Array(memory.buffer, splats, (painted + grown) * SPLAT).slice(),
      painted,
      originX,
      originY,
      extentWidth,
      extentHeight,
      mask: { blocks: new Uint8Array(memory.buffer, blocks, blockCount).slice(), width, height, scale },
    }
  }

  /** The sites (a, b, c in the face's lattice coordinates) of the splats of the chunk baked last, in their order. @internal */
  sites(count: number): Int32Array {
    return new Int32Array(this.baker.memory.buffer, this.baker.sites(), count * 3).slice()
  }

  /** Calls `bake` and returns its result's numbers. */
  private call({ face, level, layer, cutoff, i, j, eye, fill }: Omit<ChunkRequest, 'grid'>): Float64Array {
    const index = faceIndex.get(face)
    if (index === undefined) throw new Error(`no face ${face}`)
    // Before reading memory.buffer: the call may grow (and so replace) it.
    const at = this.baker.bake(index, level, layer, cutoff, i, j, ...eye, fill ? 1 : 0)
    if (at === 0) throw new Error('the baker has no world')
    return new Float64Array(this.baker.memory.buffer, at, 12)
  }
}

/** Numbers per request in a `want` message (see encodeRequests). */
const REQUEST = 11

/** Requests packed into numbers, REQUEST each, to post to a worker cheaply. */
export function encodeRequests(requests: readonly ChunkRequest[]): Float64Array {
  const packed = new Float64Array(requests.length * REQUEST)
  requests.forEach(({ face, level, layer, cutoff, i, j, eye, fill, grid }, n) => {
    const index = faceIndex.get(face)
    if (index === undefined) throw new Error(`no face ${face}`)
    packed.set([index, level, layer, cutoff, i, j, ...eye, fill ? 1 : 0, grid], n * REQUEST)
  })
  return packed
}

/** The n-th request of encodeRequests' numbers. */
function decodeRequest(packed: Float64Array, n: number): ChunkRequest {
  const [face, level, layer, cutoff, i, j, a, b, c, fill, grid] = packed.subarray(n * REQUEST, (n + 1) * REQUEST)
  return { face: FACES[face].name, level, layer, cutoff, i, j, eye: [a, b, c], fill: fill === 1, grid }
}

/** A key per chunk request. */
const requestKey = ({ face, level, layer, cutoff, i, j, eye, fill, grid }: ChunkRequest) => [face, level, layer, cutoff, i, j, ...eye, fill, grid].join()

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
    const { keys, tables } = new LayerIndex(grid, FACES.map(f => f.lattice))
    // Just the fields: a class's methods would not survive posting.
    const { minX, minY, minZ, nx, ny, nz, bricks, cells, materials } = grid
    post({ type: 'part', generation, part, grid: { minX, minY, minZ, nx, ny, nz, bricks, cells, materials }, index: { keys, tables } }, [bricks.buffer, cells.buffer, tables[0].low.buffer, tables[0].high.buffer])
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
