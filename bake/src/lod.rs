//! Native's chunks: no slices. A chunk is
//! a whole cell of 2^level·FCC (cells.rs), seen along one of the 12 faces'
//! axes in relief over the plane through its site, drawn as the voxels of
//! grid max(0, level − shift): at most M = 2^shift of them across, as
//! slices' atom shift. It depends on nothing else, the camera least of all.
//!
//! The camera only picks the chunks a face composes, and by its depth along
//! the face's axis alone, in whole layers. Each level claims the cells whose
//! sites are at most 2Q · 2^L layers ahead of the camera's layer, counted
//! from a layer of its own sites (`Lod::reach`), and a site is drawn at the finest
//! level claiming it: a level draws the cells it claims that are not all
//! within the finer one's claims. A site claimed by level L − 1 is by level
//! L (Q ≥ 3), so each level's claims hold the finer ones'. Where a cell
//! overlaps finer cells, both draw what is there, and the finer is painted
//! first: in front.
//!
//! Painted level by level, finest first, and within a level by their sites'
//! distance from the eye, near to far, chunks need no clipping where they
//! meet, for any ray of the face from anywhere. A face draws the rays nearer
//! its axis F than any other of the 12 directions, and a ray crossing from a
//! cell into its neighbour across n (one of the 12, scaled) has v · n > 0;
//! were n · F < 0, n would be m − F or −F for one of them, m, and
//! v · n = v · m − v · F ≤ 0. So along a ray of the face, a level's cells
//! come layer after layer, and once it leaves a level's claims it is never
//! back: levels never decrease along it. And of two cells of one level a ray
//! crosses (pieces of them even), the first has its site nearer the eye:
//! |x − a|² − |x − b|² is linear along the ray.

use crate::slicing::{GRID_LEVELS, LAYER};

/** Layers ahead of the camera's layer that the nearest voxels drawn are in: nearer, one would fill the screen. */
const NEAR: i32 = 4;

/** A chunk: the cell of 2^level·FCC around `site`, drawn as the voxels of grid `atoms`, in whose units, in a face's lattice coordinates, the site is. */
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct CellChunk {
    pub level: i32,
    pub atoms: i32,
    pub site: [i32; 3],
}

impl CellChunk {
    /** M: the spacing of its level's sites in its grid's units, its cell 2M layers deep. */
    pub fn spacing(&self) -> i32 {
        1 << (self.level - self.atoms)
    }

    /** The sites it may hold, a box of (a, b, c) in its grid's units, inclusive: its cell's. */
    pub fn bounds(&self) -> ([i32; 3], [i32; 3]) {
        let m = self.spacing();
        (self.site.map(|v| v - m), self.site.map(|v| v + m))
    }
}

/** How the cells' levels follow depth: grids `shift` levels finer than their cells, level L claiming cells up to 2Q · 2^L layers ahead. */
pub struct Lod {
    pub shift: i32,
    pub q: i32,
}

impl Lod {
    /**
     * For `detail` (viewing distance over thickness, as slices have it):
     * a cell 2N layers deep is drawn from Q · N layers ahead on, Q at
     * least 2 · detail; and at least 4, so that no cell but single voxels
     * reaches the near plane.
     */
    pub fn new(shift: i32, detail: f64) -> Lod {
        Lod { shift, q: ((2.0 * detail).ceil() as i32).max(4) }
    }

    /** The coarsest level: cells of the coarsest grid. */
    fn top(&self) -> i32 {
        GRID_LEVELS as i32 - 1 + self.shift
    }

    /**
     * The last layer whose cells `level` claims, for a camera in layer
     * `camera`: 2Q · N layers past the last of the level's layers of
     * sites not past the camera's; none for the coarsest, which claims all.
     */
    fn reach(&self, level: i32, camera: i32) -> i64 {
        let n = 1i64 << level;
        if level == self.top() { i64::MAX } else { (camera as i64).div_euclid(n) * n + 2 * self.q as i64 * n }
    }

