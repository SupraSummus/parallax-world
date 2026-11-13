/**
 * The camera's view and the 12 faces of a rhombic dodecahedron around it,
 * one towards each nearest neighbour of the FCC lattice (fcc.ts). Each face
 * has its own stack of slices, cut along its axis, and draws the rays
 * closer to its axis than to any other's. See docs/architecture.md,
 * section 4.
 */
import type { Vec3 } from './world'

/** Degrees. Yaw turns right (towards +x), pitch looks up (towards +y). */
export interface Orientation {
  yaw: number
  pitch: number
}

/** Axes in world coordinates: screen right, screen up, and the view direction. */
export interface Basis {
  right: Vec3
  up: Vec3
  forward: Vec3
}

export function viewBasis({ yaw, pitch }: Orientation): Basis {
  const y = (yaw * Math.PI) / 180
  const p = (pitch * Math.PI) / 180
  return {
    right: { x: Math.cos(y), y: 0, z: -Math.sin(y) },
    up: { x: -Math.sin(y) * Math.sin(p), y: Math.cos(p), z: -Math.cos(y) * Math.sin(p) },
    forward: { x: Math.sin(y) * Math.cos(p), y: Math.sin(p), z: Math.cos(y) * Math.cos(p) },
  }
}

/**
 * A face: the camera's axes looking straight at it. Its slices are cut
 * along `forward`, in face coordinates (x, y, z) = world x · right + y · up
 * + z · forward.
 *
 * `lattice` holds integer vectors R, U, F: R = √2 · right, U = up (a world
 * axis), F = √2 · forward. World site p is at lattice coordinates
 * (a, b, c) = (p · R, p · U, p · F), with a, b and c all even or all odd,
 * and back at p = (a · R + c · F) / 2 + b · U. Layer c is the plane
 * z = c · LAYER of face coordinates, and site (a, b, c) is at
 * (a · LAYER, b, c · LAYER). Every face sees the same lattice: the faces
 * are turns of each other that map sites to sites.
 */
export interface Face extends Basis {
  name: string
  lattice: Basis
}

/** Spacing of a face's layers, and of its lattice along `right`: half of a neighbour's distance, √2. */
export const LAYER = Math.SQRT1_2

const vector = ([x, y, z]: number[]): Vec3 => ({ x, y, z })
const scaled = (v: Vec3, s: number): Vec3 => ({ x: v.x * s, y: v.y * s, z: v.z * s })
const crossProduct = (a: Vec3, b: Vec3): Vec3 => ({ x: a.y * b.z - a.z * b.y, y: a.z * b.x - a.x * b.z, z: a.x * b.y - a.y * b.x })

/** One face per neighbour direction F; up is the world axis across F, right is up × forward. */
export const FACES: readonly Face[] = [1, 0, 2].flatMap(k => [[1, 1], [-1, 1], [1, -1], [-1, -1]].map(([s, t]) => {
  const f = [0, 0, 0]
  f[(k + 1) % 3] = s
  f[(k + 2) % 3] = t
  const u = [0, 0, 0]
  u[k] = 1
  const [F, U] = [vector(f), vector(u)]
  const R = crossProduct(U, F)
  const name = f.map((v, i) => (v === 0 ? '' : `${v > 0 ? '+' : '-'}${'xyz'[i]}`)).join('')
  return { name, right: scaled(R, LAYER), up: U, forward: scaled(F, LAYER), lattice: { right: R, up: U, forward: F } }
}))

/**
 * Whether a ray (x, y, z) in face coordinates is the face's: as close to
 * its axis as to any other face's, |x| + √2 · |y| ≤ z. The face's rays are
 * a rhombus of tangents, reaching 1 along x (towards the world axes) and
 * 1/√2 along y (towards the cube's diagonals).
 */
export function inFace(x: number, y: number, z: number): boolean {
  return Math.abs(x) + Math.SQRT2 * Math.abs(y) <= z
}

