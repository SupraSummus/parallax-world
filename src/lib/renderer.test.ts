import { describe, it, expect, beforeEach } from 'vitest'
import { ParallaxRenderer, DEFAULT_RENDER_DISTANCE, type Chunk, type Frame } from './renderer'
import { World } from './world'
import { selectSlices, chunkCell, eyeOf } from './slice-selection'
import { FACES, LAYER, faceTangents, focalLength, projector, toLocal, viewBasis, type Face, type Orientation } from './view'
import type { Vec3 } from './world'

// A small world with trees: rasterising the forest (~700k voxels)
// takes seconds per test. The renderer never modifies its world.
const world = new World(12345, 'hills_with_trees')
// Degrees: coarser than the default, so fewer chunks to bake.
const MAX_LEAK = 2.3

const faceNamed = (name: string) => FACES.find(f => f.name === name) as Face
// Looking along +x+z (yaw 45°), that face's layers are parallel to the
// screen, as a cube face's were looking straight at it.
const ALONG = faceNamed('+x+z')

/** The slices the renderer should show, face by face (in FACES order), near to far, for a 800 × 600 view. */
function expectedSlices(camera: Vec3, orientation: Orientation, detail: number, distance = DEFAULT_RENDER_DISTANCE) {
  const view = { orientation, width: 800, height: 600, focal: focalLength(90, 800, 600) }
  return FACES.filter(face => faceTangents(view, face)).flatMap(face =>
    selectSlices(toLocal(face, camera).z / LAYER, 4, distance / LAYER, detail).map(s => ({ face: face.name, ...s })))
}

