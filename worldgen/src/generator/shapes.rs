//! Shapes: the solids things are made of, each drawn alone into any box
//! at any level.

use super::*;

/** The area of an opaque body stopping as many rays as one of `area` and mean chord `chord`, filled with chance `fill`. */
fn effective_area(area: f64, chord: f64, fill: &Fill, centre: Point) -> f64 {
    match fill.at(centre[0], centre[1], centre[2]) {
        // Opaque: the power of nothing is nothing.
        1.0 => area,
        f => area * (1.0 - js::pow(1.0 - f, chord / CHORD)),
    }
}

pub(super) fn at(p: Point) -> At {
    p.map(fixed)
}

fn mid(p: Point, q: Point) -> Point {
    [0, 1, 2].map(|i| (p[i] + q[i]) / 2.0)
}

/** The first and last site of a level-0 box from min to max (fixed point), inclusive. */
fn sites_between(min: At, max: At) -> Box3 {
    Box3 { min: min.map(|v| -((-v) >> 16)), max: max.map(|v| (v >> 16) + 1) }
}

fn dot(a: At, b: At) -> i64 {
    (0..3).map(|i| a[i] * b[i]).sum()
}

pub fn ellipsoid(centre: Point, radii: Point, kind: Kind, fill: Fill, op: Op) -> Shape {
    let [a, b, c] = radii;
    let p = 1.6075; // Knud Thomsen's approximation of the surface area, exact for a ball
    let area = if a == b && b == c {
        4.0 * PI * a * a
    } else {
        4.0 * PI * js::pow((js::pow(a * b, p) + js::pow(a * c, p) + js::pow(b * c, p)) / 3.0, 1.0 / p)
    };
    let (centre_at, radii) = (at(centre), at(radii));
    Shape {
        kind,
        op,
        centre: centre_at,
        bounds: sites_between([0, 1, 2].map(|i| centre_at[i] - radii[i]), [0, 1, 2].map(|i| centre_at[i] + radii[i])),
        extents: radii.map(|r| Fx::from_bits(2 * r)),
        area: effective_area(area, ((16.0 / 3.0) * PI * a * b * c) / area, &fill, centre),
        fill,
        solid: Solid::Ellipsoid { centre: centre_at, radii },
    }
}

pub fn ball(centre: Point, radius: f64, kind: Kind, fill: Fill, op: Op) -> Shape {
    ellipsoid(centre, [radius, radius, radius], kind, fill, op)
}

/** The points within `radius` of the segment from p to q: twigs, logs, trunks; a straight tube, its ends round. */
pub fn capsule(p: Point, q: Point, radius: impl Into<Radius>, kind: Kind, fill: Fill, op: Op) -> Shape {
    swept(&straight(), p, q, radius.into(), false, kind, fill, op)
}

/** A solid of revolution: a crown, a tower, a window's disc; a straight tube, its ends flat. */
pub fn lathe(p: Point, q: Point, radius: impl Into<Radius>, kind: Kind, fill: Fill, op: Op) -> Shape {
    swept(&straight(), p, q, radius.into(), true, kind, fill, op)
}

/**
 * The points within `radius` of `curve` from p to q: a branch, a root.
 * At sites s apart it is drawn along the points of the coarsest level
 * the curve strays from by s / 2 or less, so within half a site of
 * where it is.
 */
pub fn tube(curve: &Curve<3>, p: Point, q: Point, radius: impl Into<Radius>, kind: Kind, fill: Fill, op: Op) -> Shape {
    swept(curve, p, q, radius.into(), false, kind, fill, op)
}

/** A curve refined no times: the segment. */
fn straight() -> Curve<3> {
    Curve::new(Vec::new(), |_, a, _| a)
}

