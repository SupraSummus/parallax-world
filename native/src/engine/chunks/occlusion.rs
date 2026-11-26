//! Which cells of a face's pick nothing could show, found on the CPU before
//! they are baked, or their tiles made: rays cast from the eye through the
//! corners of a buffer's pixels over the face's tangents (x / z, y / z),
//! each stopped where it is (renderer.rs's Hiding says where), and a cell
//! is hidden if every ray round every pixel its box may cover stops nearer
//! than its nearest. A ray is cast when a cell asks for it, and as far as
//! that cell: so the rays cost as the cells asked about and how far they
//! are, a pick's new ones as the eye moves, most of them near it, not as
//! the buffer and the render distance. Between rays something may show: so
//! the renderer asks the GPU about what it hides (compositor.rs's probe).
//!
//! Depths are along the face's axis, from the eye, which every ray of the
//! face goes along, so that along a ray they order as distances do.

/** The buffer's pixels across x / z over the face's rays, and as many per unit up y / z. */
const WIDTH: usize = 128;

/** A box of a face's coordinates in units of grid 0 (x = a · LAYER, y = b, z = c · LAYER): [low, high] each way. */
pub type Box3 = [[f64; 2]; 3];

/**
 * What a ray cast found: nothing stops it as far as a depth (infinity:
 * ever); it waits at a depth for what is not known there yet, nothing
 * stopping it before; or it stops at a depth.
 */
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Cast {
    Clear(f64),
    Waits(f64),
    Stops(f64),
}

/**
 * Where a face's rays from `eye` stop, as far as they were cast, each with
 * the round it was cast in last; and how many wait. Its buffer's pixels
 * cover the face's rays' tangents, `half` either way across and up, rays
 * through their corners, `across` a row.
 */
pub struct Occlusion {
    eye: [f64; 3],
    half: [f64; 2],
    height: usize,
    rays: Vec<(Cast, u32)>,
    round: u32,
    waiting: usize,
}

impl Occlusion {
    /** None cast yet, from `eye`, in a face's coordinates in units of grid 0, its rays' tangents `half` either way across and up at most. */
    pub fn new(eye: [f64; 3], half: [f64; 2]) -> Occlusion {
        let height = (WIDTH as f64 * half[1] / half[0]).ceil() as usize;
        Occlusion { eye, half, height, rays: vec![(Cast::Clear(0.0), 0); (WIDTH + 1) * (height + 1)], round: 0, waiting: 0 }
    }

    /** Whether any ray waits for what was not known when it was cast. */
    pub fn waits(&self) -> bool {
        self.waiting > 0
    }

    /** From now on, a ray that waits is cast on from there, when next asked for past it. */
    pub fn again(&mut self) {
        self.round += 1;
    }

