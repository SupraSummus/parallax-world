//! The statics experiments (docs/statics.md), one per argument:
//! cantilever: the longest beam that holds, against beam theory;
//! compare [name]: scenes settled: what fell, at what cost;
//! tree: a leaning tree cut deeper and deeper, does it fall;
//! members: how much of their strength a lattice tower's members use, at
//! every level, whole and with a leg cut;
//! speed: what a tree costs, settled and gathered, level by level;
//! full: a full cell gathered, its beams against `SECTION`;
//! edits [name]: what an edit brings down, at each level, the world as
//! generated standing (MARGIN).

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
    /// Held: what lies this deep in bulk, if not only what the fill holds.
    bulk: Option<f64>,
}

impl Scene {
    fn build(&self, level: u32) -> Lattice {
        let mut cells = solids(level, self.lo, self.hi, &*self.fill);
        if let Some(radius) = self.bulk {
            hold_bulk(level, &mut cells, radius);
        }
        build(level, &cells)
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
        bulk: None,
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
        bulk: None,
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
        bulk: None,
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
        bulk: None,
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
        print!("{:42} {:6} nodes: ", sc.name, sc.build(0).nodes.len());
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
        Some("members") => exp_members(),
        Some("edits") => exp_edits(args.next()),
        Some("full") => exp_full(),
        Some("speed") => exp_speed(),
        _ => eprintln!("cantilever | compare [name] | tree | members | edits [name] | full | speed"),
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
/// its members `thick` across, on four pads of steel; `cut` of its legs
/// gone from 1 m to 2.5 m. Nothing in it is held but what is deep in rock.
fn tower(mat: Mat, cut: usize) -> Scene {
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
        name: format!("{} tower, {cut} legs cut", mat.name),
        lo: [-base / 2.0 - 2.0, -2.5, -base / 2.0 - 2.0],
        hi: [base / 2.0 + 2.0, high + 0.5, base / 2.0 + 2.0],
        fill: Box::new(move |x: V3| {
            for k in 0..4 {
                let foot = leg(k, 0.0);
                if inside(x[1], -2.5, 0.0) && (x[0] - foot[0]).abs() < 1.5 && (x[2] - foot[2]).abs() < 1.5 {
                    return Some((STEEL, false));
                }
            }
            let gone = (0..cut).any(|k| inside(x[1], 1.0, 2.5) && to_segment(x, leg(k, -1.0), leg(k, high)) < thick);
            (!gone && members.iter().any(|(a, b)| to_segment(x, *a, *b) < thick / 2.0)).then_some((mat, false))
        }),
        bulk: Some(0.75),
    }
}

/// How much of its strength the tower's members use, by height, at each
/// level: intact and with a leg cut.
fn exp_members() {
    for cut in [0, 1] {
        let sc = tower(STEEL, cut);
        let fine = sc.build(0);
        println!("## {}: the most of steel's strength used, by height (m)", sc.name);
        println!("          {}", (0..10).map(|k| format!("{:>6}", format!("{}-{}", 3 * k, 3 * k + 3))).collect::<String>());
        let mut lat = fine;
        for level in 0..=4 {
            if level > 0 {
                lat = gathered(&lat);
            }
            let mut band = [0.0f64; 10];
            for (b, u) in lat.bonds.iter().zip(used_standing(&lat)) {
                if b.p[1] >= 0.0 && !(lat.nodes[b.i].held && lat.nodes[b.j].held) {
                    let h = ((b.p[1] / 3.0) as usize).min(9);
                    band[h] = band[h].max(u);
                }
            }
            println!("level {level}   {}", band.iter().map(|u| format!("{u:6.2}")).collect::<String>());
        }
    }
}

/// A short stone beam out of a wall, a small block hanging under its end;
/// `apart`, a layer between them gone.
fn pendant(apart: bool) -> Scene {
    Scene {
        name: format!("a block hanging{}", if apart { ", let go" } else { "" }),
        lo: [-0.6, -0.6, -0.6],
        hi: [1.2, 1.1, 1.1],
        fill: Box::new(move |x: V3| {
            if inside(x[0], -0.5, 0.0) && inside(x[1], -0.5, 1.0) && inside(x[2], -0.5, 1.0) {
                Some((STONE, true))
            } else if inside(x[0], 0.0, 1.0) && inside(x[1], 0.0, 0.5) && inside(x[2], 0.0, 0.5)
                || inside(x[0], 0.5, 0.75) && inside(x[1], -0.375, if apart { -0.125 } else { 0.0 }) && inside(x[2], 0.0, 0.25)
            {
                Some((STONE, false))
            } else {
                None
            }
        }),
        bulk: None,
    }
}

/// The most of its strength any bond uses, all standing as it is.
fn most_used(lat: &Lattice) -> f64 {
    used_standing(lat).into_iter().fold(0.0, f64::max)
}

fn exp_edits(only: Option<String>) {
    println!("## Edits: what falls of the edited, its strength no less than the generated needs ({MARGIN}×)");
    println!("   at level 0 first, uncalibrated, the most used of their strength before and after");
    let cases: Vec<(Scene, Scene)> = vec![
        (tower(STEEL, 0), tower(STEEL, 1)),
        (tower(STEEL, 0), tower(STEEL, 2)),
        (tree(8.0, 1.0, 0.0), tree(8.0, -1.0, 0.3)),
        (tree(8.0, 1.0, 0.0), tree(8.0, -1.0, 0.7)),
        (tree(8.0, 1.0, 0.0), tree(8.0, -1.0, 1.0)),
        (portico(0, 0.5), portico(1, 0.5)),
        (portico(0, 0.5), portico(2, 0.5)),
        (cantilever(STONE, 0.5, 3.0), cantilever(STONE, 0.5, 4.0)),
        (cantilever(STONE, 0.5, 3.0), cantilever(STONE, 0.5, 6.0)),
        (pendant(false), pendant(true)),
    ];
    for (generated, edited) in cases.iter().filter(|(_, e)| only.as_ref().is_none_or(|o| e.name.contains(o.as_str()))) {
        let (mut g, mut e) = (generated.build(0), edited.build(0));
        print!("{:36} used {:.2} -> {:.2}", edited.name, most_used(&g), most_used(&e));
        for level in 0..=4 {
            if level > 0 {
                g = gathered(&g);
                e = gathered(&e);
            }
            let (mut gs, mut es) = (g.clone(), e.clone());
            as_generated(&mut gs, &g);
            as_generated(&mut es, &g);
            let (before, after) = (settle(&gs), settle(&es));
            print!(" | {level}: {:3.0}% {:3.0}%", 100.0 * before.fallen, 100.0 * after.fallen);
            std::io::Write::flush(&mut std::io::stdout()).unwrap();
        }
        println!();
    }
}

/// A cell of stone gathered from full cells, a level and two up: each
/// class of its beams (by the angle between the faces they join), their
/// section over the spacing squared and their strength against stone's.
fn exp_full() {
    println!("## A full cell gathered: section / s², strength / stone's, by the angle between the faces; SECTION {SECTION:?}");
    let mut lat = build(0, &solids(0, [-1.6; 3], [1.6; 3], &|_| Some((STONE, false))));
    for level in 1..=2 {
        lat = gathered(&lat);
        let s = spacing(level);
        let centre = [0.0; 3];
        let mut class: [Vec<(f64, f64)>; 4] = Default::default();
        for b in lat.bonds.chunks(PER_BEAM) {
            let (x, y) = (lat.nodes[b[0].i].x, lat.nodes[b[0].j].x);
            // Only the cell at the middle, whose faces are s/√2 from it.
            if (norm(sub(x, centre)) - s / 2f64.sqrt()).abs() > 1e-9 || (norm(sub(y, centre)) - s / 2f64.sqrt()).abs() > 1e-9 {
                continue;
            }
            let cos = dot(sub(x, centre), sub(y, centre)) / (s * s / 2.0);
            let k = (2.0 + 2.0 * cos).round() as usize;
            class[k].push((4.0 * b[0].a / (s * s), b[0].mat.ft / STONE.ft));
        }
        for (k, angle) in ["180", "120", "90", "60"].iter().enumerate() {
            let c = &class[k];
            let mean = |f: &dyn Fn(&(f64, f64)) -> f64| c.iter().map(f).sum::<f64>() / c.len().max(1) as f64;
            println!("level {level} {angle:>3}°: {:2} beams, section {:.4}, strength {:.3}", c.len(), mean(&|v| v.0), mean(&|v| v.1));
        }
    }
}

/// What an uncut tree costs at each level: gathering it from the level
/// below, settling it.
fn exp_speed() {
    let mut lat = tree(8.0, 1.0, 0.0).build(0);
    for level in 0..=4 {
        let t = Instant::now();
        if level > 0 {
            lat = gathered(&lat);
        }
        let gather = t.elapsed().as_secs_f64();
        let t = Instant::now();
        let o = settle(&lat);
        println!(
            "level {level}: {:6} nodes {:7} beams, gathered in {gather:6.2}s, settled in {:6.2}s ({} rounds)",
            lat.nodes.len(),
            lat.bonds.len() / PER_BEAM,
            t.elapsed().as_secs_f64(),
            o.rounds
        );
    }
}
