import { describe, it, expect } from 'vitest'
import { ParallaxRenderer, type Frame } from './renderer'
import { World } from './world'
import { projector } from './view'

/**
 * Behavioral regression tests for previously fixed rendering bugs. They check
 * observable output only: the pixels of the chunks in a frame, where the
 * frame puts them, and the per-render stats.
 */

// A small world with trees: rasterising the forest (~700k voxels)
// takes seconds per test. The renderer never modifies its world.
const world = new World(12345, 'hills_with_trees')

function makeWorldWithVoxels(voxels: World['voxels']): World {
  const custom = new World(1, 'hills') // cheapest world to generate; its voxels are replaced
  custom.voxels = voxels
  return custom
}

/** Screen positions (CSS pixels) of the centres of all drawn chunk pixels. */
function drawnPixels(frame: Frame): { x: number; y: number }[] {
  return frame.layers.flatMap(({ face, distance, placements }) => placements.flatMap(({ chunk, x, y }) => {
    const points: { x: number; y: number }[] = []
    const project = projector(frame, face)
    chunk.pixels.forEach((pixel, i) => {
      if (pixel === 0) return
      const point = project({
        x: x + ((i % chunk.width) + 0.5) / chunk.scale,
        y: y - (Math.floor(i / chunk.width) + 0.5) / chunk.scale,
        z: distance,
      })
      if (point) points.push(point)
    })
    return points
  }))
}

/**
 * Screen pixels in columns [fromX, toX) that nothing covers, between covered
 * ones, compositing the layers of the face `along` as the compositor does
 * (nearest chunk pixel). The view must look straight along that face.
 */
function gapPixels(frame: Frame, along: string, fromX: number, toX: number): number {
  const { width, height, focal } = frame
  const covered = new Uint8Array(width * height)
  for (const { face, distance, placements } of frame.layers.filter(l => l.face.name === along)) {
    for (const { chunk, x, y } of placements) {
      const corner = projector(frame, face)({ x, y, z: distance })
      if (!corner) continue
      // Screen pixels per chunk pixel.
      const scale = focal / distance / chunk.scale
      const left = Math.max(0, Math.ceil(corner.x - 0.5))
      const right = Math.min(width, corner.x + chunk.width * scale - 0.5)
      for (let row = Math.max(0, Math.ceil(corner.y - 0.5)); row < Math.min(height, corner.y + chunk.height * scale - 0.5); row++) {
        const v = Math.floor((row + 0.5 - corner.y) / scale)
        for (let column = left; column < right; column++) {
          const u = Math.floor((column + 0.5 - corner.x) / scale)
          if (chunk.pixels[v * chunk.width + u] !== 0) covered[row * width + column] = 1
        }
      }
    }
  }
  let gaps = 0
  for (let x = fromX; x < toX; x++) {
    const column = Array.from({ length: height }, (_, y) => covered[y * width + x])
    gaps += column.slice(column.indexOf(1), column.lastIndexOf(1)).filter(c => c === 0).length
  }
  return gaps
}

