//! The statics experiments (docs/statics.md), one per argument:
//! cantilever: the longest beam that holds, against beam theory;
//! compare [name]: scenes settled: what fell, at what cost;
//! tree: a leaning tree cut deeper and deeper, does it fall;
//! tower: a lattice tower, a leg cut, at every level;
//! members: how much of their strength its members use, at every level;
//! sponge: frames with and without braces gathered into sponges;
//! levels [name]: scenes settled at level 0 and gathered coarser.

use statics::bodies::*;
use statics::gather::gathered;
use statics::lattice::*;
use statics::model::*;
use std::time::Instant;

fn inside(v: f64, lo: f64, hi: f64) -> bool {
    v >= lo - 1e-9 && v < hi - 1e-9
}

struct Scene {
    name: String,
    lo: V3,
    hi: V3,
    fill: Box<dyn Fn(V3) -> Option<(Mat, bool)>>,
}

impl Scene {
    fn build(&self, level: u32) -> Lattice {
        build(level, self.lo, self.hi, &*self.fill)
    }
}

/// A beam of section h × h, l long, out of a held wall.
fn cantilever(mat: Mat, h: f64, l: f64) -> Scene {
    Scene {
        name: format!("cantilever {} h={h} L={l}", mat.name),
        lo: [-0.6, -0.6, -0.6],
        hi: [l + 0.2, h + 0.6, h + 0.6],
        fill: Box::new(move |x: V3| {
            if inside(x[0], -0.5, 0.0) && inside(x[1], -0.5, h + 0.5) && inside(x[2], -0.5, h + 0.5) {
                Some((STONE, true))
            } else if inside(x[0], 0.0, l) && inside(x[1], 0.0, h) && inside(x[2], 0.0, h) {
                Some((mat, false))
            } else {
                None
            }
        }),
    }
}

/// A tree leaning `lean`° on held ground, cut through one layer at 0.5 m
/// from the side it leans to (`side` 1) or the other (−1), `depth` of its
/// width.
fn tree(lean: f64, side: f64, depth: f64) -> Scene {
    let (r, h, crown) = (0.3, 8.0, 2.0);
    let axis = [lean.to_radians().sin(), lean.to_radians().cos(), 0.0];
    let middle = mul(axis, h + crown * 0.8);
    Scene {
        name: format!("tree lean {lean} cut {} {:.0}%", if side > 0.0 { "front" } else { "back" }, depth * 100.0),
        lo: [-4.0, -0.5, -3.0],
        hi: [6.0, h + 2.0 * crown + 0.5, 3.0],
        fill: Box::new(move |x: V3| {
            if x[1] < 0.0 {
                return (x[0].abs() < 1.0 && x[2].abs() < 1.0 && x[1] >= -0.4).then_some((STONE, true));
            }
            let along = dot(x, axis);
            let off = sub(x, mul(axis, along.clamp(0.0, h)));
            if norm(off) < r && along <= h {
                let cut = inside(x[1], 0.5, 0.5 + UNIT) && side * off[0] > r - 2.0 * r * depth;
                return (!cut).then_some((WOOD, false));
            }
            (norm(sub(x, middle)) < crown).then_some((LEAVES, false))
        }),
    }
}

/// A stone slab on four stone columns, `cut` of them cut through.
fn portico(cut: usize, slab: f64) -> Scene {
    let (w, high, c) = (6.0, 3.0, 0.5);
    let columns = [[0.0, 0.0], [0.0, w - c], [w - c, 0.0], [w - c, w - c]];
    Scene {
        name: format!("slab {w}m t={slab} on 4 columns, {cut} cut"),
        lo: [-0.5, -0.5, -0.5],
        hi: [w + 0.5, high + slab + 0.5, w + 0.5],
        fill: Box::new(move |x: V3| {
            if inside(x[1], -0.25, 0.0) && inside(x[0], -0.5, w + 0.5) && inside(x[2], -0.5, w + 0.5) {
                return Some((STONE, true));
            }
            if inside(x[1], high, high + slab) && inside(x[0], 0.0, w) && inside(x[2], 0.0, w) {
                return Some((STONE, false));
            }
            for (k, col) in columns.iter().enumerate() {
                if inside(x[1], 0.0, high) && inside(x[0], col[0], col[0] + c) && inside(x[2], col[1], col[1] + c) {
                    return (k < 4 - cut || !inside(x[1], 1.0, 1.0 + UNIT)).then_some((STONE, false));
                }
            }
            None
        }),
    }
}

