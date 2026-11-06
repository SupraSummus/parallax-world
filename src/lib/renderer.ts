import { selectSlices, leakFreeDetail, sliceCentre, chunkCell, chunkCells, cellMargin, matchTilings, sliceKey as sliceName, siteLayers, GRID_LEVELS, atomSpacing, type SliceId } from './slice-selection'
import { ChunkBaker, SPLAT, type ChunkSplats, type ChunkRequest } from './chunk-baker'
import { generatePart, World, worldTypes, type GridPart, type Vec3, type WorldType } from './world'
import { LayerIndex, type LayerIndexData, type Material, type VoxelData } from './voxel-grid'
import { Coverage } from './occlusion'
import { FACES, LAYER, faceTangents, inFace, focalLength, onScreen, screenBox, toLocal, type Face, type Orientation, type ScreenBox, type TangentBox, type View } from './view'

export type Camera = Vec3

/** A baked, aligned piece of a slice (see chunkCell). */
export interface Chunk extends ChunkSplats {
  /** Its cell (see chunkCell). */
  i: number
  j: number
  /** Which of its level's grids it is baked from (see ChunkRequest). */
  grid: number
}

/**
 * A chunk in a frame: x, y of its top-left corner on its layer's plane,
 * relative to the camera, in the face's coordinates. It spans
 * chunk.extentWidth to the right and chunk.extentHeight down.
 */
export interface Placement {
  chunk: Chunk
  x: number
  y: number
  /** Which of its clusters (see ChunkSplats.clusters) may show, a byte each, 0 if not; all if absent. */
  show?: Uint8Array
}

/**
 * What to draw, seen through `View` (see projector() in view.ts). Layers
 * go face by face, near to far; a face draws only its own rays (inFace in
 * view.ts), and nearer layers hide what is behind them. A layer is a
 * slice's centre plane, `distance` ahead of the camera along its face's
 * axis; the slice's sites are in the face's layers [depth, depth + size)
 * (see Face in view.ts, siteLayers). `fog` is the share of the horizon colour mixed into a
 * layer's pixels, and into the sky where no layer is drawn.
 */
export interface Frame extends View {
  fog: number
  layers: { face: Face; slice: SliceId; depth: number; size: number; distance: number; placements: Placement[]; fog: number }[]
}

interface UpdateStats {
  renderTime: number
  /** Distance / max slice thickness, from the leak tolerance (see leakFreeDetail). */
  detail: number
  layerCount: number
  /** Non-empty chunks in view. */
  chunkCount: number
  chunksReused: number
  /** Chunks in view shown for the first time since they were baked. */
  chunksRegenerated: number
  /** Chunks in view still baking (with a Bakery); an older bake of the same cell, if any, stands in. */
  chunksPending: number
  /** Levels whose grids are being generated (World.generating); their chunks are wanted once there. */
  levelsGenerating: number
  /** Cells and chunks in view covered by nearer layers, so not drawn (see Coverage). */
  chunksCovered: number
  /** Voxels painted in chunks baked since the previous render (including any whose content turned out to be off screen). */
  voxelsRendered: number
  /** Percentage of chunks in view served from the cache. */
  cacheEfficiency: number
  /** Bytes of all cached chunks, in view or not. */
  cachedBytes: number
}

/**
 * A view in a URL's query, for the app and the 3D viewer:
 * ?seed=&world=&camera=x,y,z&look=yaw,pitch&fov=&distance=&leak=
 * (the max occlusion leak, in degrees).
 */
export interface ViewQuery {
  seed?: number
  worldType?: WorldType
  camera?: Camera
  look?: Orientation
  fov?: number
  distance?: number
  leak?: number
}

export function viewQuery({ seed, worldType, camera, look, fov, distance, leak }: Required<ViewQuery>): string {
  const { x, y, z } = camera
  return new URLSearchParams({
    seed: String(seed), world: worldType, camera: [x, y, z].join(), look: [look.yaw, look.pitch].join(), fov: String(fov), distance: String(distance), leak: String(leak),
  }).toString()
}

