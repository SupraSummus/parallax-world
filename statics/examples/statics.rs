//! The statics experiments (docs/statics.md), one per argument:
//! cantilever: the longest beam that holds, against beam theory;
//! compare [name]: scenes settled: what fell, at what cost;
//! tree: a leaning tree cut deeper and deeper, does it fall.

use statics::bodies::*;
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
        _ => eprintln!("cantilever | compare [name] | tree"),
    }
}