/// A rock under a cap of soil overhanging it by `over`.
fn rock(w: f64, over: f64) -> Scene {
    let h = w / 2.0;
    Scene {
        name: format!("rock {w}m, soil cap overhanging {over}m"),
        lo: [-0.5, -0.5, -0.5],
        hi: [w + over + 0.5, h + 1.5, w + 0.5],
        fill: Box::new(move |x: V3| {
            if inside(x[1], -0.25, 0.0) && inside(x[0], -0.5, w + over + 0.5) && inside(x[2], -0.5, w + 0.5) {
                Some((STONE, true))
            } else if inside(x[1], 0.0, h) && inside(x[0], 0.0, w) && inside(x[2], 0.0, w) {
                Some((STONE, false))
            } else if inside(x[1], h, h + 1.0) && inside(x[0], 0.0, w + over) && inside(x[2], 0.0, w) {
                Some((SOIL, false))
            } else {
                None
            }
        }),
    }
}

fn settled(sc: &Scene) -> (Outcome, f64) {
    let lat = sc.build(0);
    let t = Instant::now();
    let o = settle(&lat);
    (o, t.elapsed().as_secs_f64())
}

fn falls(sc: &Scene) -> bool {
    settled(sc).0.fallen > 0.5
}

fn exp_cantilever() {
    println!("## The longest cantilever that holds, against beam theory, sqrt(f h / 3 ρ g)");
    println!("   f the material's strength along the beam: the least of ft, fc and 2 fs");
    for mat in [STONE, MASONRY, WOOD] {
        for h in [0.25, 0.5, 1.0] {
            let f = mat.ft.min(mat.fc).min(2.0 * mat.fs);
            let theory = (f * h / (3.0 * mat.rho * GRAVITY)).sqrt();
            let steps = (2.0 * theory / UNIT) as i64;
            let (mut lo, mut hi) = (1, steps);
            while hi - lo > 1 {
                let mid = (lo + hi) / 2;
                // Holds: nothing breaks.
                if settled(&cantilever(mat, h, mid as f64 * UNIT)).0.rounds > 1 { hi = mid } else { lo = mid }
            }
            println!("{:8} h {h:4.2} m: {:6.2} m, theory {theory:6.2} m", mat.name, lo as f64 * UNIT);
        }
    }
}

fn exp_compare(only: Option<String>) {
    let scenes = [
        cantilever(STONE, 0.5, 4.5),
        cantilever(STONE, 0.5, 5.5),
        cantilever(MASONRY, 0.5, 1.0),
        cantilever(MASONRY, 0.5, 1.5),
        tree(8.0, 1.0, 0.0),
        tree(8.0, -1.0, 0.7),
        tree(8.0, 1.0, 0.7),
        portico(0, 0.5),
        portico(1, 0.5),
        portico(2, 0.5),
        rock(6.0, 1.0),
    ];
    for sc in scenes.iter().filter(|s| only.as_ref().is_none_or(|o| s.name.contains(o.as_str()))) {
        print!("{:42} {:6} voxels: ", sc.name, sc.build(0).voxels.len());
        let (o, secs) = settled(sc);
        println!("fallen {:5.1}%  rounds {:3}  work {:9.2e}  {secs:6.2}s", 100.0 * o.fallen, o.rounds, o.work);
    }
}

fn exp_tree() {
    println!("## A leaning tree cut 10..95% deep: X falls, . stands");
    for lean in [3.0, 8.0] {
        for side in [1.0, -1.0] {
            let line: String = (10..=95).step_by(5).map(|d| if falls(&tree(lean, side, d as f64 / 100.0)) { 'X' } else { '.' }).collect();
            println!("lean {lean}° cut {:5}: {line}", if side > 0.0 { "front" } else { "back" });
        }
    }
}

fn main() {
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        Some("cantilever") => exp_cantilever(),
        Some("compare") => exp_compare(args.next()),
        Some("tree") => exp_tree(),
        Some("tower") => exp_tower(),
        Some("members") => exp_members(),
        Some("sponge") => exp_sponge(),
        Some("levels") => exp_levels(args.next()),
        _ => eprintln!("cantilever | compare [name] | tree | tower"),
    }
}

/// The distance from x to the segment a–b.
fn to_segment(x: V3, a: V3, b: V3) -> f64 {
    let ab = sub(b, a);
    let t = (dot(sub(x, a), ab) / dot(ab, ab)).clamp(0.0, 1.0);
    norm(sub(x, add(a, mul(ab, t))))
}

