import { describe, it, expect } from 'vitest'
import { FACES, LAYER, faceTangents, inFace, onScreen, projector, screenBox, screenRay, toLocal, viewBasis, type Face, type View } from './view'
import type { Vec3 } from './world'

/** Seeded uniform samples in [lo, hi), so failures reproduce. */
function sampler(seed: number) {
  let state = seed
  return (lo: number, hi: number) => {
    state = (state * 1103515245 + 12345) % 2147483648
    return lo + ((hi - lo) * state) / 2147483648
  }
}

const dot = (a: Vec3, b: Vec3) => a.x * b.x + a.y * b.y + a.z * b.z
const cross = (a: Vec3, b: Vec3): Vec3 => ({ x: a.y * b.z - a.z * b.y, y: a.z * b.x - a.x * b.z, z: a.x * b.y - a.y * b.x })
const view = (yaw: number, pitch: number, focal = 400): View => ({ orientation: { yaw, pitch }, width: 800, height: 600, focal })
const face = (name: string) => FACES.find(f => f.name === name) as Face
// Not a face of the renderer: face coordinates are world coordinates.
const axes = { right: { x: 1, y: 0, z: 0 }, up: { x: 0, y: 1, z: 0 }, forward: { x: 0, y: 0, z: 1 } }
const identity: Face = { name: 'world', ...axes, lattice: axes }

describe('viewBasis', () => {
  it('is orthonormal, keeps right level, and turns right and up for positive yaw and pitch', () => {
    const random = sampler(1)
    for (let i = 0; i < 100; i++) {
      const yaw = random(-80, 80)
      const pitch = random(-80, 80)
      const { right, up, forward } = viewBasis({ yaw, pitch })
      for (const [a, b] of [[right, up], [up, forward], [forward, right]]) expect(dot(a, b)).toBeCloseTo(0)
      for (const v of [right, up, forward]) expect(dot(v, v)).toBeCloseTo(1)
      expect(right.y).toBe(0)
      expect(Math.sign(forward.x)).toBe(Math.sign(yaw))
      expect(Math.sign(forward.y)).toBe(Math.sign(pitch))
    }
  })
})

describe('FACES', () => {
  // A mirrored face would draw its world mirrored.
  it('are rotations, one looking at each nearest neighbour, with integer lattice axes', () => {
    for (const { right, up, forward, lattice } of FACES) {
      expect(dot(cross(right, up), forward)).toBeCloseTo(1)
      for (const [unit, int, scale] of [[right, lattice.right, LAYER], [up, lattice.up, 1], [forward, lattice.forward, LAYER]] as const) {
        for (const k of ['x', 'y', 'z'] as const) {
          expect(Number.isInteger(int[k])).toBe(true)
          expect(unit[k]).toBeCloseTo(int[k] * scale)
        }
      }
    }
    const forwards = FACES.map(({ lattice: { forward: { x, y, z } } }) => `${String(x)},${String(y)},${String(z)}`)
    expect(new Set(forwards).size).toBe(12)
  })

  // Each ray belongs to the face whose axis is nearest; inFace must agree.
  it('share out the rays: each is in the face of the nearest axis', () => {
    const random = sampler(4)
    for (let i = 0; i < 1000; i++) {
      const ray = { x: random(-1, 1), y: random(-1, 1), z: random(-1, 1) }
      const nearest = FACES.reduce((best, f) => (dot(ray, f.forward) > dot(ray, best.forward) ? f : best))
      const owners = FACES.filter(f => { const p = toLocal(f, ray); return inFace(p.x, p.y, p.z) })
      expect(owners).toEqual([nearest])
    }
  })
})