/** The view in `search` (viewQuery's); what it leaves out or gets wrong is undefined. */
export function parseViewQuery(search: string): ViewQuery {
  const params = new URLSearchParams(search)
  const numbers = (name: string, count: number) => {
    const values = (params.get(name) ?? '').split(',').map(v => (v.trim() === '' ? NaN : Number(v)))
    return values.length === count && values.every(Number.isFinite) ? values : undefined
  }
  const [camera, look, world] = [numbers('camera', 3), numbers('look', 2), params.get('world')]
  return {
    seed: numbers('seed', 1)?.[0],
    worldType: worldTypes().find(t => t.value === world)?.value,
    camera: camera && { x: camera[0], y: camera[1], z: camera[2] },
    look: look && { yaw: look[0], pitch: look[1] },
    fov: numbers('fov', 1)?.[0],
    distance: numbers('distance', 1)?.[0],
    leak: numbers('leak', 1)?.[0],
  }
}

export interface SessionStats {
  totalUpdates: number
  totalRenderTime: number
  totalChunksRegenerated: number
  totalVoxelsRendered: number
  averageCacheEfficiency: number
  lastUpdate: UpdateStats | null
}

// Field of view across the longer viewport side, in degrees (see focalLength).
export const DEFAULT_FOV = 90
export const MIN_FOV = 5
export const MAX_FOV = 120

// How far geometry that perspective would hide may show through the gaps
// between slices, in degrees: an angle, as slicing knows no field of view.
// Larger means thicker slices, fewer layers.
export const DEFAULT_MAX_LEAK = 6
export const MIN_MAX_LEAK = 1
export const MAX_MAX_LEAK = 12

// Visible depth range, relative to the camera: from NEAR_DEPTH to the render
// distance, in world units (a voxel's neighbours are √2 away, see fcc.ts).
// Four layers: a nearer slice, drawn at focal / d pixels per unit, would
// blow single voxels up over the screen.
const NEAR_DEPTH = 4 * LAYER
export const DEFAULT_RENDER_DISTANCE = 8000
export const MIN_RENDER_DISTANCE = 40
export const MAX_RENDER_DISTANCE = 40000

// Slices and chunks outside the view are kept so that moving or turning
// back reuses them, up to these caps (a face has about 150 slices, and 12 faces
// look all around). What is in view is never evicted.
const MAX_CACHED_SLICES = 2048
const MAX_HIDDEN_CHUNK_BYTES = 32_000_000

// Exponential distance fog towards the horizon colour: at viewing distance d
// a voxel keeps exp(-d / fogDistance) of its own colour. The fog distance
// scales with the render distance so the far cut-off stays hazy.
export const FOG_PER_RENDER_DISTANCE = 0.7

/** What a chunk holds in memory. */
const bytesOf = (chunk: Chunk) => chunk.splats.byteLength + chunk.mask.blocks.byteLength + chunk.cluster.byteLength + chunk.clusters.byteLength

function sliceKey(face: string, slice: SliceId): string {
  return `${face}:${sliceName(slice)}`
}

/** A cached slice and the chunks baked so far. */
interface Slice {
  id: SliceId
  /** Its voxels may be in its face's layers [depth, depth + size) (see siteLayers). */
  depth: number
  size: number
  /** Baked chunks by cell (keyed by cellKey). */
  cells: Map<number, CellChunks>
  /** Bytes of its chunks. */
  bytes: number
  /** By cellKey: the least and greatest b of the voxels its chunks may draw (see cellMargin), or null if none. */
  extents: Map<number, [number, number] | null>
}

/** The chunks baked of a cell, one per grid, and the most recent one, shown while the one wanted is baking. */
interface CellChunks {
  chunks: Chunk[]
  latest: Chunk | undefined
}

/**
 * A chunk to bake, how far ahead, whether an older bake of its cell stands
 * in meanwhile, and, for a slice not drawn until its re-tiled segment is
 * baked, how many chunks the segment misses.
 */
interface Missing {
  request: ChunkRequest
  distance: number
  stale: boolean
  segmentMisses?: number
}

