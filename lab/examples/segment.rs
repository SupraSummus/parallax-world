//! Does EM on adjacency rule-sets find the valley's domains, and do biomes
//! of biomes mean anything? Prints Markdown tables; writes pictures to out/.
//!
//! cargo run --release --example segment

use std::collections::HashMap;
use std::time::Instant;

use lab::experiment::{self, OUT, class_rgb, domain_rgb};
use lab::hierarchy::{self, Options};
use lab::image::{Image, palette, write_side_by_side};
use lab::mixture::{self, Tiles};
use lab::valley::Valley;

const MAX_M: usize = 16;
const SEED: u64 = 1;

fn main() {
    let v = experiment::data();
    let (w, h) = (v.area.nx, v.area.nz);
    let oracle = experiment::domain_of_classes(&v.class);
    let agree = oracle.data.iter().zip(&v.owner.data).filter(|(a, b)| a == b).count() as f64 / oracle.data.len() as f64;
    let share = |d: u16| v.owner.data.iter().filter(|o| **o == d).count() as f64 / v.owner.data.len() as f64;
    println!("Columns {w} × {h}; land: forest {:.1}%, town {:.1}%, fields {:.1}%", 100.0 * share(0), 100.0 * share(1), 100.0 * share(2));
    println!("A column's class names its owner (the kinds' domain) for {:.1}% of columns\n", 100.0 * agree);
    write_side_by_side(
        &format!("{OUT}/valley.png"),
        &[(w, h, v.class.to_rgb(&class_rgb, 1)), (w, h, v.owner.to_rgb(&domain_rgb, 1)), (w, h, oracle.to_rgb(&domain_rgb, 1))],
        16,
    );

    // Question 1: one level, by block size and tiles.
    println!("## One level: biomes against the owner of each block (its commonest)\n");
    println!(
        "| B | tiles | vocabulary | M by MDL | ARI | as domains: ARI, F1 town, F1 fields | M = 3: ARI | as domains | oracle: ARI, F1 town, F1 fields | seconds |"
    );
    println!("|---|---|---|---|---|---|---|---|---|---|");
    let mut main_fit = None;
    for b in [8, 16, 32] {
        let truth = v.owner.majority(b);
        let by_kinds = oracle.majority(b);
        for tiles in [Tiles::Labels, Tiles::Patterns, Tiles::Histogram] {
            let start = Instant::now();
            let blocks = mixture::blocks(&v.class, b, tiles, &mut HashMap::new());
            let (fit, fits) = mixture::select(&blocks, MAX_M, SEED, 1, 0.0);
            let seconds = start.elapsed().as_secs_f64();
            let three = fits[2].image(&blocks);
            let ari = experiment::ari(&fit.image(&blocks), &truth);
            let ari3 = experiment::ari(&three, &truth);
            let domains = |p: &Image| {
                let (a, t, f) = experiment::as_domains(p, &truth);
                format!("{a:.3}, {t:.3}, {f:.3}")
            };
            println!(
                "| {b} | {tiles:?} | {} | {} | {ari:.3} | {} | {ari3:.3} | {} | {} | {seconds:.1} |",
                blocks.vocabulary,
                fit.m,
                domains(&fit.image(&blocks)),
                domains(&three),
                domains(&by_kinds)
            );
            if b == 16 && tiles == Tiles::Labels {
                main_fit = Some((fit, fits));
            }
        }
    }
    let (fit, fits) = main_fit.expect("B = 16 with labels");
    println!("\n### B = 16, labels: description length by M (kbit, relative to M = 1)\n");
    println!("| M | data | model | total |");
    println!("|---|---|---|---|");
    for f in &fits {
        println!(
            "| {} | {:.1} | {:.1} | {:.1} |",
            f.m,
            (f.data_bits - fits[0].data_bits) / 1e3,
            (f.model_bits - fits[0].model_bits) / 1e3,
            (f.bits() - fits[0].bits()) / 1e3
        );
    }
    let biomes = Image::new(w / 16, h / 16, fit.assign.clone());
    println!("\n### B = 16, labels, M = {}: the biomes\n", fit.m);
    experiment::describe(&biomes, 16, &v);
    println!("\n### B = 16, labels, M = 3\n");
    experiment::describe(&Image::new(w / 16, h / 16, fits[2].assign.clone()), 16, &v);

    // Question 2: biomes of biomes, as fitted and with neighbouring blocks' biomes favoured.
    // Above level 1, pairs of biomes are many tokens for few blocks; histograms of them are few.
    biomes_of_biomes(&v, "hierarchy", &[Tiles::Labels], 0.0);
    biomes_of_biomes(&v, "hierarchy-histograms", &[Tiles::Labels, Tiles::Histogram], 0.0);
    biomes_of_biomes(&v, "hierarchy-smooth", &[Tiles::Labels], SMOOTH);
}

/** Nats a block's biome gains per side shared with a neighbour's, in the smoothed hierarchy. */
const SMOOTH: f64 = 10.0;

/** A hierarchy with `tiles` (finest level first, the last repeating) and `smooth`, its tables, and a picture `name`. */
fn biomes_of_biomes(v: &Valley, name: &str, tiles: &[Tiles], smooth: f64) {
    let (w, h) = (v.area.nx, v.area.nz);
    let options = Options { sizes: vec![16, 4], tiles: tiles.to_vec(), max_m: MAX_M, min_blocks: 32, restarts: 2, seed: SEED, smooth };
    let start = Instant::now();
    let levels = hierarchy::learn(&v.class, &options);
    println!(
        "\n## Biomes of biomes: {tiles:?}, smoothing {smooth} (blocks of 16 columns, then 4 cells each level up): {:.1} s\n",
        start.elapsed().as_secs_f64()
    );
    println!("| level | cell, m | cells | M by MDL | ARI vs owner | as domains: ARI, F1 town, F1 fields |");
    println!("|---|---|---|---|---|---|");
    let mut scale = 1;
    let mut pictures = vec![(w, h, v.class.to_rgb(&class_rgb, 1))];
    for (k, level) in levels.iter().enumerate() {
        scale *= level.b;
        let truth = v.owner.majority(scale);
        let ari = experiment::ari(&level.image, &truth);
        let (merged, town, fields) = experiment::as_domains(&level.image, &truth);
        println!(
            "| {} | {scale} | {} × {} | {} | {ari:.3} | {merged:.3}, {town:.3}, {fields:.3} |",
            k + 1,
            level.image.w,
            level.image.h,
            level.fit.m
        );
        pictures.push((level.image.w * scale, level.image.h * scale, level.image.to_rgb(&palette, scale)));
    }
    pictures.push((w, h, v.owner.to_rgb(&domain_rgb, 1)));
    write_side_by_side(&format!("{OUT}/{name}.png"), &pictures, 16);
    scale = 1;
    for (k, level) in levels.iter().enumerate() {
        scale *= level.b;
        println!("\n### Level {}: {} biomes of {scale} m cells\n", k + 1, level.fit.m);
        let bits: Vec<String> = level.bits.iter().map(|(m, b)| format!("{m}: {:.1}", (b - level.bits[0].1) / 1e3)).collect();
        println!("Description length by M, kbit from M = 1: {}\n", bits.join(", "));
        experiment::describe(&level.image, scale, v);
    }
}
