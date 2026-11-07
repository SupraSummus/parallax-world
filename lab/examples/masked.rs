//! One learner for every scene (masked.rs): flat towns, turned towns,
//! villages of winding lanes; in 3D the forest, a town, caves, islands.
//! The same options for all but the shape of the data. For each, the
//! model's held-out bits per level, statistics of samples against held-out
//! scenes, and whether a window drawn alone is that window of the whole;
//! for the flat ones, the split grammar (grammar.rs) beside it. Pictures
//! to out/masked/.
//!
//! cargo run --release --example masked [-- scenes...]
//! (scenes: towns turned villages forest town caves islands; all by default; `show` only draws the data)

use std::time::Instant;

use lab::OUT;
use lab::geometry::Geometry;
use lab::grammar::Grammar;
use lab::image::write_grid;
use lab::masked::{Model, Options};
use lab::scenes::{self, AIR, LABELS3, label3_rgb};
use lab::stats;
use lab::towns::{self, LABELS, ROOF, STREET, label_rgb};
use lab::vol::Vol;
use worldgen::noise::hash64;

const TRAIN_SEEDS: u64 = 18;
const ALL_SEEDS: u64 = 21;
const SAMPLES: u64 = 4;
const BOX3: [usize; 3] = [96, 48, 96];

fn flat_options() -> Options {
    Options {
        levels: 6,
        codes: 32,
        subdivisions: 4,
        radius: [2, 0, 2],
        steps: 16,
        hidden: 64,
        examples: 300_000,
        epochs: 2,
        position: [true, false, true],
        augment: false,
        seed: 1,
    }
}

fn solid_options() -> Options {
    Options { levels: 4, subdivisions: 2, radius: [2, 2, 2], position: [false, true, false], ..flat_options() }
}

struct Scene {
    name: String,
    train: Vec<Vol>,
    held: Vec<Vol>,
    flat: bool,
    /** Whether it faces no way, so its turns and mirrors are as good to learn from (the towns face their path). */
    augment: bool,
}

fn scene(name: &str) -> Scene {
    let dir = format!("{OUT}/masked");
    let start = Instant::now();
    let towns = || towns::gather(&(1..=ALL_SEEDS).collect::<Vec<_>>(), -2..=2, 2, &format!("{OUT}/towns"));
    let (train, held, flat) = match name {
        "towns" => {
            let (a, b) = scenes::towns_flat(&towns(), TRAIN_SEEDS);
            (a, b, true)
        }
        "turned" => {
            let (a, b) = scenes::turned_flat(&towns(), TRAIN_SEEDS);
            (a, b, true)
        }
        "villages" => {
            let (a, b) = scenes::villages_flat(160, 12);
            (a, b, true)
        }
        "caves" => {
            ((0..24).map(|s| scenes::caves(s, [96, 64, 96])).collect(), (100..104).map(|s| scenes::caves(s, [96, 64, 96])).collect(), false)
        }
        "islands" => (
            (0..24).map(|s| scenes::islands(s, [96, 64, 96])).collect(),
            (100..104).map(|s| scenes::islands(s, [96, 64, 96])).collect(),
            false,
        ),
        "forest" => {
            let boxes = |seeds: std::ops::Range<u64>| -> Vec<Vol> {
                seeds
                    .map(|s| {
                        let world = worlds::forest::forest(1);
                        let h = hash64(s, 0, 0, 5);
                        let (x, z) = ((h % 1600) as i64 - 800, ((h >> 20) % 1600) as i64 - 800);
                        scenes::world_box(&world, (x, z), BOX3, &format!("{dir}/forest-{s}"))
                    })
                    .collect()
            };
            (boxes(0..12), boxes(100..102), false)
        }
        "town" => {
            let boxes = |seeds: &[u64]| -> Vec<Vol> {
                seeds
                    .iter()
                    .map(|s| {
                        let world = worlds::valley::valley(*s);
                        let t = lab::valley::towns(*s);
                        let unit = worldgen::units::m(1.0);
                        let town = t.near(0).expect("a town");
                        let h = hash64(*s, 0, 0, 6);
                        let (x, z) = (town.x / unit - 48 + (h % 100) as i64 - 50, town.z / unit - 48 + ((h >> 20) % 100) as i64 - 50);
                        scenes::world_box(&world, (x, z), BOX3, &format!("{dir}/town-{s}"))
                    })
                    .collect()
            };
            (boxes(&(1..=12).collect::<Vec<_>>()), boxes(&[19, 20]), false)
        }
        _ => panic!("no scene {name}"),
    };
    eprintln!(
        "{name}: {} to learn from, {} held out, {:?}, in {:.1} s",
        train.len(),
        held.len(),
        train[0].n,
        start.elapsed().as_secs_f64()
    );
    let augment = !matches!(name, "towns" | "town");
    Scene { name: name.to_string(), train, held, flat, augment }
}

