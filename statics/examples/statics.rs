//! The statics experiments (docs/statics.md), one per argument:
//! cantilever, mesh, compare [scene], speed, section, rounds, loads.
//! QUICK=1 leaves the plain coarse models out of compare;
//! BREAK=f breaks, each round, the faces over f times the worst.

#![allow(clippy::needless_range_loop)]

use statics::coarse::*;
use statics::lattice::*;
use statics::model::*;
use std::time::Instant;

fn inside(v: f64, lo: f64, hi: f64) -> bool {
    v >= lo - 1e-9 && v < hi - 1e-9
}

/// A beam of section h×h, length l, out of a held wall at x < 0.
fn cantilever(level: u32, mat: Mat, h: f64, l: f64) -> Lat {
    let f = move |x: V3| {
        if inside(x[0], -0.5, 0.0) && inside(x[1], -0.5, h + 0.5) && inside(x[2], -0.5, h + 0.5) {
            Some((STONE, true))
        } else if inside(x[0], 0.0, l) && inside(x[1], 0.0, h) && inside(x[2], 0.0, h) {
            Some((mat, false))
        } else {
            None
        }
    };
    build(level, [-0.6, -0.6, -0.6], [l + 0.2, h + 0.6, h + 0.6], &f)
}

fn holds(lat: &mut Lat) -> (bool, Outcome) {
    let o = settle(&mut lat.s, false);
    (o.fallen_mass < 1e-6 * o.total_mass, o)
}

/// The longest beam that holds, by bisection on whole steps.
fn longest(level: u32, mat: Mat, h: f64, max: f64) -> (f64, usize) {
    let s = spacing(level);
    let (mut lo, mut hi) = (1i64, (max / s) as i64);
    let mut iters = 0;
    if holds(&mut cantilever(level, mat, h, hi as f64 * s)).0 {
        return (f64::INFINITY, 0);
    }
    while hi - lo > 1 {
        let mid = (lo + hi) / 2;
        let (ok, o) = holds(&mut cantilever(level, mat, h, mid as f64 * s));
        iters += o.iters;
        if ok { lo = mid } else { hi = mid }
    }
    (lo as f64 * s, iters)
}

fn exp_cantilever() {
    println!("## Cantilever, level 0: longest that holds vs beam theory sqrt(ft h / 3 rho g)");
    println!("{:8} {:>6} {:>10} {:>10} {:>8}", "mat", "h[m]", "lattice", "theory", "iters");
    for mat in [STONE, MASONRY, WOOD, SOIL] {
        for h in [0.25, 0.5, 1.0] {
            let theory = (mat.ft * h / (3.0 * mat.rho * G)).sqrt();
            let (l, it) = longest(0, mat, h, (theory * 3.0).clamp(1.0, 60.0));
            println!("{:8} {:>6.2} {:>10.2} {:>10.2} {:>8}", mat.name, h, l, theory, it);
        }
    }
}

fn main() {
    let which = std::env::args().nth(1).unwrap_or_default();
    match which.as_str() {
        "cantilever" => exp_cantilever(),
        "mesh" => exp_mesh(),
        "speed" => exp_speed(),
        "section" => exp_section(),
        "rounds" => exp_rounds(),
        "loads" => exp_loads(),
        "compare" | "" => exp_compare(std::env::args().nth(2)),
        _ => eprintln!("unknown"),
    }
}

struct Scene {
    name: String,
    lo: V3,
    hi: V3,
    fill: Box<dyn Fn(V3) -> Option<(Mat, bool)>>,
}

