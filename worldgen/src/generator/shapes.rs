//! Shapes: the solids things are made of, each drawn alone into any box
//! at any level.

use super::*;

/** The area of an opaque body stopping as many rays as one of `area` and mean chord `chord`, filled with chance `fill`. */
fn effective_area(area: f64, chord: f64, fill: &Fill, centre: Point) -> f64 {
    let f = fill.at(centre[0], centre[1], centre[2]);
    area * (1.0 - js::pow(1.0 - f, chord / CHORD))
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

/**
 * Which side of a face a site on it counts as on. A box or span keeps the
 * sites whose centres are in [from, to); on a plane of a level's sites
 * (y = 0 at every level), half its lines of sites have one on each face,
 * kept at the bottom, dropped at the top, so it sinks half a site, more
 * the coarser the level. Level 0's sink, half a unit, is the reference:
 * a site on a face counts as above it, along `axis`, with the chance
 * ½ + 1/(2s), sites s apart: always at level 0, nearly half the time far
 * away, so a face is where level 0 has it on average (docs/generators.md,
 * "Levels"). By a hash of the position and axis alone, so boxes and
 * spans meeting on a face share its sites, and a site above at one level
 * is above at the finer ones.
 */
#[derive(Clone, Copy)]
pub(super) struct Tie {
    s: i64,
    /** Above when the hash is under this, of 2^32. */
    under: u64,
}

impl Tie {
    pub(super) fn new(s: i64) -> Tie {
        Tie { s, under: (1 << 31) + (1 << 31) / s as u64 }
    }

    /**
     * Whether coordinate `axis` of site `p` is in [lo, hi), all in units
     * `one` to a lattice unit. A box one site thick keeps its bottom face's
     * sites, as at level 0: each line across it has a site on one face
     * only, and halves of both would let rays through.
     */
    pub(super) fn within(&self, axis: usize, p: [i64; 3], lo: i64, hi: i64, one: i64) -> bool {
        let v = p[axis];
        if lo < v && v < hi {
            return true;
        }
        if v != lo && v != hi {
            return false;
        }
        if hi - lo <= self.s * one {
            return v == lo;
        }
        let [x, y, z] = p.map(|c| c.div_euclid(one));
        ((hash(TIE_SEEDS[axis], x, y, z) as u64) < self.under) == (v == lo)
    }
}

const TIE_SEEDS: [u64; 3] = [0x7a1e_0001, 0x7a1e_0002, 0x7a1e_0003];

fn dot(a: At, b: At) -> i64 {
    (0..3).map(|i| a[i] * b[i]).sum()
}

pub fn ellipsoid(centre: Point, radii: Point, kind: Kind, fill: Fill, op: Op) -> Shape {
    let [a, b, c] = radii;
    let p = 1.6075; // Knud Thomsen's approximation of the surface area
    let area = 4.0 * PI * js::pow((js::pow(a * b, p) + js::pow(a * c, p) + js::pow(b * c, p)) / 3.0, 1.0 / p);
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
    let caps = if flat { 1.0 } else { 2.0 };
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

/** A shape's test for one `min`, with what does not depend on the point worked out. */
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
    Cuboid {
        lo: At,
        hi: At,
        min: Fixed,
        tie: Tie,
    },
    /** Under the slopes: |2u − lo − hi| · height ≤ width · (top − y), with 8 bits after the point. */
    Gable {
        lo: At,
        hi: At,
        min: Fixed,
        across: usize,
        tie: Tie,
    },
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
            Inside::Cuboid { lo, hi, min, tie } => (0..3).all(|i| {
                if hi[i] - lo[i] >= *min { tie.within(i, x, lo[i], hi[i], ONE) } else { (2 * x[i] - lo[i] - hi[i]).abs() <= *min }
            }),
            Inside::Gable { lo, hi, min, across, tie } => {
                let a = *across;
                let ridge = 2 - a;
                let side = |i: usize| {
                    if hi[i] - lo[i] >= *min { tie.within(i, x, lo[i], hi[i], ONE) } else { (2 * x[i] - lo[i] - hi[i]).abs() <= *min }
                };
                if !side(ridge) || !side(1) || !side(a) {
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
    /** Whether a point is inside the shape with each of its sizes made at least `min`, at sites `s` apart. */
    pub(super) fn inside(&self, min: Fixed, s: i64) -> Inside<'_> {
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
                // The coarsest level within half a site of the curve; level l is pieces 2^l − 1 to 2^(l+1) − 2.
                let level = (0..).find(|&l| pieces[(1 << l) - 1].stray <= s * ONE / 2).expect("the finest strays nowhere");
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
            Solid::Cuboid { min: lo, max: hi } => Inside::Cuboid { lo: *lo, hi: *hi, min, tie: Tie::new(s) },
            Solid::Gable { min: lo, max: hi, ridge } => Inside::Gable { lo: *lo, hi: *hi, min, across: 2 - ridge, tie: Tie::new(s) },
        }
    }
}

/**
 * Draws a shape at sites `s` apart, by how many of its sizes are under s;
 * returns how many sites it tested.
 */
pub(super) fn draw_shape(shape: &Shape, seed: u64, si: i64, clip: &Box3, put: &mut dyn FnMut(u32, u32, i64, i64, i64)) -> u64 {
    let s = si as f64;
    let spacing = si * ONE;
    let small = shape.extents.iter().filter(|e| e.to_bits() < spacing).count();
    let picked = |x: i64, y: i64, z: i64| to_pick(hash(seed, x * si, y * si, z * si));
    if small == 3 {
        let [x, y, z] = nearest_site(shape.centre, si);
        let chance = shape.area / (VOXEL_AREA * s * s);
        if clip.holds([x, y, z]) {
            put(share(chance), picked(x, y, z), x, y, z);
        }
        return 1;
    }
    // A sheet a site thick holds a plane of sites: shapes' sheets lie across an axis, where planes of sites are a site
    // apart (across (1, 1, 1), 2/√3).
    let min = [0, spacing, spacing * 8 / 5][small];
    let b = &shape.bounds;
    // A box's sites on its top faces may count as under them (Tie), past its bounds but at level 0.
    let past = if matches!(shape.solid, Solid::Cuboid { .. } | Solid::Gable { .. }) { 0 } else { ONE };
    let [x0, y0, z0] = b.min.map(|v| -(-(v * ONE - min / 2)).div_euclid(spacing));
    let [x1, y1, z1] = b.max.map(|v| (v * ONE - past + min / 2).div_euclid(spacing));
    let inside = shape.inside(min, si);
    // Visits the sites within that are inside; returns about how many it tested (half of each row's points).
    let sites = |within: &Box3, visit: &mut dyn FnMut(i64, i64, i64)| {
        let (z_from, z_to) = (z0.max(within.min[2]), z1.min(within.max[2] - 1));
        let (y_from, y_to) = (y0.max(within.min[1]), y1.min(within.max[1] - 1));
        let (from, x_to) = (x0.max(within.min[0]), x1.min(within.max[0] - 1));
        for z in z_from..=z_to {
            for y in y_from..=y_to {
                let mut x = from + ((from + y + z) & 1);
                while x <= x_to {
                    if inside.holds([x, y, z].map(|v| v * spacing)) {
                        visit(x, y, z);
                    }
                    x += 2;
                }
            }
        }
        [z_to - z_from, y_to - y_from, (x_to - from).div_euclid(2)].iter().map(|n| (n + 1).max(0) as u64).product::<u64>()
    };
    // A site's share: its fill over `chords` of the level-0 voxel's chord, once a shape if its fill is constant.
    let filled = |chords: f64| {
        let of = move |fill: f64| share(1.0 - js::pow(1.0 - fill, chords));
        let constant = match shape.fill {
            Fill::Const(fill) => Some(of(fill)),
            Fill::At(_) => None,
        };
        move |x: i64, y: i64, z: i64| constant.unwrap_or_else(|| of(shape.fill.at((x * si) as f64, (y * si) as f64, (z * si) as f64)))
    };
    match small {
        0 => {
            let share = filled(s);
            sites(clip, &mut |x, y, z| put(share(x, y, z), picked(x, y, z), x, y, z))
        }
        1 => {
            let share = filled(shape.extents.into_iter().min().expect("three").to_f64() / CHORD);
            sites(clip, &mut |x, y, z| put(share(x, y, z), picked(x, y, z), x, y, z))
        }
        _ => {
            // Counted whole, so every box gets the same chance.
            let mut count = 0;
            let tested = sites(&EVERYWHERE, &mut |_, _, _| count += 1);
            let chance = shape.area / (count as f64 * VOXEL_AREA * s * s);
            tested + sites(clip, &mut |x, y, z| put(share(chance), picked(x, y, z), x, y, z))
        }
    }
}
