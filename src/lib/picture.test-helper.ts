/** Tests' stand-in for the compositor (compositor.ts needs WebGL, which tests lack). */
import { SPLAT, type ChunkSplats } from './chunk-baker'
import type { Frame, Placement } from './renderer'
import { halfLayer } from './slice-selection'
import { inFace, screenBox, screenRay } from './view'

const rasters = new WeakMap<ChunkSplats, { key: string; pixels: Uint32Array }>()

/**
 * A chunk's splats as the compositor paints them placed at (x, y) on the
 * plane z = distance of a slice of `level`, seen from the camera: at each
 * of its mask's pixels (see Mask) on that plane, RGBA of the last splat
 * the camera's ray through it meets at the splat's depth, 0 if none.
 */
export function rasterise(chunk: ChunkSplats, { x, y }: Omit<Placement, 'chunk'>, distance: number, level: number): Uint32Array {
  const key = [x, y, distance, level].join()
  const cached = rasters.get(chunk)
  if (cached?.key === key) return cached.pixels
  const { splats, mask: { width, height, scale } } = chunk
  const pixels = new Uint32Array(width * height)
  const colours = new Uint32Array(splats.buffer, splats.byteOffset, splats.length)
  for (let k = 0; k < splats.length / SPLAT; k++) {
    const [sx, sy, sw, sh] = splats.subarray(k * SPLAT, k * SPLAT + 4)
    const colour = colours[k * SPLAT + 4]
    // Its site and size, camera-relative; and its depth over the plane's.
    const [cx, cy, z] = [x + sx, y + sy, distance + ((colour >>> 24) - 128) * halfLayer(level)]
    const [w, h, ratio] = [sw * (1 + Math.abs(cx / z)), sh, distance / z]
    // Mask pixels from a camera-relative point on the plane.
    const [col, row] = [(px: number) => (px - x) * scale - 0.5, (py: number) => (y - py) * scale - 0.5]
    for (let r = Math.max(0, Math.ceil(row((cy + h) * ratio))); r <= Math.min(height - 1, row((cy - h) * ratio)); r++) {
      for (let c = Math.max(0, Math.ceil(col((cx - w) * ratio))); c <= Math.min(width - 1, col((cx + w) * ratio)); c++) {
        // Where the ray through the pixel's centre is at the splat's depth.
        const [u, v] = [Math.abs((x + (c + 0.5) / scale) / ratio - cx), Math.abs((y - (r + 0.5) / scale) / ratio - cy)]
        if (u < w && u / Math.SQRT2 + v < h) pixels[r * width + c] = colour | 0xff000000
      }
    }
  }
  rasters.set(chunk, { key, pixels })
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
  frame.layers.forEach(({ face, slice, distance, placements }, index) => {
    const boxOf = screenBox(frame, face)
    for (const { chunk, x, y } of placements) {
      const box = boxOf(x, x + chunk.extentWidth, y - chunk.extentHeight, y, distance)
      if (!box) continue
      const pixels = rasterise(chunk, { x, y }, distance, slice.level)
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