    /**
     * The chunks a face composes: the cells each level draws, from the
     * eye's layer to `far` (world units) ahead
     * along the axis, whose rays have x / z and y / z within `tangents`
     * (min x, max x, min y, max y; the face's coordinates, see view.ts),
     * each with its site's distance from the eye: painted by level, then by
     * that. `eye` is in the face's lattice coordinates, in units of grid 0.
     */
    pub fn select(&self, eye: [f64; 3], far: f64, tangents: [f64; 4], mut visit: impl FnMut(CellChunk, f64)) {
        let [ea, eb, ec] = eye;
        // Any whole layer near the eye would do: the painting order holds for any.
        let camera = ec.floor() as i32;
        let last = camera as i64 + (far / LAYER).ceil() as i64;
        for level in 0..=self.top() {
            let n = 1i32 << level;
            let nf = n as f64;
            let atoms = (level - self.shift).max(0);
            let g = 1 << atoms;
            // Claimed, and not all within the finer claims: its sites reach N layers past its own.
            let first = if level == 0 { camera as i64 + NEAR as i64 } else { self.reach(level - 1, camera) - n as i64 + 1 };
            let to = self.reach(level, camera).min(last + n as i64);
            if first > last {
                break;
            }
            for k in first.div_euclid(n as i64) + i64::from(first.rem_euclid(n as i64) != 0)..=to.div_euclid(n as i64) {
                let (k, c) = (k as i32, k as i32 * n);
                // The cell's depths ahead of the eye, along the axis.
                let (z0, z1) = ((((c - n) as f64 - ec) * LAYER).max(0.0), ((c + n) as f64 - ec) * LAYER);
                if z1 <= 0.0 {
                    continue;
                }
                let span = |lo: f64, hi: f64| (lo.min(lo * z0 / z1) * z1, hi.max(hi * z0 / z1) * z1);
                let (x0, x1) = span(tangents[0], tangents[1]);
                let (y0, y1) = span(tangents[2], tangents[3]);
                let (a0, a1) = (((x0 / LAYER + ea) / nf - 1.0).floor() as i32, ((x1 / LAYER + ea) / nf + 1.0).ceil() as i32);
                let (b0, b1) = (((y0 + eb) / nf - 1.0).floor() as i32, ((y1 + eb) / nf + 1.0).ceil() as i32);
                for j in b0..=b1 {
                    // Sites of N·FCC: a / N, b / N, c / N of one parity.
                    if (j ^ k) & 1 != 0 {
                        continue;
                    }
                    for i in (a0 + ((a0 ^ k) & 1)..=a1).step_by(2) {
                        let site = [i * n, j * n, c];
                        let [a, b, c] = std::array::from_fn(|d| site[d] as f64 - eye[d]);
                        let distance = ((a * a + c * c) / 2.0 + b * b).sqrt();
                        visit(CellChunk { level, atoms, site: site.map(|v| v / g) }, distance);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cells::owner;

    /** The chunks a face composes from `eye`, by level and site in units of grid 0, and their painting order: (level, distance). */
    fn composed(lod: &Lod, eye: [f64; 3], far: f64) -> std::collections::HashMap<(i32, [i32; 3]), (i32, f64)> {
        let rhombus = [-1.0, 1.0, -std::f64::consts::FRAC_1_SQRT_2, std::f64::consts::FRAC_1_SQRT_2];
        let mut chunks = std::collections::HashMap::new();
        lod.select(eye, far, rhombus, |chunk, d| {
            let key = (chunk.level, chunk.site.map(|v| v << chunk.atoms));
            assert!(chunks.insert(key, (chunk.level, d)).is_none(), "{chunk:?} twice");
        });
        chunks
    }

    /** Whether a point (a, b, c), in a face's lattice coordinates, is in its rays and within `far` of `eye`, well within. */
    fn in_view(eye: [f64; 3], p: [f64; 3], far: f64) -> bool {
        let (x, y, z) = ((p[0] - eye[0]) * LAYER, p[1] - eye[1], (p[2] - eye[2]) * LAYER);
        x.abs() + std::f64::consts::SQRT_2 * y.abs() <= z - 0.01 && (x * x + y * y + z * z).sqrt() <= far
    }

    #[test]
    fn every_site_ahead_is_drawn_first_at_the_finest_level_claiming_it() {
        let far = 150.0;
        for (k, shift) in [(0, 1), (1, 2), (2, 3)] {
            let lod = Lod { shift, q: 4 + k };
            let eye = [0.3 + 7.1 * k as f64, -2.6 + k as f64, 1.7 - 3.3 * k as f64];
            let camera = eye[2].floor() as i32;
            let near = camera + NEAR;
            let chunks = composed(&lod, eye, far);
            let levels: std::collections::BTreeSet<i32> = chunks.keys().map(|k| k.0).collect();
            assert!(levels.len() >= 3, "levels {levels:?}");
            let mut ahead = 0;
            for a in -90..=90 {
                for b in -24..=24 {
                    for c in -10..=150 {
                        let p = [a, b, c];
                        if (a ^ c) & 1 != 0 || (b ^ c) & 1 != 0 || !in_view(eye, p.map(f64::from), far) {
                            continue;
                        }
                        // The chunks holding it, finest first.
                        let holding: Vec<i32> = levels.iter().copied().filter(|&l| chunks.contains_key(&(l, owner(1 << l, p)))).collect();
                        if c < near {
                            assert!(holding.is_empty(), "shift {shift}: {p:?}, before the near plane, in {holding:?}");
                            continue;
                        }
                        let finest = (0..).find(|&l| owner(1 << l, p)[2] as i64 <= lod.reach(l, camera));
                        assert_eq!(holding.first().copied(), finest, "shift {shift} site {p:?}");
                        ahead += 1;
                    }
                }
            }
            assert!(ahead > 10000, "{ahead}");
        }
    }

    /**
     * The chunks a ray from `eye` meets, where each shows (the one painted
     * first of those holding a point): how many times it goes from one to
     * another, and how many of those against the painting order.
     */
    fn crossings(lod: &Lod, eye: [f64; 3], far: f64) -> (usize, usize) {
        let chunks = composed(lod, eye, far);
        let shows = |x: [f64; 3]| {
            (0..=lod.top()).filter_map(|l| chunks.get(&(l, nearest(1 << l, x))).copied()).min_by(|p, q| p.partial_cmp(q).expect("ordered"))
        };
        let (mut met, mut wrong) = (0, 0);
        for r in 0..200 {
            let (u, v) = ((r * 37 % 101) as f64 / 101.0 * 1.9 - 0.95, (r * 53 % 97) as f64 / 97.0 * 1.3 - 0.65);
            let dir = [u / LAYER, v * (1.0 - u.abs()) / std::f64::consts::SQRT_2, 1.0 / LAYER];
            let mut last: Option<(i32, f64)> = None;
            for t in 6..1600 {
                let x: [f64; 3] = std::array::from_fn(|d| eye[d] + dir[d] * t as f64 * 0.07);
                if !in_view(eye, x, far) {
                    continue;
                }
                let Some(now) = shows(x) else { continue };
                if let Some(before) = last
                    && before != now
                {
                    met += 1;
                    wrong += usize::from(now < before);
                }
                last = Some(now);
            }
        }
        (met, wrong)
    }

    #[test]
    fn chunks_a_ray_meets_come_in_painting_order_from_anywhere() {
        let (mut met, mut wrong) = (0, 0);
        for e in 0..6 {
            let lod = Lod { shift: 2, q: 4 + e % 3 };
            let eye = [0.37 * e as f64 - 2.0, 0.21 * (e % 5) as f64 - 0.4, 0.29 * (e % 7) as f64 - 30.0 * e as f64];
            let (m, w) = crossings(&lod, eye, 120.0);
            (met, wrong) = (met + m, wrong + w);
        }
        assert!(met > 10000 && wrong == 0, "{wrong} of {met} crossings out of painting order");
    }

    /** The site of N·FCC nearest point x, in a face's lattice coordinates. */
    fn nearest(n: i32, x: [f64; 3]) -> [i32; 3] {
        let mut best = ([0; 3], f64::INFINITY);
        let base = x.map(|v| (v / n as f64).floor() as i32);
        for da in -1..=2 {
            for db in -1..=2 {
                for dc in -1..=2 {
                    let (i, j, k) = (base[0] + da, base[1] + db, base[2] + dc);
                    if (i ^ k) & 1 != 0 || (j ^ k) & 1 != 0 {
                        continue;
                    }
                    let s = [i * n, j * n, k * n];
                    let (a, b, c) = (s[0] as f64 - x[0], s[1] as f64 - x[1], s[2] as f64 - x[2]);
                    let d = (a * a + c * c) / 2.0 + b * b;
                    if d < best.1 {
                        best = (s, d);
                    }
                }
            }
        }
        best.0
    }
}