/// A lattice tower, legs from a square `base` wide to one `top` wide,
/// `high`, rings every `panel` and crosses in every face between them,
/// its members `thick` across, on four pads of steel; with `cut`, one leg
/// gone from 1 m to 2.5 m. Nothing in it is held but what is deep in rock.
fn tower(mat: Mat, cut: bool) -> Scene {
    let (base, top, high, panel, thick) = (10.0, 2.0, 30.0, 3.0, 0.3);
    let leg = move |k: usize, y: f64| {
        let w = (base + (top - base) * y / high) / 2.0;
        let (sx, sz) = [(1.0, 1.0), (-1.0, 1.0), (-1.0, -1.0), (1.0, -1.0)][k];
        [sx * w, y, sz * w]
    };
    let mut members = Vec::new();
    for k in 0..4 {
        members.push((leg(k, -1.0), leg(k, high)));
        let mut y = 0.0;
        while y < high - 1e-9 {
            let next = y + panel;
            members.push((leg(k, next), leg((k + 1) % 4, next)));
            members.push((leg(k, y), leg((k + 1) % 4, next)));
            members.push((leg((k + 1) % 4, y), leg(k, next)));
            y = next;
        }
    }
    Scene {
        name: format!("{} tower{}", mat.name, if cut { ", a leg cut" } else { "" }),
        lo: [-base / 2.0 - 2.0, -2.5, -base / 2.0 - 2.0],
        hi: [base / 2.0 + 2.0, high + 0.5, base / 2.0 + 2.0],
        fill: Box::new(move |x: V3| {
            for k in 0..4 {
                let foot = leg(k, 0.0);
                if inside(x[1], -2.5, 0.0) && (x[0] - foot[0]).abs() < 1.5 && (x[2] - foot[2]).abs() < 1.5 {
                    return Some((STEEL, false));
                }
            }
            let gone = cut && inside(x[1], 1.0, 2.5) && to_segment(x, leg(0, -1.0), leg(0, high)) < thick;
            (!gone && members.iter().any(|(a, b)| to_segment(x, *a, *b) < thick / 2.0)).then_some((mat, false))
        }),
    }
}

/// Steel of 0.38 of its strength: where the tower stands whole, at
/// level 0, and falls with a leg cut (see `members`).
const WEAK_STEEL: Mat = Mat { name: "weak steel", rho: 7850.0, e: 200e9, g: 80e9, ft: 95e6, fc: 95e6, fs: 55e6 };

fn exp_tower() {
    println!("## A lattice tower: what falls of it, at level 0 and from its voxels gathered coarser");
    for mat in [WEAK_STEEL] {
        for cut in [false, true] {
            let sc = tower(mat, cut);
            let mut fine = sc.build(0);
            hold_bulk(&mut fine, 0.75);
            let free = fine.voxels.iter().filter(|v| !v.held).count();
            let t = Instant::now();
            let o = settle(&fine);
            println!(
                "{}: level 0, {free} free voxels: fallen {:5.1}%  rounds {}  {:.1}s",
                sc.name,
                100.0 * o.fallen,
                o.rounds,
                t.elapsed().as_secs_f64()
            );
            let mut coarse = fine;
            for level in 1..=4 {
                coarse = gathered(&coarse);
                let free = coarse.voxels.iter().filter(|v| !v.held).count();
                let t = Instant::now();
                let o = settle(&coarse);
                println!(
                    "{:>w$}  level {level} ({:.2} m), {free} free voxels: fallen {:5.1}%  rounds {}  {:.2}s",
                    "",
                    spacing(level),
                    100.0 * o.fallen,
                    o.rounds,
                    t.elapsed().as_secs_f64(),
                    w = sc.name.len()
                );
            }
        }
    }
}

/// How much of its strength the tower's members use, by height, at each
/// level: intact and with a leg cut.
fn exp_members() {
    for cut in [false, true] {
        let sc = tower(STEEL, cut);
        let mut fine = sc.build(0);
        hold_bulk(&mut fine, 0.75);
        println!("## {}: the most of steel's strength used, by height (m)", sc.name);
        println!("          {}", (0..10).map(|k| format!("{:>6}", format!("{}-{}", 3 * k, 3 * k + 3))).collect::<String>());
        let mut lat = fine;
        for level in 0..=4 {
            if level > 0 {
                lat = gathered(&lat);
            }
            let alive = vec![true; lat.bonds.len()];
            let gone = vec![false; lat.voxels.len()];
            let f = forces(&lat, &alive, &gone, &mut Work(0.0));
            let s = stresses(&lat, &f);
            let mut band = [0.0f64; 10];
            for (k, b) in lat.bonds.iter().enumerate() {
                let ends: Vec<usize> = [b.i, b.j].into_iter().filter(|&v| !lat.voxels[v].held).collect();
                if ends.is_empty() || b.p[1] < 0.0 {
                    continue;
                }
                let mean: M3 =
                    std::array::from_fn(|r| std::array::from_fn(|c| ends.iter().map(|&v| s[v][r][c]).sum::<f64>() / ends.len() as f64));
                let u = b.mat.utilisation(&b.stress(&mean, f[k]));
                let h = ((b.p[1] / 3.0) as usize).min(9);
                band[h] = band[h].max(u);
            }
            println!("level {level}   {}", band.iter().map(|u| format!("{u:6.2}")).collect::<String>());
        }
    }
}

