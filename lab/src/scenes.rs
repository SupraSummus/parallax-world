//! Scenes to learn from: flat ones (the valley's towns, the same towns
//! turned, villages of winding lanes) and 3D ones (the forest, a town,
//! caves, floating islands), in cells and on the FCC lattice.
//! The forest and town come from worldgen; the villages, caves and
//! islands are drawn here from noise, as shapes no world makes yet.

use worldgen::World;
use worldgen::generator::generate;
use worldgen::grid::Box3;
use worldgen::noise::{Rng, hash64, unit};
use worldgen::units::m;

use crate::image::Image;
use crate::towns::{self, CROP, MARGIN, TownCrop};
use crate::valley;
use crate::vol::Vol;

/** Labels of the volumes in 3D: air, then what the voxels are, coarser than the valley's classes. */
pub struct Label3 {
    pub name: &'static str,
    pub rgb: u32,
}

pub const LABELS3: [Label3; 11] = [
    Label3 { name: "air", rgb: 0xf4f4f8 },
    Label3 { name: "ground", rgb: 0x74a83c },
    Label3 { name: "rock", rgb: 0x7d7d78 },
    Label3 { name: "wood", rgb: 0x5b3d22 },
    Label3 { name: "leaves", rgb: 0x2e5a2a },
    Label3 { name: "undergrowth", rgb: 0x4d8a30 },
    Label3 { name: "roof", rgb: 0xa8432c },
    Label3 { name: "wall", rgb: 0xece6d6 },
    Label3 { name: "street, paving", rgb: 0x8a857c },
    Label3 { name: "fittings, water", rgb: 0x2b4d70 },
    Label3 { name: "fields", rgb: 0xe0c050 },
];
pub const AIR: u16 = 0;
const GROUND: u16 = 1;
const ROCK: u16 = 2;
const WOOD: u16 = 3;
const LEAVES: u16 = 4;

pub fn label3_rgb(l: u16) -> u32 {
    LABELS3[l as usize].rgb
}

/** A valley class's label in 3D (valley.rs's classes). */
fn label3_of_class(class: u8) -> u16 {
    match class {
        0..=2 => 1,
        3 => 2,
        4 => 3,
        5..=7 => 4,
        8 | 9 => 5,
        10 => 6,
        11 => 7,
        12 | 13 => 8,
        14 | 15 => 9,
        _ => 10,
    }
}

/** Bumped whenever world_box or world_lattice draws differently, so that what they cached is drawn again. */
const CACHE_VERSION: u32 = 1;

/** The volume cached at `path` (with CACHE_VERSION), or `make`'s, cached there. */
fn cached(path: &str, n: [usize; 3], make: impl FnOnce() -> Vol) -> Vol {
    let path = format!("{path}-v{CACHE_VERSION}.bin");
    if let Ok(bytes) = std::fs::read(&path)
        && bytes.len() == n[0] * n[1] * n[2]
    {
        return Vol::new(n, bytes.iter().map(|b| *b as u16).collect());
    }
    let v = make();
    if let Some(dir) = std::path::Path::new(&path).parent() {
        std::fs::create_dir_all(dir).expect("cache directory");
    }
    std::fs::write(&path, v.data.iter().map(|l| *l as u8).collect::<Vec<u8>>()).expect("cache written");
    v
}

/** Each material's label: the forest's kinds come first in the valley's, so the valley's classes serve both. */
fn materials(world: &World) -> Vec<u16> {
    static VALLEY_KINDS: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
    let class = valley::class_of_kind(*VALLEY_KINDS.get_or_init(|| worlds::valley::valley(0).generator.kinds.len()));
    let mut material = vec![AIR];
    for (kind, def) in world.generator.kinds.iter().enumerate() {
        material.extend(def.colors.iter().map(|_| label3_of_class(class[kind])));
    }
    material
}

