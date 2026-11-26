//! What lies on the forest's floor at (x, y, z): undergrowth (blades, stems
//! and leaves a unit or two thick), boulders and fallen logs.

use std::sync::Arc;

use worldgen::generator::{Fill, Op, Shape, ball, capsule, ellipsoid};
use worldgen::js::{self, round};
use worldgen::noise::{Rng, fx};

use super::trees::{GroundAt, direction};
use super::{K, PI, k, vox};

fn log_radius() -> f64 {
    vox(0.45)
}

/** A tuft of `count` blades about `tall` high. */
pub(super) fn tuft(x: f64, y: f64, z: f64, count: f64, tall: f64, kind: K, random: &mut Rng) -> Vec<Shape> {
    (0..count as usize)
        .map(|_| {
            let dx = random.below(3) as f64 - 1.0;
            let dz = random.below(3) as f64 - 1.0;
            let blade = 1f64.max(round(tall * (0.6 + random.next() * 0.6)));
            Shape::cuboid([x + dx, y, z + dz], [x + dx + 2.0, y + blade, z + dz + 1.0], kind)
        })
        .collect()
}

const FLOWERS: [K; 4] = [K::flowerYellow, K::flowerViolet, K::flowerWhite, K::flowerRed];

/** A flower: a stem and a head two sites wide. */
pub(super) fn flower(x: f64, y: f64, z: f64, random: &mut Rng) -> Vec<Shape> {
    let stem = 2.0 + random.below(2) as f64;
    let colour = random.pick(&FLOWERS);
    vec![
        Shape::cuboid([x, y, z], [x + 2.0, y + stem, z + 1.0], K::tallGrass),
        Shape::cuboid([x - 1.0, y + stem, z - 1.0], [x + 1.0, y + stem + 1.0, z + 1.0], colour),
    ]
}

/** A fern or bracken clump about a metre across. */
pub(super) fn fern(x: f64, y: f64, z: f64, kind: K, random: &mut Rng) -> Vec<Shape> {
    let fronds = 5.0 + random.below(4) as f64;
    let mut shapes = Vec::new();
    for i in 0..fronds as usize {
        let angle = (i as f64 / fronds) * PI * 2.0 + random.next() * 0.5;
        let (c, s) = (js::cos(angle), js::sin(angle));
        let length = vox(0.45 + random.next() * 0.35);
        let rise = vox(0.25 + random.next() * 0.2);
        let bend = [x + c * length * 0.5, y + rise, z + s * length * 0.5];
        shapes.push(capsule([x, y, z], bend, 1.1, k(kind), Fill::Const(0.85), Op::Fill));
        shapes.push(capsule(bend, [x + c * length, y + rise * 0.3, z + s * length], 1.0, k(kind), Fill::Const(0.7), Op::Fill));
    }
    shapes
}

/** A bush, half the time with red berries on its upper side. */
pub(super) fn bush(x: f64, y: f64, z: f64, random: &mut Rng) -> Vec<Shape> {
    let radius = vox(0.5 + random.next() * 0.5);
    let mut shapes = vec![ball([x, y, z], radius, k(K::bush), Fill::Const(0.8), Op::Fill)];
    if random.chance(fx(0.5)) {
        let mut n = 4.0 + random.below(6) as f64;
        while n > 0.0 {
            let [dx, dy, dz] = direction(random);
            let (px, py, pz) = (round(x + dx * radius * 0.9), round(y + dy.abs() * radius * 0.9), round(z + dz * radius * 0.9));
            let at = [px, py, pz];
            shapes.push(Shape::cuboid(at, [at[0] + 1.0, at[1] + 1.0, at[2] + 1.0], K::berry).paint());
            n -= 1.0;
        }
    }
    shapes
}

/** One to three mushrooms: fly agarics (white-spotted red caps) or boletes. */
pub(super) fn mushrooms(x: f64, y: f64, z: f64, kind: K, random: &mut Rng) -> Vec<Shape> {
    let mut shapes = Vec::new();
    let mut n = 1.0 + random.below(3) as f64;
    while n > 0.0 {
        let mx = x + random.below(5) as f64 - 2.0;
        let mz = z + random.below(5) as f64 - 2.0;
        let stem = 1.0 + random.below(2) as f64;
        let cap = [mx + 0.5, y + stem, mz];
        let rx = 1.4 + random.next() * 0.4;
        let radii = [rx, 1.0, 1.4 + random.next() * 0.4];
        shapes.push(Shape::cuboid([mx, y, mz], [mx + 2.0, y + stem, mz + 1.0], K::mushroomStem));
        shapes.push(Shape::ellipsoid(cap, radii, kind));
        if kind == K::amanita {
            let top = cap[1];
            shapes.push(ellipsoid(
                cap,
                radii,
                k(K::amanitaSpot),
                Fill::At(Arc::new(move |_, py, _| if py > top { 0.35 } else { 0.0 })),
                Op::Paint,
            ));
        }
        n -= 1.0;
    }
    shapes
}

/** A boulder's radii across, in metres: the least and the most. */
pub(super) const BOULDER_RADIUS: [f64; 2] = [0.8, 2.2];

/** A half-buried rock with moss on its upper side. */
pub(super) fn boulder(x: f64, ground: f64, z: f64, random: &mut Rng) -> Vec<Shape> {
    let [least, most] = BOULDER_RADIUS;
    let rx = vox(least + random.next() * (most - least));
    let ry = rx * (0.6 + random.next() * 0.3);
    let rz = vox(least + random.next() * (most - least));
    vec![
        Shape::ellipsoid([x, ground, z], [rx, ry, rz], K::stone).paint(),
        ellipsoid(
            [x, ground, z],
            [rx, ry, rz],
            k(K::mossyStone),
            Fill::At(Arc::new(move |_, y, _| if y - ground > ry * 0.4 { 0.7 } else { 0.0 })),
            Op::Paint,
        ),
    ]
}

/** A fallen trunk lying across the ground, with moss on top. */
/**
 * A fallen log from (x, z) and moss on it; none where the ground under
 * it falls over FALL from end to end, as on a mountainside: it would
 * roll, and reach past its population's reach.
 */
pub(super) fn fallen_log(x: f64, z: f64, ground: GroundAt, random: &mut Rng) -> Option<Vec<Shape>> {
    const FALL: f64 = 4.0;
    let length = vox(4.0 + random.next() * 5.0);
    let angle = random.next() * PI;
    let dx = js::cos(angle);
    let dz = js::sin(angle);
    let (ex, ez) = (x + dx * length, z + dz * length);
    let lift = log_radius();
    let at = |x: f64, z: f64| ground(x as i64, z as i64) as f64;
    let heights = (0..=(length / vox(0.5)) as usize).map(|i| at(round(x + dx * i as f64 * vox(0.5)), round(z + dz * i as f64 * vox(0.5))));
    let (low, high) = heights.chain([at(round(ex), round(ez))]).fold((f64::MAX, f64::MIN), |(l, h), y| (l.min(y), h.max(y)));
    if high - low > vox(FALL) {
        return None;
    }
    let mut shapes =
        vec![Shape::capsule([x, at(x, z) + lift, z], [ex, at(round(ex), round(ez)) + lift, ez], log_radius(), K::deadwood).paint()];
    // Moss in patches on top.
    let mut i = 0.0;
    while i <= length {
        let px = round(x + dx * i);
        let pz = round(z + dz * i);
        if random.chance(fx(0.5)) {
            shapes.push(Shape::ball([px, at(px, pz) + lift * 2.0 + vox(0.25), pz], vox(0.3), K::moss));
        }
        i += vox(0.5);
    }
    Some(shapes)
}
