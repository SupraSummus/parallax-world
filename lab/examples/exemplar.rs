//! Synthesis by coordinates (exemplar.rs) on the flat scenes: villages of
//! winding lanes, turned towns, the towns; beside the masked model
//! (masked.rs) and the split grammar (grammar.rs). Statistics against the
//! held-out crops, how much each draw copies, whether a window drawn alone
//! is that window of the whole. Pictures to out/exemplar/.
//!
//! cargo run --release --example exemplar [-- scenes...] [quick]
//! (scenes: villages turned towns; all by default; `quick` leaves out the other models)

use std::time::Instant;

use lab::exemplar::{self, Exemplars, Options};
use lab::experiment::OUT;
use lab::geometry::Geometry;
use lab::grammar::{Grammar, Leaves};
use lab::image::{Image, write_grid};
use lab::masked::{self, Model};
use lab::scenes;
use lab::stats;
use lab::towns::{self, LABELS, ROOF, STREET, label_rgb};
use lab::vol::Vol;

const SAMPLES: u64 = 4;
/** The most examples synthesis copies from (their tables are kept whole). */
const EXAMPLES: usize = 64;

fn options() -> Options {
    Options {
        levels: 6,
        passes: 3,
        jitter: vec![0.5, 0.5, 0.4, 0.3, 0.2, 0.0, 0.0],
        clusters: 64,
        pool: 4,
        position: 4.0,
        mixed: true,
        guide: 0.0,
        lines: 0.0,
        seed: 1,
    }
}

