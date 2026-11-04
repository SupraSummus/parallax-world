//! Shared splits: do a split grammar's cross streets cross once the two
//! sides of a cut share their strips (grammar.rs)? The towns of geometry's
//! data and split, against W1's grammar (each side on its own, or both on
//! one seed). Prints Markdown tables, writes pictures to out/.
//!
//! cargo run --release --example shared

use std::time::Instant;

use lab::experiment::OUT;
use lab::geometry::{Crossings, Geometry};
use lab::grammar::{Edge, Grammar, Leaves};
use lab::image::Image;
use lab::stats;
use lab::towns::{self, CROP, LABELS, ROOF, STREET, TownCrop, label_rgb};

const TRAIN_SEEDS: u64 = 18;
const ALL_SEEDS: u64 = 24;
const SAMPLES: u64 = 24;
const KINDS: usize = 32;
/** The variant shown as W2. */
const W2: usize = 5;
const DISTANCES: [usize; 4] = [1, 4, 16, 64];

fn main() {
    let all = towns::gather(&(1..=ALL_SEEDS).collect::<Vec<_>>(), -2..=2, 2, &format!("{OUT}/towns"));
    let (train, test): (Vec<&TownCrop>, Vec<&TownCrop>) = all.iter().partition(|t| t.seed <= TRAIN_SEEDS);
    let shifted = towns::training(&train);
    let real = towns::held_out(&test);
    println!("{} towns to learn from ({} crops), {} held out; crops of {CROP} m\n", train.len(), shifted.len(), real.len());

    let start = Instant::now();
    let mut grammar = Grammar::learn(&shifted, LABELS.len(), KINDS, 1, true);
    let cuts = grammar.nodes.iter().filter(|nd| nd.cut.is_some()).count();
    let sharing = grammar.nodes.iter().filter(|nd| !nd.shared.is_empty()).count();
    let strips: usize = grammar.nodes.iter().map(|nd| nd.shared.len()).sum();
    let snap = |e: Edge| grammar.nodes.iter().filter(|nd| nd.snap == Some(e)).count();
    println!(
        "Grammar: {} rectangles, {cuts} cuts, in {KINDS} kinds, {:.1} s. Cuts sharing strips across: {sharing} ({strips} strips); cuts on a guide's edge: {} low, {} high\n",
        grammar.nodes.len(),
        start.elapsed().as_secs_f64(),
        snap(Edge::Lo),
        snap(Edge::Hi)
    );

    // The variants, each SAMPLES crops of the same seeds.
    // (name, shared, matching, snap, same seed)
    let variants: [(&str, bool, bool, usize, bool); 7] = [
        ("W1 grammar (sides apart)", false, false, 0, false),
        ("W1, both sides one seed", false, false, 0, true),
        ("W2 guides, cuts snap (16 m)", true, false, 16, false),
        ("W2 guides, rules fit them", true, true, 0, false),
        ("W2 shared splits: fit, snap 8 m", true, true, 8, false),
        ("W2 shared splits: fit, snap 16 m", true, true, 16, false),
        ("W2 shared splits: fit, snap 32 m", true, true, 32, false),
    ];
    let mut samples: Vec<(&str, Vec<Image>)> = Vec::new();
    for (name, shared, matching, snap, same_seed) in variants {
        (grammar.shared, grammar.matching, grammar.snap, grammar.same_seed) = (shared, matching, snap, same_seed);
        let start = Instant::now();
        let drawn: Vec<_> = (0..SAMPLES).map(|s| grammar.sample(s, (0, 0, CROP, CROP), Leaves::Pixels)).collect();
        let k = SAMPLES as f64;
        let mean = |f: &dyn Fn(&lab::grammar::Drawn) -> usize| drawn.iter().map(f).sum::<usize>() as f64 / k;
        println!(
            "{name}: {:.4} s a crop; per crop {:.0} rectangles, {:.1} guides set, {:.1} snapping cuts, {:.1} with an edge to snap to, {:.1} moved\n",
            start.elapsed().as_secs_f64() / k,
            mean(&|d| d.rectangles),
            mean(&|d| d.guides),
            mean(&|d| d.snaps[0]),
            mean(&|d| d.snaps[1]),
            mean(&|d| d.snaps[2])
        );
        let maps: Vec<Image> = drawn.into_iter().map(|d| d.image).collect();
        samples.push((name, maps));
    }
    let training = shifted.iter().step_by(4).take(real.len()).cloned().collect::<Vec<_>>();
    let mut rows: Vec<(&str, &[Image])> = vec![("towns held out", &real), ("training towns", &training)];
    rows.extend(samples.iter().map(|(n, m)| (*n, m.as_slice())));

    println!("Means over crops (crossings summed): the held-out towns, and samples.\n");
    println!(
        "| map | crops | street % | street run m | street parts | largest part | roofs | roof median m² | roof rectangular | edges straight | long streets met both sides | cross streets each | aligned ±2 m |"
    );
    println!("|---|---|---|---|---|---|---|---|---|---|---|---|---|");
    for (name, maps) in &rows {
        let g = Geometry::mean(&maps.iter().map(|m| Geometry::of(m, STREET, ROOF)).collect::<Vec<_>>());
        let c = Crossings::sum(&maps.iter().map(|m| Crossings::of(m, STREET)).collect::<Vec<_>>());
        println!(
            "| {name} | {} | {:.1} | {:.1} | {:.1} | {:.2} | {:.1} | {:.0} | {:.2} | {:.2} | {:.1} | {:.1} | {:.2} |",
            maps.len(),
            100.0 * g.street / (CROP * CROP) as f64,
            g.street_run,
            g.street_parts,
            g.street_largest,
            g.roofs,
            g.roof_size,
            g.roof_rectangular,
            g.edges_straight,
            c.streets as f64 / maps.len() as f64,
            c.cross as f64 / c.streets.max(1) as f64,
            c.share()
        );
    }

    let n = LABELS.len();
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
    println!("\nAgainst the held-out towns (pooled): total variation of label shares, and of the joint of labels d apart.\n");
    println!("| map | shares | joint at 1 / 4 / 16 / 64 |\n|---|---|---|");
    for (name, maps) in rows.iter().skip(1) {
        let tv: Vec<String> = DISTANCES.iter().map(|d| format!("{:.3}", stats::tv(&pooled(maps, *d), &pooled(&real, *d)))).collect();
        println!("| {name} | {:.3} | {} |", stats::tv(&shares(maps), &shares(&real)), tv.join(" / "));
    }

    // A box alone is the same box of the whole, shared splits on.
    (grammar.shared, grammar.matching, grammar.snap, grammar.same_seed) = (true, true, 16, false);
    let boxes = [(0, 0, 100, 100), (117, 203, 150, 111), (300, 20, 84, 300), (191, 192, 1, 1), (150, 0, 3, 384)];
    let same = (0..4u64)
        .flat_map(|s| boxes.iter().map(move |b| (s, b)))
        .filter(|(s, (x, y, w, h))| {
            grammar.sample(*s, (*x, *y, *w, *h), Leaves::Pixels).image == samples[W2].1[*s as usize].crop(*x, *y, *w, *h)
        })
        .count();
    println!("\nBoxes alone equal to the same box of the whole crop, shared splits: {same} of {}", 4 * boxes.len());

    // Pictures: real | W1 | W2 crops, three rows; the towns zoomed.
    let whole = |im: &Image| (CROP, CROP, im.to_rgb(&label_rgb, 1));
    let zoom = |im: &Image| (2 * 256, 2 * 256, im.crop(64, 64, 256, 256).to_rgb(&label_rgb, 2));
    let pick = [0usize, 2, 4];
    let grid: Vec<Vec<_>> = pick.iter().map(|i| vec![whole(&real[*i]), whole(&samples[0].1[*i]), whole(&samples[W2].1[*i])]).collect();
    lab::image::write_grid(&format!("{OUT}/shared.png"), &grid, 8);
    let grid: Vec<Vec<_>> = pick.iter().map(|i| vec![zoom(&real[*i]), zoom(&samples[0].1[*i]), zoom(&samples[W2].1[*i])]).collect();
    lab::image::write_grid(&format!("{OUT}/shared-zoom.png"), &grid, 8);
    let more: Vec<Vec<_>> = (0..3).map(|r| (0..6).map(|c| whole(&samples[W2].1[6 + r * 6 + c])).collect()).collect();
    lab::image::write_grid(&format!("{OUT}/shared-more.png"), &more, 8);
    let parses: Vec<_> = [0, 8, 16].iter().map(|c| (CROP, CROP, parse_picture(&grammar, *c))).collect();
    let zoomed = |rgb: &[u8]| {
        let (s, x0, y0, side) = (3, 64, 64, 256);
        let mut out = vec![0u8; side * side * s * s * 3];
        for py in 0..side * s {
            for px in 0..side * s {
                let (x, y) = (x0 + px / s, CROP - y0 - side + py / s);
                out[(py * side * s + px) * 3..][..3].copy_from_slice(&rgb[(y * CROP + x) * 3..][..3]);
            }
        }
        (side * s, side * s, out)
    };
    lab::png::write(&format!("{OUT}/shared-parse-zoom.png"), 768, 768, &zoomed(&parses[1].2).2);
    lab::image::write_side_by_side(&format!("{OUT}/shared-parses.png"), &parses, 8);
}

