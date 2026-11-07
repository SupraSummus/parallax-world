//! Curves: lines through space, such as a path, a river or a tunnel, made
//! top-down: each level puts a point between each two of the level before,
//! within a corridor that narrows level by level, so a query looks only
//! where the curve can be. See "Curves" in docs/generators.md.

use crate::noise::{Fixed, Fx};

const DOCS: &str = "docs/generators.md";

/** Room the corridors leave for rounding, in Fx's least steps: a midpoint's, and a distance's. */
const SLACK: Fixed = 4;

/**
 * A curve between two ends, in D dimensions, refined level by level:
 * `refine(level, a, b)` puts a point between each two neighbours a and
 * b, at most `wander[level]` from their middle, reading only them and
 * what is coarser. So the curve between two neighbours of a level stays
 * within the finer levels' wanders, summed, of the segment between them
 * (its corridor), and is, at the finest level, the polyline through its
 * points. The rule is the curve's, the ends are each query's, so one
 * rule serves every piece of a curve too long to make whole.
 */
pub struct Curve<const D: usize> {
    wander: Vec<Fixed>,
    /** By level, how far the curve strays from a segment of it: the finer levels' wanders, summed; none at the finest. */
    corridor: Vec<Fixed>,
    refine: Box<Refine<D>>,
}

/** Where a level puts the point between two neighbours. */
type Refine<const D: usize> = dyn Fn(usize, [Fx; D], [Fx; D]) -> [Fx; D];

/** Where a curve passes nearest a point. */
#[derive(Clone, Copy, Debug)]
pub struct Near<const D: usize> {
    /** How far, with its fraction. */
    pub distance: Fx,
    /**
     * How far along: the finest segments before the nearest point, and the
     * fraction of the one it is on. A parameter for what runs along the
     * curve, the same at every query; its arc length where the finest
     * points are a unit apart.
     */
    pub along: Fx,
    /** The nearest point. */
    pub point: [Fx; D],
}

impl<const D: usize> Curve<D> {
    /** A curve refined once per `wander`, coarsest first. */
    pub fn new(wander: Vec<Fx>, refine: impl Fn(usize, [Fx; D], [Fx; D]) -> [Fx; D] + 'static) -> Curve<D> {
        let wander: Vec<Fixed> = wander.into_iter().map(Fx::to_bits).collect();
        let mut corridor = vec![0; wander.len() + 1];
        for level in (0..wander.len()).rev() {
            corridor[level] = corridor[level + 1] + wander[level] + SLACK;
        }
        Curve { wander, corridor, refine: Box::new(refine) }
    }

    /**
     * Where the curve from `ends[0]` to `ends[1]` passes nearest `p`, if
     * nearer than `within`. It looks only into segments whose corridor
     * reaches nearer, so it costs little but near the curve.
     */
    pub fn near(&self, ends: [[Fx; D]; 2], p: [Fx; D], within: Fx) -> Option<Near<D>> {
        let mut found = (within.to_bits(), None);
        let [a, b] = ends;
        self.visit(0, 0, a, b, gap2(a, b, p), p, &mut found);
        found.1
    }

    /**
     * Segment `i` of `level`, from a to b, its box `gap` from p (squared;
     * no further than the segment, and cheaper): the nearest of its curve
     * to p, if nearer than found.
     */
    #[allow(clippy::too_many_arguments)]
    fn visit(&self, level: usize, i: i64, a: [Fx; D], b: [Fx; D], gap: i128, p: [Fx; D], found: &mut (Fixed, Option<Near<D>>)) {
        let reach = i128::from(found.0 + self.corridor[level] + SLACK);
        if gap >= reach * reach {
            return;
        }
        if level == self.wander.len() {
            let to = to_segment(a, b, p);
            let distance = (to.distance2 as u128).isqrt() as Fixed;
            if distance < found.0 {
                let near = Near { distance: Fx::from_bits(distance), along: Fx::from_bits((i << 16) + to.t), point: to.point };
                *found = (distance, Some(near));
            }
            return;
        }
        let m = (self.refine)(level, a, b);
        let off: i128 = (0..D).map(|k| i128::from(m[k].to_bits() - ((a[k].to_bits() + b[k].to_bits()) >> 1)).pow(2)).sum();
        let wander = i128::from(self.wander[level]);
        assert!(
            off <= wander * wander,
            "a curve's point of level {level} stands {:.3} units off its neighbours' middle, past its wander of {:.3}; see \"Curves\" in {DOCS}",
            (off as f64).sqrt() / 65536.0,
            Fx::from_bits(self.wander[level]).to_f64()
        );
        let halves = [(2 * i, a, m, gap2(a, m, p)), (2 * i + 1, m, b, gap2(m, b, p))];
        // The nearer half first, so the other is more likely passed over.
        let order = if halves[1].3 < halves[0].3 { [1, 0] } else { [0, 1] };
        for h in order {
            let (i, a, b, gap) = halves[h];
            self.visit(level + 1, i, a, b, gap, p, found);
        }
    }
}