/** One render's settings and what it gathers across slices. */
interface Pass {
  view: View
  coverage: Coverage | null
  missing: Missing[]
  /** Chunks of the cells in view. */
  inView: Set<Chunk>
  /** Chunks in view still baking. */
  pending: number
}

/** A face as seen in one render: its part of the screen (see faceTangents), the camera in its coordinates, screenBox. */
interface FaceView {
  face: Face
  tangents: TangentBox
  camera: Vec3
  boxOf: ReturnType<typeof screenBox>
}

/** What drawSlice found: the layer's distance and placements, cells and chunks covered, and (probing) cells in view never baked. */
interface Drawn {
  distance: number
  placements: Placement[]
  covered: number
  unbaked: number
}

/** |v| for the v in [lo, hi] nearest to 0. */
const closestToZero = (lo: number, hi: number) => (lo > 0 ? lo : hi < 0 ? -hi : 0)

/** A number per cell: cells of a slice are far fewer than 2^20 across. */
function cellKey(i: number, j: number): number {
  return (i + 2 ** 20) * 2 ** 21 + j + 2 ** 20
}

/** The cell's chunk from the `grid`-th grid of its level. */
function findChunk(cell: CellChunks | undefined, grid: number): Chunk | undefined {
  return cell?.chunks.find(chunk => chunk.grid === grid)
}

/**
 * Bakes chunks off the main thread (see WorkerBakery). The renderer lists
 * the chunks it is missing on every render; each baked one comes back
 * through the callback given to connect().
 */
export interface Bakery {
  /** Takes the callbacks for baked chunks, and for generated parts of grids with their LayerIndex. */
  connect(deliver: (request: ChunkRequest, chunk: ChunkSplats) => void, generated: (part: GridPart, grid: VoxelData, index: LayerIndexData) => void): void
  /**
   * Replaces the world: its materials and its grids so far, by level
   * (World.level); chunks of the previous one are no longer delivered.
   */
  setWorld(materials: readonly Material[], levels: readonly (VoxelData | undefined)[]): void
  /** Adds the grid of a level to the world, or replaces it. */
  addGrid(level: number, grid: VoxelData): void
  /** Generates parts of the world's grids (World.missingParts), each passed to the `generated` callback. */
  generate(seed: number, worldType: WorldType, parts: GridPart[]): void
  /** The chunks to bake, most wanted first. Replaces the previous list. */
  want(requests: ChunkRequest[]): void
}

export interface RendererOptions {
  /** Where chunks are baked. Without one, render() bakes every missing chunk itself before returning. */
  bakery?: Bakery
  /** Called whenever the bakery delivers chunks; render() again to show them. */
  onBaked?: () => void
  /** Where the camera starts, if not at the world's spawn point: its grids are generated there first. */
  camera?: Camera
}

/**
 * Renders a World as a stack of flat, cached slices, each split into chunks
 * so that only what is in view is baked. render() returns where each
 * chunk goes on screen; drawing it is the compositor's job. See
 * docs/architecture.md for the slicing and caching scheme.
 */
export class ParallaxRenderer {
  private world: World
  private camera: Camera
  private orientation: Orientation = { yaw: 0, pitch: 0 }
  private fov = DEFAULT_FOV
  private maxLeak = DEFAULT_MAX_LEAK
  private renderDistance = DEFAULT_RENDER_DISTANCE
  private skipCovered = true
  private width = 0
  private height = 0

  // Keyed by sliceKey(face, slice). Map insertion order doubles as
  // LRU order (oldest first).
  private slices = new Map<string, Slice>()
  // Per face, the slices it drew (see matchTilings).
  private tilings = new Map<string, SliceId[]>()
  // Chunks of the cells in view in the most recent render, drawn or not:
  // evicted, one would be baked again on the next render.
  private inView = new Set<Chunk>()
  // Bytes of all cached chunks.
  private bytes = 0
  // Kept across renders for its buffers.
  private coverage: Coverage | null = null


