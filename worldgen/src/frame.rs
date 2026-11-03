//! Frames: things built in coordinates of their own (u, v, w), placed in
//! their parent's by quarter turns about the vertical, mirrors and an
//! offset, so boxes stay boxes and a part's sites land on its parent's.
//! See "Model" in docs/generators.md.

use crate::generator::{Fill, Kind, Op, Point, SOLID, Shape, ball, cuboid, gable, lathe};

/**
 * Where a frame's coordinates are in its parent's: `axes[i]` is where
 * its axis i points, a parent axis or its opposite, and `origin` the
 * parent site its site (0, 0, 0) is at. A box [min, max) holds the sites
 * min to max − 1; turned or mirrored, it holds the sites they go to.
 */
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Frame {
    origin: [i64; 3],
    axes: [[i64; 3]; 3],
}

impl Frame {
    /** The parent's own coordinates. */
    pub const SAME: Frame = Frame { origin: [0; 3], axes: [[1, 0, 0], [0, 1, 0], [0, 0, 1]] };

    /** Its u, v and w along `axes` (each one of the parent's, or its opposite), site (0, 0, 0) at `origin`. */
    pub fn new(origin: [i64; 3], axes: [[i64; 3]; 3]) -> Frame {
        assert!(
            axes.iter().all(|a| a.iter().map(|c| c.abs()).sum::<i64>() == 1 && a.iter().all(|c| c.abs() <= 1)),
            "a frame's axes are the parent's"
        );
        Frame { origin, axes }
    }

    /** The parent's coordinates from `origin` on. */
    pub fn at(origin: [i64; 3]) -> Frame {
        Frame { origin, ..Frame::SAME }
    }

    /** Turned by `quarters` about its v axis, from u towards w. */
    pub fn turned(self, quarters: i64) -> Frame {
        let [u, v, w] = self.axes;
        let neg = |a: [i64; 3]| a.map(|c| -c);
        let axes = match quarters.rem_euclid(4) {
            0 => [u, v, w],
            1 => [w, v, neg(u)],
            2 => [neg(u), v, neg(w)],
            _ => [neg(w), v, u],
        };
        Frame { axes, ..self }
    }

    /** With its axis `axis` reversed. */
    pub fn mirrored(self, axis: usize) -> Frame {
        let mut axes = self.axes;
        axes[axis] = axes[axis].map(|c| -c);
        Frame { axes, ..self }
    }

    /** `inner`, a frame in this one's coordinates, in the parent's. */
    pub fn then(&self, inner: Frame) -> Frame {
        let axes = inner.axes.map(|a| self.vector(a));
        Frame { origin: self.site(inner.origin), axes }
    }

    fn vector(&self, v: [i64; 3]) -> [i64; 3] {
        [0, 1, 2].map(|i| (0..3).map(|j| v[j] * self.axes[j][i]).sum())
    }

    /** Site `p` of the frame, in the parent's. */
    pub fn site(&self, p: [i64; 3]) -> [i64; 3] {
        let v = self.vector(p);
        [0, 1, 2].map(|i| self.origin[i] + v[i])
    }

    /** Point `p` of the frame, in the parent's. */
    pub fn point(&self, p: Point) -> Point {
        [0, 1, 2].map(|i| self.origin[i] as f64 + (0..3).map(|j| p[j] * self.axes[j][i] as f64).sum::<f64>())
    }

    /** The box from `min` to `max` (exclusive), in the parent's. */
    pub fn cells(&self, min: [f64; 3], max: [f64; 3]) -> (Point, Point) {
        let (a, b) = (self.point(min), self.point(max));
        // Along a reversed axis, sites min to max − 1 go to o − max + 1 to o − min.
        let reversed = |i: usize| (0..3).any(|j| self.axes[j][i] < 0);
        let shift = |i: usize| if reversed(i) { 1.0 } else { 0.0 };
        ([0, 1, 2].map(|i| a[i].min(b[i]) + shift(i)), [0, 1, 2].map(|i| a[i].max(b[i]) + shift(i)))
    }

    /** The parent axis its axis `axis` lies along. */
    pub fn axis(&self, axis: usize) -> usize {
        (0..3).find(|i| self.axes[axis][*i] != 0).expect("an axis")
    }
}

/** Its axes and their opposites, to give frames by. */
pub const U: [i64; 3] = [1, 0, 0];
pub const V: [i64; 3] = [0, 1, 0];
pub const W: [i64; 3] = [0, 0, 1];
pub const fn back(a: [i64; 3]) -> [i64; 3] {
    [-a[0], -a[1], -a[2]]
}

/**
 * The four walls of a footprint u in [0, length), w in [0, depth): each a
 * frame whose u runs along the wall, w into the footprint, with its
 * length; the first along w = 0, then on round by quarter turns.
 */