/**
 * A box of `world` at a metre a cell, n metres from (x0, z0) metres and
 * from 12 m under its ground (the median of its columns' topmost ground,
 * rock, street or field) up: generated at level 2 (sites half a metre
 * apart) over all the world's heights, each cell the commonest label of
 * the sites in it that is not air, else air. Cached under `cache` (see `cached`).
 */
pub fn world_box(world: &World, (x0, z0): (i64, i64), n: [usize; 3], cache: &str) -> Vol {
    cached(&format!("{cache}-{x0}-{z0}"), n, || {
        let material = materials(world);
        // Level 2's sites are 4 lattice units apart.
        let (lo, hi) = world.heights_at(2);
        let (lo, hi) = (lo.div_euclid(2) * 2, hi.div_euclid(2) * 2 + 2);
        let tall = [n[0], ((hi - lo) / 2) as usize, n[2]];
        let min = [m(x0 as f64) / 4, lo, m(z0 as f64) / 4];
        let b = Box3 { min, max: [min[0] + 2 * n[0] as i64, hi, min[2] + 2 * n[2] as i64] };
        let grid = generate(&world.generator, &b, 2);
        let mut whole = Vol::filled(tall, AIR);
        let mut count = [0u8; LABELS3.len()];
        for y in 0..tall[1] {
            for z in 0..tall[2] {
                for x in 0..tall[0] {
                    count.iter_mut().for_each(|c| *c = 0);
                    for d in 0..8 {
                        let p = [
                            min[0] + 2 * x as i64 + (d & 1),
                            min[1] + 2 * y as i64 + ((d >> 1) & 1),
                            min[2] + 2 * z as i64 + ((d >> 2) & 1),
                        ];
                        if worldgen::grid::is_site(p[0], p[1], p[2]) {
                            count[material[grid.get(p[0], p[1], p[2]) as usize] as usize] += 1;
                        }
                    }
                    let best = (1..LABELS3.len()).max_by_key(|l| (count[*l], std::cmp::Reverse(*l))).filter(|l| count[*l] > 0);
                    whole.set([x, y, z], best.map_or(AIR, |l| l as u16));
                }
            }
        }
        let mut tops: Vec<usize> = Vec::new();
        for z in 0..n[2] {
            for x in 0..n[0] {
                if let Some(y) = (0..tall[1]).rev().find(|y| matches!(whole.at([x, *y, z]), 1 | 2 | 8 | 10)) {
                    tops.push(y);
                }
            }
        }
        tops.sort_unstable();
        let ground = tops.get(tops.len() / 2).copied().unwrap_or(tall[1] / 2);
        let y0 = ground.saturating_sub(12).min(tall[1] - n[1]);
        whole.crop([0, y0, 0], n)
    })
}

/**
 * A box of `world` on its own lattice at level 3 (sites a metre apart):
 * the points [0, e] on each axis from (x0, z0) metres (both even) and from
 * 12 m under its ground up (the median of its sites' topmost ground,
 * rock, street or field), labels at sites only (masked_fcc.rs's volumes).
 * Cached under `cache` (see `cached`).
 */
