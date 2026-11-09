//! Cells of the coarser lattices (cells.ts restated): in a face's lattice
//! coordinates (a, b, c), the sites with a, b and c multiples of N and
//! a / N, b / N, c / N of one parity are N·FCC, again an FCC lattice. A site
//! belongs to the cell of the nearest of them: the rhombic dodecahedron of
//! N·FCC around it. Sites exactly between cells go to the one with the
//! greatest c, then b, then a.

/**
 * Twice the squared distance between two sites (a, b, c) apart: a and c
 * are LAYER = 1/√2 per unit, b is 1 (see Face in view.ts).
 */
fn distance2(a: i32, b: i32, c: i32) -> i64 {
    let (a, b, c) = (a as i64, b as i64, c as i64);
    a * a + 2 * b * b + c * c
}

/** The site of N·FCC whose cell holds site (a, b, c), found directly. */
pub fn owner(n: i32, site: [i32; 3]) -> [i32; 3] {
    let mut best = [0; 3];
    let mut best_d = i64::MAX;
    for parity in 0..2 {
        // Per coordinate, the two multiples of N of this parity around it.
        let around = |x: i32| {
            let t = (x - parity * n).div_euclid(2 * n);
            [n * (2 * t + parity), n * (2 * t + 2 + parity)]
        };
        let [xa, xb, xc] = site.map(around);
        for a in xa {
            for b in xb {
                for c in xc {
                    let d = distance2(a - site[0], b - site[1], c - site[2]);
                    if d < best_d || (d == best_d && (c, b, a) > (best[2], best[1], best[0])) {
                        best = [a, b, c];
                        best_d = d;
                    }
                }
            }
        }
    }
    best
}

/**
 * Which site of N·FCC owns each site, from a table over a period of the
 * lattice: N·FCC repeats every 2N along each of a, b and c, and so does the
 * owner relative to the site.
 */
pub struct Cells {
    n: i32,
    /** Per residue (a, b, c) mod 2N: the owner minus the site. */
    offsets: Vec<[i8; 3]>,
}

impl Cells {
    pub fn new(n: i32) -> Cells {
        let period = 2 * n;
        let mut offsets = vec![[0; 3]; (period * period * period) as usize];
        for a in 0..period {
            for b in 0..period {
                for c in 0..period {
                    // Only sites with a, b, c of one parity exist; the rest stay 0.
                    if (a ^ c) & 1 == 0 && (b ^ c) & 1 == 0 {
                        let o = owner(n, [a, b, c]);
                        offsets[((a * period + b) * period + c) as usize] = [o[0] - a, o[1] - b, o[2] - c].map(|v| v as i8);
                    }
                }
            }
        }
        Cells { n, offsets }
    }

    /** The site of N·FCC owning site (a, b, c). */
    #[inline]
    pub fn owner(&self, a: i32, b: i32, c: i32) -> [i32; 3] {
        let period = 2 * self.n;
        let index = (a.rem_euclid(period) * period + b.rem_euclid(period)) * period + c.rem_euclid(period);
        let [da, db, dc] = self.offsets[index as usize];
        [a + da as i32, b + db as i32, c + dc as i32]
    }

    /** c of the site of N·FCC owning site (a, b, c): a multiple of N. */
    #[inline]
    pub fn owner_c(&self, a: i32, b: i32, c: i32) -> i32 {
        self.owner(a, b, c)[2]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /** Sites of the face's lattice (a, b, c of one parity) in a box. */
    fn sites(from: i32, to: i32) -> Vec<[i32; 3]> {
        let mut out = Vec::new();
        for a in from..to {
            for b in from..to {
                for c in from..to {
                    if (a ^ c) & 1 == 0 && (b ^ c) & 1 == 0 {
                        out.push([a, b, c]);
                    }
                }
            }
        }
        out
    }

    #[test]
    fn owner_is_the_nearest_coarse_site_ties_to_the_greatest_c_then_b_then_a() {
        for n in [1, 2, 4] {
            let coarse: Vec<[i32; 3]> =
                sites(-4 * n - 9, 4 * n + 9).into_iter().filter(|q| q.iter().all(|v| v % n == 0 && (v / n - q[2] / n) & 1 == 0)).collect();
            for site in sites(-9, 9) {
                let d = |q: &[i32; 3]| distance2(q[0] - site[0], q[1] - site[1], q[2] - site[2]);
                let nearest = coarse.iter().map(d).min().unwrap();
                let last = coarse.iter().filter(|q| d(q) == nearest).max_by_key(|q| (q[2], q[1], q[0])).unwrap();
                assert_eq!(owner(n, site), *last, "n {n} site {site:?}");
            }
        }
    }

    #[test]
    fn a_cell_holds_n_cubed_sites_from_n_layers_before_its_own_to_at_most_n_minus_1_after() {
        for n in [2, 4, 8] {
            let mut count = 0;
            let (mut lo, mut hi) = (i32::MAX, i32::MIN);
            for site in sites(-2 * n, 2 * n) {
                if owner(n, site) == [0, 0, 0] {
                    count += 1;
                    lo = lo.min(site[2]);
                    hi = hi.max(site[2]);
                }
            }
            assert_eq!(count, n * n * n);
            // Sites exactly between layers go to the farther one: at N = 2 all of layer 1 does.
            assert_eq!((lo, hi), (-n, if n == 2 { 0 } else { n - 1 }));
        }
        // At N = 1 a cell is its site.
        assert!(sites(-3, 3).into_iter().all(|s| owner(1, s) == s));
    }

    #[test]
    fn the_table_agrees_with_the_direct_search() {
        for n in [1, 2, 4, 8] {
            let cells = Cells::new(n);
            for site in sites(-3 * n - 2, 3 * n + 2) {
                assert_eq!(cells.owner(site[0], site[1], site[2]), owner(n, site), "n {n} site {site:?}");
            }
        }
    }
}