fn masked_options() -> masked::Options {
    masked::Options {
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

fn scene(name: &str) -> (Vec<Image>, Vec<Image>, bool) {
    let all = || towns::gather(&(1..=21).collect::<Vec<_>>(), -2..=2, 2, &format!("{OUT}/towns"));
    let flat = |(a, b): (Vec<Vol>, Vec<Vol>)| (a.iter().map(Vol::to_image).collect(), b.iter().map(Vol::to_image).collect());
    match name {
        "villages" => {
            let (a, b) = flat(scenes::villages_flat(160, 12));
            (a, b, true)
        }
        "turned" => {
            let (a, b) = flat(scenes::turned_flat(&all(), 18));
            (a, b, true)
        }
        "towns" => {
            let (a, b) = flat(scenes::towns_flat(&all(), 18));
            (a, b, false)
        }
        _ => panic!("no scene {name}"),
    }
}

fn pooled(maps: &[Image], n: usize, d: usize) -> (Vec<f64>, Vec<f64>) {
    let (mut s, mut j) = (vec![0.0; n], vec![0.0; n * n]);
    for m in maps {
        s.iter_mut().zip(stats::shares(m, n)).for_each(|(a, b)| *a += b / maps.len() as f64);
        j.iter_mut().zip(stats::cooccurrence(m, n, d)).for_each(|(a, b)| *a += b / maps.len() as f64);
    }
    (s, j)
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let quick = args.iter().any(|a| a == "quick");
    let mut names: Vec<&str> = args.iter().map(|a| a.as_str()).filter(|a| *a != "quick").collect();
    if names.is_empty() {
        names = vec!["villages", "turned", "towns"];
    }
    let dir = format!("{OUT}/exemplar");
    std::fs::create_dir_all(&dir).expect("out dir");
    let n = LABELS.len();
    for name in names {
        let (train, held, isotropic) = scene(name);
        let (w, h) = (train[0].w, train[0].h);
        // Examples: the training crops, at most EXAMPLES of them, evenly; with their turns where the scene faces no way.
        let step = train.len().div_ceil(EXAMPLES);
        let examples: Vec<Image> = train.iter().step_by(step).cloned().collect();
        println!("\n## {name}\n");
        let start = Instant::now();
        let ex = Exemplars::new(&examples, n, options());
        println!(
            "{} examples of {w} × {h} (of {} to learn from), indexed in {:.1} s. {:?}\n",
            examples.len(),
            train.len(),
            start.elapsed().as_secs_f64(),
            ex.options
        );
        let start = Instant::now();
        let drawn: Vec<(Image, Vec<exemplar::Coord>)> = (0..SAMPLES).map(|s| ex.sample(s, (0, 0, w, h))).collect();
        println!("Synthesis: {:.1} s a crop.\n", start.elapsed().as_secs_f64() / SAMPLES as f64);
        let (at, ww) = (w / 3, w / 4);
        let (alone, _) = ex.sample(0, (at, at, ww, ww));
        let same = alone == drawn[0].0.crop(at, at, ww, ww);
        println!("Window {ww} × {ww} at ({at}, {at}) drawn alone is that window of the whole: {same}.\n");
        assert!(same, "locality");
        let copies: Vec<(f64, f64)> = drawn.iter().map(|(_, c)| exemplar::copying(c, w)).collect();
        println!(
            "Copying: coherent pieces of {:.0} cells on average (area-weighted; a crop is {}); {:.0}% of a crop from its commonest example.\n",
            copies.iter().map(|c| c.0).sum::<f64>() / copies.len() as f64,
            w * h,
            100.0 * copies.iter().map(|c| c.1).sum::<f64>() / copies.len() as f64
        );
        let mut rows: Vec<(String, Vec<Image>)> =
            vec![("held out".into(), held.clone()), ("training (some)".into(), train.iter().take(4).cloned().collect())];
        rows.push(("coordinates".into(), drawn.into_iter().map(|d| d.0).collect()));
        if !quick {
            let start = Instant::now();
            let vols: Vec<Vol> = train.iter().map(Vol::of_image).collect();
            let hv: Vec<Vol> = held.iter().map(Vol::of_image).collect();
            let model = Model::learn(&vols, &hv, n, masked::Options { augment: isotropic, ..masked_options() });
            eprintln!("masked model in {:.0} s", start.elapsed().as_secs_f64());
            rows.push(("masked model".into(), (0..SAMPLES).map(|s| model.sample(s, [0; 3], model.size()).to_image()).collect()));
            let mut grammar = Grammar::learn(&train, n, 32, 1, true);
            (grammar.shared, grammar.matching, grammar.snap) = (true, true, lab::grammar::SNAP);
            rows.push(("split grammar".into(), (0..SAMPLES).map(|s| grammar.sample(s, (0, 0, w, h), Leaves::Pixels).image).collect()));
        }
        let dists = [1, 4, 16, 64];
        let reference: Vec<(Vec<f64>, Vec<f64>)> = dists.iter().map(|d| pooled(&rows[0].1, n, *d)).collect();
        println!(
            "| map | TV shares | TV joint d=1 | TV joint d=4 | TV joint d=16 | TV joint d=64 | street run m | street parts | largest street part | roofs | roof median m² | roof rectangular |"
        );
        println!("|---|---|---|---|---|---|---|---|---|---|---|---|");
        for (label, maps) in &rows {
            let tvs: Vec<String> =
                dists.iter().zip(&reference).map(|(d, r)| format!("{:.3}", stats::tv(&pooled(maps, n, *d).1, &r.1))).collect();
            let g = Geometry::mean(&maps.iter().map(|m| Geometry::of(m, STREET, ROOF)).collect::<Vec<_>>());
            println!(
                "| {label} | {:.3} | {} | {:.1} | {:.1} | {:.2} | {:.1} | {:.0} | {:.2} |",
                stats::tv(&pooled(maps, n, 1).0, &reference[0].0),
                tvs.join(" | "),
                g.street_run,
                g.street_parts,
                g.street_largest,
                g.roofs,
                g.roof_size,
                g.roof_rectangular
            );
        }
        let grid: Vec<Vec<_>> = rows
            .iter()
            .map(|(_, maps)| maps.iter().take(SAMPLES as usize).map(|m| (m.w, m.h, m.to_rgb(&label_rgb, 1))).collect())
            .collect();
        write_grid(&format!("{dir}/{name}.png"), &grid, 6);
        println!("\nPictures: rows {} (out/exemplar/{name}.png).", rows.iter().map(|r| r.0.as_str()).collect::<Vec<_>>().join(", "));
    }
}