pub fn world_lattice(world: &World, (x0, z0): (i64, i64), e: [usize; 3], cache: &str) -> Vol {
    let n = e.map(|v| v + 1);
    cached(&format!("{cache}-{x0}-{z0}"), n, || {
        assert!(x0 % 2 == 0 && z0 % 2 == 0, "even corners keep sites sites");
        let material = materials(world);
        let (lo, hi) = world.heights_at(3);
        let lo = lo.div_euclid(2) * 2;
        let tall = [n[0], (hi - lo) as usize, n[2]];
        let b = Box3 { min: [x0, lo, z0], max: [x0 + n[0] as i64, hi, z0 + n[2] as i64] };
        let grid = generate(&world.generator, &b, 3);
        let mut whole = Vol::filled(tall, AIR);
        for y in 0..tall[1] {
            for z in 0..tall[2] {
                for x in 0..tall[0] {
                    let p = [x0 + x as i64, lo + y as i64, z0 + z as i64];
                    if worldgen::grid::is_site(p[0], p[1], p[2]) {
                        whole.set([x, y, z], material[grid.get(p[0], p[1], p[2]) as usize]);
                    }
                }
            }
        }
        let mut tops: Vec<usize> = Vec::new();
        for z in 0..n[2] {
            for x in 0..n[0] {
                if let Some(y) = (0..tall[1]).rev().find(|y| matches!(whole.at([x, *y, z]), 1 | 2 | 8 | 10)) {
                    tops.push(y);
                }
            }
        }
        tops.sort_unstable();
        let ground = tops.get(tops.len() / 2).copied().unwrap_or(tall[1] / 2);
        // An even start, so the box's sites are the world's.
        let y0 = (ground.saturating_sub(12).min(tall[1] - n[1])) / 2 * 2;
        whole.crop([0, y0, 0], n)
    })
}

/** A volume's labels at sites of the lattice only (x + y + z even), 0 elsewhere. */
pub fn on_lattice(v: &Vol) -> Vol {
    let mut out = v.clone();
    for y in 0..v.n[1] {
        for z in 0..v.n[2] {
            for x in 0..v.n[0] {
                if (x + y + z) % 2 == 1 {
                    out.set([x, y, z], AIR);
                }
            }
        }
    }
    out
}

/** A lattice volume as a dense one to look at: each point not a site takes the site above it (below, at the top). */
pub fn densify(v: &Vol) -> Vol {
    let mut out = v.clone();
    for y in 0..v.n[1] {
        for z in 0..v.n[2] {
            for x in 0..v.n[0] {
                if (x + y + z) % 2 == 1 {
                    let from = if y + 1 < v.n[1] { y + 1 } else { y - 1 };
                    out.set([x, y, z], v.at([x, from, z]));
                }
            }
        }
    }
    out
}

/** Value noise in 3D, in [0, 1], smooth between lattice points `scale` apart. */
fn noise3(seed: u64, p: [f64; 3], scale: f64) -> f64 {
    let q = p.map(|v| v / scale);
    let i = q.map(|v| v.floor() as i64);
    let f = [0, 1, 2].map(|a| {
        let t = q[a] - i[a] as f64;
        t * t * (3.0 - 2.0 * t)
    });
    let mut v = 0.0;
    for c in 0..8 {
        let d = [c & 1, (c >> 1) & 1, (c >> 2) & 1];
        let w: f64 = (0..3).map(|a| if d[a] == 1 { f[a] } else { 1.0 - f[a] }).product();
        v += w * unit((hash64(seed, i[0] + d[0] as i64, i[1] + d[1] as i64, i[2] + d[2] as i64) >> 32) as u32);
    }
    v
}

/** Fractal value noise: octaves from `scale` down, in about [0, 1]. */
fn fbm(seed: u64, p: [f64; 3], scale: f64, octaves: usize) -> f64 {
    let (mut v, mut amp, mut total, mut s) = (0.0, 1.0, 0.0, scale);
    for o in 0..octaves {
        v += amp * noise3(seed.wrapping_add(o as u64 * 7919), p, s);
        total += amp;
        amp *= 0.5;
        s *= 0.5;
    }
    v / total
}

/** Carves (or fills) a ball. */
fn ball(v: &mut Vol, c: [f64; 3], r: f64, label: u16, only_air: bool) {
    let lo = c.map(|x| (x - r).floor().max(0.0) as usize);
    for y in lo[1]..((c[1] + r).ceil() as usize + 1).min(v.n[1]) {
        for z in lo[2]..((c[2] + r).ceil() as usize + 1).min(v.n[2]) {
            for x in lo[0]..((c[0] + r).ceil() as usize + 1).min(v.n[0]) {
                let d2 = (x as f64 + 0.5 - c[0]).powi(2) + (y as f64 + 0.5 - c[1]).powi(2) + (z as f64 + 0.5 - c[2]).powi(2);
                if d2 <= r * r && (!only_air || v.at([x, y, z]) == AIR) {
                    v.set([x, y, z], label);
                }
            }
        }
    }
}

