//! Regimes over a whole area (regimes.rs): sites of a plan, the split
//! grammar with shared splits in their boxes down to texture kinds, W0's
//! texture model everywhere else. Real areas of held-out seeds against
//! samples, with regimes and without (the grammar filling its boxes).
//! Prints Markdown tables, writes pictures to out/.
//!
//! cargo run --release --example regimes

use std::time::Instant;

use lab::experiment::OUT;
use lab::geometry::{self, Crossings, Geometry};
use lab::grammar::Grammar;
use lab::hierarchy::Options;
use lab::image::Image;
use lab::mixture::Tiles;
use lab::regimes::{self, Regimes};
use lab::stats;
use lab::towns::{self, CROP, LABELS, ROOF, STREET, TownCrop, label_rgb};

const TRAIN_SEEDS: u64 = 18;
const KINDS: usize = 32;
/** An area's side, metres; the seeds whose areas give the countryside, and those held out. */
const SIDE: usize = 1536;
const COUNTRYSIDE: [u64; 4] = [1, 2, 3, 4];
const HELD_OUT: [u64; 3] = [19, 20, 21];
const SAMPLES: u64 = 6;
const DISTANCES: [usize; 4] = [1, 4, 16, 64];
const CANOPY: u16 = 5;
const GROUND: u16 = 6;

