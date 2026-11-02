import { describe, it, expect } from 'vitest'
import { selectSlices, leakFreeDetail, leakAngle, matchTilings, sliceCentre, thickness, sliceKey, chunkCell, chunkCells, holds, atomSpacing, type SliceId } from './slice-selection'
import { LAYER } from './view'

const name = ({ level, layer, cutoff }: SliceId) => `${String(level)}:${String(layer)}${Number.isFinite(cutoff) ? `|${String(cutoff)}` : ''}`

/** How many of `slices` hold each site (a, b, c) with a and b in a small window and c in [from, to). */
function holders(slices: SliceId[], from: number, to: number): Map<string, number> {
  const count = new Map<string, number>()
  for (let c = from; c < to; c++) {
    for (let a = -6 + (c & 1); a <= 6; a += 2) {
      for (let b = -6 + (c & 1); b <= 6; b += 2) {
        count.set(`${String(a)},${String(b)},${String(c)}`, slices.filter(s => holds(s, [a, b, c])).length)
      }
    }
  }
  return count
}

describe('selectSlices', () => {
  // docs/architecture.md, section 1 (detail 1): level:layer, |cutoff.
  it('matches the documented slicing for camera z=0', () => {
    expect(selectSlices(0, 1, 2000, 1).map(name).join(' ')).toBe(
      '0:1 0:2 0:3 1:2 1:3 2:2 2:3 3:2 3:3 4:2 4:3 5:2 5:3 6:2 6:3|256 6:4|256 7:2 7:3|512 7:4|512 8:2 8:3|1024 8:4|1024 9:2 9:3')
  })

  it('returns nothing for an empty range', () => {
    expect(selectSlices(0, 200, 100, 4)).toEqual([])
    expect(selectSlices(0.5, 10, 10, 4)).toEqual([])
  })

  it('makes slices thinner as detail increases', () => {
    const counts = [1, 2, 4, 8, 16].map(detail => selectSlices(-30, 1, 400, detail).length)
    expect(counts).toEqual([...counts].sort((a, b) => a - b))
    expect(new Set(counts).size).toBe(counts.length)
  })

  describe.each([1, 2, 4, 8, 16])('invariants with detail %s', detail => {
    const cameras = Array.from({ length: 21 }, (_, i) => -100 + i * 9.37)

    it.each(cameras)('camera z=%s', cameraZ => {
      const slices = selectSlices(cameraZ, 1, 120, detail)
      const near = Math.ceil(cameraZ + 1)

      // Every site from the first layer ahead to the far end is in exactly
      // one slice; none nearer (a slice close to the camera would be blown
      // up over the screen).
      const held = holders(slices, near - 3, Math.floor(cameraZ + 120))
      for (const [site, count] of held) {
        const c = Number(site.split(',')[2])
        expect(count, site).toBe(c < near ? 0 : 1)
      }

      for (let i = 1; i < slices.length; i++) {
        // Near to far: level by level, layer by layer. (Where a level ends,
        // its last slices hold sites of cells one level up before the next
        // level's first layer, so they come first, though their centre
        // planes may be a little farther.)
        const [p, q] = [slices[i - 1], slices[i]]
        expect(q.level > p.level || (q.level === p.level && q.layer === p.layer + 1), `${name(p)} ${name(q)}`).toBe(true)
        expect(sliceCentre(q)).toBeGreaterThan(sliceCentre(p) - 2 * LAYER)
      }

      for (const s of slices) {
        // Past level 0, never thicker than (distance to the near side) / detail.
        const a = s.layer * 2 ** s.level - atomSpacing(s.level) - cameraZ
        if (s.level > 0) expect(thickness(s.level)).toBeLessThanOrEqual(a / detail)
      }

      // Logarithmic, not linear, in the view distance.
      expect(slices.length).toBeLessThanOrEqual(12 * detail + 30)
    })
  })

  it('uses thicker slabs farther from the camera', () => {
    for (const cameraZ of [-100, -67, -30, 0, 17]) {
      const slices = selectSlices(cameraZ, 1, 200, 1)
      expect(slices[0].level).toBe(0)
      expect(slices.map(s => s.level)).toEqual(slices.map(s => s.level).sort((a, b) => a - b))
      expect(slices[slices.length - 1].level).toBeGreaterThanOrEqual(4)
    }
  })

  it('names the same slice the same from nearby cameras', () => {
    // Moving one layer forward keeps all but the slices near where levels change.
    const before = new Set(selectSlices(0, 1, 400, 16).map(sliceKey))
    const after = selectSlices(1, 1, 400, 16).map(sliceKey)
    expect(after.filter(k => before.has(k)).length).toBeGreaterThan(after.length * 0.9)
  })
})