type Picture = (usize, usize, Vec<u8>);

fn picture(v: &Vol, flat: bool) -> Vec<Picture> {
    if flat {
        vec![v.top_rgb(&label_rgb, u16::MAX, 1)]
    } else {
        vec![v.top_rgb(&label3_rgb, AIR, 2), v.slice_rgb(v.n[2] / 2, &label3_rgb, AIR, 2), v.level_rgb(v.n[1] / 3, &label3_rgb, AIR, 2)]
    }
}

/** Shares and co-occurrence at distances d along every axis, pooled over `vols`. */
fn pooled(vols: &[Vol], n: usize, d: usize) -> (Vec<f64>, Vec<f64>) {
    let mut shares = vec![0.0; n];
    let mut joint = vec![0.0; n * n];
    let mut total = 0.0f64;
    for v in vols {
        for (i, l) in v.data.iter().enumerate() {
            shares[*l as usize] += 1.0;
            let p = [i % v.n[0], i / (v.n[0] * v.n[2]), (i / v.n[0]) % v.n[2]];
            for a in 0..3 {
                if v.n[a] == 1 {
                    continue;
                }
                let mut q = p;
                q[a] += d;
                if q[a] < v.n[a] {
                    joint[*l as usize * n + v.at(q) as usize] += 1.0;
                    total += 1.0;
                }
            }
        }
    }
    let cells: f64 = shares.iter().sum();
    shares.iter_mut().for_each(|s| *s /= cells);
    joint.iter_mut().for_each(|j| *j /= total.max(1.0));
    (shares, joint)
}

/** 6-connected parts of cells `wanted`: their number per volume, and the largest's share of them. */
fn parts(vols: &[Vol], wanted: &dyn Fn(u16) -> bool) -> (f64, f64) {
    let (mut count, mut largest) = (0.0, 0.0);
    for v in vols {
        let mut seen = vec![false; v.data.len()];
        let (mut big, mut cells, mut k) = (0usize, 0usize, 0usize);
        for s in 0..v.data.len() {
            if seen[s] || !wanted(v.data[s]) {
                continue;
            }
            seen[s] = true;
            let mut stack = vec![s];
            let mut size = 0;
            while let Some(i) = stack.pop() {
                size += 1;
                let p = [i % v.n[0], i / (v.n[0] * v.n[2]), (i / v.n[0]) % v.n[2]];
                for a in 0..3 {
                    for dir in [-1i64, 1] {
                        let mut q = p.map(|x| x as i64);
                        q[a] += dir;
                        if let Some(l) = v.get(q)
                            && wanted(l)
                        {
                            let j = v.index(q.map(|x| x as usize));
                            if !seen[j] {
                                seen[j] = true;
                                stack.push(j);
                            }
                        }
                    }
                }
            }
            big = big.max(size);
            cells += size;
            k += (size >= 8) as usize;
        }
        count += k as f64;
        largest += big as f64 / cells.max(1) as f64;
    }
    (count / vols.len() as f64, largest / vols.len() as f64)
}

fn report(rows: &[(String, Vec<Vol>)], n: usize, flat: bool) {
    let real = &rows[0].1;
    let dists = if flat { [1, 4, 16, 64] } else { [1, 2, 4, 16] };
    let reference: Vec<(Vec<f64>, Vec<f64>)> = dists.iter().map(|d| pooled(real, n, *d)).collect();
    let header = dists.iter().map(|d| format!("TV joint d={d}")).collect::<Vec<_>>().join(" | ");
    if flat {
        println!(
            "| map | TV shares | {header} | street run m | street parts | largest street part | roofs | roof median m² | roof rectangular |"
        );
        println!("|---|---|---|---|---|---|---|---|---|---|---|---|---|");
    } else {
        println!("| volume | TV shares | {header} | solid parts | largest solid | air parts | largest air |");
        println!("|---|---|---|---|---|---|---|---|---|---|---|");
    }
    for (name, vols) in rows {
        let tvs: Vec<String> =
            dists.iter().zip(&reference).map(|(d, r)| format!("{:.3}", stats::tv(&pooled(vols, n, *d).1, &r.1))).collect();
        let tv_shares = stats::tv(&pooled(vols, n, 1).0, &reference[0].0);
        if flat {
            let g = Geometry::mean(&vols.iter().map(|v| Geometry::of(&v.to_image(), STREET, ROOF)).collect::<Vec<_>>());
            println!(
                "| {name} | {tv_shares:.3} | {} | {:.1} | {:.1} | {:.2} | {:.1} | {:.0} | {:.2} |",
                tvs.join(" | "),
                g.street_run,
                g.street_parts,
                g.street_largest,
                g.roofs,
                g.roof_size,
                g.roof_rectangular
            );
        } else {
            let (sp, sl) = parts(vols, &|l| l != AIR);
            let (ap, al) = parts(vols, &|l| l == AIR);
            println!("| {name} | {tv_shares:.3} | {} | {sp:.1} | {sl:.2} | {ap:.1} | {al:.2} |", tvs.join(" | "));
        }
    }
}

