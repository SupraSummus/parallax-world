import { describe, it, expect } from 'vitest'
import { owner, ownedLayers, spacing } from './cells'

type Site = [number, number, number]

/** Sites of a face's lattice (a, b, c of one parity) in a cube. */
function sites(from: number, to: number): Site[] {
  const out: Site[] = []
  for (let a = from; a < to; a++) {
    for (let b = from; b < to; b++) {
      for (let c = from; c < to; c++) if (((a ^ c) & 1) === 0 && ((b ^ c) & 1) === 0) out.push([a, b, c])
    }
  }
  return out
}

/** The sites of the cell of N·FCC around the origin. */
const cellAtOrigin = (n: number) => sites(-2 * n, 2 * n).filter(s => owner(n, s).every(v => v === 0))

describe('cells of N·FCC', () => {
  it.each([0, 1, 2, 3])('a cell of level %s holds N³ sites in the layers ownedLayers gives', level => {
    const n = spacing(level)
    const cell = cellAtOrigin(n)
    expect(cell.length).toBe(n ** 3)
    const layers = cell.map(s => s[2])
    expect([Math.min(...layers), Math.max(...layers)]).toEqual(ownedLayers(level))
  })

  it('gives a site to a nearest coarse site, ties to the one with the greatest c, b, a', () => {
    const d2 = ([a, b, c]: Site, [p, q, r]: Site) => (a - p) ** 2 + 2 * (b - q) ** 2 + (c - r) ** 2
    const coarse = (n: number) => sites(-5 * n, 5 * n).filter(s => s.every(v => v % n === 0) && s.every(v => ((v / n - s[2] / n) & 1) === 0))
    for (const n of [2, 4]) {
      const all = coarse(n)
      for (const site of sites(-n - 1, n + 1)) {
        const o = owner(n, site)
        const nearest = Math.min(...all.map(q => d2(site, q)))
        expect(d2(site, o)).toBe(nearest)
        const tied = all.filter(q => d2(site, q) === nearest).sort((p, q) => p[2] - q[2] || p[1] - q[1] || p[0] - q[0])
        expect(o).toEqual(tied[tied.length - 1])
      }
    }
  })

  it('makes cells as round as rhombic dodecahedra: no direction preferred', () => {
    // Second moments about the cell's centroid: equal along the world axes, none between them.
    const cell = cellAtOrigin(8)
    const mean = [0, 1, 2].map(k => cell.reduce((s, p) => s + p[k], 0) / cell.length)
    // Face coordinates are a · LAYER, b, c · LAYER; the world axes are b and (a ± c) / 2 in them.
    const world = cell.map(([a, b, c]) => [(a - mean[0] + c - mean[2]) / 2, b - mean[1], (c - mean[2] - a + mean[0]) / 2])
    const moment = (i: number, j: number) => world.reduce((s, p) => s + p[i] * p[j], 0) / world.length
    const diagonal = [moment(0, 0), moment(1, 1), moment(2, 2)]
    for (const m of diagonal) expect(m / diagonal[0]).toBeCloseTo(1, 1)
    for (const [i, j] of [[0, 1], [0, 2], [1, 2]]) expect(Math.abs(moment(i, j)) / diagonal[0]).toBeLessThan(0.05)
  })

  it('does not nest: cells of 2N·FCC cut cells of N·FCC', () => {
    // The site (2, 2, 2) of 2·FCC is halfway between (0, 0, 0) and (4, 4, 4) of 4·FCC, on their cells' common face.
    const cell = sites(-8, 8).filter(s => owner(2, s).join() === '2,2,2')
    expect(cell.length).toBe(8)
    expect(new Set(cell.map(s => owner(4, s).join()))).toEqual(new Set(['0,0,0', '4,4,4']))
  })
})