/**
 * Rock under a rolling ground, grass on top, and caves: winding tunnels
 * (worms that turn by noise, wider and narrower), some up to the surface,
 * and a cavern or two where they meet.
 */
pub fn caves(seed: u64, n: [usize; 3]) -> Vol {
    let mut v = Vol::filled(n, AIR);
    let height = |x: usize, z: usize| n[1] as f64 * 0.62 + 9.0 * (fbm(seed, [x as f64, 0.0, z as f64], 48.0, 3) - 0.5) * 2.0;
    for z in 0..n[2] {
        for x in 0..n[0] {
            let g = height(x, z);
            for y in 0..n[1] {
                if (y as f64) < g {
                    v.set([x, y, z], if y as f64 >= g - 1.5 { GROUND } else { ROCK });
                }
            }
        }
    }
    let mut rng = Rng::new(seed ^ 0xca7e);
    let worms = 10 + rng.below(5) as usize;
    let mut points = Vec::new();
    for w in 0..worms {
        let mut p = [rng.next() * n[0] as f64, 6.0 + rng.next() * (n[1] as f64 * 0.45), rng.next() * n[2] as f64];
        let (mut yaw, mut pitch) = (rng.next() * std::f64::consts::TAU, (rng.next() - 0.5) * 0.4);
        let up = rng.next() < 0.3;
        for s in 0..260 {
            let t = s as f64;
            let r = 1.6 + 2.2 * noise3(seed + w as u64, [t, 0.0, 0.0], 30.0);
            let (carve, c) = (r, p);
            let lo = c.map(|x| (x - carve).floor().max(0.0) as usize);
            for y in lo[1]..((c[1] + carve).ceil().max(0.0) as usize + 1).min(n[1]) {
                for z in lo[2]..((c[2] + carve).ceil().max(0.0) as usize + 1).min(n[2]) {
                    for x in lo[0]..((c[0] + carve).ceil().max(0.0) as usize + 1).min(n[0]) {
                        let d2 = (x as f64 + 0.5 - c[0]).powi(2) + (y as f64 + 0.5 - c[1]).powi(2) + (z as f64 + 0.5 - c[2]).powi(2);
                        if d2 <= carve * carve {
                            v.set([x, y, z], AIR);
                        }
                    }
                }
            }
            yaw += (noise3(seed + 31 * w as u64, [t, 1.0, 0.0], 18.0) - 0.5) * 0.5;
            let rise = if up && s > 120 { 0.35 } else { 0.0 };
            pitch = (pitch + (noise3(seed + 57 * w as u64, [t, 2.0, 0.0], 22.0) - 0.5) * 0.2 + rise * 0.1).clamp(-0.5, 0.7);
            p = [p[0] + yaw.cos() * pitch.cos(), p[1] + pitch.sin(), p[2] + yaw.sin() * pitch.cos()];
            if p[1] < 4.0 {
                pitch = pitch.abs();
            }
            // Turned back at the volume's sides, so the tunnels stay in it.
            if p[0] < 3.0 || p[0] > n[0] as f64 - 3.0 {
                yaw = std::f64::consts::PI - yaw;
                p[0] = p[0].clamp(3.0, n[0] as f64 - 3.0);
            }
            if p[2] < 3.0 || p[2] > n[2] as f64 - 3.0 {
                yaw = -yaw;
                p[2] = p[2].clamp(3.0, n[2] as f64 - 3.0);
            }
            if s % 60 == 30 {
                points.push(p);
            }
        }
    }
    for p in points.iter().take(2) {
        let r = 5.0 + rng.next() * 4.0;
        for k in 0..6 {
            let o = [(rng.next() - 0.5) * r, (rng.next() - 0.5) * r * 0.4, (rng.next() - 0.5) * r];
            ball(&mut v, [p[0] + o[0], p[1] + o[1], p[2] + o[2]], r * (0.5 + 0.1 * k as f64), AIR, false);
        }
    }
    v
}