describe('Renderer regressions', () => {
  // Near layers used to be rasterised at their full perspective scale, so a
  // voxel right in front of the camera landed far outside the canvas bitmap.
  // Layers are now rasterised at a fixed scale and scaled when compositing.
  it('shows voxels right in front of the camera inside the viewport', () => {
    // A column from the ground up to eye level, 2 units ahead of the camera.
    const voxels: World['voxels'] = []
    for (let y = 0; y <= 30; y += 2) {
      voxels.push({ x: 0, y, z: -58, color: '#00ff00', type: 1 })
    }
    const renderer = new ParallaxRenderer(makeWorldWithVoxels(voxels))
    renderer.resize(800, 600)
    renderer.setCamera({ x: 0, y: 30, z: -60 })

    const visible = drawnPixels(renderer.render()).filter(p => p.x >= 0 && p.x < 800 && p.y >= 0 && p.y < 600)
    expect(visible.length).toBeGreaterThan(0)
  })

  // Small z moves used to invalidate every layer, including distant ones
  // whose content had not changed.
  describe('z-movement cache churn', () => {
    // Near layers, which change as the camera moves, hide most of the view
    // here; these tests are about the distant ones, so nothing is skipped.
    it('reuses the distant chunks after a 2-unit Z move from the initial camera position', () => {
      const renderer = new ParallaxRenderer(world)
      renderer.resize(800, 600)
      // Coarser than the default, so that levels from 4 on are in range.
      renderer.setMaxLeak(2.3)
      renderer.setSkipCovered(false)
      renderer.setCamera({ x: 0, y: 30, z: -60 })

      const before = new Set(renderer.render().layers.flatMap(l => l.placements.map(p => p.chunk)))
      renderer.setCamera({ x: 0, y: 30, z: -58 })
      // The near levels' eyes move (their cells are a few units across, and
      // the camera is on a boundary of level 3's), so they are baked again;
      // the cells of levels from 4 on reach 16 units and more.
      const distant = renderer.render().layers.filter(l => l.slice.level >= 4).flatMap(l => l.placements.map(p => p.chunk))
      expect(distant.length).toBeGreaterThan(10)
      expect(distant.filter(chunk => before.has(chunk)).length).toBeGreaterThan(distant.length * 0.9)
    })

    it('does not re-render every chunk on every keypress when moving along Z', () => {
      const renderer = new ParallaxRenderer(world)
      renderer.resize(800, 600)
      renderer.setSkipCovered(false)
      renderer.setCamera({ x: 0, y: 30, z: -60 })

      const chunkCount = renderer.render().layers.flatMap(l => l.placements).length
      const initialRegens = renderer.getStats().totalChunksRegenerated

      // Simulate holding W for 8 frames, a unit per frame.
      for (let i = 1; i <= 8; i++) {
        renderer.setCamera({ x: 0, y: 30, z: -60 + i })
        renderer.render()
      }

      const regensDuringMovement =
        renderer.getStats().totalChunksRegenerated - initialRegens

      // Only the near levels, whose eyes move every unit or two, plus newly
      // entering chunks, should re-render: well under one full re-render per frame.
      expect(regensDuringMovement).toBeLessThan(chunkCount * 8 * 0.6)
    })
  })

  // Layers leaving the view used to be deleted, so moving forward and back
  // re-rendered them. They are now kept in an LRU cache.
  describe('layers leaving and re-entering view', () => {
    it('reuses chunks when the camera returns to an already-visited Z position', () => {
      const renderer = new ParallaxRenderer(world)
      renderer.resize(800, 600)
      renderer.setSkipCovered(false)
      renderer.setCamera({ x: 0, y: 30, z: -60 })

      renderer.render() // warm cache at z=-30

      // Take a round trip: forward a few units, then back to the exact start.
      renderer.setCamera({ x: 0, y: 30, z: -50 })
      renderer.render()
      renderer.setCamera({ x: 0, y: 30, z: -60 })
      renderer.render()

      const returnStats = renderer.getStats().lastUpdate
      expect(returnStats).toBeDefined()
      expect(returnStats?.chunksReused ?? 0).toBeGreaterThan(returnStats?.chunksRegenerated ?? 0)
    })
  })

  // Slices move what they show by up to the leak bound, so a thin floor
  // seen from high above showed the sky between them. Grown faces cover it.
  describe('one layer thick floor seen from high above', () => {
    // A strip along +x+z: its sites at y = 0, a = x - z in [-160, 160) and
    // c = x + z in [0, 2400) (lattice coordinates of +x+z, see view.ts).
    const voxels: World['voxels'] = []
    for (let a = -160; a < 160; a += 2) {
      for (let c = 0; c < 2400; c += 2) voxels.push({ x: (a + c) / 2, y: 0, z: (c - a) / 2, color: '#40a040', type: 1 })
    }
    const floor = makeWorldWithVoxels(voxels)
    const gaps = (fillGaps: boolean) => {
      const renderer = new ParallaxRenderer(floor)
      renderer.resize(1280, 800)
      renderer.setRenderDistance(800)
      // Looking along the strip from just before it (a = 3.8, c = -8), off
      // the eyes enough for gaps to open. From higher up, the far slices,
      // of coarser voxels (thicker sheets), hide them.
      renderer.setCamera({ x: -2.1, y: 50, z: -5.9 })
      renderer.setOrientation({ yaw: 45, pitch: 0 })
      renderer.setFillGaps(fillGaps)
      // Columns well inside the floor: grown faces may stick out past its
      // sides, and its sides are jagged (voxels are rhombic dodecahedra).
      return gapPixels(renderer.render(), '+x+z', 540, 720)
    }

    // Filling is approximate: a stray pixel may stay.
    it('fills nearly every gap between slices', () => {
      const [unfilled, filled] = [gaps(false), gaps(true)]
      // Without filling the gaps show (the test would pass vacuously otherwise).
      expect(unfilled).toBeGreaterThan(20)
      expect(filled).toBeLessThanOrEqual(unfilled / 20)
    }, 60_000)
  })
})