  private bakery: Bakery | null
  private onBaked: () => void
  // Bakes on the main thread when there is no bakery.
  private baker: ChunkBaker | null = null
  // Baked but not shown yet: counted as regenerated when first shown.
  private fresh = new WeakSet<Chunk>()
  // Voxels painted in chunks baked since the last render.
  private voxelsBaked = 0

  private stats: SessionStats = {
    totalUpdates: 0,
    totalRenderTime: 0,
    totalChunksRegenerated: 0,
    totalVoxelsRendered: 0,
    averageCacheEfficiency: 0,
    lastUpdate: null,
  }

  constructor(world: World, { bakery, onBaked, camera }: RendererOptions = {}) {
    this.world = world
    this.camera = { ...(camera ?? world.spawn) }
    this.bakery = bakery ?? null
    this.onBaked = onBaked ?? (() => undefined)
    this.bakery?.connect((request, chunk) => { this.receive(request, chunk) }, (part, grid, index) => { this.receivePart(part, grid, index) })
    this.useWorld()
  }

  render(): Frame {
    const startTime = performance.now()
    this.generateAround()

    const detail = this.getDetail()
    const view = this.view()
    if (this.skipCovered) {
      if (this.coverage) this.coverage.setView(view)
      else this.coverage = new Coverage(view)
    }
    const pass: Pass = {
      view,
      coverage: this.skipCovered ? this.coverage : null,
      missing: [],
      inView: new Set(),
      pending: 0,
    }

    let chunksRegenerated = 0
    let chunksReused = 0
    let chunksCovered = 0
    let farthest = this.renderDistance
    const frame: Frame = { ...view, fog: 0, layers: [] }
    // All slices of the view (also those still baking).
    const keep = new Set<Slice>()
    const tilings = new Map<string, SliceId[]>()

    for (const face of FACES) {
      // The face's part of the screen, on a plane at distance d: tangents · d around the camera.
      const tangents = faceTangents(view, face)
      if (!tangents) continue
      const camera = toLocal(face, this.camera)
      pass.coverage?.setFace(face)
      const of = { face, tangents, camera, boxOf: screenBox(view, face) }
      const drawn: SliceId[] = []
      const commit = (slice: Slice, result: Drawn) => {
        const { id, depth, size } = slice
        for (const { chunk } of result.placements) {
          if (this.fresh.delete(chunk)) chunksRegenerated++
          else chunksReused++
        }
        chunksCovered += result.covered
        farthest = Math.max(farthest, result.distance)
        frame.layers.push({ face, slice: id, depth, size, distance: result.distance, placements: result.placements, fog: this.fog(result.distance) })
        pass.coverage?.cover(result.placements, result.distance, id.level, camera)
        drawn.push(id)
      }

      const wanted = selectSlices(camera.z / LAYER, NEAR_DEPTH / LAYER, this.renderDistance / LAYER, detail)
      for (const segment of matchTilings(wanted, this.tilings.get(face.name) ?? [])) {
        const slices = segment.wanted.map(id => this.getSlice(face, id))
        for (const slice of slices) keep.add(slice)
        // If the slices re-tiled, the ones they replace stand in until they
        // have something in every cell they show (baking on the main thread,
        // they always have).
        const standIns = this.baker ? null : this.standIns(face, segment.previous, slices)
        if (!standIns || slices.every(slice => this.drawSlice(pass, of, slice, 'probe').unbaked === 0)) {
          for (const slice of slices) commit(slice, this.drawSlice(pass, of, slice, 'want'))
          continue
        }
        // Not drawn, but their chunks are baked.
        const from = pass.missing.length
        for (const slice of slices) this.drawSlice(pass, of, slice, 'want')
        for (const m of pass.missing.slice(from)) m.segmentMisses = pass.missing.length - from
        for (const slice of standIns) {
          keep.add(slice)
          commit(slice, this.drawSlice(pass, of, slice, 'standIn'))
        }
      }
      tilings.set(face.name, drawn)
    }
    // The last slice may reach past the render distance; the sky is behind it.
    frame.fog = this.fog(farthest)

    this.inView = pass.inView
    this.tilings = tilings
    this.evict(keep)
    // Cells that show nothing go first; the others' older bakes stand in.
    // Then re-tiled segments, fewest misses first: one shows only once all
    // its chunks are baked, and moving fast, most re-tiled again before
    // that. Then the rest; near to far within each.
    const group = (m: Missing) => (m.segmentMisses !== undefined ? 1 : m.stale ? 2 : 0)
    this.bakery?.want(pass.missing
      .sort((a, b) => group(a) - group(b) || (a.segmentMisses ?? 0) - (b.segmentMisses ?? 0) || a.distance - b.distance)
      .map(m => m.request))
    const voxelsRendered = this.voxelsBaked
    this.voxelsBaked = 0

    const chunkCount = chunksReused + chunksRegenerated
    const renderTime = performance.now() - startTime
    const cacheEfficiency = chunkCount > 0 ? (chunksReused / chunkCount) * 100 : 100
    const s = this.stats
    s.totalUpdates++
    s.totalRenderTime += renderTime
    s.totalChunksRegenerated += chunksRegenerated
    s.totalVoxelsRendered += voxelsRendered
    s.averageCacheEfficiency += (cacheEfficiency - s.averageCacheEfficiency) / s.totalUpdates
    s.lastUpdate = {
      renderTime,
      detail,
      layerCount: frame.layers.length,
      chunkCount,
      chunksReused,
      chunksRegenerated,
      chunksPending: pass.pending,
      levelsGenerating: this.world.generating,
      chunksCovered,
      voxelsRendered,
      cacheEfficiency,
      cachedBytes: this.bytes,
    }
    return frame
  }

