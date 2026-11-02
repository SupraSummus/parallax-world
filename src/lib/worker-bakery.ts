import { encodeRequests, type BakeMessage, type BakeResult, type ChunkImage, type ChunkRequest } from './chunk-baker'
import type { Bakery } from './renderer'
import type { LayerIndexData, Material, VoxelData } from './voxel-grid'
import type { GridPart, WorldType } from './world'

/** The part of a Worker a WorkerBakery uses. */
export interface BakeWorker {
  postMessage(message: BakeMessage): void
  onmessage: ((event: MessageEvent<BakeResult>) => void) | null
  terminate(): void
}

interface Entry {
  worker: BakeWorker
  // The seq of its latest result taken.
  received: number
  // What the latest `want` message sent it.
  sent: { requests: Float64Array; received: number } | null
}

/** Which of `count` workers bakes a chunk: always the same for a cell, so that its surface is cached once. */
function workerOf({ face, level, layer, cutoff, i, j }: ChunkRequest, count: number): number {
  let hash = 0x811c9dc5
  for (let k = 0; k < face.length; k++) hash = Math.imul(hash ^ face.charCodeAt(k), 0x01000193)
  // An infinite cutoff hashes as 0, like none.
  for (const n of [level, layer, cutoff, i, j]) hash = Math.imul(hash ^ n, 0x01000193)
  return (hash >>> 0) % count
}

const equal = (a: Float64Array, b: Float64Array) => a.length === b.length && a.every((x, k) => x === b[k])

/**
 * Bakes chunks in a pool of workers (bake.worker.ts), each with its own copy
 * of the world. Each want() splits the list by cell between the workers,
 * keeping its order, and sends each worker its share, which replaces the
 * previous one; the workers bake their lists without waiting for the main
 * thread (see bakeHandler). They generate the world's grids too, a part
 * each in turn.
 */
export class WorkerBakery implements Bakery {
  private workers: Entry[]
  private generation = 0
  private deliver: (request: ChunkRequest, image: ChunkImage) => void = () => undefined
  private generated: (part: GridPart, grid: VoxelData, index: LayerIndexData) => void = () => undefined

  constructor(workers: BakeWorker[]) {
    this.workers = workers.map(worker => {
      const entry: Entry = { worker, received: 0, sent: null }
      worker.onmessage = event => { this.receive(entry, event.data) }
      return entry
    })
  }

  connect(deliver: (request: ChunkRequest, image: ChunkImage) => void) {
    this.deliver = deliver
  }

  setWorld(materials: readonly Material[], levels: readonly (VoxelData | undefined)[]) {
    this.generation++
    this.generated = () => undefined
    for (const entry of this.workers) {
      entry.sent = null
      entry.worker.postMessage({ type: 'world', generation: this.generation, materials, levels })
    }
  }

  addGrid(level: number, grid: VoxelData) {
    for (const { worker } of this.workers) worker.postMessage({ type: 'grid', generation: this.generation, level, grid })
  }

  generate(seed: number, worldType: WorldType, parts: GridPart[], done: (part: GridPart, grid: VoxelData, index: LayerIndexData) => void) {
    this.generated = done
    const count = this.workers.length
    this.workers.forEach(({ worker }, k) => {
      const share = parts.filter((_, n) => n % count === k)
      if (share.length > 0) worker.postMessage({ type: 'generate', generation: this.generation, seed, worldType, parts: share })
    })
  }

  want(requests: ChunkRequest[]) {
    const count = this.workers.length
    const shares = this.workers.map((): ChunkRequest[] => [])
    for (const request of requests) shares[workerOf(request, count)].push(request)
    this.workers.forEach((entry, k) => {
      const packed = encodeRequests(shares[k])
      const { sent, received } = entry
      if (sent && sent.received === received && equal(sent.requests, packed)) return
      entry.sent = { requests: packed, received }
      entry.worker.postMessage({ type: 'want', requests: packed, received })
    })
  }

  /** Stops the workers; chunks they baked and did not deliver yet never are. */
  dispose() {
    for (const { worker } of this.workers) worker.terminate()
    this.workers = []
  }

  private receive(entry: Entry, result: BakeResult) {
    // A worker answers in order, so results of an older world arrive before
    // the new world's; they are dropped.
    const current = result.generation === this.generation
    if (result.type === 'part') {
      if (current) this.generated(result.part, result.grid, result.index)
      return
    }
    entry.received = result.seq
    if (current) this.deliver(result.request, result.image)
  }
}

/** A WorkerBakery with `count` workers. */
export function startBakeWorkers(count: number): WorkerBakery {
  return new WorkerBakery(Array.from({ length: count }, () =>
    new Worker(new URL('./bake.worker.ts', import.meta.url), { type: 'module' })))
}
