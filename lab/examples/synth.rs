//! Worlds sampled from the learned hierarchy, against the valley and
//! against flat WFC (one rule-set): statistics by scale, pictures side by
//! side, a box alone against the whole, and the costs.
//!
//! cargo run --release --example synth

use std::collections::HashMap;
use std::time::Instant;

use lab::experiment::{self, OUT, class_rgb};
use lab::hierarchy::{self, Options};
use lab::image::{Image, palette, write_side_by_side};
use lab::mixture::{self, Fit, Tiles};
use lab::stats;
use lab::synth::{Model, Report, Window, sample};
use lab::valley::CLASSES;

const SEEDS: [u64; 3] = [7, 8, 9];
const ROOT: usize = 8;
const FLAT_BLOCK: usize = 16;
const DISTANCES: [usize; 4] = [1, 4, 16, 64];

fn main() {
    let v = experiment::data();
    let (w, h) = (v.area.nx, v.area.nz);

    let start = Instant::now();
    let options = Options {
        sizes: vec![16, 4],
        tiles: vec![Tiles::Labels, Tiles::Histogram],
        max_m: 16,
        min_blocks: 32,
        restarts: 2,
        seed: 1,
        smooth: 0.0,
    };
    let mut levels = hierarchy::learn(&v.class, &options);
    while levels.last().is_some_and(|l| l.fit.m == 1) {
        levels.pop();
    }
    let learn_seconds = start.elapsed().as_secs_f64();
    let start = Instant::now();
    let biomes: Vec<(usize, &Image)> = levels.iter().map(|l| (l.b, &l.image)).collect();
    let models = [
        ("hierarchical", Model::new(&v.class, &biomes, ROOT, true)),
        ("flat", Model::new(&v.class, &[], FLAT_BLOCK, true)),
        ("hierarchical, pairs", Model::new(&v.class, &biomes, ROOT, false)),
        ("flat, pairs", Model::new(&v.class, &[], FLAT_BLOCK, false)),
    ];
    let rules_seconds = start.elapsed().as_secs_f64();
    let ms: Vec<String> = levels.iter().map(|l| format!("{} of {}", l.fit.m, l.b)).collect();
    println!(
        "Learned {} levels (biomes of blocks: {}) in {learn_seconds:.1} s; rules in {rules_seconds:.1} s\n",
        levels.len(),
        ms.join(", ")
    );

    // Questions 3 and 5: maps of each model, their costs and statistics (means over the seeds).
    let fit1 = &levels[0].fit;
    println!("| model | seconds per map | columns per second | parts per level | retried | without constraints |");
    println!("|---|---|---|---|---|---|");
    let mut rows = vec![("valley", Measures::of(&v.class, &v.class, fit1))];
    let mut maps = vec![v.class.clone()];
    for (name, model) in &models {
        let mut report = Report::default();
        let mut seconds = 0.0;
        let mut measures = Vec::new();
        for (i, seed) in SEEDS.into_iter().enumerate() {
            let start = Instant::now();
            let image = sample(model, seed, 0, Window::new(0, 0, w, h), &mut report);
            seconds += start.elapsed().as_secs_f64() / SEEDS.len() as f64;
            measures.push(Measures::of(&image, &v.class, fit1));
            if i == 0 {
                maps.push(image);
            }
        }
        let rate =
            |v: &[usize]| v.iter().zip(&report.parts).map(|(a, p)| format!("{:.2}%", 100.0 * *a as f64 / *p as f64)).collect::<Vec<_>>();
        let parts: Vec<usize> = report.parts.iter().map(|p| p / SEEDS.len()).collect();
        println!(
            "| {name} | {seconds:.1} | {:.1e} | {parts:?} | {} | {} |",
            (w * h) as f64 / seconds,
            rate(&report.retried).join(", "),
            rate(&report.fallbacks).join(", ")
        );
        rows.push((name, Measures::mean(&measures)));
    }
    println!("\nMeans over {} seeds.\n", SEEDS.len());
    println!("| map | TV of class shares | same class at 1 / 4 / 16 / 64 | TV of joint at 1 / 4 / 16 / 64 |");
    println!("|---|---|---|---|");
    let list = |v: &[f64]| v.iter().map(|x| format!("{x:.3}")).collect::<Vec<_>>().join(" / ");
    for (name, m) in &rows {
        println!("| {name} | {:.3} | {} | {} |", m.shares_tv, list(&m.same), list(&m.joint_tv));
    }
    println!("\nBlocks of 16 m by the domain their classes say: how many, and their patches (count; area-weighted mean size, in blocks).");
    println!("Level-1 biomes as the learned mixture classifies the blocks: patches likewise.\n");
    println!("| map | town blocks | town patches | fields blocks | fields patches | biome patches |");
    println!("|---|---|---|---|---|---|");
    for (name, m) in &rows {
        let [tb, tn, ts, fb, fne, fs, bn, bs] = m.patches;
        println!("| {name} | {tb:.0} | {tn:.1}; {ts:.1} | {fb:.0} | {fne:.1}; {fs:.1} | {bn:.0}; {bs:.1} |");
    }

    // Question 4: a box alone is the same box of the whole.
    let whole = &maps[1];
    let boxes = [(0, 0, 100, 100), (517, 1203, 300, 211), (1000, 2000, 536, 304), (63, 64, 1, 1), (1279, 15, 257, 512)];
    let same = boxes
        .iter()
        .filter(|(x, y, bw, bh)| {
            sample(&models[0].1, SEEDS[0], 0, Window::new(*x as i64, *y as i64, *bw, *bh), &mut Report::default())
                == whole.crop(*x, *y, *bw, *bh)
        })
        .count();
    println!("\nBoxes of the hierarchical map generated alone that equal the same box of the whole: {same} of {}", boxes.len());

    // Pictures: the whole maps, crops, the level-1 biomes they are classified into, the sampled hierarchy.
    let pictures: Vec<(usize, usize, Vec<u8>)> = maps.iter().map(|im| (w, h, im.to_rgb(&class_rgb, 1))).collect();
    write_side_by_side(&format!("{OUT}/synth.png"), &pictures[..3], 16);
    write_side_by_side(&format!("{OUT}/synth-pairs.png"), &[pictures[0].clone(), pictures[3].clone(), pictures[4].clone()], 16);
    let crop = |im: &Image, (x, y): (usize, usize)| (512, 512, im.crop(x, y, 512, 512).to_rgb(&class_rgb, 1));
    let valley_town = (768 - 256, 1736 - 256);
    write_side_by_side(&format!("{OUT}/synth-crop.png"), &maps[..3].iter().map(|im| crop(im, valley_town)).collect::<Vec<_>>(), 16);
    write_side_by_side(&format!("{OUT}/synth-town.png"), &[crop(&maps[0], valley_town), crop(&maps[1], town(&maps[1]))], 16);
    let biome_pictures: Vec<(usize, usize, Vec<u8>)> = maps[..3]
        .iter()
        .map(|im| {
            let b = classify(fit1, im);
            (b.w * 16, b.h * 16, b.to_rgb(&palette, 16))
        })
        .collect();
    write_side_by_side(&format!("{OUT}/synth-biomes.png"), &biome_pictures, 16);
    let mut scale = 1;
    let mut levels_pictures = vec![pictures[1].clone()];
    let model = &models[0].1;
    for k in 1..model.levels.len() {
        scale *= model.levels[k - 1].b;
        let im = sample(model, SEEDS[0], k, Window::new(0, 0, w / scale, h / scale), &mut Report::default());
        levels_pictures.push((im.w * scale, im.h * scale, im.to_rgb(&palette, scale)));
    }
    write_side_by_side(&format!("{OUT}/synth-levels.png"), &levels_pictures, 16);
}