/**
 * Islands in the air: flat grassy tops, rock below tapering to a point
 * (rough with noise), some with a tree or two; nothing under them.
 */
pub fn islands(seed: u64, n: [usize; 3]) -> Vol {
    let mut v = Vol::filled(n, AIR);
    let mut rng = Rng::new(seed ^ 0x151a);
    let count = 3 + rng.below(4) as usize;
    for i in 0..count {
        let r = 7.0 + rng.next() * 15.0;
        let c = [rng.next() * n[0] as f64, n[1] as f64 * (0.35 + rng.next() * 0.45), rng.next() * n[2] as f64];
        let s = seed.wrapping_add(i as u64 * 1013);
        let lo = [(c[0] - 1.4 * r).max(0.0) as usize, (c[2] - 1.4 * r).max(0.0) as usize];
        let hi = [((c[0] + 1.4 * r) as usize + 1).min(n[0]), ((c[2] + 1.4 * r) as usize + 1).min(n[2])];
        for z in lo[1]..hi[1] {
            for x in lo[0]..hi[0] {
                let (dx, dz) = (x as f64 + 0.5 - c[0], z as f64 + 0.5 - c[2]);
                let d = (dx * dx + dz * dz).sqrt() / (r * (0.75 + 0.5 * noise3(s, [dx.atan2(dz) * 4.0, 0.0, 0.0], 3.0)));
                if d >= 1.0 {
                    continue;
                }
                let top = c[1] + 1.5 * (noise3(s + 1, [x as f64, 0.0, z as f64], 9.0) - 0.5);
                let depth = r * (0.6 + 0.6 * noise3(s + 2, [x as f64, 0.0, z as f64], 6.0)) * (1.0 - d * d).powf(0.8);
                for y in 0..n[1] {
                    let yf = y as f64 + 0.5;
                    if yf <= top && yf >= top - depth.max(1.0) {
                        v.set([x, y, z], if yf > top - 1.5 { GROUND } else { ROCK });
                    }
                }
            }
        }
        // Trees on the top, near the middle.
        let trees = rng.below(3);
        for _ in 0..trees {
            let (a, d) = (rng.next() * std::f64::consts::TAU, rng.next() * r * 0.5);
            let (x, z) = (c[0] + a.cos() * d, c[2] + a.sin() * d);
            let (xi, zi) = (x as i64, z as i64);
            let Some(top) = (0..n[1] as i64).rev().find(|y| v.get([xi, *y, zi]) == Some(GROUND)) else { continue };
            let h = 4 + rng.below(4) as i64;
            for y in top + 1..(top + 1 + h).min(n[1] as i64) {
                v.set([xi as usize, y as usize, zi as usize], WOOD);
            }
            ball(&mut v, [x, (top + h) as f64 + 1.0, z], 2.2 + rng.next() * 1.5, LEAVES, true);
        }
    }
    v
}

/** The town's crop turned by an angle (10° to 80°, by `i`), CROP_TURNED wide, mirrored if `mirror`. */
pub const CROP_TURNED: usize = 320;