const dot = (a: Vec3, b: Vec3) => a.x * b.x + a.y * b.y + a.z * b.z

/** World vector v in the coordinates of `axes`. */
export function toLocal(axes: Basis, v: Vec3): Vec3 {
  return { x: dot(v, axes.right), y: dot(v, axes.up), z: dot(v, axes.forward) }
}

/** The camera's axes in a face's coordinates. */
export function faceBasis(face: Face, orientation: Orientation): Basis {
  const { right, up, forward } = viewBasis(orientation)
  return { right: toLocal(face, right), up: toLocal(face, up), forward: toLocal(face, forward) }
}

/**
 * Focal length in pixels: the longer viewport side spans `fov` degrees.
 * Looking straight at a plane d ahead, it is drawn at focal / d pixels per
 * world unit.
 */
export function focalLength(fov: number, width: number, height: number): number {
  return Math.max(width, height) / 2 / Math.tan((fov * Math.PI) / 360)
}

/** A view: orientation, viewport in CSS pixels and focal length in pixels. */
export interface View {
  orientation: Orientation
  width: number
  height: number
  focal: number
}

/** Direction through the screen point (sx, sy), in CSS pixels from the top-left, in a face's coordinates. @internal */
export function screenRay(view: View, face: Face, sx: number, sy: number): Vec3 {
  const { right, up, forward } = faceBasis(face, view.orientation)
  const a = sx - view.width / 2
  const b = view.height / 2 - sy
  return {
    x: forward.x * view.focal + right.x * a + up.x * b,
    y: forward.y * view.focal + right.y * a + up.y * b,
    z: forward.z * view.focal + right.z * a + up.z * b,
  }
}

/**
 * Clips a convex polygon to the half-space distance(p) ≥ 0 (Sutherland–
 * Hodgman), for a linear `distance`. Points can be rays: through the
 * origin, a half-space of rays is a half-space of vectors.
 */
function clip(polygon: Vec3[], distance: (p: Vec3) => number): Vec3[] {
  const out: Vec3[] = []
  polygon.forEach((p, i) => {
    const q = polygon[(i + 1) % polygon.length]
    const [dp, dq] = [distance(p), distance(q)]
    if (dp >= 0) out.push(p)
    if ((dp < 0) !== (dq < 0)) {
      const t = dp / (dp - dq)
      out.push({ x: p.x + (q.x - p.x) * t, y: p.y + (q.y - p.y) * t, z: p.z + (q.z - p.z) * t })
    }
  })
  return out
}

// The sides of a face's pyramid of rays (see inFace).
const PYRAMID_SIDES = [1, -1].flatMap(s => [1, -1].map(t => (p: Vec3) => p.z - s * p.x - t * Math.SQRT2 * p.y))

/** Bounds of x / z and y / z of a face's rays within a convex cone of rays (see faceTangents). */
export interface TangentBox {
  minX: number
  maxX: number
  minY: number
  maxY: number
}

/**
 * Bounds of x / z and y / z of the face's rays (see inFace) within the
 * convex cone spanned by `rays` (in order around it), or null if it has
 * none of them, or only along an edge.
 */
function tangentBox(rays: Vec3[]): TangentBox | null {
  for (const side of PYRAMID_SIDES) rays = clip(rays, side)
  const tangents = rays.map(r => ({ x: r.x / r.z, y: r.y / r.z }))
  // Shoelace area: a cone the face only touches along an edge shows nothing of it.
  const area = tangents.reduce((sum, p, i) => {
    const q = tangents[(i + 1) % tangents.length]
    return sum + p.x * q.y - q.x * p.y
  }, 0)
  if (!(Math.abs(area) > 1e-9)) return null
  const xs = tangents.map(t => t.x)
  const ys = tangents.map(t => t.y)
  return { minX: Math.min(...xs), maxX: Math.max(...xs), minY: Math.min(...ys), maxY: Math.max(...ys) }
}

