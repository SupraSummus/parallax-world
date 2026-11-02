import { describe, it, expect } from 'vitest'
import { ball, box, generate, type Box, type Child, type Field, type Generator, type Population } from './generator'
import { forest } from './forest'
import { rng } from './noise'
import type { VoxelGrid } from './voxel-grid'

const KINDS = {
  fog: { type: 8, colors: ['#cccccc'] },
  rock: { type: 3, colors: ['#777777'] },
  wall: { type: 10, colors: ['#eeeeee'] },
  wood: { type: 4, colors: ['#664422'] },
  grass: { type: 1, colors: ['#44aa44'] },
}
type Kind = keyof typeof KINDS

/** One population of one thing, near the origin: cells reach 1024 + `reach` from their centres. */
const single = (make: Population<Kind>['make'], reach = 64): Population<Kind> => ({
  name: 'single', order: 1, cell: 1024, reach, size: Infinity,
  anchor: ({ i, j, k }) => (i === 0 && j === 0 && k === 0 ? [64, 64, 64] : undefined),
  make,
})

/** A ball of fog, each site kept with chance 0.3: what rays get through depends on the chance at coarse levels. */
const fog: Generator<Kind> = { kinds: KINDS, seed: 1, layers: [single(() => ({ shapes: [ball([64, 64, 64], 56, 'fog', { fill: 0.3 })] }))] }

/** Rock with winding tunnels carved out of it. */
const ROCK: Field<Kind> = { name: 'rock', order: 0, axis: 1, spans: () => [{ from: -1000, to: 1000, kind: 'rock' }] }
const caves: Generator<Kind> = {
  kinds: KINDS,
  seed: 2,
  layers: [ROCK, {
    name: 'tunnels', order: 1, op: 'carve',
    at: (x, y, z) => (Math.sin(x / 23) + Math.sin(y / 17) + Math.sin(z / 29) + Math.sin((x + z) / 31) > 1.4 ? 'rock' : undefined),
  }],
}

/** Houses on a grass plain: thin walls with a door carved through, rooms of tables inside. */
const GROUND: Field<Kind> = { name: 'ground', order: 0, axis: 1, spans: () => [{ from: -64, to: 0, kind: 'grass' }] }
const village: Generator<Kind> = {
  kinds: KINDS,
  seed: 3,
  layers: [GROUND, {
    name: 'houses', order: 1, cell: 64, reach: 0, size: Infinity,
    // On the plane y = 0: the cells centred on it, every other one of a square grid.
    anchor: ({ j, centre }) => (j === 0 ? centre : undefined),
    make: ([x, , z], cell) => {
      const [w, d, h] = [16 + Math.floor(cell.random('size')() * 12), 16 + Math.floor(cell.random('size')() * 12), 20]
      const [x0, z0, x1, z1] = [x - w, z - d, x + w, z + d]
      const tables = (clip: Box): Child<Kind>[] => Array.from({ length: 4 }, (_, n): Child<Kind> => {
        const [tx, tz] = [x0 + 6 + n * 6, z0 + 6]
        return { name: `table ${String(n)}`, size: 2, shapes: [box([tx, 0, tz], [tx + 4, 3, tz + 3], 'wood')] }
      }).filter(t => t.shapes[0].bounds.minX < clip.maxX)
      return {
        bounds: { minX: x0, minY: 0, minZ: z0, maxX: x1, maxY: h + 1, maxZ: z1 },
        shapes: [
          box([x0, 0, z0], [x1, h, z0 + 1], 'wall'), box([x0, 0, z1 - 1], [x1, h, z1], 'wall'),
          box([x0, 0, z0], [x0 + 1, h, z1], 'wall'), box([x1 - 1, 0, z0], [x1, h, z1], 'wall'),
          box([x0, h, z0], [x1, h + 1, z1], 'wall'),
          box([x - 3, 0, z0], [x + 3, 10, z0 + 1], 'wall', { op: 'carve' }),
        ],
        children: tables,
      }
    },
  }],
}