pub fn turned(t: &TownCrop, i: i64, mirror: bool) -> Image {
    let h = hash64(t.seed, t.n, i, 991);
    let angle = (10.0 + 70.0 * unit((h >> 32) as u32)).to_radians();
    let shift = [((h & 0xff) as f64 / 255.0 - 0.5) * 24.0, (((h >> 8) & 0xff) as f64 / 255.0 - 0.5) * 24.0];
    let src = &t.labels;
    let c = (CROP + 2 * MARGIN) as f64 / 2.0;
    let half = CROP_TURNED as f64 / 2.0;
    let (s, co) = angle.sin_cos();
    let mut out = Image::filled(CROP_TURNED, CROP_TURNED, 0);
    for y in 0..CROP_TURNED {
        for x in 0..CROP_TURNED {
            let xm = if mirror { CROP_TURNED - 1 - x } else { x };
            let (px, py) = (xm as f64 + 0.5 - half + shift[0], y as f64 + 0.5 - half + shift[1]);
            let (sx, sy) = (co * px - s * py + c, s * px + co * py + c);
            let (sx, sy) = ((sx.floor().max(0.0) as usize).min(src.w - 1), (sy.floor().max(0.0) as usize).min(src.h - 1));
            out.set(x, y, src.at(sx, sy));
        }
    }
    out
}

/**
 * A village of winding lanes, a crop CROP wide around it (labels of
 * towns.rs): a few roads out of a middle that wander as they go, lanes
 * off them, an uneven green paved in the middle, houses along both,
 * turned to face the lane, gardens about them, fields further out and
 * forest beyond.
 */