fn main() {
    let all = towns::gather(&(1..=TRAIN_SEEDS).collect::<Vec<_>>(), -2..=2, 2, &format!("{OUT}/towns"));
    let train: Vec<&TownCrop> = all.iter().collect();
    let start = Instant::now();
    let grammar = Grammar::learn(&towns::training(&train), LABELS.len(), KINDS, 1, true);
    eprintln!("grammar in {:.1} s", start.elapsed().as_secs_f64());

    let start = Instant::now();
    let dir = format!("{OUT}/areas");
    let countryside: Vec<Image> = COUNTRYSIDE.iter().map(|s| regimes::area(*s, SIDE, 2, &dir)).collect();
    let real: Vec<Image> = HELD_OUT.iter().map(|s| regimes::area(*s, SIDE, 2, &dir)).collect();
    eprintln!("areas in {:.1} s", start.elapsed().as_secs_f64());
    let mosaic = regimes::countryside(&countryside, 8);

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
    let mut r = Regimes::learn(grammar, &mosaic, &options);
    let textures = r.texture.iter().filter(|t| **t).count();
    println!(
        "Countryside: {} tiles of {CROP} m from the areas of seeds {COUNTRYSIDE:?} away from their towns; {}, learned in {:.1} s. Texture rules: {} of {}\n",
        mosaic.w / CROP * (mosaic.h / CROP),
        r.report,
        start.elapsed().as_secs_f64(),
        textures,
        r.texture.len()
    );

    // Samples: areas centred on their site 0, with regimes and without.
    let window = |seed: u64| {
        let s = regimes::sites(seed, (-1, -1, 2, 2)).into_iter().find(|s| s.j == 0).expect("site 0");
        (s.x - SIDE as i64 / 2, s.y - SIDE as i64 / 2, SIDE, SIDE)
    };
    let mut with = Vec::new();
    let mut without = Vec::new();
    let start = Instant::now();
    for seed in 0..SAMPLES {
        with.push(r.sample(seed, window(seed), true));
    }
    let seconds = start.elapsed().as_secs_f64() / SAMPLES as f64;
    for seed in 0..SAMPLES {
        without.push(r.sample(seed, window(seed), false));
    }
    println!("Seconds per area of {SIDE} m sampled, with regimes: {seconds:.1}\n");

    // The town's box: the middle crop of an area.
    let middle = (SIDE - CROP) / 2;
    let boxes = |maps: &[Image]| maps.iter().map(|m| m.crop(middle, middle, CROP, CROP)).collect::<Vec<_>>();
    let rows: [(&str, &[Image]); 3] = [("real, held out", &real), ("regimes (W2)", &with), ("grammar fills its boxes", &without)];
    let n = LABELS.len();
    println!(
        "Whole areas ({SIDE} m) and their town's box ({CROP} m). Blockiness: the share of label edges in straight runs of 8 m or more, \
         of all labels or of the countryside's (canopy, ground, fields) among themselves. Clearings: patches of ground.\n"
    );
    println!(
        "| map | areas | long edges, box | countryside's long edges, box | countryside's long edges, outside box | canopy % | clearings per km² | clearing m² (cell-weighted) |"
    );
    println!("|---|---|---|---|---|---|---|---|");
    let country = |l: u16| l >= CANOPY;
    for (name, maps) in rows {
        let k = maps.len() as f64;
        let mean = |f: &dyn Fn(&Image) -> f64| maps.iter().map(f).sum::<f64>() / k;
        let outside = |m: &Image| {
            let mut o = m.clone();
            for y in middle..middle + CROP {
                for x in middle..middle + CROP {
                    o.set(x, y, geometry::IGNORED);
                }
            }
            geometry::long_edges(&o, 8, &country)
        };
        let in_box = |m: &Image| m.crop(middle, middle, CROP, CROP);
        let km2 = (SIDE * SIDE) as f64 / 1e6;
        println!(
            "| {name} | {} | {:.3} | {:.3} | {:.3} | {:.1} | {:.0} | {:.0} |",
            maps.len(),
            mean(&|m| geometry::long_edges(&in_box(m), 8, &|_| true)),
            mean(&|m| geometry::long_edges(&in_box(m), 8, &country)),
            mean(&outside),
            100.0 * mean(&|m| stats::shares(m, n)[CANOPY as usize]),
            mean(&|m| stats::patches(m, &|l| l == GROUND).0 as f64) / km2,
            mean(&|m| stats::patches(m, &|l| l == GROUND).1)
        );
    }

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
    println!(
        "\nAgainst the real areas (pooled): total variation of label shares, and of the joint of labels d apart; whole areas, and boxes.\n"
    );
    println!("| map | shares | joint at 1 / 4 / 16 / 64 | box: shares | box: joint at 1 / 4 / 16 / 64 |\n|---|---|---|---|---|");
    let real_boxes = boxes(&real);
    for (name, maps) in rows.iter().skip(1) {
        let b = boxes(maps);
        let tv = |a: &[Image], r: &[Image]| {
            DISTANCES.iter().map(|d| format!("{:.3}", stats::tv(&pooled(a, *d), &pooled(r, *d)))).collect::<Vec<_>>().join(" / ")
        };
        println!(
            "| {name} | {:.3} | {} | {:.3} | {} |",
            stats::tv(&shares(maps), &shares(&real)),
            tv(maps, &real),
            stats::tv(&shares(&b), &shares(&real_boxes)),
            tv(&b, &real_boxes)
        );
    }

    println!("\nThe town in its box.\n");
    println!(
        "| map | street parts | largest part | street run m | roof rectangular | long streets met both sides | aligned ±2 m |\n|---|---|---|---|---|---|---|"
    );
    for (name, maps) in rows {
        let b = boxes(maps);
        let g = Geometry::mean(&b.iter().map(|m| Geometry::of(m, STREET, ROOF)).collect::<Vec<_>>());
        let c = Crossings::sum(&b.iter().map(|m| Crossings::of(m, STREET)).collect::<Vec<_>>());
        println!(
            "| {name} | {:.1} | {:.2} | {:.1} | {:.2} | {:.1} | {:.2} |",
            g.street_parts,
            g.street_largest,
            g.street_run,
            g.roof_rectangular,
            c.streets as f64 / b.len() as f64,
            c.share()
        );
    }

    // A window alone is the same window of the whole.
    let (x0, y0, _, _) = window(0);
    let windows = [(700, 700, 120, 90), (0, 0, 64, 64), (560, 900, 300, 40), (768, 768, 1, 1), (1000, 300, 200, 200)];
    let same = windows
        .iter()
        .filter(|(x, y, w, h)| r.sample(0, (x0 + *x as i64, y0 + *y as i64, *w, *h), true) == with[0].crop(*x, *y, *w, *h))
        .count();
    println!("\nWindows alone equal to the same window of the whole area: {same} of {}", windows.len());

    // Pictures: real | regimes | grammar whole, halved; and their middles at a metre a pixel.
    let half = |im: &Image| (SIDE / 2, SIDE / 2, im.majority(2).to_rgb(&label_rgb, 1));
    let grid: Vec<Vec<_>> = (0..2).map(|i| vec![half(&real[i]), half(&with[i]), half(&without[i])]).collect();
    lab::image::write_grid(&format!("{OUT}/regimes.png"), &grid, 8);
    let zoom = 640;
    let at = (SIDE - zoom) / 2;
    let mid = |im: &Image| (zoom, zoom, im.crop(at, at, zoom, zoom).to_rgb(&label_rgb, 1));
    let grid: Vec<Vec<_>> = (0..2).map(|i| vec![mid(&real[i]), mid(&with[i]), mid(&without[i])]).collect();
    lab::image::write_grid(&format!("{OUT}/regimes-zoom.png"), &grid, 8);
}
