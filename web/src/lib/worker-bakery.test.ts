import { describe, it, expect, vi } from 'vitest'
import { bakeHandler, ChunkBaker, encodeRequests, type BakeMessage, type BakeResult, type ChunkRequest } from './chunk-baker'
import { ParallaxRenderer, type Bakery, type Camera, type Frame } from './renderer'
import type { Orientation } from './view'
import { picture } from './picture.test-helper'
import { VoxelGrid } from './voxel-grid'
import { WorkerBakery, type BakeWorker } from './worker-bakery'
import { GRID_LEVELS } from './slice-selection'
import { World } from './world'
import { hills } from './hills.test-helper'

// A small stand-in for the forest, which the workers generate in moments.
vi.mock('./worldgen', async () => (await import('./small-forest.test-helper')).smallWorldgen)

// A small world with trees, as in renderer.test.ts.
const world = hills(12345)
const levels = Array.from({ length: GRID_LEVELS }, (_, level) => world.levelAround(level, world.spawn))

/** bake.worker.ts in microtasks: asynchronous and in order, like a worker, and done before the next timer. */
class FakeWorker implements BakeWorker {
  onmessage: ((event: MessageEvent<BakeResult>) => void) | null = null
  baked: ChunkRequest[] = []
  private handle = bakeHandler(result => {
    if (result.type === 'chunks') this.baked.push(...result.baked.map(({ request }) => request))
    queueMicrotask(() => { this.onmessage?.({ data: result } as MessageEvent<BakeResult>) })
  }, queueMicrotask, 0)

  postMessage(message: BakeMessage) {
    queueMicrotask(() => { this.handle(message) })
  }

  terminate() {}
}

/** A chunk of a far slice, cheap to bake. */
const at = (i: number): ChunkRequest => ({ face: '+x+z', level: 6, atoms: 2, layer: 0, cutoff: Infinity, i, j: 0, grid: 0 })

const tick = () => new Promise(resolve => setTimeout(resolve))

function workerRenderer(workers = [new FakeWorker(), new FakeWorker()]) {
  const renderer = new ParallaxRenderer(world, { bakery: new WorkerBakery(workers) })
  renderer.resize(800, 600)
  return renderer
}

/**
 * A renderer whose chunks bake only when `bake(count)` says, the first
 * `count` of the latest list, as workers slower than renders do.
 * `bake` returns how many it baked, `baked` how many in all.
 */
function slowRenderer() {
  const baker = new ChunkBaker(world.materials, levels)
  let wanted: ChunkRequest[] = []
  let deliver: Parameters<Bakery['connect']>[0] = () => undefined
  const renderer = new ParallaxRenderer(world, { bakery: { connect(d) { deliver = d }, setWorld() {}, addGrid() {}, generate() {}, want(r) { wanted = r } } })
  renderer.resize(800, 600)
  let baked = 0
  const bake = (count = Infinity) => {
    const requests = wanted.slice(0, count)
    for (const request of requests) deliver(request, baker.bake(request))
    baked += requests.length
    return requests.length
  }
  return { renderer, bake, baked: () => baked }
}

/** Remembers the latest list of chunks wanted, and the chunks delivered. */
class RecordingBakery extends WorkerBakery {
  latest: ChunkRequest[] = []
  delivered: ChunkRequest[] = []
  override want(requests: ChunkRequest[]) {
    this.latest = requests
    super.want(requests)
  }

  override connect(...[deliver, generated]: Parameters<WorkerBakery['connect']>) {
    super.connect((request, chunk) => {
      this.delivered.push(request)
      deliver(request, chunk)
    }, generated)
  }
}

/** A chunk's cell: its face, slice and position. */
const cell = ({ face, level, layer, cutoff, i, j }: ChunkRequest) => [face, level, layer, cutoff, i, j].join()

const pending = (renderer: ParallaxRenderer) => renderer.getStats().lastUpdate?.chunksPending ?? 0

/**
 * The generated forest through `bakery`, settled, then moved forward
 * until its first grid moves with the camera: the cells baked from it
 * still in view are baked again, from the new one. Every chunk wanted
 * so far is baked.
 */
