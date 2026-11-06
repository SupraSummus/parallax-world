/**
 * Which parts of the screen nearer layers already cover, so chunks behind
 * them need not be drawn. See docs/architecture.md, section 4.
 */
import type { Chunk, Placement } from './renderer'
import { FACES, faceBasis, type Face, type ScreenBox, type View } from './view'
import { CLUSTER } from './chunk-baker'
import { chunkCell } from './slice-selection'
import { copyIn, copyOut, wasmInstance } from './wasm'

/** The coverage exports (bake/src/coverage.rs). */
interface Exports {
  memory: WebAssembly.Memory
  coverage_view: (width: number, height: number, focal: number) => void
  coverage_face: (face: number, rx: number, ry: number, rz: number, ux: number, uy: number, uz: number, fx: number, fy: number, fz: number) => void
  coverage_hides: (minX: number, maxX: number, minY: number, maxY: number) => number
  coverage_use: (handle: number) => number
  coverage_blocks: (count: number) => number
  coverage_clusters: (count: number) => number
  coverage_add: (handle: number, width: number, height: number, scale: number) => void
  coverage_hides_chunk: (handle: number, x: number, y: number, distance: number) => number
  coverage_show_clusters: (handle: number, x: number, y: number, distance: number) => number
  coverage_shown: () => number
  coverage_cover: (handle: number, x: number, y: number, cellLeft: number, cellBottom: number, cellWidth: number, cellHeight: number, distance: number) => void
  coverage_end_layer: (distance: number) => void
}

/**
 * The screen in cells, each covered or not by the layers drawn so far, for
 * the face being drawn: Coverage in bake/src/coverage.rs, in its own
 * WebAssembly instance. Faces come one by one, each with its layers near
 * to far.
 */
export class Coverage {
  private wasm = wasmInstance() as unknown as Exports
  private view!: View
  // Chunks are handed over once, by a number; coverage.rs forgets those unused for a while, and they are handed over again.
  private handles = new WeakMap<Chunk, number>()
  private count = 0

  constructor(view: View) {
    this.setView(view)
  }

  /** Starts a frame seen through `view`. */
  setView(view: View) {
    this.view = view
    this.wasm.coverage_view(view.width, view.height, view.focal)
  }

  /** Starts a face; its layers follow, near to far. */
  setFace(face: Face) {
    const { right: r, up: u, forward: f } = faceBasis(face, this.view.orientation)
    this.wasm.coverage_face(FACES.indexOf(face), r.x, r.y, r.z, u.x, u.y, u.z, f.x, f.y, f.z)
  }

  /** Whether a screen box, with a margin around it, lies on covered parts of the screen. */
  hides(box: ScreenBox): boolean {
    return this.wasm.coverage_hides(box.minX, box.maxX, box.minY, box.maxY) !== 0
  }

  /**
   * Whether a chunk placed at (x, y) on the plane z = distance (see
   * Placement) is hidden: each part of it with a pixel drawn is hidden
   * (see hides) or off screen.
   */
  hidesChunk(chunk: Chunk, x: number, y: number, distance: number): boolean {
    return this.wasm.coverage_hides_chunk(this.handle(chunk), x, y, distance) !== 0
  }

  /**
   * Which of a chunk's clusters (see ChunkSplats.clusters), placed as in
   * hidesChunk, may show: none (null, hidden whole), all (undefined),
   * or which, a byte per cluster, 0 if hidden (see hides) or off screen.
   */
  showClusters(chunk: Chunk, x: number, y: number, distance: number): Uint8Array | null | undefined {
    const { wasm } = this
    const count = wasm.coverage_show_clusters(this.handle(chunk), x, y, distance)
    if (count === 0) return null
    const clusters = chunk.clusters.length / CLUSTER
    return count === clusters ? undefined : copyOut(wasm.memory, Uint8Array, wasm.coverage_shown() >>> 0, clusters)
  }

  /**
   * Adds what a layer of the current face draws opaque: its placements
   * (chunks of a slice of `level`, see chunkCell) on the plane
   * z = distance. `camera` is the camera's position in face coordinates.
   */
  cover(placements: Placement[], distance: number, level: number, camera: { x: number; y: number }) {
    for (const { chunk, x, y } of placements) {
      const cell = chunkCell(level, chunk.i, chunk.j)
      this.wasm.coverage_cover(this.handle(chunk), x, y, cell.x - camera.x, cell.y - camera.y, cell.width, cell.height, distance)
    }
    this.wasm.coverage_end_layer(distance)
  }

  /** The chunk's number, handed over unless coverage.rs has it. */
  private handle(chunk: Chunk): number {
    const { wasm } = this
    let handle = this.handles.get(chunk)
    if (handle === undefined) this.handles.set(chunk, handle = ++this.count)
    if (wasm.coverage_use(handle)) return handle
    const { width, height, scale, blocks } = chunk.mask
    copyIn(wasm.memory, wasm.coverage_blocks(blocks.length) >>> 0, blocks)
    copyIn(wasm.memory, wasm.coverage_clusters(chunk.clusters.length) >>> 0, chunk.clusters)
    wasm.coverage_add(handle, width, height, scale)
    return handle
  }
}
