//! Temporary diagnostics: the lamp's light on the floor, against the reference.
use super::*;

const CACHE: &str = "/tmp/claude-0/-home-user-parallax-world/9e3c453e-31a2-560a-8971-336e916ec7bf/scratchpad/ref-cache.txt";

fn cached(name: &str, world: &World, brightness: Brightness, sites: &[[i64; 3]], samples: u64) -> Vec<f64> {
    let kept = std::fs::read_to_string(CACHE).unwrap_or_default();
    let map: std::collections::HashMap<String, f64> =
        kept.lines().filter_map(|l| l.rsplit_once(" = ").map(|(k, v)| (k.to_string(), v.parse().unwrap()))).collect();
    let mut reference = None;
    let mut add = String::new();
    let out = sites
        .iter()
        .map(|s| {
            let key = format!("{name} {s:?} {samples}");
            *map.get(&key).unwrap_or_else(|| {
                let r = reference.get_or_insert_with(|| reference::Reference::new(world, brightness));
                let v = r.voxel(*s, samples).0[0] as f64;
                add += &format!("{key} = {v}\n");
                Box::leak(Box::new(v))
            })
        })
        .collect();
    if !add.is_empty() {
        std::fs::write(CACHE, kept + &add).unwrap();
    }
    out
}

pub fn room(colour: &'static [&'static str], h: f64, closed: bool) -> World {
    world_of(colour, &[[0.0, h, 0.0]], move |x, y, z, s| {
        let wall = |c: i64| (32..32 + s).contains(&c.abs());
        let within = |c: i64| c.abs() < 32 + s;
        y < 0 || closed && within(x) && within(z) && (0..48 + s).contains(&y) && (wall(x) || wall(z) || y >= 48)
    })
}

fn albedo(colour: &str) -> f64 {
    srgb_to_linear(u8::from_str_radix(&colour[1..3], 16).unwrap() as f32 / 255.0) as f64
}

/** Floor light ours / reference: total and by rings of r (units). */
pub fn budget(name: &str, colour: &'static [&'static str], h: f64, closed: bool, half: i64, step: i64, rings: &[f64], shared: &Shared) -> String {
    let night = Brightness { sky: 0.0, sun: 0.0, glow: 1.0 };
    let a = albedo(colour[0]);
    let mut samples = Vec::new();
    for x in (-half..=half).step_by(step as usize) {
        for z in (-half..=half).step_by(step as usize) {
            let r = ((x * x + z * z) as f64).sqrt();
            if r < *rings.last().unwrap() {
                samples.push((ground_near(x as f64, z as f64), r));
            }
        }
    }
    let sites: Vec<_> = samples.iter().map(|s| s.0).collect();
    let theirs: Vec<f64> = cached(&format!("{name} {colour:?} {h} {closed}"), &room(colour, h, closed), night, &sites, 128).iter().map(|v| v / a).collect();
    let mut light = Light::new(room(colour, h, closed), Shared::new(night).stepping(shared.steps()).settling(shared.tolerance()));
    let ours: Vec<f64> = sites.iter().map(|s| lit(&mut light, *s)[0] as f64 / a).collect();
    let mut line = format!("{name}: total {:.2};", ours.iter().sum::<f64>() / theirs.iter().sum::<f64>());
    for w in rings.windows(2) {
        let k = |v: &Vec<f64>| samples.iter().zip(v).filter(|((_, r), _)| *r >= w[0] && *r < w[1]).map(|(_, o)| o).sum::<f64>();
        line += &format!(" {}-{}: {:.2}", w[0], w[1], k(&ours) / k(&theirs));
    }
    line
}

#[test]
#[ignore]
fn probe() {
    let shared = Shared::new(Brightness { sky: 0.0, sun: 0.0, glow: 1.0 });
    let open = [0.0, 12.0, 24.0, 48.0, 96.0, 192.0];
    let inside = [0.0, 12.0, 24.0, 32.0];
    for h in [24.0] {
        println!("{}", budget(&format!("open h{h}"), &["#808080"], h, false, 192, 6, &open, &shared));
    }
    if std::env::var("ONLY_OPEN").is_ok() { return; }
    println!("{}", budget("black room", &["#080808"], 24.0, true, 30, 3, &inside, &shared));
    println!("{}", budget("white room", &["#f0f0f0"], 24.0, true, 30, 3, &inside, &shared));
}

