//! Can a town's geometry be learned? Towns of several seeds (towns.rs),
//! held out by seed, against samples of three models: W0's (biomes of
//! neighbour pairs, synth.rs), pixel conditionals with position
//! (template.rs) and a split grammar (grammar.rs). Prints Markdown tables,
//! writes pictures to out/.
//!
//! cargo run --release --example geometry

use std::time::Instant;

use worldgen::noise::hash64;

use lab::experiment::OUT;
use lab::geometry::Geometry;
use lab::grammar::{Grammar, Leaves};
use lab::hierarchy::{self, Options};
use lab::image::{Image, write_side_by_side};
use lab::mixture::Tiles;
use lab::stats;
use lab::synth::{self, Report, Window};
use lab::template::{self, OUTSIDE};
use lab::towns::{self, CROP, LABELS, MARGIN, ROOF, STREET, TownCrop, label_rgb};

const TRAIN_SEEDS: u64 = 18;
const ALL_SEEDS: u64 = 24;
const SAMPLES: u64 = 12;
const DISTANCES: [usize; 4] = [1, 4, 16, 64];
/** Templates per level of the template quadtree (blocks of 4: 4, 16, 64 m), and kinds of the grammar. */
const TEMPLATES: [usize; 3] = [32, 32, 24];
const KINDS: usize = 32;

/** A crop's shift from its town's centre, number `i`: anywhere in ±MARGIN, so a town is not aligned to the cells of any level. */
fn shift(t: &TownCrop, i: i64) -> (i64, i64) {
    let h = hash64(t.seed, t.n, i, 77);
    let m = 2 * MARGIN as u64;
    ((h % m) as i64 - MARGIN as i64, ((h >> 20) % m) as i64 - MARGIN as i64)
}