/** A training crop with its parse's cuts in black, and each shared strip's edges across its cut's rectangle in magenta. */
fn parse_picture(g: &Grammar, crop: usize) -> Vec<u8> {
    let mut rgb = g.crops[crop].to_rgb(&label_rgb, 1);
    let mut put = |x: usize, y: usize, c: [u8; 3]| rgb[((CROP - 1 - y) * CROP + x) * 3..][..3].copy_from_slice(&c);
    let mut stack = vec![g.roots[crop]];
    let mut strips = Vec::new();
    while let Some(i) = stack.pop() {
        let nd = &g.nodes[i];
        let Some((axis, at)) = nd.cut else { continue };
        for j in 0..if axis == 0 { nd.h } else { nd.w } {
            if axis == 0 { put(nd.x0 + at, nd.y0 + j, [0, 0, 0]) } else { put(nd.x0 + j, nd.y0 + at, [0, 0, 0]) }
        }
        strips.extend(nd.shared.iter().map(|s| (i, axis, *s)));
        stack.extend(nd.kids);
    }
    for (i, axis, (lo, hi)) in strips {
        let nd = &g.nodes[i];
        for e in [lo, hi.min(if axis == 0 { nd.h } else { nd.w } - 1)] {
            for j in 0..if axis == 0 { nd.w } else { nd.h } {
                if axis == 0 { put(nd.x0 + j, nd.y0 + e, [255, 0, 255]) } else { put(nd.x0 + e, nd.y0 + j, [255, 0, 255]) }
            }
        }
    }
    rgb
}
