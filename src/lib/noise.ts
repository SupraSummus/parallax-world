// Deterministic noise and randomness for the city (city.ts).

/** Integer hash of a lattice point to [0, 1). */
export function hash(seed: number, x: number, y: number, z = 0): number {
  let h = (seed | 0) + Math.imul(x | 0, 374761393) + Math.imul(y | 0, 668265263) + Math.imul(z | 0, 1440662683)
  h = Math.imul(h ^ (h >>> 13), 1274126177)
  h ^= h >>> 16
  return (h >>> 0) / 4294967296
}

/** Seeded PRNG (mulberry32) returning [0, 1). */
export function rng(seed: number): () => number {
  let a = seed | 0
  return () => {
    a = (a + 0x6d2b79f5) | 0
    let t = Math.imul(a ^ (a >>> 15), 1 | a)
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296
  }
}