/** Every generator, with a region with a bit of everything in it, and the level up to which it keeps what a camera sees. */
const GENERATORS: { name: string; generator: Generator<string>; region: Box; levels: number }[] = [
  // Near the spawn point: path, trees, undergrowth, from under the ground to above the crowns.
  // From level 3 its trees are a medium; at level 6 too dense yet (docs/generators.md).
  { name: 'forest', generator: forest(42).generator, region: { minX: -64, minY: -32, minZ: -192, maxX: 64, maxY: 192, maxZ: -64 }, levels: 5 },
  { name: 'fog', generator: fog, region: { minX: 32, minY: 32, minZ: 32, maxX: 96, maxY: 96, maxZ: 96 }, levels: 4 },
  { name: 'caves', generator: caves, region: { minX: 0, minY: 0, minZ: 0, maxX: 128, maxY: 128, maxZ: 128 }, levels: 3 },
  { name: 'village', generator: village, region: { minX: -64, minY: -16, minZ: -64, maxX: 64, maxY: 48, maxZ: 64 }, levels: 5 },
]

/** The box of `level`'s sites covering `region` and a margin of two sites. */
const boxAt = (region: Box, level: number): Box => {
  const s = 2 ** level
  return {
    minX: Math.floor(region.minX / s) - 2, minY: Math.floor(region.minY / s) - 2, minZ: Math.floor(region.minZ / s) - 2,
    maxX: Math.ceil(region.maxX / s) + 2, maxY: Math.ceil(region.maxY / s) + 2, maxZ: Math.ceil(region.maxZ / s) + 2,
  }
}

/** Whether `part` holds the same voxels as `whole` over the part's box. */
function sameOver(part: VoxelGrid, whole: VoxelGrid, box: Box): boolean {
  for (let z = box.minZ; z < box.maxZ; z++) {
    for (let y = box.minY; y < box.maxY; y++) {
      for (let x = box.minX + ((box.minX + y + z) & 1); x < box.maxX; x += 2) {
        if (part.get(x, y, z) !== whole.get(x, y, z)) return false
      }
    }
  }
  return true
}

/** The site of the lattice nearest to a point: its voxel is the one the point is in. */
function nearestSite(p: readonly number[]): number[] {
  const r = p.map(Math.round)
  if ((r[0] + r[1] + r[2]) & 1) {
    // Off by one in the coordinate rounded the furthest.
    const errors = p.map((v, i) => Math.abs(v - r[i]))
    const k = errors.indexOf(Math.max(...errors))
    r[k] += p[k] > r[k] ? 1 : -1
  }
  return r
}

interface Ray { origin: number[]; direction: number[] }

interface Hit { t: number; type: number }

/** How far each ray goes before entering a voxel of `grid` (at `level`), and its material type; or till it leaves `region`, type 0. */
function firstHits(grid: VoxelGrid, level: number, rays: readonly Ray[], region: Box): Hit[] {
  const s = 2 ** level
  const inside = (p: number[]) => p[0] >= region.minX && p[0] < region.maxX && p[1] >= region.minY && p[1] < region.maxY && p[2] >= region.minZ && p[2] < region.maxZ
  return rays.map(({ origin, direction }) => {
    for (let t = 0; ; t += 0.1 * Math.min(s, 4)) {
      const p = origin.map((o, i) => o + t * direction[i])
      if (!inside(p)) return { t, type: 0 }
      const [x, y, z] = nearestSite(p.map(v => v / s))
      const id = grid.get(x, y, z)
      if (id !== 0) return { t, type: grid.materials[id].type }
    }
  })
}

/** Rays from random points of `region`, in random directions, seeded so failures reproduce. */
function randomRays(region: Box, count: number): Ray[] {
  const random = rng(11)
  return Array.from({ length: count }, () => {
    const origin = [region.minX + random() * (region.maxX - region.minX), region.minY + random() * (region.maxY - region.minY), region.minZ + random() * (region.maxZ - region.minZ)]
    const z = 2 * random() - 1
    const a = 2 * Math.PI * random()
    return { origin, direction: [Math.sqrt(1 - z * z) * Math.cos(a), Math.sqrt(1 - z * z) * Math.sin(a), z] }
  })
}

const even = (n: number) => 2 * Math.round(n / 2)
/**
 * How far a level may stray from the one two finer, in the share of rays
 * through and of materials hit. Surfaces a site rougher and sheets a site
 * thicker cost up to ~0.06; losing content, or a wrong fill law, 0.14 and more.
 */
const TOLERANCE = 0.1