/** Floor flux of level `level`'s own floor sites, over the reference's (from level 0 samples), within r < `r`. */
pub fn level_flux(name: &str, colour: &'static [&'static str], h: f64, closed: bool, r: f64, shared: &Shared) -> String {
    let night = Brightness { sky: 0.0, sun: 0.0, glow: 1.0 };
    let a = albedo(colour[0]);
    let step = 6i64;
    let half = r as i64;
    let mut sites = Vec::new();
    for x in (-half..=half).step_by(step as usize) {
        for z in (-half..=half).step_by(step as usize) {
            if (((x * x + z * z) as f64).sqrt()) < r {
                sites.push(ground_near(x as f64, z as f64));
            }
        }
    }
    let theirs: f64 = cached(&format!("{name} {colour:?} {h} {closed}"), &room(colour, h, closed), night, &sites, 128).iter().sum::<f64>() / a * (step * step) as f64;
    let mut light = Light::new(room(colour, h, closed), Shared::new(night).stepping(shared.steps()).settling(shared.tolerance()));
    let mut line = format!("{name}:");
    for level in 0..GRID_LEVELS as u32 {
        let s = 1i64 << level;
        let n = (r / s as f64).ceil() as i64;
        let mut flux = 0.0;
        for x in -n..=n {
            for z in -n..=n {
                if (x + z).rem_euclid(2) != 1 || (((x * x + z * z) as f64).sqrt() * s as f64) >= r {
                    continue;
                }
                let site = [x, -1, z];
                let i = level as usize;
                let open: Vec<usize> = (0..N).filter(|&f| !light.whole(i, add(site, FACES[f]))).collect();
                if !light.whole(i, site) || open.is_empty() {
                    continue;
                }
                let faces = light.faces(level, site);
                let mean = open.iter().map(|&f| faces[f][0] as f64).sum::<f64>() / open.len() as f64;
                flux += mean / a * (2 * s * s) as f64;
            }
        }
        line += &format!(" L{level} {:.2}", flux / theirs);
    }
    line
}

#[test]
#[ignore]
fn probe_levels() {
    let shared = Shared::new(Brightness { sky: 0.0, sun: 0.0, glow: 1.0 });
    println!("{}", level_flux("open h24", &["#808080"], 24.0, false, 192.0, &shared));
    println!("{}", level_flux("open h40", &["#808080"], 40.0, false, 192.0, &shared));
}

/** Inner surface sites of the closed room (half 32, height 48), every `step`, with the area each stands for. */
fn room_surfaces(step: i64) -> Vec<([i64; 3], f64, &'static str)> {
    let mut out = Vec::new();
    let a = (step * step) as f64;
    let fix = |p: [i64; 3], free: usize| {
        let mut p = p;
        if (p[0] + p[1] + p[2]).rem_euclid(2) != 0 {
            p[free] += 1;
        }
        p
    };
    for u in (-30..=30).step_by(step as usize) {
        for v in (-30..=30).step_by(step as usize) {
            out.push((fix([u, -1, v], 0), a, "floor"));
            out.push((fix([u, 48, v], 0), a, "ceiling"));
        }
        for y in (1..47).step_by(step as usize) {
            out.push((fix([32, y, u], 1), a, "walls"));
            out.push((fix([-32, y, u], 1), a, "walls"));
            out.push((fix([u, y, 32], 1), a, "walls"));
            out.push((fix([u, y, -32], 1), a, "walls"));
        }
    }
    out
}