    /**
     * Whether what may be drawn within `shown` can show nowhere: every ray
     * round the pixels it may cover stops nearer than its nearest. Not if
     * it reaches the eye's plane, or past the buffer. A ray not known as
     * far as that is cast on first, unless it waits (till `again`): `stop`
     * says what it finds, given its tangents (x / z, y / z), from the depth
     * it is clear to, as far as the depth asked, at least.
     */
    pub fn hides(&mut self, shown: Box3, stop: impl Fn([f64; 2], f64, f64) -> Cast) -> bool {
        let [[x0, x1], [y0, y1], [z0, z1]] = std::array::from_fn(|d| shown[d].map(|v| v - self.eye[d]));
        if z0 <= 0.0 {
            return false;
        }
        let [hx, hy] = self.half;
        let scale = WIDTH as f64 / 2.0 / hx;
        let [i0, i1, j0, j1] =
            [(x0 / z0).min(x0 / z1) + hx, (x1 / z0).max(x1 / z1) + hx, (y0 / z0).min(y0 / z1) + hy, (y1 / z0).max(y1 / z1) + hy]
                .map(|v| v * scale);
        if i0 < 0.0 || j0 < 0.0 || i1 >= WIDTH as f64 || j1 >= self.height as f64 {
            return false;
        }
        // The corners of the pixels it touches.
        let (i0, i1, j0, j1) = (i0 as usize, i1 as usize + 1, j0 as usize, j1 as usize + 1);
        (j0..=j1).all(|j| {
            (i0..=i1).all(|i| {
                let (cast, round) = &mut self.rays[j * (WIDTH + 1) + i];
                let from = match *cast {
                    Cast::Clear(to) if to < z0 => Some(to),
                    Cast::Waits(at) if at < z0 && *round != self.round => Some(at),
                    _ => None,
                };
                if let Some(from) = from {
                    self.waiting -= usize::from(matches!(cast, Cast::Waits(_)));
                    (*cast, *round) = (stop([i as f64 / scale - hx, j as f64 / scale - hy], from, z0), self.round);
                    self.waiting += usize::from(matches!(cast, Cast::Waits(_)));
                }
                matches!(*cast, Cast::Stops(at) if at < z0)
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    /** What a ray from the origin with tangents `t` finds: a wall at depth 10, over x in [−5, 5], y in [−3, 3], or nothing ever. */
    fn wall([tx, ty]: [f64; 2]) -> Cast {
        if (tx * 10.0).abs() <= 5.0 && (ty * 10.0).abs() <= 3.0 { Cast::Stops(10.0) } else { Cast::Clear(f64::INFINITY) }
    }

    #[test]
    fn a_wall_hides_what_is_wholly_behind_it_and_nothing_else() {
        let mut occlusion = Occlusion::new([0.0, 0.0, 0.0], [1.0, std::f64::consts::FRAC_1_SQRT_2]);
        let mut hides = |shown| occlusion.hides(shown, |t, _, _| wall(t));
        // Behind it, within its shadow from the eye: hidden; nearer than it, or past its shadow: not.
        assert!(hides([[-1.0, 1.0], [-1.0, 1.0], [20.0, 22.0]]));
        assert!(!hides([[-1.0, 1.0], [-1.0, 1.0], [9.0, 13.0]]));
        assert!(!hides([[4.0, 12.0], [-1.0, 1.0], [20.0, 22.0]]));
        // Behind the eye, or reaching past the face: not.
        assert!(!hides([[-1.0, 1.0], [-1.0, 1.0], [-1.0, 1.0]]));
        assert!(!hides([[-30.0, 1.0], [-1.0, 1.0], [20.0, 22.0]]));
        assert!(!occlusion.waits(), "none waits");
    }

    #[test]
    fn a_ray_is_cast_as_far_as_what_asks_is_and_on_from_there() {
        let mut occlusion = Occlusion::new([0.0, 0.0, 0.0], [1.0, std::f64::consts::FRAC_1_SQRT_2]);
        // Asked about what is nearer than the wall, rays go no farther than it is; then on from there, to the wall.
        let farthest = Cell::new(0.0);
        let near = |_: [f64; 2], from: f64, to: f64| {
            assert!(from == 0.0 && to == 4.0);
            Cast::Clear(to)
        };
        assert!(!occlusion.hides([[-1.0, 1.0], [-1.0, 1.0], [4.0, 6.0]], near));
        let on = |t: [f64; 2], from: f64, to: f64| {
            farthest.set(to.max(farthest.get()));
            assert!(from == 0.0 || from == 4.0, "from where it was clear to");
            wall(t)
        };
        assert!(occlusion.hides([[-1.0, 1.0], [-1.0, 1.0], [20.0, 22.0]], on) && farthest.get() == 20.0);
        // What they found is kept.
        assert!(occlusion.hides([[-1.0, 1.0], [-1.0, 1.0], [30.0, 32.0]], |_, _, _| panic!("cast again")));
    }

    #[test]
    fn rays_that_wait_go_on_from_where_they_were_once_told_to() {
        let mut occlusion = Occlusion::new([0.0, 0.0, 0.0], [1.0, std::f64::consts::FRAC_1_SQRT_2]);
        let behind = [[-1.0, 1.0], [-1.0, 1.0], [20.0, 22.0]];
        // At first, the wall's left half is not known: those rays wait at 5.
        let half = |t: [f64; 2], _: f64, _: f64| if t[0] < 0.0 { Cast::Waits(5.0) } else { wall(t) };
        assert!(!occlusion.hides(behind, half) && occlusion.waits());
        // Asked again, none that waits is cast, until they are to be; then from where they waited.
        let waited = Cell::new(0);
        let whole = |t: [f64; 2], from: f64, _: f64| {
            if from > 0.0 {
                assert!(t[0] < 0.0 && from == 5.0, "only those that waited, from where they were");
                waited.set(waited.get() + 1);
            }
            wall(t)
        };
        assert!(!occlusion.hides(behind, whole) && waited.get() == 0);
        occlusion.again();
        assert!(occlusion.hides(behind, whole) && waited.get() > 0 && !occlusion.waits());
    }
}