/** A tube, its ends round, or `flat`: cut across the curve there. */
#[allow(clippy::too_many_arguments)]
fn swept(curve: &Curve<3>, p: Point, q: Point, radius: Radius, flat: bool, kind: Kind, fill: Fill, op: Op) -> Shape {
    let points: Vec<At> = curve.points([p, q].map(|e| at(e).map(Fx::from_bits))).into_iter().map(|e| e.map(Fx::to_bits)).collect();
    let n = points.len() - 1;
    let between = |i: usize, j: usize| [0, 1, 2].map(|k| points[j][k] - points[i][k]);
    // How far along each point is, from its start.
    let mut along: Vec<Fixed> = vec![0];
    for i in 0..n {
        let d = between(i, i + 1);
        along.push(along[i] + (dot(d, d) as u64).isqrt() as Fixed);
    }
    let length = along[n];
    let t = |i: usize| if length == 0 { 0 } else { ((i128::from(along[i]) << 16) / i128::from(length)) as Fixed };
    // Level by level, coarsest first, 1, 2, 4… pieces, in one allocation: capsules are many, twigs and leaves.
    let mut pieces = Vec::with_capacity(2 * n - 1);
    for (level, stray) in curve.corridors().enumerate() {
        let step = n >> level;
        pieces.extend((0..n).step_by(step).map(|i| Piece {
            p: points[i],
            segment: Segment::new(between(i, i + step)),
            stray: stray.to_bits(),
            t: [t(i), t(i + step)],
            flat: [flat && i == 0, flat && i + step == n],
        }));
    }
    let (area, volume, widest) = revolved(&radius, float(length), flat);
    let w = fixed(widest);
    // Around each of its points a ball, or at a flat end a disc across it.
    let around = |i: usize| match (flat, i) {
        (true, 0) => disc(between(0, 1), w),
        (true, i) if i == n => disc(between(n - 1, n), w),
        _ => [w; 3],
    };
    Shape {
        kind,
        op,
        centre: at(mid(p, q)),
        bounds: sites_between(
            [0, 1, 2].map(|k| (0..=n).map(|i| points[i][k] - around(i)[k]).min().expect("ends")),
            [0, 1, 2].map(|k| (0..=n).map(|i| points[i][k] + around(i)[k]).max().expect("ends")),
        ),
        extents: [length + if flat { 0 } else { 2 * w }, 2 * w, 2 * w].map(Fx::from_bits),
        area: effective_area(area, (4.0 * volume) / area, &fill, mid(p, q)),
        fill,
        solid: Solid::Tube { pieces, radius, widest: w, length },
    }
}

/**
 * How far a disc of radius w across direction d reaches along each axis:
 * w · sin of the angle to it, sin² with 22 bits after the point
 * (d · d < 2^40), rounded up.
 */
fn disc(d: At, w: Fixed) -> At {
    let (d8, l2) = (d.map(|v| v >> 8), dot(d.map(|v| v >> 8), d.map(|v| v >> 8)));
    d8.map(|v| {
        if l2 == 0 {
            return w;
        }
        let sin2 = (((l2 - v * v) << 22) + l2 - 1) / l2;
        let sin = sin2.isqrt() + i64::from(sin2.isqrt().pow(2) < sin2);
        (w * sin + 2047) >> 11
    })
}

/**
 * A solid of revolution `length` long, of `radius`, by slices: its area,
 * volume and widest radius, its ends half balls, or `flat` discs. The area
 * is its convex hull's, as is the shadow it casts: a crown's whorls have
 * undersides that cast no more (Cauchy holds for convex bodies).
 */
fn revolved(radius: &Radius, length: f64, flat: bool) -> (f64, f64, f64) {
    let caps = if flat { 1.0 } else { 2.0 };
    if let Radius::Const(r) = *radius {
        // A cylinder and its ends: what the slices add up to, without them.
        let ends = if flat { 0.0 } else { (2.0 / 3.0) * PI * 2.0 * js::pow(r, 3.0) };
        return (caps * PI * (r * r + r * r) + PI * (r + r) * length, ends + PI * r * r * length, r);
    }
    let n = 8f64.max(length.ceil()) as usize;
    let r: Vec<f64> = (0..=n).map(|i| radius.at(i as f64 / n as f64)).collect();
    let mut hull: Vec<usize> = Vec::new();
    for i in 0..=n {
        let cross = |a: usize, b: usize| (b as f64 - a as f64) * (r[i] - r[a]) - (i as f64 - a as f64) * (r[b] - r[a]);
        while hull.len() >= 2 && cross(hull[hull.len() - 2], hull[hull.len() - 1]) >= 0.0 {
            hull.pop();
        }
        hull.push(i);
    }
    let mut area = caps * PI * (r[0] * r[0] + r[n] * r[n]);
    let mut volume = if flat { 0.0 } else { (2.0 / 3.0) * PI * (js::pow(r[0], 3.0) + js::pow(r[n], 3.0)) };
    for h in 0..hull.len().saturating_sub(1) {
        let (a, b) = (hull[h], hull[h + 1]);
        area += PI * (r[a] + r[b]) * js::hypot(&[((b - a) as f64 * length) / n as f64, r[b] - r[a]]);
    }
    for i in 0..n {
        volume += (PI * (r[i] * r[i] + r[i + 1] * r[i + 1]) / 2.0) * (length / n as f64);
    }
    (area, volume, js::max(r.iter().copied()))
}