fn scene_cantilever(mat: Mat, h: f64, l: f64) -> Scene {
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

/// A leaning tree on held ground, cut at 0.5 m from the side `side` (+1: the
/// side it leans to) to `depth` of its diameter.
fn scene_tree(lean_deg: f64, side: f64, depth: f64) -> Scene {
    let (r, h, crown) = (0.3, 8.0, 2.0);
    let t = lean_deg.to_radians();
    let top = [h * t.sin(), h * t.cos(), 0.0];
    let cc = add(top, mul(unit(top), crown * 0.8));
    Scene {
        name: format!("tree lean {lean_deg} cut {} {:.0}%", if side > 0.0 { "front" } else { "back" }, depth * 100.0),
        lo: [-4.0, -0.5, -3.0],
        hi: [6.0, h + 2.0 * crown + 0.5, 3.0],
        fill: Box::new(move |x: V3| {
            if x[1] < 0.0 {
                return if x[0].abs() < 1.0 && x[2].abs() < 1.0 && x[1] >= -0.4 { Some((STONE, true)) } else { None };
            }
            // along the axis
            let s = dot(x, unit(top)).clamp(0.0, h);
            let on = mul(unit(top), s);
            let off = sub(x, on);
            if norm(off) < r && dot(x, unit(top)) <= h {
                // the cut: one layer at 0.5 m
                if inside(x[1], 0.5, 0.5 + UNIT) && side * off[0] > r - 2.0 * r * depth {
                    return None;
                }
                return Some((WOOD, false));
            }
            if norm(sub(x, cc)) < crown {
                return Some((LEAVES, false));
            }
            None
        }),
    }
}

/// A stone slab on four columns; `gone` of them (from the corner +x+z) cut.
fn scene_house(gone: usize, slab: f64) -> Scene {
    let (w, hcol, c) = (6.0, 3.0, 0.5);
    let cols = [[0.0, 0.0], [0.0, w - c], [w - c, 0.0], [w - c, w - c]];
    Scene {
        name: format!("slab {w}m t={slab} on 4 columns, {gone} cut"),
        lo: [-0.5, -0.5, -0.5],
        hi: [w + 0.5, hcol + slab + 0.5, w + 0.5],
        fill: Box::new(move |x: V3| {
            if inside(x[1], -0.25, 0.0) && inside(x[0], -0.5, w + 0.5) && inside(x[2], -0.5, w + 0.5) {
                return Some((STONE, true));
            }
            if inside(x[1], hcol, hcol + slab) && inside(x[0], 0.0, w) && inside(x[2], 0.0, w) {
                return Some((STONE, false));
            }
            for (k, cl) in cols.iter().enumerate() {
                if inside(x[1], 0.0, hcol) && inside(x[0], cl[0], cl[0] + c) && inside(x[2], cl[1], cl[1] + c) {
                    if k >= 4 - gone && inside(x[1], 1.0, 1.0 + UNIT) {
                        return None;
                    }
                    return Some((STONE, false));
                }
            }
            None
        }),
    }
}

/// A rock with a cap of soil overhanging it by `over` on +x.
fn scene_rock(w: f64, over: f64) -> Scene {
    let (h, cap) = (w / 2.0, 1.0);
    Scene {
        name: format!("rock {w}m, soil cap overhanging {over}m"),
        lo: [-0.5, -0.5, -0.5],
        hi: [w + over + 0.5, h + cap + 0.5, w + 0.5],
        fill: Box::new(move |x: V3| {
            if inside(x[1], -0.25, 0.0) && inside(x[0], -0.5, w + over + 0.5) && inside(x[2], -0.5, w + 0.5) {
                return Some((STONE, true));
            }
            if inside(x[1], 0.0, h) && inside(x[0], 0.0, w) && inside(x[2], 0.0, w) {
                return Some((STONE, false));
            }
            if inside(x[1], h, h + cap) && inside(x[0], 0.0, w + over) && inside(x[2], 0.0, w) {
                return Some((SOIL, false));
            }
            None
        }),
    }
}

struct Run {
    fallen: f64,
    nodes: usize,
    work: f64,
    secs: f64,
}

fn report(how: &str, r: &Run) {
    println!("  {how:28} fallen {:6.1}%  nodes {:7}  work {:10.3e}  {:6.2}s", 100.0 * r.fallen, r.nodes, r.work, r.secs);
}

fn work_of(s: &Structure, o: &Outcome) -> f64 {
    (o.iters as f64) * (s.links.iter().filter(|l| !l.broken).count() as f64 + s.nodes.len() as f64)
}

fn run_fixed(s: &mut Structure) -> Run {
    let t = Instant::now();
    let o = settle(s, false);
    Run {
        fallen: o.fallen_mass / o.total_mass,
        nodes: s.nodes.iter().filter(|n| !n.fixed).count(),
        work: work_of(s, &o),
        secs: t.elapsed().as_secs_f64(),
    }
}

/// Resolution driven by forces: start coarse; where a face is loaded over
/// `theta` of its strength between aggregates coarser than level 0, refine
/// both one level; break only between level-0 voxels.
fn run_adaptive(fine: &mut Lat, start: u32, theta: f64, local: bool) -> Run {
    let t = Instant::now();
    let n = fine.s.nodes.len();
    let total: f64 = fine.s.nodes.iter().filter(|n| !n.fixed).map(|n| n.m).sum();
    let mut level = vec![start; n];
    let mut fallen_mass = 0.0;
    let mut work = 0.0;
    let mut rounds = 0;
    let mut last_nodes;
    loop {
        rounds += 1;
        let (_, grounded) = fine.s.components();
        for k in 0..n {
            if !grounded[k] && fine.s.nodes[k].m > 0.0 && !fine.s.nodes[k].fixed {
                fallen_mass += fine.s.nodes[k].m;
                fine.s.nodes[k].m = 0.0;
            }
        }
        for li in 0..fine.s.links.len() {
            let (i, j) = (fine.s.links[li].i, fine.s.links[li].j);
            if !grounded[i] || !grounded[j] {
                fine.s.links[li].broken = true;
            }
        }
        let agg = aggregate(fine, &level);
        last_nodes = agg.s.nodes.iter().filter(|n| !n.fixed && n.m > 0.0).count();
        let f = agg.s.loads();
        let sol = agg.s.solve(&f, None, 1e-7, 200_000);
        work += sol.iters as f64 * (agg.s.links.len() + agg.s.nodes.len()) as f64;
        let u = agg.s.utilisation(&sol.x);
        let mut refine = vec![false; agg.s.nodes.len()];
        let mut any = false;
        if local {
            let skip = vec![false; agg.s.nodes.len()];
            let (inner, w) = local_check(&fine.s, &agg, &sol.x, &skip);
            work += w;
            for (a, &ua) in inner.iter().enumerate() {
                if ua > theta {
                    refine[a] = true;
                }
            }
        }
        for (cl, l) in agg.s.links.iter().enumerate() {
            if u[cl] > theta {
                for a in [l.i, l.j] {
                    if !agg.s.nodes[a].fixed && !refine[a] {
                        refine[a] = true;
                    }
                }
            }
        }
        for k in 0..n {
            if refine[agg.of[k]] && level[k] > 0 {
                level[k] -= 1;
                any = true;
            }
        }
        if any {
            continue;
        }
        let umax = u.iter().cloned().fold(0.0, f64::max);
        if umax <= 1.0 || rounds > 2000 {
            break;
        }
        let frac: f64 = std::env::var("BREAK").ok().and_then(|v| v.parse().ok()).unwrap_or(0.95);
        let cut = (frac * umax).max(1.0);
        for (li, cl) in agg.link_of.iter().enumerate() {
            if let Some(cl) = cl
                && u[*cl] > cut
            {
                fine.s.links[li].broken = true;
            }
        }
    }
    let _ = rounds;
    Run { fallen: fallen_mass / total, nodes: last_nodes, work, secs: t.elapsed().as_secs_f64() }
}

fn compare(sc: &Scene) {
    let fine = build(0, sc.lo, sc.hi, &*sc.fill);
    println!("### {}  ({} voxels at level 0)", sc.name, fine.s.nodes.len());
    let mut f = build(0, sc.lo, sc.hi, &*sc.fill);
    report("reference, level 0", &run_fixed(&mut f.s));
    let quick = std::env::var("QUICK").is_ok();
    for l in 1..=2 {
        if quick {
            break;
        }
        let mut c = build(l, sc.lo, sc.hi, &*sc.fill);
        report(&format!("bodies coarse, level {l}"), &run_fixed(&mut c.s));
    }
    for l in 1..=3 {
        if quick {
            break;
        }
        let all = vec![l; fine.s.nodes.len()];
        let mut a = aggregate(&fine, &all);
        report(&format!("aggregates, level {l}"), &run_fixed(&mut a.s));
    }
    for (l, th, local) in [(3, 0.5, false), (3, 0.25, false), (3, 0.5, true), (3, 0.8, true)] {
        let mut f = build(0, sc.lo, sc.hi, &*sc.fill);
        let how = if local { "global-local" } else { "by forces" };
        report(&format!("{how}, from {l}, θ={th}"), &run_adaptive(&mut f, l, th, local));
    }
}

fn exp_compare(only: Option<String>) {
    let scenes = vec![
        scene_cantilever(STONE, 0.5, 4.5),
        scene_cantilever(STONE, 0.5, 5.5),
        scene_cantilever(MASONRY, 0.5, 1.0),
        scene_cantilever(MASONRY, 0.5, 1.5),
        scene_tree(8.0, 1.0, 0.0),
        scene_tree(8.0, -1.0, 0.7),
        scene_tree(8.0, -1.0, 0.9),
        scene_tree(8.0, 1.0, 0.7),
        scene_house(0, 0.5),
        scene_house(1, 0.5),
        scene_house(2, 0.5),
        scene_rock(6.0, 1.0),
    ];
    for sc in &scenes {
        if let Some(o) = &only {
            if o.starts_with('=') && sc.name != o[1..] {
                continue;
            }
            if !sc.name.contains(o.trim_start_matches('=')) {
                continue;
            }
        }
        compare(sc);
    }
}

/// Does the longest beam depend on the voxel size? The same beam at levels.
fn exp_mesh() {
    println!(
        "## Mesh dependence: stone cantilever, longest that holds, by level (theory {:.2} / {:.2})",
        (STONE.ft * 1.0 / (3.0 * STONE.rho * G)).sqrt(),
        (STONE.ft * 2.0 / (3.0 * STONE.rho * G)).sqrt()
    );
    for (h, levels) in [(1.0, vec![2u32, 1, 0]), (2.0, vec![3, 2, 1])] {
        for l in levels {
            let (len, it) = longest(l, STONE, h, 20.0);
            println!("h={h} level {l} (spacing {:.3}): {len:.2} m  iters {it}", spacing(l));
        }
    }
}

/// How many iterations a solve takes: block Jacobi against the multilevel one.
fn exp_speed() {
    let scenes = vec![
        scene_cantilever(STONE, 0.5, 4.0),
        scene_cantilever(STONE, 0.5, 8.0),
        scene_cantilever(STONE, 0.5, 16.0),
        scene_cantilever(STONE, 1.0, 16.0),
        scene_tree(8.0, 1.0, 0.0),
        scene_house(0, 0.5),
        scene_rock(6.0, 0.0),
    ];
    for sc in &scenes {
        let lat = build(0, sc.lo, sc.hi, &*sc.fill);
        let f = lat.s.loads();
        let t = Instant::now();
        let a = lat.s.solve(&f, None, 1e-7, 400_000);
        let ta = t.elapsed().as_secs_f64();
        println!("### {} ({} voxels)", sc.name, lat.s.nodes.len());
        println!("  block Jacobi:           {:7} iterations {:7.2}s", a.iters, ta);
        for top in [3u32, 5, 6] {
            let t = Instant::now();
            let ml = Multilevel::new(&lat, top);
            let setup = t.elapsed().as_secs_f64();
            let pc = |r: &[[f64; 6]], z: &mut [[f64; 6]]| ml.apply(&lat.s, r, z);
            let b = lat.s.solve_with(&f, None, 1e-7, 400_000, &pc);
            let diff: f64 = a.x.iter().zip(&b.x).map(|(p, q)| (0..6).map(|i| (p[i] - q[i]).powi(2)).sum::<f64>()).sum::<f64>().sqrt();
            let size: f64 = a.x.iter().map(|p| (0..6).map(|i| p[i] * p[i]).sum::<f64>()).sum::<f64>().sqrt();
            println!(
                "  multilevel to level {top}: {:7} iterations {:7.2}s (setup {setup:.2}s), differs {:.1e}",
                b.iters,
                t.elapsed().as_secs_f64(),
                diff / size
            );
        }
    }
}

/// The tree cut deeper and deeper: the reference against two rigid bodies,
/// all above the cut and all below, joined by the faces left in the cut.
fn exp_section() {
    println!("## Tree: does it fall? reference (level 0) vs two bodies split at the cut");
    for lean in [3.0, 8.0] {
        for side in [1.0, -1.0] {
            let mut line_r = String::new();
            let mut line_t = String::new();
            for d in (10..=95).step_by(5) {
                let sc = scene_tree(lean, side, d as f64 / 100.0);
                let mut f = build(0, sc.lo, sc.hi, &*sc.fill);
                let r = run_fixed(&mut f.s);
                let fine = build(0, sc.lo, sc.hi, &*sc.fill);
                let key: Vec<(bool, bool)> = fine.s.nodes.iter().map(|n| (n.fixed, n.x[1] > 0.5)).collect();
                let mut a = aggregate_by(&fine.s, &key);
                let t = run_fixed(&mut a.s);
                line_r.push(if r.fallen > 0.5 { 'X' } else { '.' });
                line_t.push(if t.fallen > 0.5 { 'X' } else { '.' });
            }
            println!(
                "lean {lean}° cut from {:5}  depth 10..95%  reference {line_r}  two bodies {line_t}",
                if side > 0.0 { "front" } else { "back" }
            );
        }
    }
}

/// Progressive failure: how many rounds, and whether breaking more at once
/// changes what falls.
fn exp_rounds() {
    let scenes = vec![
        scene_cantilever(STONE, 0.5, 5.5),
        scene_tree(8.0, -1.0, 0.6),
        scene_tree(8.0, -1.0, 0.55),
        scene_tree(3.0, 1.0, 0.8),
        scene_tree(3.0, 1.0, 0.75),
        scene_house(1, 0.5),
        scene_house(2, 0.5),
    ];
    for sc in &scenes {
        let mut line = format!("{:40}", sc.name);
        for frac in [0.95, 0.5, 0.0] {
            unsafe { std::env::set_var("BREAK", format!("{frac}")) };
            let mut f = build(0, sc.lo, sc.hi, &*sc.fill);
            let t = Instant::now();
            let o = settle(&mut f.s, false);
            line += &format!(
                " | break>{frac}·max: fallen {:5.1}% rounds {:4} {:6.2}s",
                100.0 * o.fallen_mass / o.total_mass,
                o.rounds,
                t.elapsed().as_secs_f64()
            );
        }
        println!("{line}");
    }
}

/// Strength at level 0, loads from coarse levels: the two-body tree, its
/// upper body's mass and centre taken from the voxels of level l.
fn exp_loads() {
    println!("## Tree, two bodies: the load above the cut from level l, strength from level 0");
    for lean in [3.0, 8.0] {
        for side in [1.0, -1.0] {
            let mut lines = vec![String::new(); 5];
            for d in (10..=95).step_by(5) {
                let sc = scene_tree(lean, side, d as f64 / 100.0);
                let fine = build(0, sc.lo, sc.hi, &*sc.fill);
                let key: Vec<(bool, bool)> = fine.s.nodes.iter().map(|n| (n.fixed, n.x[1] > 0.5)).collect();
                for (row, l) in [0u32, 2, 3, 4, 5].iter().enumerate() {
                    let mut a = aggregate_by(&fine.s, &key);
                    if *l > 0 {
                        let c = build(*l, sc.lo, sc.hi, &*sc.fill);
                        let (mut mass, mut mx) = (0.0, [0.0; 3]);
                        for n in c.s.nodes.iter().filter(|n| !n.fixed && n.x[1] > 0.5) {
                            mass += n.m;
                            mx = add(mx, mul(n.x, n.m));
                        }
                        if mass == 0.0 {
                            lines[row].push('?');
                            continue;
                        }
                        let up = (0..a.s.nodes.len()).max_by(|&p, &q| a.s.nodes[p].m.partial_cmp(&a.s.nodes[q].m).unwrap()).unwrap();
                        a.s.nodes[up].m = mass;
                        a.s.nodes[up].x = mul(mx, 1.0 / mass);
                    }
                    let t = run_fixed(&mut a.s);
                    lines[row].push(if t.fallen > 0.5 { 'X' } else { '.' });
                }
            }
            for (row, l) in [0u32, 2, 3, 4, 5].iter().enumerate() {
                println!(
                    "lean {lean}° cut {:5}  load from level {l} ({:4.2} m)  {}",
                    if side > 0.0 { "front" } else { "back" },
                    spacing(*l),
                    lines[row]
                );
            }
        }
    }
}
