/**
 * Cells of the coarser lattices. In a face's lattice coordinates (a, b, c)
 * (see Face in view.ts), the sites with a, b and c multiples of N and a / N,
 * b / N, c / N of one parity are N·FCC: the same lattice, N times coarser,
 * and so its own layers along every face's axis. A site belongs to the cell
 * of the nearest of them, the rhombic dodecahedron of N·FCC around it;
 * sites exactly between cells (on a face or a corner of it) go to the one
 * with the greatest c, then b, then a. N is a power of two, 2^level.
 *
 * Cells do not nest: a cell of 2N·FCC cuts cells of N·FCC (the twelve
 * around its face centres in half, the six at its corners in six), so
 * each level is used on its own. See docs/architecture.md, section 1.
 * bake/src/cells.rs restates this, with a table over a period of the lattice.
 */

/** N, the coarse lattice's spacing in layers, of cells of `level`. */
export function spacing(level: number): number {
  return 2 ** level
}

/**
 * The layers c − c0 of the sites a cell of `level` holds, c0 its own site's
 * layer: [0, 0] at level 0 (a site is its own cell), [−N, N − 1] from N = 4
 * on, [−2, 0] at N = 2 (where all of layer 1 is exactly between cells and
 * goes to the farther one). The near end is the cell's face across the
 * axis, whose sites it takes from the cell 2N nearer.
 */
export function ownedLayers(level: number): [number, number] {
  const n = spacing(level)
  return level === 0 ? [0, 0] : level === 1 ? [-2, 0] : [-n, n - 1]
}

/** Twice the squared distance between sites (a, b, c) apart: a and c are LAYER = 1/√2 per unit, b is 1. */
const distance2 = (a: number, b: number, c: number) => a * a + 2 * b * b + c * c

/** The site of N·FCC whose cell holds site (a, b, c) (owner in bake/src/cells.rs). */
export function owner(n: number, [a, b, c]: readonly [number, number, number]): [number, number, number] {
  let best: [number, number, number] = [0, 0, 0]
  let bestD = Infinity
  for (const parity of [0, 1]) {
    // Per coordinate, the two multiples of N of this parity around it.
    const around = (x: number) => {
      const t = Math.floor((x - parity * n) / (2 * n))
      return [n * (2 * t + parity), n * (2 * t + 2 + parity)]
    }
    for (const qa of around(a)) {
      for (const qb of around(b)) {
        for (const qc of around(c)) {
          const d = distance2(qa - a, qb - b, qc - c)
          const later = qc > best[2] || (qc === best[2] && (qb > best[1] || (qb === best[1] && qa > best[0])))
          if (d < bestD || (d === bestD && later)) {
            best = [qa, qb, qc]
            bestD = d
          }
        }
      }
    }
  }
  return best
}