/// The distance from x to the nearest multiple of a.
fn off_grid(x: f64, a: f64) -> f64 {
    (x - a * (x / a).round()).abs()
}

/// A block of steel frame, struts of radius r along the edges of cubes a
/// wide; `braced`, one diagonal on every face too.
fn frame(a: f64, r: f64, braced: bool, size: f64) -> impl Fn(V3) -> Option<(Mat, bool)> {
    move |x: V3| {
        if x.iter().any(|v| v.abs() > size / 2.0) {
            return None;
        }
        let d = x.map(|v| off_grid(v, a));
        let edge = (0..3).any(|k| {
            let (p, q) = ((k + 1) % 3, (k + 2) % 3);
            (d[p] * d[p] + d[q] * d[q]).sqrt() < r
        });
        let diagonal = braced
            && (0..3).any(|k| {
                let (p, q) = ((k + 1) % 3, (k + 2) % 3);
                let across = off_grid(x[p] - x[q], a) / 2f64.sqrt();
                (d[k] * d[k] + across * across).sqrt() < r
            });
        (edge || diagonal).then_some((STEEL, false))
    }
}

fn exp_sponge() {
    println!("## Frames of steel, cubes of 1 m, gathered into sponges: a cell of 2 m in the middle of a block,");
    println!("   its stiffness and strength along each of the six directions, as shares of solid steel");
    for r in [0.14, 0.2] {
        for braced in [false, true] {
            let fill = frame(1.0, r, braced, 10.0);
            let mut lat = build(0, [-5.2; 3], [5.2; 3], &fill);
            let fill_share = lat.voxels.len() as f64 / build(0, [-5.0; 3], [5.0; 3], &|_| Some((STEEL, false))).voxels.len() as f64;
            for _ in 0..4 {
                lat = gathered(&lat);
            }
            let middle = (0..lat.voxels.len()).min_by(|&p, &q| norm(lat.voxels[p].x).total_cmp(&norm(lat.voxels[q].x))).unwrap();
            let mut along: Vec<(V3, f64, f64)> = lat
                .bonds
                .iter()
                .filter(|b| b.i == middle || b.j == middle)
                .step_by(PER_FACE)
                .map(|b| (b.n, b.mat.e / STEEL.e, b.mat.ft / STEEL.ft))
                .collect();
            along.sort_by(|a, b| a.0.map(|v| (v * 10.0) as i64).cmp(&b.0.map(|v| (v * 10.0) as i64)));
            along.dedup_by(|a, b| a.0.map(|v| v.abs()) == b.0.map(|v| v.abs()));
            println!(
                "{} r={r}: filled {:.2}; stiffness {}; strength {}",
                if braced { "braced" } else { "frame " },
                fill_share,
                along.iter().map(|a| format!("{:.4}", a.1)).collect::<Vec<_>>().join(" "),
                along.iter().map(|a| format!("{:.3}", a.2)).collect::<Vec<_>>().join(" ")
            );
        }
    }
}

fn exp_levels(only: Option<String>) {
    let scenes = [
        cantilever(STONE, 0.5, 4.5),
        cantilever(STONE, 0.5, 5.5),
        cantilever(STEEL, 0.25, 8.0),
        tree(8.0, 1.0, 0.0),
        tree(8.0, -1.0, 0.7),
        portico(0, 0.5),
        portico(2, 0.5),
    ];
    for sc in scenes.iter().filter(|s| only.as_ref().is_none_or(|o| s.name.contains(o.as_str()))) {
        let mut lat = sc.build(0);
        let mut line = format!("{:36}", sc.name);
        for level in 0..=4 {
            if level > 0 {
                lat = gathered(&lat);
            }
            let t = Instant::now();
            let o = settle(&lat);
            line += &format!(" | {level}: {:5.1}% {:5.2}s", 100.0 * o.fallen, t.elapsed().as_secs_f64());
        }
        println!("{line}");
    }
}