#[test]
#[ignore]
fn probe_conservation() {
    let night = Brightness { sky: 0.0, sun: 0.0, glow: 1.0 };
    let colour: &'static [&'static str] = &["#080808"];
    let a = albedo(colour[0]);
    let surfaces = room_surfaces(4);
    let sites: Vec<_> = surfaces.iter().map(|s| s.0).collect();
    let theirs = cached("box black", &room(colour, 24.0, true), night, &sites, 128);
    let steps: usize = std::env::var("STEPS").map_or(1, |v| v.parse().unwrap());
    let mut light = Light::new(room(colour, 24.0, true), Shared::new(night).stepping(steps));
    let mut line = String::from("black box:");
    let mut totals = (0.0, 0.0);
    for part in ["floor", "walls", "ceiling"] {
        let (mut o, mut t) = (0.0, 0.0);
        for ((site, area, p), r) in surfaces.iter().zip(&theirs) {
            if *p != part { continue; }
            let open: Vec<usize> = (0..N).filter(|&f| !light.whole(0, add(*site, FACES[f]))).collect();
            assert!(light.whole(0, *site) && !open.is_empty(), "{site:?} is no surface");
            let faces = light.faces(0, *site);
            o += open.iter().map(|&f| faces[f][0] as f64).sum::<f64>() / open.len() as f64 / a * area;
            t += r / a * area;
        }
        totals = (totals.0 + o, totals.1 + t);
        line += &format!(" {part} {:.2} (of {:.0}%)", o / t, 0.0);
        let _ = t;
    }
    line += &format!(" all {:.2}; ref flux/pi {:.0}", totals.0 / totals.1, totals.1);
    println!("{line}");
}

/** Flux along each direction through lattice planes at growing distance from a lamp in empty space, over the nearest plane's. */
#[test]
#[ignore]
fn probe_beam() {
    let night = Brightness { sky: 0.0, sun: 0.0, glow: 1.0 };
    let y0: i64 = std::env::var("LAMP_Y").map_or(400, |v| v.parse().unwrap());
    let lamp = [0.0, y0 as f64, 0.0];
    let site = [0i64, y0, 0];
    let make = || world(&[lamp], |_, _, _, _| false);
    let steps: usize = std::env::var("STEPS").map_or(1, |v| v.parse().unwrap());
    let ks = [4i64, 8, 12, 16, 24, 32, 40, 48, 64, 80, 96, 128, 160, 192];
    let mut light = Light::new(make(), Shared::new(night).stepping(steps));
    let mut mean = vec![0.0f64; ks.len()];
    for (d, dir) in DIRECTIONS.iter().enumerate() {
        let rest = (0..3).find(|&i| dir[i] != 0).unwrap();
        let (a, b) = match rest { 0 => (1, 2), 1 => (0, 2), _ => (0, 1) };
        let mut line = format!("dir {dir:?}:");
        let mut first = None;
        for (j, &k) in ks.iter().enumerate() {
            let w = k + 24;
            let mut flux = 0.0f64;
            for u in -w..=w {
                for v in -w..=w {
                    let mut q = [0i64; 3];
                    q[a] = u;
                    q[b] = v;
                    q[rest] = (-k - dir[a] * u - dir[b] * v) * dir[rest];
                    let p = add(site, q);
                    if (p[0] + p[1] + p[2]).rem_euclid(2) != 0 { continue; }
                    flux += light.light(0, p)[d][0] as f64;
                }
            }
            let f = *first.get_or_insert(flux);
            mean[j] += flux / f / N as f64;
            line += &format!(" {k}:{:.2}", flux / f);
        }
        if std::env::var("VERBOSE").is_ok() { println!("{line}"); }
    }
    println!("mean over directions: {}", ks.iter().zip(&mean).map(|(k, m)| format!("{k}:{m:.2}")).collect::<Vec<_>>().join(" "));
}