fn run(s: &Scene) {
    let dir = format!("{OUT}/masked");
    std::fs::create_dir_all(&dir).expect("out dir");
    let (n, options) = if s.flat { (LABELS.len(), flat_options()) } else { (LABELS3.len(), solid_options()) };
    let options = Options { augment: s.augment, ..options };
    let start = Instant::now();
    let model = Model::learn(&s.train, &s.held, n, options);
    println!("\n## {}\n", s.name);
    println!(
        "{} volumes of {:?} to learn from, {} held out. Model: {} parameters, learned in {:.0} s. {}\n",
        s.train.len(),
        s.train[0].n,
        s.held.len(),
        model.parameters(),
        start.elapsed().as_secs_f64(),
        model.describe()
    );
    println!(
        "Held-out bits a cell, level 0 (labels) to the top: {}\n",
        model.bits.iter().map(|b| format!("{b:.3}")).collect::<Vec<_>>().join(", ")
    );
    let start = Instant::now();
    let samples: Vec<Vol> = (0..SAMPLES).map(|seed| model.sample(seed, [0; 3], model.size())).collect();
    println!("Sampling: {:.1} s a volume.\n", start.elapsed().as_secs_f64() / SAMPLES as f64);

    // A window drawn alone against the same window of the whole.
    let size = model.size();
    let at = size.map(|v| v / 3);
    let w = size.map(|v| (v / 4).max(1));
    let alone = model.sample(0, at, w);
    let same = alone == samples[0].crop(at, w);
    println!("Window {w:?} at {at:?} drawn alone is that window of the whole: {same}.\n");
    assert!(same, "locality");

    let mut rows: Vec<(String, Vec<Vol>)> =
        vec![("held out".into(), s.held.clone()), ("training (some)".into(), s.train.iter().take(s.held.len().max(4)).cloned().collect())];
    rows.push(("masked model".into(), samples.clone()));
    if s.flat {
        let images: Vec<_> = s.train.iter().map(|v| v.to_image()).collect();
        let mut grammar = Grammar::learn(&images, n, 32, 1);
        (grammar.shared, grammar.matching, grammar.snap) = (true, true, lab::grammar::SNAP);
        let side = images[0].w;
        let drawn: Vec<Vol> = (0..SAMPLES).map(|seed| Vol::of_image(&grammar.sample(seed, (0, 0, side, side)).image)).collect();
        rows.push(("split grammar".into(), drawn));
    }
    report(&rows, n, s.flat);

    let grid: Vec<Vec<Picture>> =
        rows.iter().map(|(_, vols)| vols.iter().take(SAMPLES as usize).flat_map(|v| picture(v, s.flat)).collect()).collect();
    write_grid(&format!("{dir}/{}.png", s.name), &grid, 6);
    println!("\nPictures: rows {} (out/masked/{}.png).", rows.iter().map(|r| r.0.as_str()).collect::<Vec<_>>().join(", "), s.name);
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let show = args.iter().any(|a| a == "show");
    let mut names: Vec<&str> = args.iter().map(|a| a.as_str()).filter(|a| *a != "show").collect();
    if names.is_empty() {
        names = vec!["towns", "turned", "villages", "forest", "town", "caves", "islands"];
    }
    for name in names {
        let s = scene(name);
        if show {
            let dir = format!("{OUT}/masked");
            std::fs::create_dir_all(&dir).expect("out dir");
            let grid: Vec<Vec<Picture>> = vec![s.train.iter().take(4).flat_map(|v| picture(v, s.flat)).collect()];
            write_grid(&format!("{dir}/data-{}.png", s.name), &grid, 6);
            continue;
        }
        run(&s);
    }
}
