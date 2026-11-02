import { describe, it, expect } from 'vitest'
import { bakeHandler, ChunkBaker, encodeRequests, type BakeMessage, type BakeResult, type ChunkRequest } from './chunk-baker'
import { ParallaxRenderer, type Bakery, type Camera, type Frame } from './renderer'
import type { Orientation } from './view'
import { picture } from './picture.test-helper'
import { VoxelGrid } from './voxel-grid'
import { WorkerBakery, type BakeWorker } from './worker-bakery'
import { levelsOf, World } from './world'

// A small world with trees, as in renderer.test.ts.
const world = new World(12345, 'hills_with_trees')

/** bake.worker.ts in microtasks: asynchronous and in order, like a worker, and done before the next timer. */
class FakeWorker implements BakeWorker {
  onmessage: ((event: MessageEvent<BakeResult>) => void) | null = null
  baked: ChunkRequest[] = []
  private handle = bakeHandler(result => {
    this.baked.push(...result.baked.map(({ request }) => request))
    queueMicrotask(() => { this.onmessage?.({ data: result } as MessageEvent<BakeResult>) })
  }, queueMicrotask, 0)

  postMessage(message: BakeMessage) {
    queueMicrotask(() => { this.handle(message) })
  }

  terminate() {}
}

/** A chunk of a far slice, cheap to bake. */
const at = (i: number): ChunkRequest => ({ face: '+x+z', level: 6, layer: 0, cutoff: Infinity, i, j: 0, eye: [0, 0, 0], fill: false })

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
  const baker = new ChunkBaker(world.levels)
  let wanted: ChunkRequest[] = []
  let deliver: Parameters<Bakery['connect']>[0] = () => undefined
  const renderer = new ParallaxRenderer(world, { bakery: { connect(d) { deliver = d }, setWorld() {}, want(r) { wanted = r } } })
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

/** Remembers the chunks wanted: all of them, and the latest list. */
class RecordingBakery extends WorkerBakery {
  all: ChunkRequest[] = []
  latest: ChunkRequest[] = []
  override want(requests: ChunkRequest[]) {
    this.all.push(...requests)
    this.latest = requests
    super.want(requests)
  }
}

/** A chunk's cell: its face, slice and position. */
const cell = ({ face, level, layer, cutoff, i, j }: ChunkRequest) => [face, level, layer, cutoff, i, j].join()

const pending = (renderer: ParallaxRenderer) => renderer.getStats().lastUpdate?.chunksPending ?? 0

/** Renders until no chunk in view is still baking. */
async function settle(renderer: ParallaxRenderer): Promise<Frame> {
  for (;;) {
    const frame = renderer.render()
    if (pending(renderer) === 0) return frame
    await tick()
  }
}

/** FNV-1a: comparing (and diffing) the pixel arrays themselves is slow. */
function checksum(pixels: Uint32Array): number {
  let hash = 0x811c9dc5
  for (const pixel of pixels) hash = Math.imul(hash ^ pixel, 0x01000193)
  return hash >>> 0
}

/** Each layer's chunks: content and where they go. */
const shown = (frame: Frame) => frame.layers.map(l => l.placements.map(({ chunk, x, y }) =>
  ({ i: chunk.i, j: chunk.j, eye: chunk.eye.join(), pixels: checksum(chunk.pixels), x, y })))

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

  it("shows a cell's older bake while its new eye's is baking", async () => {
    const renderer = workerRenderer()
    await settle(renderer)
    // Far enough sideways that the eyes of the nearest slices change.
    renderer.setCamera({ x: renderer.getCamera().x + 3 })
    const baking = shown(renderer.render())
    const target = reference(renderer.getCamera())
    const standIns = baking.flatMap((layer, i) => layer.filter(c =>
      target[i].some(t => t.i === c.i && t.j === c.j && t.eye !== c.eye)))
    expect(standIns.length).toBeGreaterThan(0)
    expect(shown(await settle(renderer))).toEqual(target)
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
    const handle = bakeHandler(({ seq, baked: [{ request }] }) => posts.push([seq, request.i]), step => steps.push(step), 0)
    const want = (received: number, ...cells: number[]) => {
      handle({ type: 'want', requests: encodeRequests(cells.map(at)), received })
    }
    const run = (count = Infinity) => { for (; count > 0 && steps.length > 0; count--) steps.shift()?.() }
    handle({ type: 'world', generation: 1, levels: world.levels })
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
    const handle = bakeHandler(({ baked }) => posts.push(baked.length), s => { step = s }, Infinity)
    handle({ type: 'world', generation: 1, levels: world.levels })
    handle({ type: 'want', requests: encodeRequests([0, 1, 2].map(at)), received: 0 })
    step()
    expect(posts).toEqual([3])
  })

  it('bakes cells that show nothing before rebaking cells an older bake stands in for', async () => {
    const bakery = new RecordingBakery([new FakeWorker(), new FakeWorker()])
    const renderer = new ParallaxRenderer(world, { bakery })
    renderer.resize(800, 600)
    // Covered cells are not baked: bake every cell, so each has an older bake.
    renderer.setSkipCovered(false)
    await settle(renderer)
    const baked = new Set(bakery.all.map(cell))
    // Forward and sideways: near slices re-tile, and the near levels' eyes move.
    const { x, z } = renderer.getCamera()
    renderer.setCamera({ x: x + 3, z: z + 3 })
    renderer.render()
    const standsIn = bakery.latest.map(r => baked.has(cell(r)))
    expect(standsIn.indexOf(false)).toBe(0)
    expect(standsIn.lastIndexOf(false)).toBeLessThan(standsIn.indexOf(true))
  })

  it('stops baking once settled, though the cache is past its cap', async () => {
    // The city at full HD: its chunks with fill gaps on and off pass the cap
    // on hidden chunks, so eviction reaches the slices in view.
    const renderer = new ParallaxRenderer(new World(12345, 'city'), { bakery: new WorkerBakery([new FakeWorker()]) })
    renderer.resize(1920, 1080)
    await settle(renderer)
    renderer.setFillGaps(false)
    await settle(renderer)
    expect(renderer.getStats().lastUpdate?.cachedPixels).toBeGreaterThan(32_000_000)
    // settle()'s last render evicted; did it drop chunks the view wants but does not draw?
    renderer.render()
    expect(pending(renderer)).toBe(0)
  }, 60_000)

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
    // ~240 against ~680.
    expect(skipping.bakes).toBeLessThan(all.bakes / 2)
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
    const request = { face: '+x+z', level: 5, layer: 0, cutoff: Infinity, i: 0, j: 0, eye: [0, 0, 0] as const, fill: false }
    expect(new ChunkBaker(world.levels).bake(request).facesDrawn).toBeGreaterThan(0)

    const bakery = new WorkerBakery([new FakeWorker()])
    const delivered: number[] = []
    bakery.connect((_, image) => { delivered.push(image.facesDrawn) })
    bakery.setWorld(world.levels)
    bakery.want([request])
    bakery.setWorld(levelsOf(VoxelGrid.fromVoxels([])))
    bakery.want([request])
    await tick()
    expect(delivered).toEqual([0])
  })

  it('bakes the new world after regeneration', async () => {
    const renderer = workerRenderer()
    renderer.regenerateWorld(99999, 'hills_with_trees')
    expect(shown(await settle(renderer))).toEqual(reference(undefined, new World(99999, 'hills_with_trees')))
  })
})