  /** The cached slices `previous`, if they differ from `slices` and are all still cached. */
  private standIns(face: Face, previous: SliceId[], slices: Slice[]): Slice[] | null {
    const cached = previous.map(id => this.slices.get(sliceKey(face.name, id)))
    if (cached.length === 0 || cached.some(s => !s)) return null
    if (cached.length === slices.length && cached.every((s, k) => s === slices[k])) return null
    return cached as Slice[]
  }

  /**
   * The b of the voxels the chunks of a slice's cell may draw (see
   * cellMargin), or null if there are none; found on first use. Undefined
   * while the world has no grid of the slice's level of detail yet.
   */
  private cellExtent(face: Face, slice: Slice, i: number, j: number): [number, number] | null | undefined {
    const key = cellKey(i, j)
    let extent = slice.extents.get(key)
    if (extent === undefined) {
      const { id, depth, size } = slice
      const indices = this.world.layerIndex(id.atoms)
      if (!indices) return
      const { x, y, width, height } = chunkCell(id.level, i, j)
      // The cell's sites, in lattice units.
      const [left, right] = [x, x + width].map(v => Math.round(v / LAYER))
      const [bottom, top] = [y, y + height]
      const margin = cellMargin(id)
      // In the units of the grid the slice reads, N times larger.
      const n = atomSpacing(id)
      let [lo, hi] = [Infinity, -Infinity]
      for (const index of indices) {
        const found = index.range(face.lattice, Math.floor(depth / n), Math.ceil((depth + size) / n), Math.floor((left - margin.columns) / n), Math.ceil((right + margin.columns) / n))
        if (found) [lo, hi] = [Math.min(lo, found[0] * n), Math.max(hi, found[1] * n)]
      }
      lo = Math.max(lo, bottom - margin.rows)
      hi = Math.min(hi, top + margin.rows)
      extent = lo <= hi ? [lo, hi] : null
      slice.extents.set(key, extent)
    }
    return extent
  }