/** A map's statistics. */
#[derive(Clone)]
struct Measures {
    shares_tv: f64,
    same: Vec<f64>,
    joint_tv: Vec<f64>,
    /** Town blocks, patches, mean size; fields likewise; biome patches, mean size. */
    patches: [f64; 8],
}

impl Measures {
    fn of(image: &Image, valley: &Image, fit1: &Fit) -> Measures {
        let n = CLASSES.len();
        let joint = |im: &Image, d: usize| stats::cooccurrence(im, n, d);
        let domains = experiment::domain_of_classes(image).majority(16);
        let count = |d: u16| domains.data.iter().filter(|l| **l == d).count() as f64;
        let (town_n, town_size) = stats::patches(&domains, &|l| l == 1);
        let (fields_n, fields_size) = stats::patches(&domains, &|l| l == 2);
        let (biome_n, biome_size) = stats::patches(&classify(fit1, image), &|_| true);
        Measures {
            shares_tv: stats::tv(&stats::shares(image, n), &stats::shares(valley, n)),
            same: DISTANCES.iter().map(|d| stats::same(&joint(image, *d), n)).collect(),
            joint_tv: DISTANCES.iter().map(|d| stats::tv(&joint(image, *d), &joint(valley, *d))).collect(),
            patches: [count(1), town_n as f64, town_size, count(2), fields_n as f64, fields_size, biome_n as f64, biome_size],
        }
    }

    fn mean(all: &[Measures]) -> Measures {
        let k = all.len() as f64;
        let avg = |f: &dyn Fn(&Measures) -> f64| all.iter().map(f).sum::<f64>() / k;
        Measures {
            shares_tv: avg(&|m| m.shares_tv),
            same: (0..DISTANCES.len()).map(|i| avg(&|m| m.same[i])).collect(),
            joint_tv: (0..DISTANCES.len()).map(|i| avg(&|m| m.joint_tv[i])).collect(),
            patches: std::array::from_fn(|i| avg(&|m| m.patches[i])),
        }
    }
}

/** The level-1 biome of each 16 × 16 block of `image`, by the fitted mixture. */
fn classify(fit: &Fit, image: &Image) -> Image {
    let blocks = mixture::blocks_of(image, CLASSES.len(), 16, Tiles::Labels, &mut HashMap::new());
    Image::new(blocks.nx, blocks.ny, fit.classify(&blocks))
}

/** The corner of the 512 × 512 crop of a map with the most of the town's classes in it, on a grid of 64. */
fn town(image: &Image) -> (usize, usize) {
    let town = |x: usize, y: usize| (y..y + 512).step_by(4).flat_map(move |j| (x..x + 512).step_by(4).map(move |i| (i, j)));
    let count = |x: usize, y: usize| town(x, y).filter(|(i, j)| CLASSES[image.at(*i, *j) as usize].domain == 1).count();
    let corners = (0..=image.h - 512).step_by(64).flat_map(|y| (0..=image.w - 512).step_by(64).map(move |x| (x, y)));
    corners.max_by_key(|(x, y)| count(*x, *y)).unwrap_or((0, 0))
}