/** The points of [min, max): at level 0, the sites with min ≤ site < max. */
pub fn cuboid(min: Point, max: Point, kind: Kind, fill: Fill, op: Op) -> Shape {
    let size = [max[0] - min[0], max[1] - min[1], max[2] - min[2]];
    let area = 2.0 * (size[0] * size[1] + size[1] * size[2] + size[0] * size[2]);
    let (lo, hi) = (at(min), at(max));
    Shape {
        kind,
        op,
        centre: at(mid(min, max)),
        bounds: Box3 { min: lo.map(|v| -((-v) >> 16)), max: hi.map(|v| -((-v) >> 16)) },
        extents: [0, 1, 2].map(|i| Fx::from_bits(hi[i] - lo[i])),
        area: effective_area(area, (4.0 * size[0] * size[1] * size[2]) / area, &fill, mid(min, max)),
        fill,
        solid: Solid::Cuboid { min: lo, max: hi },
    }
}

/**
 * The points of a gable roof over [min, max): along `ridge` (x or z, 0 or
 * 2) from min to max, across the other from min to max at the bottom
 * (y = min) and narrowing to the ridge line at the top (y = max). Up is +y.
 */
pub fn gable(min: Point, max: Point, ridge: usize, kind: Kind, fill: Fill, op: Op) -> Shape {
    assert!(ridge == 0 || ridge == 2, "a gable's ridge runs along x or z");
    let across = 2 - ridge;
    let (length, width, height) = (max[ridge] - min[ridge], max[across] - min[across], max[1] - min[1]);
    let slant = js::hypot(&[width / 2.0, height]);
    let area = 2.0 * length * slant + width * height + length * width;
    let (lo, hi) = (at(min), at(max));
    Shape {
        kind,
        op,
        centre: at(mid(min, max)),
        bounds: Box3 { min: lo.map(|v| -((-v) >> 16)), max: hi.map(|v| -((-v) >> 16)) },
        extents: [hi[ridge] - lo[ridge], hi[across] - lo[across], hi[1] - lo[1]].map(Fx::from_bits),
        area: effective_area(area, (2.0 * length * width * height) / area, &fill, mid(min, max)),
        fill,
        solid: Solid::Gable { min: lo, max: hi, ridge },
    }
}

/*
 * Containment is tested on offsets from the shape with 8 bits after the
 * point: a shape and its widening span at most 2^11 units, so squares stay
 * under 2^40 and every product fits in an i64.
 */

/** `x − p`, with 8 bits after the point. */
fn offset(x: At, p: At) -> At {
    [0, 1, 2].map(|i| (x[i] - p[i]) >> 8)
}

/** A segment from 0 to d (8 bits after the point), and 2^62 / (d · d) for finding how far along it a point is without dividing. */
#[derive(Clone, Copy)]
struct Segment {
    d: At,
    l2: i64,
    /** Its length, √l2. */
    l: i64,
    inverse: i64,
}

impl Segment {
    fn new(d: At) -> Segment {
        let d = d.map(|v| v >> 8);
        let l2 = dot(d, d);
        Segment { d, l2, l: (l2 as u64).isqrt() as i64, inverse: if l2 == 0 { 0 } else { (1 << 62) / l2 } }
    }

    /** How far along the segment v's nearest point is, a fraction of ONE, given v · d. */
    fn along(&self, vd: i64) -> Fixed {
        // vd ≤ l2, so the product stays under 2^62.
        (vd.clamp(0, self.l2) * self.inverse) >> 46
    }

    /** The squared distance of v from the segment, given v · d. */
    fn away(&self, v: At, vd: i64) -> i64 {
        let t = self.along(vd);
        let a = [0, 1, 2].map(|i| v[i] - ((t * self.d[i]) >> 16));
        dot(a, a)
    }
}

/**
 * A tube's piece: from p along `segment`, of a level whose points the
 * curve strays from by `stray`, from `t[0]` to `t[1]` along the tube
 * (fractions of ONE), and whether the tube ends flat at its start or end.
 */
#[derive(Clone, Copy)]
pub(super) struct Piece {
    p: At,
    segment: Segment,
    stray: Fixed,
    t: [Fixed; 2],
    flat: [bool; 2],
}

