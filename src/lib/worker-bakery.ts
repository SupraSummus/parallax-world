import { encodeRequests, type BakeMessage, type BakeResult, type ChunkImage, type ChunkRequest } from './chunk-baker'
import type { Bakery } from './renderer'
import type { VoxelData } from './voxel-grid'

/** The part of a Worker a WorkerBakery uses. */
export interface BakeWorker {
  postMessage(message: BakeMessage): void
  onmessage: ((event: MessageEvent<BakeResult>) => void) | null
  terminate(): void
}

interface Entry {
  worker: BakeWorker
  // The seq of its latest message taken.
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
 * thread (see bakeHandler).
 */
export class WorkerBakery implements Bakery {
  private workers: Entry[]
  private generation = 0
  private deliver: (request: ChunkRequest, image: ChunkImage) => void = () => undefined

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

  setWorld(levels: VoxelData[]) {
    this.generation++
    for (const entry of this.workers) {
      entry.sent = null
      entry.worker.postMessage({ type: 'world', generation: this.generation, levels })
    }
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

  private receive(entry: Entry, { generation, seq, baked }: BakeResult) {
    entry.received = seq
    // A worker answers in order, so results of an older world arrive before
    // the new world's; they are dropped.
    if (generation === this.generation) for (const { request, image } of baked) this.deliver(request, image)
  }
}

/** A WorkerBakery with `count` workers. */
export function startBakeWorkers(count: number): WorkerBakery {
  return new WorkerBakery(Array.from({ length: count }, () =>
    new Worker(new URL('./bake.worker.ts', import.meta.url), { type: 'module' })))
}