describe('projector', () => {
  it('inverts screenRay on every face', () => {
    const random = sampler(2)
    for (let i = 0; i < 100; i++) {
      const v = view(random(-180, 180), random(-90, 90))
      const f = FACES[Math.floor(random(0, 12))]
      const [sx, sy] = [random(0, 800), random(0, 600)]
      const ray = screenRay(v, f, sx, sy)
      const d = random(0.5, 100)
      const p = projector(v, f)({ x: ray.x * d, y: ray.y * d, z: ray.z * d })
      expect(p?.x).toBeCloseTo(sx)
      expect(p?.y).toBeCloseTo(sy)
    }
  })
})

describe('faceTangents', () => {
  it("is the screen within the face's rhombus, and nothing of faces behind", () => {
    // Looking along +x+z: the screen reaches past the face's rays at its sides.
    const t = faceTangents(view(45, 0), face('+x+z'))
    expect(t?.minX).toBeCloseTo(-1)
    expect(t?.maxX).toBeCloseTo(1)
    expect(t?.maxY).toBeCloseTo(LAYER)
    expect(faceTangents(view(45, 0), face('-x-z'))).toBeNull()
  })

  it('covers every ray on screen by the face it belongs to', () => {
    const random = sampler(3)
    for (let i = 0; i < 300; i++) {
      const v = view(random(-180, 180), random(-90, 90), random(150, 1000))
      const world = screenRay(v, identity, random(0, 800), random(0, 600))
      const f = FACES.find(candidate => { const p = toLocal(candidate, world); return inFace(p.x, p.y, p.z) }) as Face
      const ray = toLocal(f, world)
      const t = faceTangents(v, f)
      expect(t).not.toBeNull()
      if (!t) continue
      expect(ray.x / ray.z).toBeGreaterThanOrEqual(t.minX - 1e-9)
      expect(ray.x / ray.z).toBeLessThanOrEqual(t.maxX + 1e-9)
      expect(ray.y / ray.z).toBeGreaterThanOrEqual(t.minY - 1e-9)
      expect(ray.y / ray.z).toBeLessThanOrEqual(t.maxY + 1e-9)
    }
  })
})

describe('screenBox', () => {
  const visible = (v: View, left: number, right: number, bottom: number, top: number, distance: number) =>
    onScreen(v, screenBox(v, identity)(left, right, bottom, top, distance))

  it('shows a rectangle around the view direction, not one to the side or behind', () => {
    // The view direction meets z = 10 at about (1.76, -1.79).
    expect(visible(view(10, -10), 1, 2, -2, -1, 10)).toBe(true)
    expect(visible(view(10, -10), -30, -20, -2, -1, 10)).toBe(false)
    expect(visible(view(180, 0), 1, 2, -2, -1, 10)).toBe(false)
  })

  it('bounds the projected corners of a rectangle in front of the camera', () => {
    const random = sampler(5)
    let checked = 0
    for (let n = 0; n < 300; n++) {
      const v = view(random(-180, 180), random(-90, 90))
      const f = FACES[Math.floor(random(0, 12))]
      const [x, y, d] = [random(-20, 20), random(-20, 20), random(1, 30)]
      const corners = [[x, y], [x + 3, y], [x + 3, y + 2], [x, y + 2]].map(([cx, cy]) => projector(v, f)({ x: cx, y: cy, z: d }))
      if (corners.some(c => !c)) continue
      const box = screenBox(v, f)(x, x + 3, y, y + 2, d)
      const xs = corners.map(c => c?.x ?? 0)
      const ys = corners.map(c => c?.y ?? 0)
      expect(box?.minX).toBeCloseTo(Math.min(...xs), 6)
      expect(box?.maxX).toBeCloseTo(Math.max(...xs), 6)
      expect(box?.minY).toBeCloseTo(Math.min(...ys), 6)
      expect(box?.maxY).toBeCloseTo(Math.max(...ys), 6)
      checked++
    }
    expect(checked).toBeGreaterThan(50)
  })

  it('shows a rectangle reaching from behind the camera into view', () => {
    // Looking along +x; the rectangle on z = 10 spans x from -50 to 50.
    expect(visible(view(90, 0), -50, 50, -1, 1, 10)).toBe(true)
  })
})