pub fn walls(length: i64, depth: i64) -> [(Frame, i64); 4] {
    [
        (Frame::at([0, 0, 0]), length),
        (Frame::at([length - 1, 0, 0]).turned(1), depth),
        (Frame::at([length - 1, 0, depth - 1]).turned(2), length),
        (Frame::at([0, 0, depth - 1]).turned(3), depth),
    ]
}

/** A thing's shapes, added in a frame: its parts are built where they are, in coordinates of their own. */
pub struct Part<'a> {
    pub frame: Frame,
    pub shapes: &'a mut Vec<Shape>,
}

impl Part<'_> {
    pub fn new(frame: Frame, shapes: &mut Vec<Shape>) -> Part<'_> {
        Part { frame, shapes }
    }

    /** A part of this one, in `frame` (in this one's coordinates). */
    pub fn within(&mut self, frame: Frame) -> Part<'_> {
        Part { frame: self.frame.then(frame), shapes: self.shapes }
    }

    /** The cells from `min` to `max` (exclusive) of `kind`, by `op`. */
    pub fn cells(&mut self, min: [i64; 3], max: [i64; 3], kind: Kind, op: Op) -> &mut Shape {
        let (a, b) = self.frame.cells(min.map(|v| v as f64), max.map(|v| v as f64));
        self.shapes.push(cuboid(a, b, kind, SOLID, op));
        self.shapes.last_mut().expect("one")
    }

    /** Solid `kind` in the cells from min to max, where empty. */
    pub fn add(&mut self, min: [i64; 3], max: [i64; 3], kind: Kind) -> &mut Shape {
        self.cells(min, max, kind, Op::Fill)
    }

    /** `kind` over whatever the thing has in the cells from min to max. */
    pub fn paint(&mut self, min: [i64; 3], max: [i64; 3], kind: Kind) -> &mut Shape {
        self.cells(min, max, kind, Op::Paint)
    }

    /** The cells from min to max cut out of the thing. */
    pub fn cut(&mut self, min: [i64; 3], max: [i64; 3]) -> &mut Shape {
        self.cells(min, max, 0, Op::Carve)
    }

    /** A gable roof over the cells from min to max, its ridge along `ridge` (u or w, 0 or 2), up along v. */
    pub fn gable(&mut self, min: [f64; 3], max: [f64; 3], ridge: usize, kind: Kind, op: Op) -> &mut Shape {
        let (a, b) = self.frame.cells(min, max);
        self.shapes.push(gable(a, b, self.frame.axis(ridge), kind, SOLID, op));
        self.shapes.last_mut().expect("one")
    }

    /** A solid of revolution from p to q, radius(t) wide. */
    pub fn lathe(&mut self, p: Point, q: Point, radius: impl Fn(f64) -> f64 + 'static, kind: Kind, fill: Fill, op: Op) -> &mut Shape {
        self.shapes.push(lathe(self.frame.point(p), self.frame.point(q), radius, kind, fill, op));
        self.shapes.last_mut().expect("one")
    }

    pub fn ball(&mut self, centre: Point, radius: f64, kind: Kind, op: Op) -> &mut Shape {
        self.shapes.push(ball(self.frame.point(centre), radius, kind, SOLID, op));
        self.shapes.last_mut().expect("one")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn walls_run_round_their_footprint_and_face_into_it() {
        for (frame, length) in walls(5, 3) {
            // Along the wall, its first and last sites are on the footprint's edge; a site in is inside.
            for p in [frame.site([0, 0, 0]), frame.site([length - 1, 0, 0]), frame.site([1, 0, 1])] {
                assert!((0..5).contains(&p[0]) && (0..3).contains(&p[2]), "{frame:?}: {p:?}");
            }
        }
    }

    #[test]
    fn turned_and_mirrored_boxes_hold_the_sites_their_points_go_to() {
        let frames = [
            Frame::at([5, 0, -3]),
            Frame::at([5, 0, -3]).turned(1),
            Frame::at([5, 0, -3]).turned(2),
            Frame::at([5, 0, -3]).turned(3),
            Frame::at([5, 0, -3]).mirrored(0),
            Frame::at([1, 2, 3]).turned(1).then(Frame::at([2, 0, 7]).mirrored(2).turned(3)),
        ];
        let (min, max) = ([1, 0, 2], [4, 2, 6]);
        for f in frames {
            let (a, b) = f.cells(min.map(|v| v as f64), max.map(|v| v as f64));
            for (u, v, w) in (min[0]..max[0]).flat_map(|u| (min[1]..max[1]).flat_map(move |v| (min[2]..max[2]).map(move |w| (u, v, w)))) {
                let p = f.site([u, v, w]);
                assert!(
                    (0..3).all(|i| a[i] <= p[i] as f64 && (p[i] as f64) < b[i]),
                    "{f:?}: ({u}, {v}, {w}) at {p:?}, outside {a:?}..{b:?}"
                );
            }
            let volume: f64 = (0..3).map(|i| b[i] - a[i]).product();
            assert_eq!(volume, 24.0, "{f:?}");
        }
    }
}