/**
 * Where the screen meets a face's pyramid of rays (see inFace):
 * bounds of x / z and y / z there. On the face's plane
 * z = d, the face draws (part of) [minX·d, maxX·d] × [minY·d, maxY·d].
 * Null if the screen shows none of the face.
 */
export function faceTangents(view: View, face: Face): TangentBox | null {
  const { width: w, height: h } = view
  return tangentBox([[0, 0], [w, 0], [w, h], [0, h]].map(([sx, sy]) => screenRay(view, face, sx, sy)))
}

/**
 * For one view and face: the screen position (CSS pixels from the
 * top-left) of a camera-relative point in face coordinates, or null if it
 * is not in front of the camera. The compositor's vertex shader does the
 * same.
 * @internal
 */
export function projector(view: View, face: Face): (p: Vec3) => { x: number; y: number } | null {
  const camera = faceBasis(face, view.orientation)
  return p => {
    const z = dot(p, camera.forward)
    if (z <= 0) return null
    return {
      x: view.width / 2 + (dot(p, camera.right) * view.focal) / z,
      y: view.height / 2 - (dot(p, camera.up) * view.focal) / z,
    }
  }
}

/** Points nearer the camera plane than this are cut off before projecting. */
const NEAR_PLANE = 1e-3

/** A rectangle on screen, CSS pixels from the top-left. */
export interface ScreenBox {
  minX: number
  maxX: number
  minY: number
  maxY: number
}

/**
 * For one view and face: the screen bounding box of the part in front of
 * the camera of a camera-relative rectangle [left, right] × [bottom, top]
 * on the face's plane z = distance (face coordinates), or null if none of
 * it is in front.
 */
export function screenBox(view: View, face: Face): (left: number, right: number, bottom: number, top: number, distance: number) => ScreenBox | null {
  const { right: r, up: u, forward } = faceBasis(face, view.orientation)
  const to = projector(view, face)
  const [cx, cy, f] = [view.width / 2, view.height / 2, view.focal]
  return (left, right, bottom, top, distance) => {
    // Depth along the camera's axis of the corners: linear in x and y.
    const zl = forward.x * left + forward.z * distance
    const zr = forward.x * right + forward.z * distance
    const zb = forward.y * bottom
    const zt = forward.y * top
    if (Math.min(zl, zr) + Math.min(zb, zt) > NEAR_PLANE) {
      // All in front (the common case): project the corners directly.
      let minX = Infinity
      let maxX = -Infinity
      let minY = Infinity
      let maxY = -Infinity
      for (let k = 0; k < 4; k++) {
        const x = k === 0 || k === 3 ? left : right
        const y = k < 2 ? bottom : top
        const z = (k === 0 || k === 3 ? zl : zr) + (k < 2 ? zb : zt)
        const sx = cx + ((r.x * x + r.y * y + r.z * distance) * f) / z
        const sy = cy - ((u.x * x + u.y * y + u.z * distance) * f) / z
        if (sx < minX) minX = sx
        if (sx > maxX) maxX = sx
        if (sy < minY) minY = sy
        if (sy > maxY) maxY = sy
      }
      return { minX, maxX, minY, maxY }
    }
    const corners = [[left, bottom], [right, bottom], [right, top], [left, top]].map(([x, y]) => ({ x, y, z: distance }))
    const points = clip(corners, p => dot(p, forward) - NEAR_PLANE).flatMap(p => to(p) ?? [])
    if (points.length === 0) return null
    const xs = points.map(p => p.x)
    const ys = points.map(p => p.y)
    return { minX: Math.min(...xs), maxX: Math.max(...xs), minY: Math.min(...ys), maxY: Math.max(...ys) }
  }
}

/** Whether a screen box overlaps the viewport. */
export function onScreen(view: View, box: ScreenBox | null): box is ScreenBox {
  return box !== null && box.minX < view.width && box.maxX > 0 && box.minY < view.height && box.maxY > 0
}