/** Light arriving at the plane y = -1 from the 4 upward directions: with the ground there, over without it. */
#[test]
#[ignore]
fn probe_floor_vs_free() {
    let night = Brightness { sky: 0.0, sun: 0.0, glow: 1.0 };
    let ups: Vec<usize> = (0..N).filter(|&d| DIRECTIONS[d][1] > 0).collect();
    for h in [24.0, 40.0, 56.0] {
        let mut line = format!("h {h}:");
        let mut sums = Vec::new();
        for ground in [false, true] {
            let w = world(&[[0.0, h, 0.0]], move |_, y, _, _| ground && y < 0);
            let mut light = Light::new(w, Shared::new(night));
            let mut by_r = [0.0f64; 4];
            for x in -160i64..=160 {
                for z in -160i64..=160 {
                    if (x + z).rem_euclid(2) != 1 { continue; }
                    let r = ((x * x + z * z) as f64).sqrt();
                    let b = if r < h { 0 } else if r < 2.0 * h { 1 } else if r < 4.0 * h { 2 } else { 3 };
                    let l = light.light(0, [x, -1, z]);
                    by_r[b] += ups.iter().map(|&d| l[d][0] as f64).sum::<f64>();
                }
            }
            sums.push(by_r);
        }
        for b in 0..4 {
            line += &format!(" r/h {}: free {:.0} floor {:.0} ({:.2})", ["<1", "1-2", "2-4", ">4"][b], sums[0][b], sums[1][b], sums[1][b] / sums[0][b]);
        }
        println!("{line}");
    }
}

/** One floor site's light from each upward direction in the black box, as levels are added (LIGHT_TOP). */
#[test]
#[ignore]
fn probe_site() {
    let night = Brightness { sky: 0.0, sun: 0.0, glow: 1.0 };
    let colour: &'static [&'static str] = &["#080808"];
    let mut light = Light::new(room(colour, 24.0, true), Shared::new(night));
    for site in [[1i64, -1, 0], [16, -1, 7], [24, -1, 1], [28, -1, 27], [31, 24, 0], [0, 48, 1], [16, 48, 8]] {
        let l = light.light(0, site);
        let ds: Vec<String> = (0..N).filter(|&d| !light.whole(0, add(site, DIRECTIONS[d]))).map(|d| format!("{:?}:{:.1}", DIRECTIONS[d], l[d][0])).collect();
        println!("{site:?} {}", ds.join(" "));
    }
}

/** Each level's own inner surfaces of the black box, by part: flux (over pi, flat area) over the reference's for that part. */
#[test]
#[ignore]
fn probe_box_levels() {
    let night = Brightness { sky: 0.0, sun: 0.0, glow: 1.0 };
    let colour: &'static [&'static str] = &["#080808"];
    let a = albedo(colour[0]);
    let surfaces = room_surfaces(4);
    let sites: Vec<_> = surfaces.iter().map(|s| s.0).collect();
    let theirs = cached("box black", &room(colour, 24.0, true), night, &sites, 128);
    let parts = ["floor", "walls", "ceiling"];
    let reft: Vec<f64> = parts.iter().map(|part| theirs.iter().zip(&surfaces).filter(|(_, s)| s.2 == *part).map(|(r, s)| r / a * s.1).sum()).collect();
    let mut light = Light::new(room(colour, 24.0, true), Shared::new(night));
    for level in 0..6u32 {
        let i = level as usize;
        let s = 1i64 << level;
        let n = 64 / s + 2;
        let mut flux = [0.0f64; 3];
        for x in -n..=n {
            for y in -n..=n {
                for z in -n..=n {
                    let p = [x, y, z];
                    if (x + y + z).rem_euclid(2) != 0 || !light.whole(i, p) || light.see(i, p, 0).1[0] > 0.0 {
                        continue;
                    }
                    let inside = |q: [i64; 3]| q[0].abs() * s < 32 && q[2].abs() * s < 32 && q[1] * s >= 0 && q[1] * s < 48;
                    let faces = light.faces(level, p);
                    for f in 0..N {
                        let q = add(p, FACES[f]);
                        if !light.whole(i, q) && inside(q) {
                            let part = if FACES[f][1] > 0 && y * s < 0 { 0 } else if FACES[f][1] < 0 && y * s >= 48 { 2 } else { 1 };
                            flux[part] += faces[f][0] as f64 / a * (s * s) as f64 * RHOMBUS / std::f64::consts::SQRT_2;
                        }
                    }
                }
            }
        }
        println!("L{level}: floor {:.2} walls {:.2} ceiling {:.2} all {:.2}", flux[0] / reft[0], flux[1] / reft[1], flux[2] / reft[2], flux.iter().sum::<f64>() / reft.iter().sum::<f64>());
    }
}