/** How far p is from the box of a and b, squared, in Fx's least steps. */
fn gap2<const D: usize>(a: [Fx; D], b: [Fx; D], p: [Fx; D]) -> i128 {
    (0..D).map(|k| i128::from((a[k].min(b[k]) - p[k]).max(p[k] - a[k].max(b[k])).max(Fx::ZERO).to_bits()).pow(2)).sum()
}

/** A point's nearest on a segment: how far it is, squared (in Fx's least steps), its fraction along, and itself. */
struct Segment<const D: usize> {
    distance2: i128,
    t: Fixed,
    point: [Fx; D],
}

fn to_segment<const D: usize>(a: [Fx; D], b: [Fx; D], p: [Fx; D]) -> Segment<D> {
    let ab: [i128; D] = std::array::from_fn(|k| i128::from((b[k] - a[k]).to_bits()));
    let ap: [i128; D] = std::array::from_fn(|k| i128::from((p[k] - a[k]).to_bits()));
    let length2: i128 = ab.iter().map(|v| v * v).sum();
    let dot = (0..D).map(|k| ap[k] * ab[k]).sum::<i128>().clamp(0, length2);
    let t = if length2 == 0 { 0 } else { ((dot << 16) / length2) as Fixed };
    let point = std::array::from_fn(|k| a[k] + Fx::from_bits(((ab[k] * i128::from(t)) >> 16) as Fixed));
    let distance2 = (0..D).map(|k| i128::from((p[k] - point[k]).to_bits()).pow(2)).sum();
    Segment { distance2, t, point }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::noise::{Rng, fx, hash64};

    /** A curve that wanders far, so it can turn back and cross itself: its first point up to `first` off its ends' middle. */
    fn wandering<const D: usize>(levels: usize, first: f64) -> Curve<D> {
        let wander: Vec<Fx> = (0..levels).map(|l| fx(first / (1 << l) as f64)).collect();
        let w = wander.clone();
        Curve::new(wander, move |level, a, b| {
            let key = |v: [Fx; D]| v.iter().fold(level as u64, |h, c| hash64(h, c.to_bits(), 0, 0));
            let mut random = Rng::new(key(a) ^ key(b).rotate_left(1));
            // A random offset in the cube inside the ball of the wander, rounded down.
            let r = w[level].to_f64() / (D as f64).sqrt();
            std::array::from_fn(|k| Fx::from_bits((a[k].to_bits() + b[k].to_bits()) >> 1) + fx((random.next() * 2.0 - 1.0) * r))
        })
    }

    /** The finest points of `curve` from a to b, by brute force. */
    fn points<const D: usize>(curve: &Curve<D>, level: usize, a: [Fx; D], b: [Fx; D], out: &mut Vec<[Fx; D]>) {
        if level == curve.wander.len() {
            out.push(a);
            return;
        }
        let m = (curve.refine)(level, a, b);
        points(curve, level + 1, a, m, out);
        points(curve, level + 1, m, b, out);
    }

    /**
     * Pruning by corridors loses nothing: whatever the point and range,
     * `near` finds as near a point as every segment tried in turn, and
     * its `along` and `point` say the same place.
     */
    fn finds_the_nearest<const D: usize>() {
        let curve = wandering::<D>(7, 40.0);
        let mut ends = [[Fx::ZERO; D]; 2];
        ends[1][0] = fx(64.0);
        let mut line = Vec::new();
        points(&curve, 0, ends[0], ends[1], &mut line);
        line.push(ends[1]);
        let mut random = Rng::new(11);
        for _ in 0..3000 {
            let p: [Fx; D] = std::array::from_fn(|_| fx(random.next() * 160.0 - 48.0));
            let within = fx(random.next() * 60.0);
            let brute = line.windows(2).map(|s| to_segment(s[0], s[1], p).distance2).min().expect("segments");
            let brute = Fx::from_bits((brute as u128).isqrt() as Fixed);
            match curve.near(ends, p, within) {
                None => assert!(brute >= within, "missed one {brute:?} off, within {within:?}"),
                Some(near) => {
                    assert_eq!(near.distance, brute);
                    // The end of one segment is the start of the next: `along` names the last's end as a whole number.
                    let i = ((near.along.to_bits() >> 16) as usize).min(line.len() - 2);
                    let on = to_segment(line[i], line[i + 1], p);
                    assert_eq!(((i as Fixed) << 16) + on.t, near.along.to_bits());
                    assert_eq!(on.point, near.point);
                }
            }
        }
    }

    #[test]
    fn finds_the_nearest_in_2d_and_3d() {
        finds_the_nearest::<2>();
        finds_the_nearest::<3>();
    }
}