// That the bound holds is leak-bound.test.ts; these pin the formula's two cases.
describe('leakFreeDetail', () => {
  it('is (r + √(r² + 8r / ε)) / 2, r = 64/65', () => {
    // 16 px at focal 640 (1280 px wide at 90°): ε = 1/40.
    // (r + √(r² + 8r · 40)) / 2 = 9.4, rounded up to a quarter octave.
    expect(leakFreeDetail(16 / 640)).toBe(2 ** 3.25)
  })

  it('is at least 3r, so that voxels are 2N layers ahead of their eye', () => {
    // r = 64/65: (r + √(r² + 8r)) / 2 = 1.98; 3r = 2.95, rounded up.
    expect(leakFreeDetail(1)).toBe(2 ** 1.75)
  })

  it.each([0.001, 0.025, 0.1, 1])('leaks at most ε = %s (leakAngle)', epsilon => {
    expect(leakAngle(leakFreeDetail(epsilon))).toBeLessThanOrEqual(epsilon)
  })
})

describe('matchTilings', () => {
  const s = (level: number, layer: number, cutoff = Infinity): SliceId => ({ level, layer, cutoff })

  it('pairs slices in both with themselves, and the ones between with what is between in the other', () => {
    const previous = [s(0, 1), s(0, 2, 6), s(0, 3, 6), s(1, 3), s(1, 4)]
    const wanted = [s(0, 1), s(0, 2), s(0, 3, 8), s(0, 4, 8), s(1, 4)]
    const segments = matchTilings(wanted, previous)
    expect(segments.map(g => [g.wanted.map(name), g.previous.map(name)])).toEqual([
      [['0:1'], ['0:1']],
      [['0:2', '0:3|8', '0:4|8'], ['0:2|6', '0:3|6', '1:3']],
      [['1:4'], ['1:4']],
    ])
  })

  it('lets the first segment stand on nearer slices, and pairs nothing past the last shared one', () => {
    const segments = matchTilings([s(0, 2), s(0, 3), s(1, 2)], [s(0, 1), s(0, 2, 6), s(0, 3)])
    expect(segments.map(g => [g.wanted.map(name), g.previous.map(name)])).toEqual([
      [['0:2'], ['0:1', '0:2|6']],
      [['0:3'], ['0:3']],
      [['1:2'], []],
    ])
  })

  it('covers every wanted slice once, in order, for any two slicings', () => {
    for (let cameraZ = -40; cameraZ < 40; cameraZ += 3) {
      for (const previousZ of [cameraZ - 5, cameraZ, cameraZ + 7]) {
        const wanted = selectSlices(cameraZ, 1, 300, 2)
        const previous = selectSlices(previousZ, 1, 300, 2)
        expect(matchTilings(wanted, previous).flatMap(g => g.wanted)).toEqual(wanted)
      }
    }
  })
})

describe('sliceCentre', () => {
  it('is halfway through what the voxels of the slice span', () => {
    // Level 3, slabs of 8 voxel layers: sites in layers 40 to 47, voxels reaching from 39 to 48.
    expect(sliceCentre({ level: 3, layer: 5, cutoff: Infinity })).toBeCloseTo(43.5 * LAYER)
    // Level 5, sites in layers 32 to 63, voxels reaching from 31 to 64.
    expect(sliceCentre({ level: 5, layer: 1, cutoff: Infinity })).toBeCloseTo(47.5 * LAYER)
    // Level 7, slabs of 64 layers of voxels of 2·FCC: their sites in layers 128 to 254, reaching from 126 to 256.
    expect(sliceCentre({ level: 7, layer: 1, cutoff: Infinity })).toBeCloseTo(191 * LAYER)
    // Level 8, voxels of 4·FCC in layers 256 to 508, reaching from 252 to 512.
    expect(sliceCentre({ level: 8, layer: 1, cutoff: Infinity })).toBeCloseTo(382 * LAYER)
  })
})

describe('chunkCell', () => {
  it('tiles the plane with bricks, each around one site of M·FCC seen along the axis', () => {
    for (const level of [0, 2]) {
      const m = 8 * 2 ** level
      // Sites of M·FCC seen along the axis: a and b multiples of M, a / M and b / M of one parity.
      for (let a = -4 * m; a <= 4 * m; a += m) {
        for (let b = -4 * m; b <= 4 * m; b += m) {
          if ((((a - b) / m) & 1) !== 0) continue
          const x = a * LAYER
          const inside = chunkCells(level, x, x + 1e-9, b, b + 1e-9).map(([i, j]) => chunkCell(level, i, j))
          expect(inside).toHaveLength(1)
          // In the middle of its brick.
          expect(inside[0].x + inside[0].width / 2).toBeCloseTo(x)
          expect(inside[0].y + inside[0].height / 2).toBeCloseTo(b)
        }
      }
      // Points of the plane are each in exactly one brick.
      for (let k = 0; k < 200; k++) {
        const [x, y] = [Math.sin(k * 12.9898) * 300, Math.cos(k * 78.233) * 300]
        const near = chunkCells(level, x - 50, x + 50, y - 50, y + 50).map(([i, j]) => chunkCell(level, i, j))
        expect(near.filter(r => x >= r.x && x < r.x + r.width && y >= r.y && y < r.y + r.height)).toHaveLength(1)
      }
    }
  })
})