async function moveFirstGrid(bakery: Bakery) {
  const renderer = new ParallaxRenderer(world, { bakery })
  renderer.resize(800, 600)
  renderer.regenerateWorld(3, 'forest')
  const generated = renderer.getWorld()
  while (Array.from({ length: GRID_LEVELS }, (_, level) => generated.level(level)).some(grid => !grid)) await tick()
  await settle(renderer)
  const { x, y, z } = renderer.getCamera()
  renderer.setCamera({ x, y, z: z + 100 })
  do {
    renderer.render()
    await tick()
  } while (generated.version(0) === 0)
  return renderer
}

/** Renders until no chunk in view is still baking. */
/** Once the workers have generated each level's grid of the renderer's world around its spawn point. */
async function gridsIn(renderer: ParallaxRenderer) {
  const world = renderer.getWorld()
  while (Array.from({ length: GRID_LEVELS }, (_, level) => world.level(level)).some(grid => !grid)) {
    renderer.render()
    await tick()
  }
}

async function settle(renderer: ParallaxRenderer): Promise<Frame> {
  for (;;) {
    const frame = renderer.render()
    if (pending(renderer) === 0) return frame
    await tick()
  }
}

/** FNV-1a of the splats' bits: comparing (and diffing) the arrays themselves is slow. */
function checksum(splats: Float32Array): number {
  let hash = 0x811c9dc5
  for (const bits of new Uint32Array(splats.buffer, splats.byteOffset, splats.length)) hash = Math.imul(hash ^ bits, 0x01000193)
  return hash >>> 0
}

/** Each layer's chunks: content and where they go. */
const shown = (frame: Frame) => frame.layers.map(l => l.placements.map(({ chunk, x, y }) =>
  ({ i: chunk.i, j: chunk.j, splats: checksum(chunk.splats), x, y })))

/** What baking on the main thread shows. */
function reference(camera?: Camera, from = world, orientation?: Orientation) {
  const renderer = new ParallaxRenderer(from)
  renderer.resize(800, 600)
  if (camera) renderer.setCamera(camera)
  if (orientation) renderer.setOrientation(orientation)
  return shown(renderer.render())
}