/**
 * How thick a tube is, tested at one `min`: within √r2 (its widest, at
 * least half min), and where its radius changes, within `radius` (at
 * least `half`) at the nearest point, so a piece is about a cone; `short`,
 * how much shorter than min it is (8 bits after the point).
 */
#[derive(Clone, Copy)]
pub(super) struct Thick<'a> {
    r2: i64,
    radius: Option<&'a dyn Fn(f64) -> f64>,
    half: Fixed,
    short: i64,
}

impl Piece {
    fn holds(&self, x: At, thick: &Thick) -> bool {
        let v = offset(x, self.p);
        let vd = dot(v, self.segment.d);
        // Nothing past a flat end, but by half what the tube is short of min (in units of v · d).
        let reach = self.segment.l * thick.short / 2;
        if (self.flat[0] && vd < -reach) || (self.flat[1] && vd > self.segment.l2 + reach) {
            return false;
        }
        let away = self.segment.away(v, vd);
        if away > thick.r2 {
            return false;
        }
        let Some(radius) = thick.radius else { return true };
        let [t0, t1] = self.t;
        let r = fixed(radius(float(t0 + (((t1 - t0) * self.segment.along(vd)) >> 16)))).max(thick.half) >> 8;
        away <= r * r
    }
}

/** A shape's test, with what does not depend on the point worked out. */
pub(super) enum Inside<'a> {
    Nothing,
    Ellipsoid {
        centre: At,
        inverse: [i64; 3],
    },
    /** A tube drawn in one piece, as most are, by value: looping over one cost a few percent at level 0. */
    Piece {
        piece: Piece,
        thick: Thick<'a>,
    },
    /** The pieces of the level drawn. */
    Tube {
        pieces: &'a [Piece],
        thick: Thick<'a>,
    },
    /** [lo, hi) along each axis at least `min` long, else within min / 2 of its middle. */
    Cuboid {
        lo: At,
        hi: At,
        min: Fixed,
    },
    /** In its box, under the slopes: |2u − lo − hi| · height ≤ width · (top − y), with 8 bits after the point. */
    Gable {
        lo: At,
        hi: At,
        min: Fixed,
        across: usize,
    },
}

/** Whether `x` is in [lo, hi) if that is at least `min` long, else within min / 2 of its middle. */
fn within(x: Fixed, lo: Fixed, hi: Fixed, min: Fixed) -> bool {
    if hi - lo >= min { lo <= x && x < hi } else { (2 * x - lo - hi).abs() <= min }
}

impl Inside<'_> {
    pub(super) fn holds(&self, x: At) -> bool {
        match self {
            Inside::Nothing => false,
            Inside::Ellipsoid { centre, inverse } => {
                let u = offset(x, *centre);
                (0..3).map(|i| ((u[i] * inverse[i]) >> 16).pow(2)).sum::<i64>() <= ONE * ONE
            }
            Inside::Piece { piece, thick } => piece.holds(x, thick),
            Inside::Tube { pieces, thick } => pieces.iter().any(|piece| piece.holds(x, thick)),
            Inside::Cuboid { lo, hi, min } => (0..3).all(|i| within(x[i], lo[i], hi[i], *min)),
            Inside::Gable { lo, hi, min, across } => {
                let a = *across;
                if !(0..3).all(|i| within(x[i], lo[i], hi[i], *min)) {
                    return false;
                }
                // Too low or narrow for its slopes to show: its box.
                if hi[1] - lo[1] < *min || hi[a] - lo[a] < *min {
                    return true;
                }
                let u = (2 * x[a] - lo[a] - hi[a]).abs() >> 8;
                u * ((hi[1] - lo[1]) >> 8) <= ((hi[a] - lo[a]) >> 8) * ((hi[1] - x[1]) >> 8)
            }
        }
    }
}

impl Shape {
    /** Whether point `p` (lattice units) is inside it, as tested at points a quarter of a unit apart (level 0's fine sites). */
    pub fn holds(&self, p: Point) -> bool {
        // Its bounds hold its sites: a point between them, a unit past.
        let near = self.bounds.widen(1);
        (0..3).all(|i| near.min[i] as f64 <= p[i] && p[i] < near.max[i] as f64) && self.inside(ONE / 4, 0).holds(at(p))
    }

