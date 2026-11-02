/**
 * What the chunk baker (bake/, Rust compiled to WebAssembly, driven by
 * chunk-baker.ts) is given besides the voxel grid: the voxel's faces in
 * lattice coordinates, and each face kind's shaded colours. See
 * docs/architecture.md, section 3.
 *
 * Face kinds are VOXEL_FACES (fcc.ts) by index. A slice is read in its
 * face's lattice coordinates (a, b, c) (view.ts), the same for every face,
 * and placed in face coordinates (a · LAYER, b, c · LAYER).
 */
import { VOXEL_FACES } from './fcc'
import type { Material } from './voxel-grid'
import { FACES, LAYER, type Basis } from './view'
import type { Vec3 } from './world'

const { right: R, up: U, forward: F } = FACES[0].lattice
const latticeOf = ([x, y, z]: readonly number[]) => [R, U, F].map(v => v.x * x + v.y * y + v.z * z)
const placed = ([a, b, c]: number[]) => [a * LAYER, b, c * LAYER]

/**
 * The voxel's faces in a face's lattice coordinates (any face's: they see
 * the same lattice), 6 numbers per kind: the offset (a, b, c) of the
 * neighbour across it, and its normal in face coordinates.
 */
export const VOXEL_GEOMETRY = Float64Array.from(VOXEL_FACES.flatMap(({ normal }) => [...latticeOf(normal), ...placed(latticeOf(normal))]))

/** Brightness of a face by the axis it faces along, shared with the reference viewer. @internal */
export const FACE_SHADE = { front: 0.9, back: 0.65, left: 0.7, right: 0.8, bottom: 0.55, top: 1 }

/**
 * Shade of a face by its outward normal in the world: the axes' shades
 * (FACE_SHADE) weighted by the squared components, so a face between the
 * top and the front gets the mean of both.
 */
export function faceShade({ x, y, z }: Vec3): number {
  const { front, back, left, right, bottom, top } = FACE_SHADE
  return (x * x * (x < 0 ? left : right) + y * y * (y < 0 ? bottom : top) + z * z * (z < 0 ? front : back)) / (x * x + y * y + z * z)
}

/**
 * Pixel colour of each (material, face kind) at index material · 12 + kind:
 * the material's colour times the face's shade, as opaque RGBA
 * (little-endian Uint32, as in ImageData). `lattice` is the face's (see
 * Face in view.ts), so a kind is shaded by the way it faces in the world.
 */
export function shadedPalette(materials: readonly Material[], { right, up, forward }: Basis): Uint32Array {
  const shades = VOXEL_FACES.map(({ normal }) => {
    const [a, b, c] = latticeOf(normal)
    return faceShade({
      x: (a * right.x + c * forward.x) / 2 + b * up.x,
      y: (a * right.y + c * forward.y) / 2 + b * up.y,
      z: (a * right.z + c * forward.z) / 2 + b * up.z,
    })
  })
  const palette = new Uint32Array(materials.length * VOXEL_FACES.length)
  materials.forEach(({ color }, id) => {
    const rgb = parseInt(color.slice(1, 7), 16)
    shades.forEach((shade, kind) => {
      const r = Math.round(((rgb >> 16) & 255) * shade)
      const g = Math.round(((rgb >> 8) & 255) * shade)
      const b = Math.round((rgb & 255) * shade)
      palette[id * VOXEL_FACES.length + kind] = ((255 << 24) | (b << 16) | (g << 8) | r) >>> 0
    })
  })
  return palette
}