/** A rhombic face's area, the site spacing 1 (neighbours √2 apart): the voxel's surface, 12 rhombi of √2 · (√6 / 4)·... */
const RHOMBUS: f64 = std::f64::consts::SQRT_2 / 2.0;

/** Open floor: our light summed over every floor site in rings, against the reference sampled every `step`. */
#[test]
#[ignore]
fn probe_floor_full() {
    let night = Brightness { sky: 0.0, sun: 0.0, glow: 1.0 };
    let colour: &'static [&'static str] = &["#808080"];
    let a = albedo(colour[0]);
    let rings = [0.0, 12.0, 24.0, 48.0, 96.0, 192.0];
    let ring = |r: f64| rings.windows(2).position(|w| r >= w[0] && r < w[1]);
    for (h, closed, cname) in [(24.0, false, "#808080"), (24.0, true, "#080808"), (24.0, true, "#f0f0f0")] {
        let colour: &'static [&'static str] = match cname { "#808080" => &["#808080"], "#080808" => &["#080808"], _ => &["#f0f0f0"] };
        let a = albedo(colour[0]);
        let half: i64 = if closed { 30 } else { 192 };
        let step = 6i64;
        let mut samples = Vec::new();
        for x in (-half..=half).step_by(step as usize) { for z in (-half..=half).step_by(step as usize) {
            let r = ((x * x + z * z) as f64).sqrt();
            if r < 192.0 && (!closed || (x.abs() < 31 && z.abs() < 31)) { samples.push((ground_near(x as f64, z as f64), r)); }
        }}
        let name = if closed { format!("{} {colour:?} {h} {closed}", if cname == "#080808" { "black room" } else { "white room" }) } else { format!("open h{h} {colour:?} {h} {closed}") };
        let sites: Vec<_> = samples.iter().map(|s| s.0).collect();
        let theirs = cached(&name, &room(colour, h, closed), night, &sites, 128);
        let mut t = vec![0.0f64; rings.len() - 1];
        for ((_, r), v) in samples.iter().zip(&theirs) { if let Some(k) = ring(*r) { t[k] += v / a * (step * step) as f64; } }
        let mut light = Light::new(room(colour, h, closed), Shared::new(night));
        let mut o = vec![0.0f64; rings.len() - 1];
        let mut o_sampled = vec![0.0f64; rings.len() - 1];
        for ((s, r), _) in samples.iter().zip(&theirs) { if let Some(k) = ring(*r) { o_sampled[k] += lit(&mut light, *s)[0] as f64 / a * (step * step) as f64; } }
        for x in -half..=half { for z in -half..=half {
            if (x + z).rem_euclid(2) != 1 || closed && (x.abs() >= 31 || z.abs() >= 31) { continue; }
            let r = ((x * x + z * z) as f64).sqrt();
            if let Some(k) = ring(r) { o[k] += lit(&mut light, [x, -1, z])[0] as f64 / a * 2.0; }
        }}
        let tot = |v: &Vec<f64>| v.iter().sum::<f64>();
        println!("{name}: every site {:.2}, sampled {:.2}; by ring every site {}", tot(&o) / tot(&t), tot(&o_sampled) / tot(&t),
            o.iter().zip(&t).map(|(o, t)| format!("{:.2}", o / t)).collect::<Vec<_>>().join(" "));
        let _ = a;
    }
    let _ = a;
}

#[test]
#[ignore]
fn probe_roof() {
    let sky = Brightness { sky: 1.0, sun: 0.0, glow: 0.0 };
    let roof = world(&[], move |x, y, z, s| y < 0 || x.abs() <= 64 && z.abs() <= 64 && (24..24 + 2 * s).contains(&y));
    let mut light = Light::new(roof, Shared::new(sky));
    for site in [[1i64, -1, 0], [41, -1, 0]] {
        let l = light.light(0, site);
        let ds: Vec<String> = (0..N).filter(|&d| DIRECTIONS[d][1] >= 0).map(|d| format!("{:?}:{:.3}", DIRECTIONS[d], l[d][0])).collect();
        println!("{site:?} {}", ds.join(" "));
    }
}