fn main() {
    let start = Instant::now();
    let all = towns::gather(&(1..=ALL_SEEDS).collect::<Vec<_>>(), -2..=2, 2, &format!("{OUT}/towns"));
    let (train, test): (Vec<&TownCrop>, Vec<&TownCrop>) = all.iter().partition(|t| t.seed <= TRAIN_SEEDS);
    println!(
        "{} towns of seeds 1–{TRAIN_SEEDS} to learn from, {} of seeds {}–{ALL_SEEDS} held out; crops of {CROP} m\n",
        train.len(),
        test.len(),
        TRAIN_SEEDS + 1
    );
    let shifted: Vec<Image> = train
        .iter()
        .flat_map(|t| (0..2).flat_map(move |i| [false, true].map(|m| (i, m))).map(move |(i, m)| (t, i, m)))
        .map(|(t, i, m)| {
            let (dx, dz) = shift(t, i);
            t.crop(dx, dz, m)
        })
        .collect();
    let aligned: Vec<Image> = train.iter().flat_map(|t| [false, true].map(|m| t.crop(0, 0, m))).collect();
    let real: Vec<Image> = test
        .iter()
        .map(|t| {
            let (dx, dz) = shift(t, 9);
            t.crop(dx, dz, false)
        })
        .collect();
    let real_aligned: Vec<Image> = test.iter().map(|t| t.crop(0, 0, false)).collect();
    eprintln!("data in {:.1} s", start.elapsed().as_secs_f64());

    // The models.
    let n = LABELS.len();
    let start = Instant::now();
    let quadtree = template::Model::learn(&shifted, n, 4, &TEMPLATES, 1);
    let quadtree_aligned = template::Model::learn(&aligned, n, 4, &TEMPLATES, 1);
    let quadtree_seconds = start.elapsed().as_secs_f64();
    let start = Instant::now();
    let mut grammar = Grammar::learn(&shifted, n, KINDS, 1, true);
    let grammar_seconds = start.elapsed().as_secs_f64();
    let leaves = grammar.nodes.iter().filter(|nd| nd.cut.is_none()).count();
    println!(
        "Template quadtree: {} templates by level, learned in {quadtree_seconds:.1} s (both). Grammar: {} rectangles ({leaves} leaves) in {KINDS} kinds, {grammar_seconds:.1} s\n",
        TEMPLATES.map(|k| k.to_string()).join(", "),
        grammar.nodes.len()
    );

    // W0's model on a mosaic of the shifted crops, one a town, 4 × 4 of them; its samples' windows with the most town in them.
    let start = Instant::now();
    let side = 4;
    let mut mosaic = Image::filled(side * CROP, side * CROP, 0);
    for (i, t) in train.iter().take(side * side).enumerate() {
        let (dx, dz) = shift(t, 0);
        let c = t.crop(dx, dz, false);
        for y in 0..CROP {
            for x in 0..CROP {
                mosaic.set((i % side) * CROP + x, (i / side) * CROP + y, c.at(x, y));
            }
        }
    }
    let options = Options {
        sizes: vec![16, 4],
        tiles: vec![Tiles::Labels, Tiles::Histogram],
        max_m: 16,
        min_blocks: 32,
        restarts: 2,
        seed: 1,
        smooth: 0.0,
    };
    let mut levels = hierarchy::learn(&mosaic, &options);
    while levels.last().is_some_and(|l| l.fit.m == 1) {
        levels.pop();
    }
    let biomes: Vec<(usize, &Image)> = levels.iter().map(|l| (l.b, &l.image)).collect();
    let w0 = synth::Model::new(&mosaic, &biomes, 8, true);
    let w0_learn = start.elapsed().as_secs_f64();
    let start = Instant::now();
    let w0_samples: Vec<Image> = (0..3)
        .map(|s| {
            let map = synth::sample(&w0, s, 0, Window::new(0, 0, side * CROP, side * CROP), &mut Report::default());
            most_town(&map)
        })
        .collect();
    println!(
        "W0: biomes {} learned in {w0_learn:.1} s; 3 maps of {} m in {:.1} s, each's crop with the most town taken\n",
        levels.iter().map(|l| format!("{} of {}", l.fit.m, l.b)).collect::<Vec<_>>().join(", "),
        side * CROP,
        start.elapsed().as_secs_f64()
    );

    let start = Instant::now();
    let quadtree_samples: Vec<Image> = (0..SAMPLES).map(|s| quadtree.sample(s, 0, 0, 0, CROP, CROP)).collect();
    let quadtree_aligned_samples: Vec<Image> = (0..SAMPLES).map(|s| quadtree_aligned.sample(s, 0, 0, 0, CROP, CROP)).collect();
    let quadtree_sample_seconds = start.elapsed().as_secs_f64() / (2 * SAMPLES) as f64;
    let start = Instant::now();
    let drawn: Vec<_> = (0..SAMPLES).map(|s| grammar.sample(s, (0, 0, CROP, CROP), Leaves::Pixels)).collect();
    let grammar_sample_seconds = start.elapsed().as_secs_f64() / SAMPLES as f64;
    let grammar_samples: Vec<Image> = drawn.iter().map(|d| d.image.clone()).collect();
    let grammar_flat: Vec<Image> = (0..SAMPLES).map(|s| grammar.sample(s, (0, 0, CROP, CROP), Leaves::Commonest).image).collect();
    grammar.contextual = false;
    let grammar_free: Vec<Image> = (0..SAMPLES).map(|s| grammar.sample(s, (0, 0, CROP, CROP), Leaves::Pixels).image).collect();
    grammar.contextual = true;
    assert!(quadtree_samples.iter().all(|im| !im.data.contains(&OUTSIDE)));
    println!("Seconds per crop sampled: quadtree {quadtree_sample_seconds:.3}, grammar {grammar_sample_seconds:.4}\n");

    // Held-out likelihood of the quadtree's levels (true parents given): how much each level knows.
    println!(
        "Quadtree, held out: nats per cell of each level given the true level above (finest first; the last is the top over the root)\n"
    );
    println!("| model | levels |\n|---|---|");
    for (name, model, crops) in [("shifted", &quadtree, &real), ("aligned", &quadtree_aligned, &real_aligned)] {
        let mut sum = vec![0.0; model.levels.len()];
        for c in crops.iter() {
            sum.iter_mut().zip(model.log_likelihood(c)).for_each(|(s, l)| *s -= l / crops.len() as f64);
        }
        println!("| {name} | {} |", sum.iter().map(|s| format!("{s:.3}")).collect::<Vec<_>>().join(" / "));
    }

    // Geometry and statistics.
    println!("\nMeans over crops: the held-out towns, and samples.\n");
    println!(
        "| map | crops | street % | street run m | street parts | largest part | roofs | roof median m² | roof rectangular | edges straight |"
    );
    println!("|---|---|---|---|---|---|---|---|---|---|");
    let rows: [(&str, &[Image]); 9] = [
        ("towns held out", &real),
        ("towns held out, aligned", &real_aligned),
        ("training towns", &shifted[..real.len().min(shifted.len())]),
        ("W0 (pairs in biomes)", &w0_samples),
        ("quadtree", &quadtree_samples),
        ("quadtree, aligned", &quadtree_aligned_samples),
        ("grammar, context-free", &grammar_free),
        ("grammar", &grammar_samples),
        ("grammar, leaves one label", &grammar_flat),
    ];
    for (name, maps) in rows {
        let g = Geometry::mean(&maps.iter().map(|m| Geometry::of(m, STREET, ROOF)).collect::<Vec<_>>());
        println!(
            "| {name} | {} | {:.1} | {:.1} | {:.1} | {:.2} | {:.1} | {:.0} | {:.2} | {:.2} |",
            maps.len(),
            100.0 * g.street / (CROP * CROP) as f64,
            g.street_run,
            g.street_parts,
            g.street_largest,
            g.roofs,
            g.roof_size,
            g.roof_rectangular,
            g.edges_straight
        );
    }
    println!("\nAgainst the held-out towns (pooled): total variation of label shares, and of the joint of labels d apart.\n");
    println!("| map | shares | joint at 1 / 4 / 16 / 64 |\n|---|---|---|");
    let pooled = |maps: &[Image], d: usize| {
        let mut j = vec![0.0; n * n];
        for m in maps {
            j.iter_mut().zip(stats::cooccurrence(m, n, d)).for_each(|(a, b)| *a += b / maps.len() as f64);
        }
        j
    };
    let shares = |maps: &[Image]| {
        let mut s = vec![0.0; n];
        for m in maps {
            s.iter_mut().zip(stats::shares(m, n)).for_each(|(a, b)| *a += b / maps.len() as f64);
        }
        s
    };
    for (name, maps) in rows.iter().skip(1) {
        let tv: Vec<String> = DISTANCES.iter().map(|d| format!("{:.3}", stats::tv(&pooled(maps, *d), &pooled(&real, *d)))).collect();
        println!("| {name} | {:.3} | {} |", stats::tv(&shares(maps), &shares(&real)), tv.join(" / "));
    }

    // How much of a grammar sample is one training town's: is it recombining or copying?
    let copied: Vec<f64> = drawn
        .iter()
        .map(|d| {
            let mut count = vec![0usize; grammar.crops.len()];
            d.source.iter().for_each(|s| count[*s as usize] += 1);
            *count.iter().max().unwrap_or(&0) as f64 / d.source.len() as f64
        })
        .collect();
    let sources: Vec<usize> = drawn
        .iter()
        .map(|d| {
            let mut s: Vec<u32> = d.source.clone();
            s.sort_unstable();
            s.dedup();
            s.len()
        })
        .collect();
    println!(
        "\nGrammar samples: {:.1} training crops (of {}) each, the largest one's share {:.2}; {:.0} rectangles drawn each",
        sources.iter().sum::<usize>() as f64 / SAMPLES as f64,
        grammar.crops.len(),
        copied.iter().sum::<f64>() / SAMPLES as f64,
        drawn.iter().map(|d| d.rectangles).sum::<usize>() as f64 / SAMPLES as f64
    );

    // A box alone is the same box of the whole.
    let boxes = [(0, 0, 100, 100), (117, 203, 150, 111), (300, 20, 84, 300), (191, 192, 1, 1)];
    let quadtree_same = boxes
        .iter()
        .filter(|(x, y, w, h)| quadtree.sample(3, 0, *x as i64, *y as i64, *w, *h) == quadtree_samples[3].crop(*x, *y, *w, *h))
        .count();
    let grammar_same = boxes
        .iter()
        .filter(|(x, y, w, h)| grammar.sample(3, (*x, *y, *w, *h), Leaves::Pixels).image == grammar_samples[3].crop(*x, *y, *w, *h))
        .count();
    println!(
        "\nBoxes alone equal to the same box of the whole crop: quadtree {quadtree_same} of {}, grammar {grammar_same} of {}",
        boxes.len(),
        boxes.len()
    );

    // Pictures: rows of real | W0 | quadtree | quadtree aligned | grammar.
    let picture = |im: &Image| (CROP, CROP, im.to_rgb(&label_rgb, 1));
    let mut rows_png = Vec::new();
    for i in 0..3 {
        rows_png.push(vec![
            picture(&real[i]),
            picture(&w0_samples[i]),
            picture(&quadtree_samples[i]),
            picture(&quadtree_aligned_samples[i]),
            picture(&grammar_free[i]),
            picture(&grammar_samples[i]),
            picture(&grammar_flat[i]),
        ]);
    }
    write_grid(&format!("{OUT}/geometry.png"), &rows_png, 8);
    let zoom = |im: &Image| (3 * 192, 3 * 192, im.crop(96, 96, 192, 192).to_rgb(&label_rgb, 3));
    write_side_by_side(&format!("{OUT}/geometry-zoom.png"), &[zoom(&real[1]), zoom(&quadtree_samples[1]), zoom(&grammar_samples[1])], 8);
    let levels_png: Vec<_> = (0..quadtree.levels.len())
        .map(|k| {
            let s = 4usize.pow(k as u32);
            let im = quadtree.sample(0, k, 0, 0, CROP / s, CROP / s);
            (CROP, CROP, im.to_rgb(&lab::image::palette, s))
        })
        .collect();
    write_side_by_side(&format!("{OUT}/geometry-quadtree-levels.png"), &levels_png, 8);
    let parse_png: Vec<_> = (0..3).map(|i| (CROP, CROP, parse_picture(&grammar, i * 4))).collect();
    write_side_by_side(&format!("{OUT}/geometry-parses.png"), &parse_png, 8);
}