    /**
     * Whether a point is inside the shape with each of its sizes made at
     * least `min`, tested at points `apart` (fixed point): a tube follows
     * its curve to within half that.
     */
    pub(super) fn inside(&self, apart: Fixed, min: Fixed) -> Inside<'_> {
        let half = min / 2;
        let q8 = |v: Fixed| v >> 8;
        match &self.solid {
            Solid::Ellipsoid { centre, radii } => {
                // Σ (u / a)² ≤ 1, u / a as u · (2^32 / a) with 16 bits after the point.
                let a = radii.map(|r| q8(r.max(half)));
                if a.contains(&0) {
                    return Inside::Nothing;
                }
                Inside::Ellipsoid { centre: *centre, inverse: a.map(|a| (1 << 32) / a) }
            }
            Solid::Tube { pieces, radius, widest, length } => {
                // The coarsest level within half the points' spacing of the curve; level l is pieces 2^l − 1 to 2^(l+1) − 2.
                let level = (0..).find(|&l| pieces[(1 << l) - 1].stray <= apart / 2).expect("the finest strays nowhere");
                let radius = match radius {
                    Radius::Const(_) => None,
                    Radius::Along(radius) => Some(radius.as_ref()),
                };
                let thick = Thick { r2: q8((*widest).max(half)).pow(2), radius, half, short: q8(0.max(min - length)) };
                match &pieces[(1 << level) - 1..(2 << level) - 1] {
                    [piece] => Inside::Piece { piece: *piece, thick },
                    pieces => Inside::Tube { pieces, thick },
                }
            }
            Solid::Cuboid { min: lo, max: hi } => Inside::Cuboid { lo: *lo, hi: *hi, min },
            Solid::Gable { min: lo, max: hi, ridge } => Inside::Gable { lo: *lo, hi: *hi, min, across: 2 - ridge },
        }
    }
}

/** Puts a site: its share, its share of each face's rays where they differ, its pick, and where it is. */
type PutSite<'a> = dyn FnMut(u32, Option<[u32; FACE_KINDS]>, u32, i64, i64, i64) + 'a;

type V3 = [f64; 3];

