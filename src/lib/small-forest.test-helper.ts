/** Tests' stand-in for forest.ts: generated at every level like it, in grids small enough to take moments. */
import type { Generator } from './generator'
import type { Vec3 } from './world'

/** Gentle hills of moss, held 64 sites of each level around the camera. */
export function smallForest(seed: number): { generator: Generator<'moss'>; spawn: Vec3; reach: number; heights: (level: number) => { minY: number; maxY: number } } {
  return {
    generator: {
      kinds: { moss: { type: 1, colors: ['#4f7a2a', '#5a8a30'] } },
      seed,
      layers: [{ name: 'ground', order: 0, axis: 1, spans: (x, z) => [{ from: -8, to: 1 + Math.round(8 * Math.sin(x / 40) * Math.sin(z / 40)), kind: 'moss' }] }],
    },
    spawn: { x: 0, y: 6, z: -4 },
    reach: 64,
    heights: level => ({ minY: Math.floor(-8 / 2 ** level), maxY: Math.ceil(10 / 2 ** level) }),
  }
}
