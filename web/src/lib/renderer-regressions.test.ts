import { describe, it, expect } from 'vitest'
import { ParallaxRenderer, type Frame } from './renderer'
import { ofVoxels, type Voxel } from './scene.test-helper'
import { hills } from './hills.test-helper'
import { projector } from './view'
import { rasterise } from './picture.test-helper'

/**
 * Behavioral regression tests for previously fixed rendering bugs. They check
 * observable output only: what the chunks in a frame draw, where the
 * frame puts them, and the per-render stats.
 */

// The renderer never modifies its world.
const world = hills(12345)

/** Screen positions (CSS pixels) of the centres of all drawn pixels of the chunks' rasters (see rasterise). */
function drawnPixels(frame: Frame): { x: number; y: number }[] {
  return frame.layers.flatMap(({ face, slice, distance, placements }) => placements.flatMap(({ chunk, x, y }) => {
    const points: { x: number; y: number }[] = []
    const project = projector(frame, face)
    const { width, scale } = chunk.mask
    rasterise(chunk, { x, y }, distance, slice).forEach((pixel, i) => {
      if (pixel === 0) return
      const point = project({
        x: x + ((i % width) + 0.5) / scale,
        y: y - (Math.floor(i / width) + 0.5) / scale,
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
 * (nearest pixel of a chunk's raster). The view must look straight along that face.
 */
function gapPixels(frame: Frame, along: string, fromX: number, toX: number): number {
  const { width, height, focal } = frame
  const covered = new Uint8Array(width * height)
  for (const { face, slice, distance, placements } of frame.layers.filter(l => l.face.name === along)) {
    for (const { chunk, x, y } of placements) {
      const corner = projector(frame, face)({ x, y, z: distance })
      if (!corner) continue
      const pixels = rasterise(chunk, { x, y }, distance, slice)
      const mask = chunk.mask
      // Screen pixels per raster pixel.
      const scale = focal / distance / mask.scale
      const left = Math.max(0, Math.ceil(corner.x - 0.5))
      const right = Math.min(width, corner.x + mask.width * scale - 0.5)
      for (let row = Math.max(0, Math.ceil(corner.y - 0.5)); row < Math.min(height, corner.y + mask.height * scale - 0.5); row++) {
        const v = Math.floor((row + 0.5 - corner.y) / scale)
        for (let column = left; column < right; column++) {
          const u = Math.floor((column + 0.5 - corner.x) / scale)
          if (pixels[v * mask.width + u] !== 0) covered[row * width + column] = 1
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
    const voxels: Voxel[] = []
    for (let y = 0; y <= 30; y += 2) {
      voxels.push({ x: 0, y, z: -58, color: '#00ff00' })
    }
    const renderer = new ParallaxRenderer(ofVoxels(voxels))
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
      renderer.setDetail(8)
      renderer.setSkipCovered(false)
      renderer.setCamera({ x: 0, y: 30, z: -60 })

      const before = new Set(renderer.render().layers.flatMap(l => l.placements.map(p => p.chunk)))
      renderer.setCamera({ x: 0, y: 30, z: -58 })
      // Chunks do not depend on where the camera is; only where the near
      // levels begin do slices re-tile.
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

  // Flat slices drew a point up to half a slice's thickness off, so a thin floor seen
  // from high above showed the sky between slices. Drawn in relief, each
  // voxel where it is, slices meet.
  describe('one layer thick floor seen from high above', () => {
    // A strip along +x+z: its sites at y = 0, a = x - z in [-160, 160) and
    // c = x + z in [0, 2400) (lattice coordinates of +x+z, see view.ts).
    const voxels: Voxel[] = []
    for (let a = -160; a < 160; a += 2) {
      for (let c = 0; c < 2400; c += 2) voxels.push({ x: (a + c) / 2, y: 0, z: (c - a) / 2, color: '#40a040' })
    }
    const floor = ofVoxels(voxels)

    it.each([2 ** 4.25, 2 ** 2.25])('shows no gaps between slices (detail %s)', detail => {
      const renderer = new ParallaxRenderer(floor)
      renderer.resize(1280, 800)
      renderer.setRenderDistance(800)
      renderer.setDetail(detail)
      // Looking along the strip from just before it (a = 3.8, c = -8).
      renderer.setCamera({ x: -2.1, y: 50, z: -5.9 })
      renderer.setOrientation({ yaw: 45, pitch: 0 })
      // Columns well inside the floor: its sides are jagged (voxels are rhombic dodecahedra).
      expect(gapPixels(renderer.render(), '+x+z', 540, 720)).toBe(0)
    }, 60_000)
  })
})