pub fn village(seed: u64) -> Image {
    let n = CROP;
    let (roof, wall, street, paving, canopy, ground, fields) = (0u16, 1u16, 2u16, 3u16, 5u16, 6u16, 7u16);
    let mut rng = Rng::new(seed ^ 0x7111);
    let centre = [n as f64 / 2.0 + (rng.next() - 0.5) * 40.0, n as f64 / 2.0 + (rng.next() - 0.5) * 40.0];
    let dist = |x: f64, y: f64| ((x - centre[0]).powi(2) + (y - centre[1]).powi(2)).sqrt();
    let mut img = Image::filled(n, n, canopy);
    for y in 0..n {
        for x in 0..n {
            let (xf, yf) = (x as f64, y as f64);
            let d = dist(xf, yf) + 40.0 * (fbm(seed, [xf, 0.0, yf], 64.0, 2) - 0.5);
            let forest = fbm(seed + 1, [xf, 0.0, yf], 24.0, 3);
            let field = fbm(seed + 2, [xf, 0.0, yf], 40.0, 2);
            let l = if d < 110.0 {
                if forest > 0.66 { canopy } else { ground }
            } else if d < 175.0 {
                if field > 0.42 {
                    fields
                } else if forest > 0.55 {
                    canopy
                } else {
                    ground
                }
            } else if forest > 0.42 {
                canopy
            } else if field > 0.6 {
                fields
            } else {
                ground
            };
            img.set(x, y, l);
        }
    }
    let mut lanes: Vec<Vec<([f64; 2], f64)>> = Vec::new(); // points and half-width
    let roads = 3 + rng.below(3) as usize;
    let base = rng.next() * std::f64::consts::TAU;
    let walk = |start: [f64; 2], heading: f64, len: f64, half: f64, s: u64| {
        let mut p = start;
        let mut h = heading;
        let mut out = Vec::new();
        let mut t = 0.0;
        while t < len {
            out.push((p, half));
            h += (noise3(s, [t, 0.0, 0.0], 40.0) - 0.5) * 0.09;
            p = [p[0] + h.cos(), p[1] + h.sin()];
            if p[0] < -40.0 || p[1] < -40.0 || p[0] > n as f64 + 40.0 || p[1] > n as f64 + 40.0 {
                break;
            }
            t += 1.0;
        }
        out
    };
    for r in 0..roads {
        let heading = base + std::f64::consts::TAU * r as f64 / roads as f64 + (rng.next() - 0.5) * 0.8;
        lanes.push(walk(centre, heading, 400.0, 2.5, seed + 100 + r as u64));
    }
    let mains = lanes.len();
    for r in 0..mains {
        let branches = rng.below(3) as usize;
        for b in 0..branches {
            let at = 25 + rng.below(110) as usize;
            let Some(&(p, _)) = lanes[r].get(at) else { continue };
            let Some(&(q, _)) = lanes[r].get(at + 1) else { continue };
            let h = (q[1] - p[1]).atan2(q[0] - p[0]);
            let turn = (0.9 + rng.next() * 1.0) * if rng.next() < 0.5 { 1.0 } else { -1.0 };
            let len = 40.0 + rng.next() * 90.0;
            lanes.push(walk(p, h + turn, len, 1.8, seed + 200 + (r * 7 + b) as u64));
        }
    }
    let set_disc = |img: &mut Image, c: [f64; 2], r: f64, l: u16| {
        let (x0, x1) = ((c[0] - r).floor().max(0.0) as usize, ((c[0] + r).ceil().max(0.0) as usize + 1).min(n));
        let (y0, y1) = ((c[1] - r).floor().max(0.0) as usize, ((c[1] + r).ceil().max(0.0) as usize + 1).min(n));
        for y in y0..y1 {
            for x in x0..x1 {
                if (x as f64 + 0.5 - c[0]).powi(2) + (y as f64 + 0.5 - c[1]).powi(2) <= r * r {
                    img.set(x, y, l);
                }
            }
        }
    };
    // Gardens: ground about the lanes near the middle.
    for lane in &lanes {
        for (p, _) in lane {
            if dist(p[0], p[1]) < 150.0 {
                set_disc(&mut img, *p, 14.0, ground);
            }
        }
    }
    for lane in &lanes {
        for (p, half) in lane {
            set_disc(&mut img, *p, *half, street);
        }
    }
    let green = 12.0 + rng.next() * 10.0;
    for k in 0..40 {
        let a = k as f64 / 40.0 * std::f64::consts::TAU;
        let r = green * (0.7 + 0.6 * noise3(seed + 3, [a * 3.0, 0.0, 0.0], 2.0));
        set_disc(&mut img, [centre[0] + a.cos() * r * 0.5, centre[1] + a.sin() * r * 0.5], r * 0.6, paving);
    }
    // Houses along the lanes, facing them.
    for lane in &lanes {
        let mut i = 4usize;
        while i + 2 < lane.len() {
            let (p, half) = lane[i];
            let q = lane[i + 2].0;
            let step = 9 + rng.below(8) as usize;
            if dist(p[0], p[1]) > 155.0 {
                i += step;
                continue;
            }
            let t = [(q[0] - p[0]) / 2.0, (q[1] - p[1]) / 2.0];
            let nrm = [-t[1], t[0]];
            for side in [-1.0, 1.0] {
                if rng.next() > 0.85 {
                    continue;
                }
                let (a, b) = (3.5 + rng.next() * 2.5, 3.0 + rng.next() * 2.0);
                let back = half + 1.5 + rng.next() * 2.0 + b;
                let c = [p[0] + side * nrm[0] * back, p[1] + side * nrm[1] * back];
                // The house's cells: within a along the lane and b across it.
                let reach = (a.max(b) + 2.0).ceil() as i64;
                let mut cells = Vec::new();
                let mut clear = true;
                for dy in -reach..=reach {
                    for dx in -reach..=reach {
                        let (x, y) = (c[0] as i64 + dx, c[1] as i64 + dy);
                        if x < 0 || y < 0 || x >= n as i64 || y >= n as i64 {
                            continue;
                        }
                        let (rx, ry) = (x as f64 + 0.5 - c[0], y as f64 + 0.5 - c[1]);
                        let (u, w) = (rx * t[0] + ry * t[1], rx * nrm[0] + ry * nrm[1]);
                        let l = img.at(x as usize, y as usize);
                        if u.abs() <= a + 1.0 && w.abs() <= b + 1.0 && matches!(l, 0..=3) {
                            clear = false;
                        }
                        if u.abs() <= a && w.abs() <= b {
                            let rim = u.abs() > a - 1.0 || w.abs() > b - 1.0;
                            cells.push((x as usize, y as usize, if rim { wall } else { roof }));
                        }
                    }
                }
                if clear {
                    cells.iter().for_each(|(x, y, l)| img.set(*x, *y, *l));
                }
            }
            i += step;
        }
    }
    img
}