fn v_dot(a: V3, b: V3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn v_sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

/** Where along the line o + t·u (u unit) `a`·t + `b` ≤ 0, within [t0, t1]. */
fn half(a: f64, b: f64, [t0, t1]: [f64; 2]) -> [f64; 2] {
    if a.abs() < 1e-12 {
        return if b <= 0.0 { [t0, t1] } else { [1.0, 0.0] };
    }
    let t = -b / a;
    if a > 0.0 { [t0, t1.min(t)] } else { [t0.max(t), t1] }
}

/** Where `a`·t² + `b`·t + `c` ≤ 0 (a ≥ 0), as an interval; empty if from > to. */
fn within_quadratic(a: f64, b: f64, c: f64) -> [f64; 2] {
    if a < 1e-12 {
        return half(b, c, [f64::NEG_INFINITY, f64::INFINITY]);
    }
    let disc = b * b - 4.0 * a * c;
    if disc < 0.0 {
        return [1.0, 0.0];
    }
    let root = disc.sqrt();
    [(-b - root) / (2.0 * a), (-b + root) / (2.0 * a)]
}

/**
 * A tube's piece as lines meet it: the points within r of the segment
 * from p, `length` along `axis` (unit), its ends round, or `flat` (cut
 * across it); made once for the lines of a site.
 */
#[derive(Clone, Copy)]
struct Capsule {
    p: V3,
    axis: V3,
    length: f64,
    r: f64,
    flat: [bool; 2],
}

impl Capsule {
    fn new(p: V3, d: V3, r: f64, flat: [bool; 2]) -> Capsule {
        let length = v_dot(d, d).sqrt();
        let axis = if length > 0.0 { d.map(|v| v / length) } else { [0.0; 3] };
        Capsule { p, axis, length, r, flat }
    }

    /** Where the line o + t·u (u unit) is in it; empty if from > to. */
    fn along(&self, o: V3, u: V3) -> [f64; 2] {
        let (w, rr) = (v_sub(o, self.p), self.r * self.r);
        let (wu, ww) = (v_dot(w, u), v_dot(w, w));
        let ball = |wu: f64, ww: f64| within_quadratic(1.0, 2.0 * wu, ww - rr);
        if self.length == 0.0 {
            return ball(wu, ww);
        }
        // Across its axis: within r of the line it is on, which holds its ends' balls too.
        let (ua, wa) = (v_dot(u, self.axis), v_dot(w, self.axis));
        let [c0, c1] = within_quadratic(1.0 - ua * ua, 2.0 * (wu - ua * wa), ww - wa * wa - rr);
        if c0 >= c1 {
            return [1.0, 0.0];
        }
        // Along it: 0 ≤ wa + t·ua ≤ length.
        let [s0, s1] = half(-ua, -wa, half(ua, wa - self.length, [f64::NEG_INFINITY, f64::INFINITY]));
        let (mut from, mut to) = if c0.max(s0) < c1.min(s1) { (c0.max(s0), c1.min(s1)) } else { (f64::INFINITY, f64::NEG_INFINITY) };
        // Its ends' balls, where the line runs past an end.
        if c0 < s0 || c1 > s1 {
            let l = self.length;
            for (flat, wu, ww) in [(self.flat[0], wu, ww), (self.flat[1], wu - ua * l, ww - 2.0 * wa * l + l * l)] {
                let [a, b] = ball(wu, ww);
                if !flat && a < b {
                    (from, to) = (from.min(a), to.max(b));
                }
            }
        }
        [from, to]
    }
}

/** How far point `c` is from the segment from p to p + d, and where along d its nearest point is (0 to 1). */
fn from_segment(c: V3, p: V3, d: V3) -> (f64, f64) {
    let (w, dd) = (v_sub(c, p), v_dot(d, d));
    let t = if dd > 0.0 { (v_dot(w, d) / dd).clamp(0.0, 1.0) } else { 0.0 };
    let off: V3 = std::array::from_fn(|i| w[i] - t * d[i]);
    (v_dot(off, off).sqrt(), t)
}

impl Shape {
    /**
     * Whether the ball of radius r about c is all inside the shape, drawn
     * at points `apart` (fixed point), or all outside: Some(true) or
     * Some(false); None if neither or not known, and then, of a tube, its
     * pieces that reach into it (all its lines need look at, `Lined::along`), into `near`.
     */
    fn about(&self, c: V3, r: f64, apart: Fixed, near: &mut Vec<Capsule>) -> Option<bool> {
        let in_box = |lo: &At, hi: &At| {
            let (lo, hi) = (lo.map(float), hi.map(float));
            if (0..3).any(|i| c[i] + r < lo[i] || c[i] - r >= hi[i]) {
                Some(false)
            } else {
                (0..3).all(|i| c[i] - r >= lo[i] && c[i] + r < hi[i]).then_some(true)
            }
        };
        match &self.solid {
            Solid::Cuboid { min, max } => in_box(min, max),
            Solid::Ellipsoid { centre, radii } => {
                // Scaled by its radii, |c| ≤ ρ: the ellipsoid holds ρ's ellipsoid grown by (1 − ρ) of itself, a ball of
                // (1 − ρ) of its least radius; grown by a ball of r, it is within (1 + r / least) of itself.
                let (centre, radii) = (centre.map(float), radii.map(float));
                let least = radii[0].min(radii[1]).min(radii[2]);
                if least <= 0.0 {
                    return Some(false);
                }
                let rho = (0..3).map(|i| ((c[i] - centre[i]) / radii[i]).powi(2)).sum::<f64>().sqrt();
                if rho >= 1.0 + r / least { Some(false) } else { (rho <= 1.0 - r / least).then_some(true) }
            }
            Solid::Gable { min, max, ridge } => {
                let a = 2 - ridge;
                let (lo, hi) = (min.map(float), max.map(float));
                let (height, width) = (hi[1] - lo[1], hi[a] - lo[a]);
                // Under each slope: side·2·height·x + width·y + … ≤ 0, its normal's length `n`.
                let n = (4.0 * height * height + width * width).sqrt();
                let over = [1.0, -1.0].map(|side| side * (2.0 * c[a] - lo[a] - hi[a]) * height - width * (hi[1] - c[1]));
                if over.iter().any(|f| *f > r * n) {
                    return Some(false);
                }
                match in_box(min, max) {
                    Some(true) => over.iter().all(|f| *f <= -r * n).then_some(true),
                    other => other,
                }
            }
            Solid::Tube { pieces, radius, widest, .. } => {
                let level = (0..).find(|&l| pieces[(1 << l) - 1].stray <= apart / 2).expect("the finest strays nowhere");
                let (mut pending, mut met) = (vec![(0usize, 0usize)], false);
                while let Some((h, l)) = pending.pop() {
                    let piece = &pieces[h];
                    let (p, d) = (piece.p.map(float), piece.segment.d.map(|v| v as f64 / 256.0));
                    let (far, t) = from_segment(c, p, d);
                    if l < level {
                        if far <= float(*widest + piece.stray) + 1e-3 + r {
                            pending.extend([(2 * h + 1, l + 1), (2 * h + 2, l + 1)]);
                        }
                        continue;
                    }
                    let radius = (radius.at(float(piece.t[0])) + radius.at(float(piece.t[1]))) / 2.0;
                    if far > radius + r {
                        continue;
                    }
                    met = true;
                    near.push(Capsule::new(p, d, radius, piece.flat));
                    // All in its capsule: within its radius, and short of a flat end.
                    let length = v_dot(d, d).sqrt();
                    let ends = [piece.flat[0] && t * length < r, piece.flat[1] && (1.0 - t) * length < r];
                    if far + r <= radius && !ends.contains(&true) {
                        return Some(true);
                    }
                }
                if met { None } else { Some(false) }
            }
        }
    }
}

/** A shape's solid in floats, as lines meet it: made once for the lines of all its sites. */
enum Lined {
    Cuboid {
        lo: V3,
        hi: V3,
    },
    Ellipsoid {
        centre: V3,
        inverse: V3,
    },
    Gable {
        lo: V3,
        hi: V3,
        across: usize,
    },
    /** By the pieces near each site (`Shape::about`). */
    Tube,
    Nothing,
}

impl Lined {
    fn of(solid: &Solid) -> Lined {
        match solid {
            Solid::Cuboid { min, max } => Lined::Cuboid { lo: min.map(float), hi: max.map(float) },
            Solid::Ellipsoid { centre, radii } if !radii.contains(&0) => {
                Lined::Ellipsoid { centre: centre.map(float), inverse: radii.map(|r| 1.0 / float(r)) }
            }
            Solid::Ellipsoid { .. } => Lined::Nothing,
            Solid::Gable { min, max, ridge } => Lined::Gable { lo: min.map(float), hi: max.map(float), across: 2 - ridge },
            Solid::Tube { .. } => Lined::Tube,
        }
    }

    /**
     * Where the line o + t·u (u unit, lattice units) is inside the solid,
     * in order, apart, into `out`; a tube by its pieces `near`.
     */
    fn along(&self, o: V3, u: V3, near: &[Capsule], out: &mut Vec<[f64; 2]>) {
        out.clear();
        let box_of = |lo: &V3, hi: &V3| {
            (0..3).fold([f64::NEG_INFINITY, f64::INFINITY], |range, i| {
                // lo ≤ o + t·u < hi along axis i.
                half(-u[i], lo[i] - o[i], half(u[i], o[i] - hi[i], range))
            })
        };
        let range = match self {
            Lined::Cuboid { lo, hi } => box_of(lo, hi),
            Lined::Ellipsoid { centre, inverse } => {
                let (w, v): (V3, V3) =
                    (std::array::from_fn(|i| (o[i] - centre[i]) * inverse[i]), std::array::from_fn(|i| u[i] * inverse[i]));
                within_quadratic(v_dot(v, v), 2.0 * v_dot(w, v), v_dot(w, w) - 1.0)
            }
            Lined::Gable { lo, hi, across: a } => {
                // In its box, under both slopes: ±(2x − lo − hi)·height ≤ width·(top − y) across it.
                let a = *a;
                let (height, width) = (hi[1] - lo[1], hi[a] - lo[a]);
                [1.0, -1.0].into_iter().fold(box_of(lo, hi), |range, side| {
                    half(
                        side * 2.0 * u[a] * height + width * u[1],
                        side * (2.0 * o[a] - lo[a] - hi[a]) * height - width * (hi[1] - o[1]),
                        range,
                    )
                })
            }
            Lined::Tube => {
                out.extend(near.iter().map(|capsule| capsule.along(o, u)).filter(|[a, b]| a < b));
                out.sort_by(|a, b| a[0].total_cmp(&b[0]));
                out.dedup_by(|next, kept| {
                    let touch = next[0] <= kept[1];
                    if touch {
                        kept[1] = kept[1].max(next[1]);
                    }
                    touch
                });
                return;
            }
            Lined::Nothing => return,
        };
        if range[0] < range[1] {
            out.push(range);
        }
    }
}

/**
 * Draws a shape at sites `s` apart: each site its voxel reaches into, as
 * what its lines (grid's `lines`) find of the shape, so the site looks as
 * what of the shape is in it does to rays through it (docs/generators.md,
 * "Levels"). Opaque, how much of each face's rays it meets (a carve, holds
 * all along); not opaque, its
 * chance per level-0 voxel's chord stopping each line by its length in
 * the shape. A shape smaller than the lines are apart every way is seen
 * as a whole: the share of its nearest site's rays its mean shadow is.
 * Returns how many lines it tested.
 */
pub(super) fn draw_shape(shape: &Shape, seed: u64, si: i64, clip: &Box3, put: &mut PutSite) -> u64 {
    let s = si as f64;
    let apart = si * ONE / grid::ACROSS;
    let picked = |x: i64, y: i64, z: i64| to_pick(hash(seed, x * si, y * si, z * si));
    if shape.extents.iter().all(|e| e.to_bits() < apart) {
        let [x, y, z] = nearest_site(shape.centre, si);
        let chance = shape.area / (VOXEL_AREA * s * s);
        if clip.holds([x, y, z]) {
            put(share(chance), None, picked(x, y, z), x, y, z);
        }
        return 1;
    }
    let lines = grid::lines();
    // What it stops per lattice unit of a ray's path, if not opaque: its fill a chance per level-0 voxel's chord.
    let extinction = |x: i64, y: i64, z: i64| match shape.fill {
        Fill::Const(1.0) => None,
        Fill::Const(fill) => Some(-js::log(1.0 - fill) / CHORD),
        Fill::At(_) => Some(-js::log(1.0 - shape.fill.at((x * si) as f64, (y * si) as f64, (z * si) as f64)) / CHORD),
    };
    // The sites whose voxels reach into its bounds: a voxel reaches a spacing from its site along each axis.
    let b = &shape.bounds;
    let [x0, y0, z0] = [0, 1, 2].map(|i| (b.min[i].div_euclid(si) - 1).max(clip.min[i]));
    let [x1, y1, z1] = [0, 1, 2].map(|i| (b.max[i].div_euclid(si) + 1).min(clip.max[i] - 1));
    let (mut tested, mut near, mut along, lined) = (0, Vec::new(), Vec::new(), Lined::of(&shape.solid));
    for z in z0..=z1 {
        for y in y0..=y1 {
            let mut x = x0 + ((x0 + y + z) & 1);
            while x <= x1 {
                // A voxel is within a spacing of its site: a ball all in or all out of the shape needs no lines.
                let centre = [x, y, z].map(|v| (v * si) as f64);
                near.clear();
                let ball = shape.about(centre, s * (1.0 + 1e-9), apart, &mut near);
                if ball == Some(false) || (ball == Some(true) && extinction(x, y, z).is_none()) {
                    if ball == Some(true) {
                        put(SHARE, None, picked(x, y, z), x, y, z);
                    }
                    x += 2;
                    continue;
                }
                tested += lines.lines.len() as u64;
                let mut full = true;
                let mut segments: grid::Segments = Vec::new();
                for (l, &(a, q, [t0, t1])) in lines.lines.iter().enumerate() {
                    let o: V3 = std::array::from_fn(|i| centre[i] + s * q[i]);
                    lined.along(o, lines.dirs[a], &near, &mut along);
                    let held = segments.len();
                    for &[a, b] in along.iter() {
                        let (a, b) = ((a / s).max(t0), (b / s).min(t1));
                        if a < b {
                            segments.push((l as u16, a as f32, b as f32));
                        }
                    }
                    full &=
                        segments.len() == held + 1 && f64::from(segments[held].1) <= t0 + 1e-6 && f64::from(segments[held].2) >= t1 - 1e-6;
                }
                if !segments.is_empty() {
                    let pick = picked(x, y, z);
                    match extinction(x, y, z) {
                        None if full => put(SHARE, None, pick, x, y, z),
                        // What it meets of each face's rays; a carve, what it takes out: those it holds all along.
                        None => {
                            let cover = if shape.op == Op::Carve { grid::through(&segments) } else { grid::met(&segments) };
                            put(share(grid::filled(&segments)), Some(cover.map(share)), pick, x, y, z)
                        }
                        Some(e) => {
                            let stops = grid::medium_stops_along(e, s, &segments).map(share);
                            let cover = stops.iter().any(|c| *c != stops[0]).then_some(stops);
                            put(stops.iter().sum::<u32>() / FACE_KINDS as u32, cover, pick, x, y, z);
                        }
                    }
                }
                x += 2;
            }
        }
    }
    tested
}