  /**
   * The chunks of a slice to draw, near to far within it, given what nearer
   * layers cover. Chunks it is missing are baked (without a bakery) or
   * listed to bake if it is `want`ed; a slice standing in for one that
   * re-tiled shows what it has. A `probe` only counts the cells in view
   * never baked.
   */
  private drawSlice(pass: Pass, { face, tangents, camera, boxOf }: FaceView, slice: Slice, mode: 'want' | 'standIn' | 'probe'): Drawn {
    const { view, coverage, inView } = pass
    const { id } = slice
    const { level, atoms, layer, cutoff } = id
    const grid = this.world.version(id.atoms)
    // Slices start at least NEAR_DEPTH ahead of the camera, so this is > 0.
    const distance = sliceCentre(id) - camera.z
    const result: Drawn = { distance, placements: [], covered: 0, unbaked: 0 }
    const covered = (box: ScreenBox) => {
      const c = coverage?.hides(box) ?? false
      if (c) result.covered++
      return c
    }

    // The bounding box of the face's part of the slice's plane.
    const left = camera.x + tangents.minX * distance
    const right = camera.x + tangents.maxX * distance
    const bottom = camera.y + tangents.minY * distance
    const top = camera.y + tangents.maxY * distance
    const h = chunkCell(level, 0, 0).height
    // How far from its voxel's b a face may be drawn where the face's rays show it.
    const reach = cellMargin(id).rows
    for (const [i, j] of chunkCells(level, left, right, bottom, top)) {
      const cellRect = chunkCell(level, i, j)
      // Nothing to draw: not baked at all. Otherwise, what it draws lies within reach of its voxels.
      const extent = this.cellExtent(face, slice, i, j)
      if (!extent) continue
      const x0 = cellRect.x - camera.x
      const x1 = x0 + cellRect.width
      const y0 = Math.max(cellRect.y, extent[0] - reach) - camera.y
      const y1 = Math.min(cellRect.y + h, extent[1] + reach) - camera.y
      if (y0 >= y1) continue
      // The face draws only its own rays: the cell must reach its rhombus (inFace).
      if (!inFace(closestToZero(x0, x1), closestToZero(y0, y1), distance)) continue
      // The screen shows a quadrilateral of the plane, less than its bounding box.
      const cellBox = boxOf(x0, x1, y0, y1, distance)
      if (!onScreen(view, cellBox)) continue
      const cell = slice.cells.get(cellKey(i, j))
      let chunk = findChunk(cell, grid)
      if (covered(cellBox)) {
        // Not drawn, nor baked, but kept if baked.
        const kept = chunk ?? cell?.latest
        if (kept) inView.add(kept)
        continue
      }
      if (mode === 'probe') {
        if (!chunk && !cell?.latest) result.unbaked++
        continue
      }
      if (!chunk) {
        // An older bake of the cell, if any, stands in.
        const latest = cell?.latest
        if (mode === 'standIn') {
          chunk = latest
        } else if (this.baker) {
          const request = { face: face.name, level, atoms, layer, cutoff, i, j, grid }
          chunk = this.store(request, this.baker.bake(request))
        } else {
          // Not wanted if what stands in is hidden anyway.
          if (latest && coverage && latest.splats.length > 0 && coverage.hidesChunk(latest, latest.originX - camera.x, latest.originY - camera.y, distance)) {
            inView.add(latest)
            result.covered++
            continue
          }
          pass.pending++
          pass.missing.push({ request: { face: face.name, level, atoms, layer, cutoff, i, j, grid }, distance, stale: !!latest })
          chunk = latest
        }
        if (!chunk) continue
      }
      inView.add(chunk)
      if (chunk.splats.length === 0) continue
      // The chunk's content, not just its cell, must reach the screen.
      const contentLeft = chunk.originX - camera.x
      const contentTop = chunk.originY - camera.y
      const box = boxOf(contentLeft, contentLeft + chunk.extentWidth, contentTop - chunk.extentHeight, contentTop, distance)
      if (!onScreen(view, box)) continue
      const show = coverage?.showClusters(chunk, contentLeft, contentTop, distance)
      if (show === null) {
        result.covered++
        continue
      }
      result.placements.push(show ? { chunk, x: contentLeft, y: contentTop, show } : { chunk, x: contentLeft, y: contentTop })
    }
    return result
  }

  getCamera(): Camera {
    return { ...this.camera }
  }