/** The towns' crops as volumes; towns of seeds up to `train` to learn from, the rest held out. */
pub fn towns_flat(all: &[TownCrop], train: u64) -> (Vec<Vol>, Vec<Vol>) {
    let (a, b): (Vec<&TownCrop>, Vec<&TownCrop>) = all.iter().partition(|t| t.seed <= train);
    (towns::training(&a).iter().map(Vol::of_image).collect(), towns::held_out(&b).iter().map(Vol::of_image).collect())
}

pub fn turned_flat(all: &[TownCrop], train: u64) -> (Vec<Vol>, Vec<Vol>) {
    let (a, b): (Vec<&TownCrop>, Vec<&TownCrop>) = all.iter().partition(|t| t.seed <= train);
    let learn = a.iter().flat_map(|t| [(0, false), (1, true)].map(|(i, m)| Vol::of_image(&turned(t, i, m)))).collect();
    let held = b.iter().map(|t| Vol::of_image(&turned(t, 9, false))).collect();
    (learn, held)
}

pub fn villages_flat(train: usize, held: usize) -> (Vec<Vol>, Vec<Vol>) {
    let learn = (0..train as u64).map(|s| Vol::of_image(&village(s))).collect();
    let out = (0..held as u64).map(|s| Vol::of_image(&village(10_000 + s))).collect();
    (learn, out)
}

/**
 * The scenes on the lattice at level 3 (sites a metre apart), as
 * masked_fcc.rs takes them: the forest and a town from worldgen, caves
 * and islands drawn here; volumes to learn from, and held out.
 */
pub fn lattice(name: &str) -> (Vec<Vol>, Vec<Vol>) {
    const BOX: [usize; 3] = [96, 48, 96];
    const TALL: [usize; 3] = [96, 64, 96];
    let dir = format!("{}/masked_fcc", crate::experiment::OUT);
    let corner = |h: u64, spread: u64| ((h % spread) as i64 - spread as i64 / 2).div_euclid(32) * 32;
    match name {
        "forest" => {
            let world = worlds::forest::forest(1);
            let boxes = |seeds: std::ops::Range<u64>| -> Vec<Vol> {
                seeds
                    .map(|s| {
                        let h = hash64(s, 0, 0, 5);
                        world_lattice(&world, (corner(h, 1600), corner(h >> 20, 1600)), BOX, &format!("{dir}/forest-{s}"))
                    })
                    .collect()
            };
            (boxes(0..12), boxes(100..102))
        }
        "town" => {
            let boxes = |seeds: &[u64]| -> Vec<Vol> {
                seeds
                    .iter()
                    .map(|s| {
                        let world = worlds::valley::valley(*s);
                        let (t, _) = valley::land(*s);
                        let unit = worldgen::units::m(1.0);
                        let town = t.near(0).expect("a town");
                        let h = hash64(*s, 0, 0, 6);
                        let (x, z) = (town.x / unit - 48 + corner(h, 100), town.z / unit - 48 + corner(h >> 20, 100));
                        world_lattice(&world, (x.div_euclid(32) * 32, z.div_euclid(32) * 32), BOX, &format!("{dir}/town-{s}"))
                    })
                    .collect()
            };
            (boxes(&(1..=12).collect::<Vec<_>>()), boxes(&[19, 20]))
        }
        "caves" => {
            let make = |s: u64| on_lattice(&caves(s, TALL.map(|v| v + 1)));
            ((0..24).map(make).collect(), (100..104).map(make).collect())
        }
        "islands" => {
            let make = |s: u64| on_lattice(&islands(s, TALL.map(|v| v + 1)));
            ((0..24).map(make).collect(), (100..104).map(make).collect())
        }
        _ => panic!("no scene {name}"),
    }
}