/** The crop of `map` with the most of a town's labels, on a grid of 64. */
fn most_town(map: &Image) -> Image {
    let town = |l: u16| l <= 4;
    let count = |x: usize, y: usize| {
        (y..y + CROP).step_by(4).flat_map(|j| (x..x + CROP).step_by(4).map(move |i| (i, j))).filter(|(i, j)| town(map.at(*i, *j))).count()
    };
    let (x, y) = (0..=map.h - CROP)
        .step_by(64)
        .flat_map(|y| (0..=map.w - CROP).step_by(64).map(move |x| (x, y)))
        .max_by_key(|(x, y)| count(*x, *y))
        .unwrap_or((0, 0));
    map.crop(x, y, CROP, CROP)
}

/** A training crop with its parse's rectangles outlined in black. */
fn parse_picture(g: &Grammar, crop: usize) -> Vec<u8> {
    let mut rgb = g.crops[crop].to_rgb(&label_rgb, 1);
    let mut stack = vec![g.roots[crop]];
    while let Some(i) = stack.pop() {
        let nd = &g.nodes[i];
        if let Some((axis, at)) = nd.cut {
            let line: Vec<(usize, usize)> = if axis == 0 {
                (0..nd.h).map(|j| (nd.x0 + at, nd.y0 + j)).collect()
            } else {
                (0..nd.w).map(|i| (nd.x0 + i, nd.y0 + at)).collect()
            };
            for (x, y) in line {
                let py = CROP - 1 - y;
                rgb[(py * CROP + x) * 3..][..3].copy_from_slice(&[0, 0, 0]);
            }
            stack.extend(nd.kids);
        }
    }
    rgb
}

/** Rows of pictures side by side, the rows one above another, `gap` white pixels between. */
fn write_grid(path: &str, rows: &[Vec<(usize, usize, Vec<u8>)>], gap: usize) {
    let w = rows.iter().map(|r| r.iter().map(|p| p.0).sum::<usize>() + gap * (r.len() - 1)).max().unwrap_or(0);
    let heights: Vec<usize> = rows.iter().map(|r| r.iter().map(|p| p.1).max().unwrap_or(0)).collect();
    let h = heights.iter().sum::<usize>() + gap * (rows.len() - 1);
    let mut out = vec![255u8; w * h * 3];
    let mut y0 = 0;
    for (row, rh) in rows.iter().zip(&heights) {
        let mut x0 = 0;
        for (pw, ph, rgb) in row {
            for y in 0..*ph {
                out[((y0 + y) * w + x0) * 3..][..pw * 3].copy_from_slice(&rgb[y * pw * 3..][..pw * 3]);
            }
            x0 += pw + gap;
        }
        y0 += rh + gap;
    }
    lab::png::write(path, w, h, &out);
}