  setCamera(camera: Partial<Camera>) {
    this.camera = { ...this.camera, ...camera }
  }

  getOrientation(): Orientation {
    return { ...this.orientation }
  }

  /** Turns the view; pitch stops at straight up or down, yaw wraps to [-180, 180). */
  setOrientation(orientation: Partial<Orientation>) {
    const { yaw, pitch } = { ...this.orientation, ...orientation }
    this.orientation = { yaw: yaw - 360 * Math.floor((yaw + 180) / 360), pitch: Math.min(90, Math.max(-90, pitch)) }
  }

  getWorld(): World {
    return this.world
  }

  getStats(): SessionStats {
    return { ...this.stats }
  }

  /** Sets the viewport size in CSS pixels. Cached chunks stay valid; call render() to reposition them. */
  resize(width: number, height: number) {
    this.width = width
    this.height = height
  }

  /** Replaces the world and moves the camera to its spawn point. */
  regenerateWorld(seed?: number, worldType?: WorldType) {
    this.world = new World(seed, worldType)
    this.camera = { ...this.world.spawn }
    this.inView.clear()
    this.tilings.clear()
    this.slices.clear()
    this.bytes = 0
    this.voxelsBaked = 0
    this.useWorld()
  }

  /**
   * Hands the world to the bakery, which generates the grids it does not
   * have yet; each shows once there. Without one, all are made here.
   */
  private useWorld() {
    const { world, bakery } = this
    if (!bakery) {
      this.baker = new ChunkBaker(world.materials, Array.from({ length: GRID_LEVELS }, (_, level) => world.levelAround(level, this.camera)))
      return
    }
    bakery.setWorld(world.materials, Array.from({ length: GRID_LEVELS }, (_, level) => world.level(level)))
    this.generateAround()
  }

  /**
   * Has the parts generated that the world's grids need around the camera
   * (World.missingParts): by the bakery, or here.
   */
  private generateAround() {
    const { world, bakery } = this
    const parts = world.missingParts(this.camera)
    if (parts.length === 0) return
    if (bakery) {
      bakery.generate(world.seed, world.worldType, parts)
      return
    }
    for (const part of parts) {
      const grid = world.receive(part, generatePart(world.seed, world.worldType, part))
      if (grid) this.useGrid(part.level, grid)
    }
  }

  /** Takes a part the bakery generated, with its LayerIndex. */
  private receivePart(part: GridPart, data: VoxelData, index: LayerIndexData) {
    const grid = this.world.receive(part, data, new LayerIndex(index))
    if (grid) this.useGrid(part.level, grid)
  }

  /**
   * Draws the world's new grid of `level` from now on: the chunks baked
   * from its old grid only stand in until baked again from this one
   * (World.version).
   */
  private useGrid(level: number, grid: VoxelData) {
    for (const slice of this.slices.values()) if (slice.id.atoms === level) slice.extents.clear()
    this.bakery?.addGrid(level, grid)
    this.baker?.addGrid(level, grid)
    this.onBaked()
  }

  getFov(): number {
    return this.fov
  }

  /**
   * Sets the field of view in degrees (clamped to [MIN_FOV, MAX_FOV]). It
   * only zooms: the slices do not depend on it (see getDetail).
   */
  setFov(fov: number) {
    this.fov = Math.min(MAX_FOV, Math.max(MIN_FOV, fov))
  }

  getRenderDistance(): number {
    return this.renderDistance
  }

  /** Sets how far ahead slices are drawn (clamped to [MIN_RENDER_DISTANCE, MAX_RENDER_DISTANCE]). */
  setRenderDistance(distance: number) {
    this.renderDistance = Math.min(MAX_RENDER_DISTANCE, Math.max(MIN_RENDER_DISTANCE, distance))
  }

  getMaxLeak(): number {
    return this.maxLeak
  }

  /**
   * Sets how far hidden geometry may show between slices, in degrees
   * (clamped to [MIN_MAX_LEAK, MAX_MAX_LEAK]). Slices are keyed by level and
   * layer, so the ones still needed stay cached.
   */
  setMaxLeak(degrees: number) {
    this.maxLeak = Math.min(MAX_MAX_LEAK, Math.max(MIN_MAX_LEAK, degrees))
  }

