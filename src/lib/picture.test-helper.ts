/** Tests' stand-in for the compositor (compositor.ts needs WebGL, which tests lack). */
import { SPLAT, type ChunkSplats } from './chunk-baker'
import type { Frame } from './renderer'
import { inFace, screenBox, screenRay } from './view'

const rasters = new WeakMap<ChunkSplats, Uint32Array>()

/**
 * A chunk's splats as the compositor paints them, sampled at its mask's
 * pixels (see Mask): RGBA,
 * 0 where nothing is drawn. Painted splats in order, then grown ones
 * where nothing is yet, nearest first.
 */
export function rasterise(chunk: ChunkSplats): Uint32Array {
  let pixels = rasters.get(chunk)
  if (pixels) return pixels
  const { splats, painted, mask: { width, height, scale } } = chunk
  pixels = new Uint32Array(width * height)
  const colours = new Uint32Array(splats.buffer, splats.byteOffset, splats.length)
  const count = splats.length / SPLAT
  const order = [...Array.from({ length: painted }, (_, k) => k), ...Array.from({ length: count - painted }, (_, k) => count - 1 - k)]
  for (const k of order) {
    const [sx, sy, sw, sh] = splats.subarray(k * SPLAT, k * SPLAT + 4)
    const [cx, cy, w, h] = [sx * scale, -sy * scale, sw * scale, sh * scale]
    for (let row = Math.max(0, Math.ceil(cy - h - 0.5)); row < Math.min(height, cy + h - 0.5); row++) {
      for (let col = Math.max(0, Math.ceil(cx - w - 0.5)); col < Math.min(width, cx + w - 0.5); col++) {
        if (Math.abs(col + 0.5 - cx) / Math.SQRT2 + Math.abs(row + 0.5 - cy) >= h) continue
        if (k >= painted && pixels[row * width + col] !== 0) continue
        pixels[row * width + col] = colours[k * SPLAT + 4]
      }
    }
  }
  rasters.set(chunk, pixels)
  return pixels
}

/**
 * The frame's colour at every `step`-th pixel, as the compositor draws it:
 * from each chunk's raster (see rasterise), nearer layers first, each
 * face's own rays; 0 for sky.
 */
export function picture(frame: Frame, step: number): Uint32Array {
  const cols = Math.ceil(frame.width / step)
  const colour = new Uint32Array(cols * Math.ceil(frame.height / step))
  const depth = new Float64Array(colour.length).fill(Infinity)
  frame.layers.forEach(({ face, distance, placements }, index) => {
    const boxOf = screenBox(frame, face)
    for (const { chunk, x, y } of placements) {
      const box = boxOf(x, x + chunk.extentWidth, y - chunk.extentHeight, y, distance)
      if (!box) continue
      const pixels = rasterise(chunk)
      for (let j = Math.max(0, Math.floor(box.minY / step)); j * step < Math.min(frame.height, box.maxY); j++) {
        for (let i = Math.max(0, Math.floor(box.minX / step)); i * step < Math.min(frame.width, box.maxX); i++) {
          const k = j * cols + i
          if (depth[k] < index) continue
          const ray = screenRay(frame, face, i * step + 0.5, j * step + 0.5)
          if (!inFace(ray.x, ray.y, ray.z)) continue
          const { width, height, scale } = chunk.mask
          const px = Math.floor(((ray.x / ray.z) * distance - x) * scale)
          const py = Math.floor((y - (ray.y / ray.z) * distance) * scale)
          if (px < 0 || py < 0 || px >= width || py >= height) continue
          const texel = pixels[py * width + px]
          if (texel === 0) continue
          depth[k] = index
          colour[k] = texel
        }
      }
    }
  })
  return colour
}