describe('ParallaxRenderer', () => {
  let renderer: ParallaxRenderer
  let frame: Frame
  const render = () => { frame = renderer.render() }

  const lastUpdate = () => {
    const update = renderer.getStats().lastUpdate
    if (!update) throw new Error('render() has not been called')
    return update
  }

  /** The last frame's slices: face (by name), depth, size and chunks. */
  const layers = () => frame.layers.map(l => ({ face: l.face.name, slice: l.slice, depth: l.depth, size: l.size, chunks: l.placements.map(p => p.chunk) }))
  const allChunks = () => layers().flatMap(l => l.chunks)
  const placed = () => frame.layers.flatMap(l => l.placements)
  const placementOf = (chunk: Chunk) => {
    const layer = frame.layers.find(l => l.placements.some(p => p.chunk === chunk))
    const placement = layer?.placements.find(p => p.chunk === chunk)
    if (!layer || !placement) throw new Error('chunk not in the last frame')
    return { ...placement, distance: layer.distance, face: layer.face }
  }

  /** Screen position of point (x, y) on a chunk's plane (face coordinates), per its placement. */
  const screenOf = (chunk: Chunk, x: number, y: number) => {
    const { x: px, y: py, distance, face } = placementOf(chunk)
    const point = projector(frame, face)({ x: px + x - chunk.originX, y: py + y - chunk.originY, z: distance })
    if (!point) throw new Error('point behind the camera')
    return point
  }

  /** Screen bounding box of a chunk's projected corners. */
  const screenBox = (chunk: Chunk) => {
    const corners = [[0, 0], [1, 0], [0, 1], [1, 1]].map(([i, j]) =>
      screenOf(chunk, chunk.originX + (i * chunk.width) / chunk.scale, chunk.originY - (j * chunk.height) / chunk.scale))
    const xs = corners.map(c => c.x)
    const ys = corners.map(c => c.y)
    return { left: Math.min(...xs), right: Math.max(...xs), top: Math.min(...ys), bottom: Math.max(...ys) }
  }

  beforeEach(() => {
    renderer = new ParallaxRenderer(world)
    renderer.resize(800, 600)
    renderer.setMaxLeak(MAX_LEAK)
  })

  describe('camera', () => {
    it("starts at the world's spawn point", () => {
      expect(renderer.getCamera()).toEqual(world.spawn)
    })

    it('merges partial updates', () => {
      renderer.setCamera({ x: 5 })
      expect(renderer.getCamera()).toEqual({ ...world.spawn, x: 5 })
    })

    it("moves to the new world's spawn point when the world is regenerated", () => {
      renderer.setCamera({ x: 5 })
      renderer.regenerateWorld(99999, 'hills')
      expect(renderer.getCamera()).toEqual(new World(99999, 'hills').spawn)
    })
  })

  describe('layers', () => {
    it('renders one layer per selected slice, face by face, near to far', () => {
      render()
      const expected = expectedSlices(renderer.getCamera(), renderer.getOrientation(), renderer.getDetail())
      // Looking along a world axis, four faces meet in the middle of the screen.
      expect(new Set(expected.map(s => s.face)).size).toBeGreaterThanOrEqual(4)
      expect(layers().map(l => ({ face: l.face, ...l.slice }))).toEqual(expected)
      expect(lastUpdate().layerCount).toBe(expected.length)
    })

    it('splits slices into aligned chunks, each clipped to its cell', () => {
      render()
      const chunks = allChunks()
      expect(chunks.length).toBeGreaterThan(10)
      for (const layer of layers()) {
        for (const chunk of layer.chunks) {
          const { x, y, width: w, height: h } = chunkCell(layer.slice.level, chunk.i, chunk.j)
          expect(Number.isInteger(chunk.i) && Number.isInteger(chunk.j)).toBe(true)
          expect(chunk.facesDrawn).toBeGreaterThan(0)
          // Sizes are rounded up to whole pixels.
          const slack = 1 / chunk.scale
          expect(chunk.originX).toBeGreaterThanOrEqual(x - 1e-9)
          expect(chunk.originX + chunk.width / chunk.scale).toBeLessThanOrEqual(x + w + slack)
          expect(chunk.originY).toBeLessThanOrEqual(y + h)
          expect(chunk.originY - chunk.height / chunk.scale).toBeGreaterThanOrEqual(y - slack)
        }
      }
      expect(lastUpdate().voxelsRendered).toBeGreaterThanOrEqual(chunks.reduce((n, c) => n + c.facesDrawn, 0))
    })

    it("bakes a level's chunks from its eye, and again only once the camera leaves the eye's cell", () => {
      render()
      const eyes = (camera: Vec3) => new Map(layers().map(l => [`${l.face}:${String(l.slice.level)}`, eyeOf(l.slice.level, toLocal(faceNamed(l.face), camera)).join()]))
      const before = eyes(renderer.getCamera())
      for (const layer of layers()) {
        for (const chunk of layer.chunks) expect(chunk.eye.join()).toBe(before.get(`${layer.face}:${String(layer.slice.level)}`))
      }
      // Up past level 0's cell, within the others'.
      renderer.setCamera({ y: renderer.getCamera().y + 0.75 })
      render()
      const after = eyes(renderer.getCamera())
      const moved = new Set([...after].filter(([key, eye]) => before.get(key) !== eye).map(([key]) => key))
      expect(moved.size).toBeGreaterThan(0)
      const rebaked = layers().filter(l => l.chunks.some(c => c.eye.join() !== before.get(`${l.face}:${String(l.slice.level)}`)))
      expect(rebaked.length).toBeGreaterThan(0)
      expect(rebaked.every(l => moved.has(`${l.face}:${String(l.slice.level)}`))).toBe(true)
    })

    it('rasterises fewer pixels for farther, thicker slices', () => {
      render()
      const scales = layers().filter(l => l.face === '+x+z' && l.chunks.length > 0).map(l => l.chunks[0].scale)
      expect(scales).toEqual([...scales].sort((a, b) => b - a))
      expect(scales[0]).toBeGreaterThan(scales[scales.length - 1])
    })

    it.each([[0, 0], [10, -10]])('only rasterises chunks that are at least partly on screen (yaw %s°, pitch %s°)', (yaw, pitch) => {
      renderer.setOrientation({ yaw, pitch })
      render()
      const wide = lastUpdate()
      for (const { chunk } of placed()) {
        const box = screenBox(chunk)
        expect(box.left).toBeLessThan(800)
        expect(box.top).toBeLessThan(600)
        expect(box.right).toBeGreaterThan(0)
        expect(box.bottom).toBeGreaterThan(0)
      }
      // A smaller view at the same focal length, so with the same slices,
      // shows less of every slice.
      const narrow = new ParallaxRenderer(world)
      narrow.resize(400, 300)
      narrow.setFov((2 * Math.atan(0.5) * 180) / Math.PI)
      narrow.setMaxLeak(MAX_LEAK)
      narrow.setOrientation({ yaw, pitch })
      narrow.render()
      expect(narrow.getStats().lastUpdate?.cachedPixels).toBeLessThan(wide.cachedPixels)
    })

  })

  describe('compositing', () => {
    /** First chunk of every layer of the face looked along with chunks in view, near to far. */
    const firstChunks = () => layers().filter(l => l.face === ALONG.name)
      .flatMap(l => l.chunks.slice(0, 1).map(chunk => ({ layer: l, chunk })))

    /** Screen pixels per unit on a chunk's plane, at its top-left corner, looking along its face. */
    const pixelsPerUnit = (chunk: Chunk) => screenOf(chunk, chunk.originX + 1, chunk.originY).x - screenOf(chunk, chunk.originX, chunk.originY).x

    beforeEach(() => {
      // Looking along +x+z at the middle of the world, 60 away.
      renderer.setCamera({ x: -42, y: 30, z: -42 })
      renderer.setOrientation({ yaw: 45, pitch: 0 })
      // Near layers hide most of the view; keep chunks of many layers to compare.
      renderer.setSkipCovered(false)
    })

    it('scales nearer layers up more than farther ones', () => {
      render()
      const first = firstChunks()
      const scales = first.map(({ chunk }) => pixelsPerUnit(chunk))
      expect(scales.length).toBeGreaterThan(3)
      // Layer by layer within a level; where a level ends, its last layers
      // may be a little farther than the next level's first (see selectSlices).
      for (let k = 1; k < first.length; k++) {
        const tolerance = first[k].layer.slice.level === first[k - 1].layer.slice.level ? 1e-9 : 0.05 * scales[k - 1]
        expect(scales[k]).toBeLessThanOrEqual(scales[k - 1] + tolerance)
      }
      expect(scales[scales.length - 1]).toBeLessThan(scales[0] / 2)
    })

    it.each([[0, 0], [10, -10], [135, 30]])('projects the point the camera looks at (yaw %s°, pitch %s°) to the viewport centre on every chunk', (yaw, pitch) => {
      renderer.setOrientation({ yaw, pitch })
      render()
      expect(new Set(frame.layers.map(l => l.face)).size).toBeGreaterThan(1)
      for (const { face, distance, placements } of frame.layers) {
        // Where the view direction meets the layer's plane, in the face's coordinates.
        const forward = toLocal(face, viewBasis({ yaw, pitch }).forward)
        if (forward.z <= 0) continue
        const camera = toLocal(face, renderer.getCamera())
        const ahead = { x: camera.x + (forward.x / forward.z) * distance, y: camera.y + (forward.y / forward.z) * distance }
        for (const { chunk } of placements) {
          expect(screenOf(chunk, ahead.x, ahead.y)).toEqual({ x: expect.closeTo(400) as number, y: expect.closeTo(300) as number })
        }
      }
    })

    it('moves nearer layers further on sideways camera movement (parallax)', () => {
      render()
      const tracked = firstChunks().map(({ chunk }) => ({ chunk, before: screenOf(chunk, 0, 0).x }))
      // A unit along the screen's right.
      const { x, z } = renderer.getCamera()
      renderer.setCamera({ x: x + ALONG.right.x, z: z + ALONG.right.z })
      render()
      const shifts = tracked.filter(t => placed().some(p => p.chunk === t.chunk)).map(t => t.before - screenOf(t.chunk, 0, 0).x)
      expect(shifts.length).toBeGreaterThan(3)
      expect(shifts.every(s => s > 0)).toBe(true)
      expect(shifts).toEqual([...shifts].sort((a, b) => b - a))
    })

    it.each([90, 45])('puts the longer viewport side at the edge of a %s° field of view', fov => {
      renderer.setFov(fov)
      render()
      for (const { chunk } of firstChunks()) {
        const focalLength = pixelsPerUnit(chunk) * placementOf(chunk).distance
        expect(focalLength).toBeCloseTo(400 / Math.tan((fov * Math.PI) / 360))
      }
    })

    it('keeps world points at their projected position across a resize', () => {
      render()
      const slices = layers().map(l => l.slice)
      // The same detail, so the same slices.
      renderer.resize(600, 800)
      render()
      expect(layers().map(l => l.slice)).toEqual(slices)
      // Straight ahead, on the face looked along.
      const ahead = toLocal(ALONG, renderer.getCamera())
      for (const { chunk } of firstChunks()) {
        const { x, y } = screenOf(chunk, ahead.x, ahead.y)
        expect(x).toBeCloseTo(300)
        expect(y).toBeCloseTo(400)
      }
    })

    it('draws nothing for empty slices and fogs farther slices more', () => {
      // Outside the world.
      renderer.setCamera({ x: -120, z: -120 })
      render()
      expect(frame.layers.some(l => l.placements.length === 0)).toBe(true)

      // By distance: where a level ends, its last layers may be a little farther than the next level's first.
      const fogs = frame.layers.filter(l => l.face === ALONG).sort((a, b) => a.distance - b.distance).map(l => l.fog)
      expect(fogs[0]).toBeGreaterThan(0)
      expect(fogs).toEqual([...fogs].sort((a, b) => a - b))
      expect(fogs[fogs.length - 1]).toBeGreaterThan(fogs[0])
      // The sky, behind every slice, is fogged most.
      expect(frame.fog).toBeGreaterThanOrEqual(fogs[fogs.length - 1])
      expect(frame.fog).toBeGreaterThan(0.5)
      expect(frame.fog).toBeLessThan(1)
    })
  })

  describe('cache', () => {
    it('reuses the chunks still in view when the camera moves in x/y', () => {
      render()
      renderer.setCamera({ x: renderer.getCamera().x + 2, y: renderer.getCamera().y + 1 })
      render()
      expect(lastUpdate().chunksReused).toBeGreaterThan(lastUpdate().chunksRegenerated * 4)
    })

    it('rasterises nothing when the camera returns to where it was', () => {
      render()
      const start = renderer.getCamera()
      const chunks = allChunks()
      renderer.setCamera({ x: start.x + 10 })
      render()
      renderer.setCamera(start)
      render()
      expect(lastUpdate().chunksRegenerated).toBe(0)
      expect(allChunks().every((c, i) => c === chunks[i])).toBe(true)
      expect(allChunks()).toHaveLength(chunks.length)
    })

    it('rasterises nothing when rendering the same camera twice', () => {
      render()
      render()
      expect(lastUpdate().chunksRegenerated).toBe(0)
      expect(lastUpdate().voxelsRendered).toBe(0)
      expect(lastUpdate().cacheEfficiency).toBe(100)
    })

    it('keeps the slices when turning, rasterising only chunks that come into view', () => {
      const slicesOf = (face: string) => layers().filter(l => l.face === face).map(l => l.slice)
      render()
      const before = new Map(FACES.map(f => [f.name, slicesOf(f.name)]))
      const chunks = new Set(allChunks())
      renderer.setOrientation({ yaw: 10, pitch: 5 })
      render()
      const kept = FACES.filter(f => slicesOf(f.name).length > 0 && (before.get(f.name)?.length ?? 0) > 0)
      expect(kept.length).toBeGreaterThan(1)
      for (const f of kept) expect(slicesOf(f.name)).toEqual(before.get(f.name))
      expect(lastUpdate().chunksReused).toBeGreaterThan(lastUpdate().chunksRegenerated)
      expect(allChunks().filter(c => !chunks.has(c))).toHaveLength(lastUpdate().chunksRegenerated)
      renderer.setOrientation({ yaw: 0, pitch: 0 })
      render()
      expect(lastUpdate().chunksRegenerated).toBe(0)
    })

    it('rebuilds every chunk after world regeneration', () => {
      render()
      renderer.regenerateWorld(99999, 'hills_with_trees')
      render()
      expect(lastUpdate().chunksReused).toBe(0)
      expect(lastUpdate().chunksRegenerated).toBe(lastUpdate().chunkCount)
      expect(placed()).toHaveLength(lastUpdate().chunkCount)
    })
  })

  describe('settings', () => {
    it('zooms without re-slicing, so the same slices leak more pixels', () => {
      render()
      const slicesOf = (face: string) => layers().filter(l => l.face === face).map(l => l.slice)
      const wide = { leak: lastUpdate().leakPixels, slices: new Map(FACES.map(f => [f.name, slicesOf(f.name)])) }
      expect(wide.leak).toBeLessThanOrEqual((MAX_LEAK * Math.PI / 180) * 400)
      // 4×: tan 45° / tan 14.04°.
      renderer.setFov((2 * Math.atan(0.25) * 180) / Math.PI)
      render()
      expect(lastUpdate().leakPixels).toBeCloseTo(4 * wide.leak)
      // Fewer faces are in view, each with the same slices.
      const faces = new Set(layers().map(l => l.face))
      expect(faces.size).toBeGreaterThan(0)
      for (const face of faces) expect(slicesOf(face)).toEqual(wide.slices.get(face))
    })

    it('uses fewer, thicker slices for a larger tolerance', () => {
      renderer.setMaxLeak(0.3)
      render()
      const strict = lastUpdate()
      renderer.setMaxLeak(6)
      render()
      expect(lastUpdate().detail).toBeLessThan(strict.detail)
      expect(lastUpdate().layerCount).toBeLessThan(strict.layerCount)
      expect(lastUpdate().layerCount).toBe(expectedSlices(world.spawn, renderer.getOrientation(), lastUpdate().detail).length)
    })
  })

  describe('render distance', () => {
    it('covers exactly the render distance', () => {
      renderer.setRenderDistance(50)
      render()
      const camera = renderer.getCamera()
      const slices = layers()
      expect(slices.map(l => ({ face: l.face, ...l.slice })))
        .toEqual(expectedSlices(camera, renderer.getOrientation(), renderer.getDetail(), 50))
      for (const face of FACES) {
        const last = slices.filter(l => l.face === face.name).pop()
        if (!last) continue
        const end = (toLocal(face, camera).z + 50) / LAYER
        expect(last.depth + last.size).toBeGreaterThanOrEqual(end)
        expect(last.depth).toBeLessThan(end)
      }
    })
  })

  describe('orientation', () => {
    it('looks all around, each way with the face of the nearest neighbour direction', () => {
      // In the middle of the world, so there is something in every direction.
      renderer.setCamera({ x: 0, y: 16, z: 0 })
      // A narrow view straight at a face shows no other.
      renderer.setFov(30)
      for (const [yaw, pitch, face] of [[-135, 0, '-x-z'], [0, 45, '+y+z'], [90, -45, '+x-y']] as const) {
        renderer.setOrientation({ yaw, pitch })
        render()
        expect(new Set(layers().map(l => l.face))).toEqual(new Set([face]))
        expect(lastUpdate().chunkCount).toBeGreaterThan(0)
      }
    })

    it('stops pitch at straight up and down, and wraps yaw', () => {
      renderer.setOrientation({ yaw: 190, pitch: 100 })
      expect(renderer.getOrientation()).toEqual({ yaw: -170, pitch: 90 })
    })
  })

  describe('stats', () => {
    it('accumulates session totals', () => {
      render()
      const first = lastUpdate()
      renderer.setCamera({ z: -40 })
      render()
      const second = lastUpdate()

      const stats = renderer.getStats()
      expect(stats.totalUpdates).toBe(2)
      expect(stats.totalChunksRegenerated).toBe(first.chunksRegenerated + second.chunksRegenerated)
      expect(stats.totalVoxelsRendered).toBe(first.voxelsRendered + second.voxelsRendered)
      expect(stats.totalRenderTime).toBeCloseTo(first.renderTime + second.renderTime)
      expect(stats.averageCacheEfficiency).toBeCloseTo((first.cacheEfficiency + second.cacheEfficiency) / 2)
    })
  })
})