  /** Whether chunks covered by nearer layers are skipped: not drawn, and baked last (see Coverage). */
  setSkipCovered(skip: boolean) {
    this.skipCovered = skip
  }

  /**
   * Slice detail for the leak tolerance. It depends on nothing else,
   * so turning, zooming and resizing keep the slices.
   */
  getDetail(): number {
    return leakFreeDetail((this.maxLeak * Math.PI) / 180)
  }

  private view(): View {
    return {
      orientation: this.orientation,
      width: this.width,
      height: this.height,
      focal: focalLength(this.fov, this.width, this.height),
    }
  }

  /** Share of the fog colour at viewing distance d. */
  private fog(d: number): number {
    return 1 - Math.exp(-d / (this.renderDistance * FOG_PER_RENDER_DISTANCE))
  }

  /** Returns the cached slice, or a new one with no chunks. */
  private getSlice(face: Face, id: SliceId): Slice {
    const key = sliceKey(face.name, id)
    const cached = this.slices.get(key)
    if (cached) {
      // Re-insert to mark as most recently used.
      this.slices.delete(key)
      this.slices.set(key, cached)
      return cached
    }

    const start = id.layer * 2 ** id.level
    const [first, last] = siteLayers(id)
    const slice: Slice = {
      id,
      depth: start + first,
      size: last - first + 1,
      cells: new Map(),
      bytes: 0,
      extents: new Map(),
    }
    this.slices.set(key, slice)
    return slice
  }

  /** Caches a baked chunk, unless its slice has been evicted meanwhile. */
  private store(request: ChunkRequest, baked: ChunkSplats): Chunk | undefined {
    const { face, level, atoms, layer, cutoff, i, j, grid } = request
    const slice = this.slices.get(sliceKey(face, { level, atoms, layer, cutoff }))
    if (!slice) return
    const key = cellKey(i, j)
    let cell = slice.cells.get(key)
    if (!cell) slice.cells.set(key, cell = { chunks: [], latest: undefined })
    if (findChunk(cell, grid)) return
    const chunk: Chunk = { ...baked, i, j, grid }
    cell.chunks.push(chunk)
    cell.latest = chunk
    slice.bytes += bytesOf(chunk)
    this.bytes += bytesOf(chunk)
    this.fresh.add(chunk)
    this.voxelsBaked += chunk.splats.length / SPLAT
    return chunk
  }

  private receive(request: ChunkRequest, chunk: ChunkSplats) {
    this.store(request, chunk)
    this.onBaked()
  }

  /**
   * Drops least-recently-used slices beyond MAX_CACHED_SLICES, then hidden
   * chunks (least recently used slices first) until they fit
   * MAX_HIDDEN_CHUNK_BYTES. What is in view is never evicted.
   */
  private evict(inView: Set<Slice>) {
    for (const [key, slice] of this.slices) {
      if (this.slices.size <= MAX_CACHED_SLICES) break
      if (inView.has(slice)) continue
      this.slices.delete(key)
      this.bytes -= slice.bytes
    }
    let hidden = this.bytes
    for (const chunk of this.inView) hidden -= bytesOf(chunk)
    for (const slice of this.slices.values()) {
      if (hidden <= MAX_HIDDEN_CHUNK_BYTES) return
      for (const [key, cell] of slice.cells) {
        const kept = cell.chunks.filter(chunk => this.inView.has(chunk))
        for (const chunk of cell.chunks) {
          if (this.inView.has(chunk)) continue
          const bytes = bytesOf(chunk)
          hidden -= bytes
          slice.bytes -= bytes
          this.bytes -= bytes
        }
        if (kept.length === 0) {
          slice.cells.delete(key)
        } else {
          cell.chunks = kept
          if (cell.latest && !this.inView.has(cell.latest)) cell.latest = undefined
        }
      }
    }
  }
}