describe('WorkerBakery', () => {
  it.each([[0, 0], [135, 30]])('ends up showing what baking on the main thread shows (yaw %s°, pitch %s°)', async (yaw, pitch) => {
    // In the middle of the world, so there is something in every direction.
    const camera = { x: 0, y: 15, z: 0 }
    const renderer = workerRenderer()
    renderer.setCamera(camera)
    renderer.setOrientation({ yaw, pitch })
    expect(shown(renderer.render()).flat()).toEqual([])
    const settled = await settle(renderer)
    expect(shown(settled)).toEqual(reference(camera, world, { yaw, pitch }))
    const faces = new Set(settled.layers.filter(l => l.placements.length > 0).map(l => l.face))
    // Straight along +z, four faces meet in the middle of the screen.
    expect(faces.size).toBeGreaterThanOrEqual(yaw === 0 ? 4 : 3)
  })

  it("shows a cell's bake from its grid's old place while the new one is baking", async () => {
    const renderer = await moveFirstGrid(new WorkerBakery([new FakeWorker(), new FakeWorker()]))
    const grids = (frame: Frame) => frame.layers.filter(l => l.slice.atoms === 0).flatMap(l => l.placements.map(p => p.chunk.grid))
    expect(grids(renderer.render())).toContain(0)
    expect(pending(renderer)).toBeGreaterThan(0)
    expect(new Set(grids(await settle(renderer)))).toEqual(new Set([1]))
  })

  it('bakes only the chunks of the latest render', async () => {
    const workers = [new FakeWorker(), new FakeWorker()]
    const renderer = workerRenderer(workers)
    renderer.render()
    expect(pending(renderer)).toBeGreaterThan(200)
    renderer.setCamera({ x: 500 })
    renderer.render()
    const needed = pending(renderer)
    await settle(renderer)
    expect(workers[0].baked.length + workers[1].baked.length).toBe(needed)
  })

  it("bakes each cell's chunks in one worker", async () => {
    const workers = [new FakeWorker(), new FakeWorker(), new FakeWorker()]
    await settle(workerRenderer(workers))
    const cells = workers.map(w => new Set(w.baked.map(cell)))
    for (const set of cells) expect(set.size).toBeGreaterThan(50)
    expect(new Set(cells.flatMap(set => [...set])).size).toBe(cells[0].size + cells[1].size + cells[2].size)
  })

  it('bakes from the latest list, a chunk again only once the main thread took it', () => {
    const posts: [number, number][] = []
    const steps: (() => void)[] = []
    const handle = bakeHandler(result => { if (result.type === 'chunks') posts.push([result.seq, result.baked[0].request.i]) }, step => steps.push(step), 0)
    const want = (received: number, ...cells: number[]) => {
      handle({ type: 'want', requests: encodeRequests(cells.map(at)), received })
    }
    const run = (count = Infinity) => { for (; count > 0 && steps.length > 0; count--) steps.shift()?.() }
    handle({ type: 'world', generation: 1, materials: world.materials, levels })
    want(0, 0, 1, 2)
    run(1)
    // Replaces what is left; 0 is on its way, so not baked again.
    want(0, 0, 3)
    run()
    expect(posts).toEqual([[1, 0], [2, 3]])
    // Taken, and listed again (evicted): baked again.
    want(2, 0)
    run()
    expect(posts).toEqual([[1, 0], [2, 3], [3, 0]])
  })

  it('posts the chunks a step bakes within its budget at once', () => {
    const posts: number[] = []
    let step: () => void = () => undefined
    const handle = bakeHandler(result => { if (result.type === 'chunks') posts.push(result.baked.length) }, s => { step = s }, Infinity)
    handle({ type: 'world', generation: 1, materials: world.materials, levels })
    handle({ type: 'want', requests: encodeRequests([0, 1, 2].map(at)), received: 0 })
    step()
    expect(posts).toEqual([3])
  })

  it('bakes cells that show nothing before rebaking cells an older bake stands in for', async () => {
    // Forward: new cells come into view, and those of the first grid are baked again.
    const bakery = new RecordingBakery([new FakeWorker(), new FakeWorker()])
    const renderer = await moveFirstGrid(bakery)
    const baked = new Set(bakery.delivered.map(cell))
    // Turned: cells never baked come into view too.
    const { yaw, pitch } = renderer.getOrientation()
    renderer.setOrientation({ yaw: yaw + 45, pitch })
    // Re-tiled segments come between, their slices drawn only once all baked.
    const drawn = new Set(renderer.render().layers.map(({ face, slice: { level, layer, cutoff } }) => [face.name, level, layer, cutoff].join()))
    const standsIn = bakery.latest.filter(r => drawn.has([r.face, r.level, r.layer, r.cutoff].join())).map(r => baked.has(cell(r)))
    expect(standsIn.indexOf(false)).toBe(0)
    expect(standsIn.lastIndexOf(false)).toBeLessThan(standsIn.indexOf(true))
  })

  it('stops baking once settled, though the cache is past its cap', async () => {
    // Sliced at one detail, then another: the first's slices are hidden.
    const settled = async (maxHiddenChunkBytes?: number) => {
      const renderer = new ParallaxRenderer(world, { bakery: new WorkerBakery([new FakeWorker()]), maxHiddenChunkBytes })
      renderer.resize(800, 600)
      renderer.setDetail(2 ** 4.25)
      await gridsIn(renderer)
      await settle(renderer)
      renderer.setDetail(32)
      await settle(renderer)
      // settle()'s last render evicted; did it drop chunks the view wants but does not draw?
      renderer.render()
      return { pending: pending(renderer), bytes: renderer.getStats().lastUpdate?.cachedBytes ?? 0 }
    }
    // With no room for hidden chunks, eviction reaches the slices in view.
    const [capped, uncapped] = [await settled(0), await settled()]
    expect(capped.pending).toBe(0)
    expect(capped.bytes).toBeLessThan(uncapped.bytes)
  })

  it('bakes no cell nearer layers cover, and ends up showing the same', () => {
    const settled = (skip: boolean) => {
      const { renderer, bake, baked } = slowRenderer()
      // Among the trees, near the ground.
      renderer.setCamera({ x: 0, y: 15, z: 0 })
      renderer.setSkipCovered(skip)
      do renderer.render(); while (bake(50) > 0)
      return { bakes: baked(), picture: picture(renderer.render(), 4) }
    }
    const [skipping, all] = [settled(true), settled(false)]
    expect(skipping.picture).toEqual(all.picture)
    // ~450 against ~880: what a chunk covers for every camera, less than for one.
    expect(skipping.bakes).toBeLessThan(all.bakes * 0.6)
  })

  it('while slices re-tile, shows the slices they replace until the new ones are baked', async () => {
    const renderer = workerRenderer()
    // Among the trees, near the ground.
    renderer.setCamera({ x: 0, y: 15, z: 0 })
    await settle(renderer)
    // Up and forward: slices re-tile, and nothing is baked for them yet.
    // Cells that come out from behind nearer ones show nothing until baked.
    renderer.setCamera({ x: 0, y: 35, z: 30 })
    const moved = picture(renderer.render(), 4)
    expect(pending(renderer)).toBeGreaterThan(0)
    const target = new ParallaxRenderer(world)
    target.resize(800, 600)
    target.setCamera(renderer.getCamera())
    const settled = picture(target.render(), 4)
    const drawn = settled.filter(p => p !== 0).length
    const gaps = settled.filter((p, k) => p !== 0 && moved[k] === 0).length
    expect(drawn).toBeGreaterThan(settled.length / 4)
    // Without stand-ins, ~15%.
    expect(gaps).toBeLessThan(drawn / 10)
  })

  it('moving faster than chunks bake, finishes the re-tiled segments that miss fewest chunks first', () => {
    const { renderer, bake } = slowRenderer()
    do renderer.render(); while (bake() > 0)
    const shown = () => renderer.getStats().totalChunksRegenerated
    const [{ z }, before] = [renderer.getCamera(), shown()]
    // Forward 6 a render, 50 chunks baked between renders: slices re-tile faster than they bake.
    for (let k = 1; k <= 16; k++) {
      renderer.setCamera({ z: z + 6 * k })
      renderer.render()
      bake(50)
    }
    // Of the 800 baked, 78 reach the screen; near first, 69 (the eyes of
    // the near levels move every render, so most of the rest is stale).
    expect(shown() - before).toBeGreaterThan(73)
  })

  it('drops chunks baked from a replaced world', async () => {
    const request = { face: '+x+z', level: 5, atoms: 1, layer: 0, cutoff: Infinity, i: 0, j: 0, grid: 0 }
    expect(new ChunkBaker(world.materials, levels).bake(request).splats.length).toBeGreaterThan(0)

    const bakery = new WorkerBakery([new FakeWorker()])
    const delivered: number[] = []
    bakery.connect((_, chunk) => { delivered.push(chunk.splats.length) })
    bakery.setWorld(world.materials, levels)
    bakery.want([request])
    const empty = new VoxelGrid({ minX: 0, minY: 0, minZ: 0, maxX: 0, maxY: 0, maxZ: 0 }, [{ color: '' }])
    bakery.setWorld(empty.materials, Array.from({ length: GRID_LEVELS }, () => empty))
    bakery.want([request])
    await tick()
    expect(delivered).toEqual([0])
  })

  it("generates a world's grids in the workers, and shows each level once its grid is in", async () => {
    const renderer = workerRenderer()
    renderer.regenerateWorld(3, 'forest')
    const generated = renderer.getWorld()
    const levels = () => Array.from({ length: GRID_LEVELS }, (_, level) => generated.level(level)).filter(Boolean).length
    // Nothing yet: no grid, so no cell is even listed to bake.
    expect(shown(renderer.render()).flat()).toEqual([])
    expect(pending(renderer)).toBe(0)
    while (levels() < GRID_LEVELS) await tick()
    const settled = await settle(renderer)
    expect(shown(settled)).toEqual(reference(undefined, new World(3, 'forest')))
    // Coarse grids among them.
    expect(settled.layers.some(l => l.slice.atoms > 0 && l.placements.length > 0)).toBe(true)
  })

  it('moves the grids with the camera: far from the spawn point, near slices still show', async () => {
    const renderer = workerRenderer()
    renderer.regenerateWorld(3, 'forest')
    const generated = renderer.getWorld()
    const levels = () => Array.from({ length: GRID_LEVELS }, (_, level) => generated.level(level)).filter(Boolean).length
    while (levels() < GRID_LEVELS) await tick()
    await settle(renderer)
    // Far enough for the grids of levels 0 and 1 to move, not 2's (see RECENTRE in world.ts).
    const camera = { ...generated.spawn, x: generated.spawn.x + 500 }
    renderer.setCamera(camera)
    renderer.render()
    while (generated.version(0) === 0 || generated.version(1) === 0 || generated.generating > 0) await tick()
    expect(generated.version(2)).toBe(0)
    const settled = await settle(renderer)
    expect(settled.layers.some(l => l.slice.level === 0 && l.placements.length > 0)).toBe(true)
    expect(shown(settled)).toEqual(reference(camera, new World(3, 'forest')))
  })
})
