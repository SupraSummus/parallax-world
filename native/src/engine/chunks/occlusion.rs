//! Which cells of a face's pick nothing could show, found on the CPU before
//! they are baked, or their tiles made: rays cast from the eye through the
//! corners of a buffer's pixels over the face's tangents (x / z, y / z),
//! each stopped where it is (renderer.rs's Hiding says where), and a cell
//! is hidden if every ray round every pixel its box may cover stops nearer
//! than its nearest. Between rays something may show: so the renderer asks
//! the GPU about what it hides (compositor.rs's probe).
//!
//! Depths are along the face's axis, which every ray of the face goes
//! along, so that along a ray they order as distances do.

/** The buffer's pixels across x / z in [−1, 1]; y / z, in [−1/√2, 1/√2], as many per unit. */
const WIDTH: usize = 128;
const HEIGHT: usize = 91;
const Y_HALF: f64 = std::f64::consts::FRAC_1_SQRT_2;
/** Rays a row, and rows: one through each corner of the pixels. */
const ACROSS: usize = WIDTH + 1;
const ROWS: usize = HEIGHT + 1;

/** A box of a face's coordinates in units of grid 0 (x = a · LAYER, y = b, z = c · LAYER): [low, high] each way. */
pub type Box3 = [[f64; 2]; 3];

/** Where a face's rays from `eye` stop, ray by ray, and how far those that may yet stop nearer went (cast). */
pub struct Occlusion {
    eye: [f64; 3],
    depths: Vec<f64>,
    open: Vec<Option<f64>>,
}

impl Occlusion {
    /** None cast yet, from `eye`, in a face's coordinates in units of grid 0. */
    pub fn new(eye: [f64; 3]) -> Occlusion {
        Occlusion { eye, depths: vec![f64::INFINITY; ACROSS * ROWS], open: vec![Some(0.0); ACROSS * ROWS] }
    }

    /**
     * Casts on the rays that may yet stop nearer, all at first, rows of
     * them on threads of their own, behind the one that draws: each stops
     * where `stop` says, given its tangents (x / z, y / z) and how far
     * along the axis it went (0 at first), or else says how far it went
     * now. Whether any may yet stop nearer.
     */
    pub fn cast(&mut self, stop: impl Fn([f64; 2], f64) -> (f64, Option<f64>) + Sync) -> bool {
        let scale = WIDTH as f64 / 2.0;
        let rows = ROWS.div_ceil(std::thread::available_parallelism().map_or(1, |n| n.get())) * ACROSS;
        std::thread::scope(|scope| {
            for (r, (depths, open)) in self.depths.chunks_mut(rows).zip(self.open.chunks_mut(rows)).enumerate() {
                let stop = &stop;
                scope.spawn(move || {
                    crate::world::behind_drawing();
                    for (k, (d, open)) in depths.iter_mut().zip(open).enumerate() {
                        let Some(from) = *open else { continue };
                        let k = r * rows + k;
                        (*d, *open) = stop([(k % ACROSS) as f64 / scale - 1.0, (k / ACROSS) as f64 / scale - Y_HALF], from);
                    }
                });
            }
        });
        self.open.iter().any(Option::is_some)
    }

    /**
     * Whether what may be drawn within `shown` can show nowhere: every ray
     * round the pixels it may cover stops nearer than its nearest. Not if
     * it reaches the eye's plane, or past the buffer.
     */
    pub fn hides(&self, shown: Box3) -> bool {
        let [[x0, x1], [y0, y1], [z0, z1]] = std::array::from_fn(|d| shown[d].map(|v| v - self.eye[d]));
        if z0 <= 0.0 {
            return false;
        }
        let scale = WIDTH as f64 / 2.0;
        let [i0, i1, j0, j1] =
            [(x0 / z0).min(x0 / z1) + 1.0, (x1 / z0).max(x1 / z1) + 1.0, (y0 / z0).min(y0 / z1) + Y_HALF, (y1 / z0).max(y1 / z1) + Y_HALF]
                .map(|v| v * scale);
        if i0 < 0.0 || j0 < 0.0 || i1 >= WIDTH as f64 || j1 >= HEIGHT as f64 {
            return false;
        }
        // The corners of the pixels it touches.
        let (i0, i1, j0, j1) = (i0 as usize, i1 as usize + 1, j0 as usize, j1 as usize + 1);
        let nearest = shown[2][0];
        (j0..=j1).all(|j| self.depths[j * ACROSS + i0..=j * ACROSS + i1].iter().all(|&d| d < nearest))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /** Where a ray from the origin with tangents `t` meets a wall at depth 10, over x in [−5, 5], y in [−3, 3]; never past it. */
    fn wall([tx, ty]: [f64; 2]) -> f64 {
        if (tx * 10.0).abs() <= 5.0 && (ty * 10.0).abs() <= 3.0 { 10.0 } else { f64::INFINITY }
    }

    #[test]
    fn a_wall_hides_what_is_wholly_behind_it_and_nothing_else() {
        let mut occlusion = Occlusion::new([0.0, 0.0, 0.0]);
        assert!(!occlusion.cast(|t, _| (wall(t), None)), "all stopped for good");
        // Behind it, within its shadow from the eye: hidden; nearer than it, or past its shadow: not.
        assert!(occlusion.hides([[-1.0, 1.0], [-1.0, 1.0], [20.0, 22.0]]));
        assert!(!occlusion.hides([[-1.0, 1.0], [-1.0, 1.0], [9.0, 13.0]]));
        assert!(!occlusion.hides([[4.0, 12.0], [-1.0, 1.0], [20.0, 22.0]]));
        // Behind the eye, or reaching past the face: not.
        assert!(!occlusion.hides([[-1.0, 1.0], [-1.0, 1.0], [-1.0, 1.0]]));
        assert!(!occlusion.hides([[-30.0, 1.0], [-1.0, 1.0], [20.0, 22.0]]));
    }

    #[test]
    fn rays_that_may_stop_nearer_go_on_from_where_they_were_and_only_they() {
        let mut occlusion = Occlusion::new([0.0, 0.0, 0.0]);
        // At first, the wall's left half is not known: those rays went 5 along, and go on from there.
        assert!(occlusion.cast(|t, from| if t[0] < 0.0 { (f64::INFINITY, Some(from + 5.0)) } else { (wall(t), None) }));
        assert!(!occlusion.hides([[-1.0, 1.0], [-1.0, 1.0], [20.0, 22.0]]));
        assert!(!occlusion.cast(|t, from| {
            assert!(t[0] < 0.0 && from == 5.0, "only those that may stop nearer, from where they were");
            (wall(t), None)
        }));
        assert!(occlusion.hides([[-1.0, 1.0], [-1.0, 1.0], [20.0, 22.0]]));
    }
}