describe.each(GENERATORS)('generator: $name', ({ generator, region, levels }) => {
  // Up to one level past the checked ones: the forest's trees as a medium.
  it.each([0, 2, levels + 1])('gives each box at level %i, generated alone and in any order, the voxels of the whole', level => {
    const whole = boxAt(region, level)
    const grid = generate(generator, whole, level)
    expect(grid.cells.some(id => id !== 0)).toBe(true)
    // Eight parts, split at random points (seeded, so failures reproduce).
    const random = rng(7)
    const split = (lo: number, hi: number, align: (n: number) => number) => align(lo + (hi - lo) * (0.2 + 0.6 * random()))
    const [x, y, z] = [split(whole.minX, whole.maxX, even), split(whole.minY, whole.maxY, Math.round), split(whole.minZ, whole.maxZ, Math.round)]
    const parts: Box[] = []
    for (const [minX, maxX] of [[whole.minX, x], [x, whole.maxX]]) {
      for (const [minY, maxY] of [[whole.minY, y], [y, whole.maxY]]) {
        for (const [minZ, maxZ] of [[whole.minZ, z], [z, whole.maxZ]]) parts.push({ minX, minY, minZ, maxX, maxY, maxZ })
      }
    }
    for (const part of parts.reverse()) {
      expect(sameOver(generate(generator, part, level), grid, part), JSON.stringify(part)).toBe(true)
    }
  })

  it.each(Array.from({ length: levels }, (_, i) => i + 1))('shows at level %i what it shows two levels finer: as many rays get through, and stop on the same materials', level => {
    const [s, finer] = [2 ** level, Math.max(0, level - 2)]
    // At least 16 sites of the level across, around the region's centre.
    const grow = (lo: number, hi: number) => Math.max(0, 8 * s - (hi - lo) / 2)
    const r = region
    const area = { minX: r.minX - grow(r.minX, r.maxX), minY: r.minY - grow(r.minY, r.maxY), minZ: r.minZ - grow(r.minZ, r.maxZ), maxX: r.maxX + grow(r.minX, r.maxX), maxY: r.maxY + grow(r.minY, r.maxY), maxZ: r.maxZ + grow(r.minZ, r.maxZ) }
    const reference = generate(generator, boxAt(area, finer), finer)
    // From where a camera could be: empty sites of the finer level.
    const rays = randomRays(area, 4000).filter(({ origin }) => reference.get(...(nearestSite(origin.map(v => v / 2 ** finer)) as [number, number, number])) === 0).slice(0, 2000)
    const fine = firstHits(reference, finer, rays, area)
    const coarse = firstHits(generate(generator, boxAt(area, level), level), level, rays, area)
    // A level is seen from afar: of the rays that got 2 of its sites far, how many get 4 more.
    const [from, d] = [2 * s, 4 * s]
    // Rays leaving the area before from + d tell nothing.
    const through = (hits: Hit[]) => {
      const seen = hits.filter(({ t, type }) => t > from && !(type === 0 && t <= from + d))
      return seen.filter(({ t }) => t > from + d).length / seen.length
    }
    // The materials they stop on.
    const types = (hits: Hit[]) => {
      const far = hits.filter(({ t }) => t > from)
      const counts = new Map<number, number>()
      for (const { type } of far) counts.set(type, (counts.get(type) ?? 0) + 1 / far.length)
      return counts
    }
    const [a, b] = [types(fine), types(coarse)]
    const tv = [...new Set([...a.keys(), ...b.keys()])].reduce((sum, t) => sum + Math.abs((a.get(t) ?? 0) - (b.get(t) ?? 0)), 0) / 2
    expect(Math.abs(through(coarse) - through(fine))).toBeLessThan(TOLERANCE)
    expect(tv).toBeLessThan(TOLERANCE)
  })
})

describe('generate', () => {
  const world = (layer: Population<Kind>): Generator<Kind> => ({ kinds: KINDS, seed: 1, layers: [layer] })
  const near = { minX: 0, minY: 0, minZ: 0, maxX: 128, maxY: 128, maxZ: 128 }

  it('refuses a thing reaching past its population\'s reach', () => {
    const wide = world(single(() => ({ shapes: [ball([64, 64, 64], 1200, 'rock')] })))
    expect(() => generate(wide, near)).toThrow(/reaches past its population's reach.*Populations/)
  })

  it('refuses a child reaching past its parent, or two children of one name', () => {
    const parent = (children: Child<Kind>[]) => world(single(() => ({ bounds: { minX: 60, minY: 60, minZ: 60, maxX: 70, maxY: 70, maxZ: 70 }, shapes: [], children: () => children })))
    const child = (name: string, radius: number): Child<Kind> => ({ name, size: Infinity, shapes: [ball([64, 64, 64], radius, 'rock')] })
    expect(() => generate(parent([child('a', 20)]), near)).toThrow(/reaches past its parent.*Things/)
    expect(() => generate(parent([child('a', 2), child('a', 2)]), near)).toThrow(/two children named a.*Things/)
  })
})
