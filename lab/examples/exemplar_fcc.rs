//! Synthesis by coordinates on the FCC lattice (exemplar_fcc.rs): caves,
//! islands, the forest and a town, beside the masked model on the lattice
//! (masked_fcc.rs, the engine's overlapping cells). Statistics against the
//! held-out volumes, how much each draw copies, whether a window drawn
//! alone is that window of the whole. Pictures to out/exemplar_fcc/.
//!
//! cargo run --release --example exemplar_fcc [-- scenes...] [quick]
//! (scenes: caves islands forest town; all by default; `quick` leaves out the masked model)

use std::collections::HashMap;
use std::time::Instant;

use lab::exemplar_fcc::{Coord, Exemplars, Options};
use lab::experiment::OUT;
use lab::fcc_stats::{picture, report};
use lab::image::write_grid;
use lab::masked_fcc::{self, Cells, Model};
use lab::scenes::{self, LABELS3};
use lab::vol::Vol;

const SAMPLES: u64 = 4;

fn options() -> Options {
    Options { levels: 4, passes: 2, jitter: vec![0.5, 0.4, 0.3, 0.0, 0.0], clusters: 64, pool: 4, height: 4.0, mixed: true, seed: 1 }
}

fn masked_options(augment: bool) -> masked_fcc::Options {
    masked_fcc::Options {
        levels: 4,
        codes: 32,
        neighbours: 8,
        parents: 12,
        steps: 16,
        hidden: 64,
        examples: 300_000,
        epochs: 2,
        height: true,
        augment,
        cells: Cells::Shared,
        seed: 1,
    }
}

/** How much of a draw is copied: the mean size (site-weighted) of its pieces whose nearest neighbours are nearest neighbours in one example. */
fn pieces(coords: &[([i64; 3], Coord)]) -> f64 {
    let at: HashMap<[i64; 3], usize> = coords.iter().enumerate().map(|(i, (p, _))| (*p, i)).collect();
    let mut links = Vec::new();
    for (i, (p, c)) in coords.iter().enumerate() {
        for d in [[1, 1, 0], [1, 0, 1], [0, 1, 1], [1, -1, 0], [1, 0, -1], [0, 1, -1]] {
            if let Some(&j) = at.get(&[p[0] + d[0], p[1] + d[1], p[2] + d[2]]) {
                let e = coords[j].1;
                if e.e == c.e && (0..3).all(|a| e.p[a] == c.p[a] + d[a] as i32) {
                    links.push((i, j));
                }
            }
        }
    }
    lab::exemplar::mean_piece(coords.len(), &links)
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let quick = args.iter().any(|a| a == "quick");
    let mut names: Vec<&str> = args.iter().map(|a| a.as_str()).filter(|a| *a != "quick").collect();
    if names.is_empty() {
        names = vec!["caves", "islands", "forest", "town"];
    }
    let dir = format!("{OUT}/exemplar_fcc");
    std::fs::create_dir_all(&dir).expect("out dir");
    let n = LABELS3.len();
    for name in names {
        let (train, held) = scenes::lattice(name);
        let size = train[0].n;
        println!("\n## {name}\n");
        let start = Instant::now();
        let ex = Exemplars::new(&train, n, options());
        println!("{} examples of {size:?} points, indexed in {:.1} s. {:?}\n", train.len(), start.elapsed().as_secs_f64(), ex.options);
        let start = Instant::now();
        let drawn: Vec<(Vol, Vec<Coord>)> = (0..SAMPLES).map(|s| ex.sample(s, [0; 3], size)).collect();
        println!("Synthesis: {:.1} s a volume.\n", start.elapsed().as_secs_f64() / SAMPLES as f64);
        let (at, w) = (size.map(|v| v / 3), size.map(|v| v / 4));
        let same = ex.sample(0, at, w).0 == drawn[0].0.crop(at, w);
        println!("Window {w:?} at {at:?} drawn alone is that window of the whole: {same}.\n");
        assert!(same, "locality");
        let mean_piece = drawn
            .iter()
            .map(|(v, c)| {
                let sites: Vec<[i64; 3]> = lab::fcc_stats::sites(v).collect();
                pieces(&sites.into_iter().zip(c.iter().copied()).collect::<Vec<_>>())
            })
            .sum::<f64>()
            / SAMPLES as f64;
        let used = drawn.iter().map(|(_, c)| lab::exemplar::dominant(c.iter().map(|c| c.e))).sum::<f64>() / SAMPLES as f64;
        println!(
            "Copying: coherent pieces of {mean_piece:.0} sites on average (site-weighted; a volume has {}); {:.0}% of a volume from its commonest example.\n",
            size.iter().product::<usize>() / 2,
            100.0 * used
        );
        let mut rows: Vec<(String, Vec<Vol>)> =
            vec![("held out".into(), held.clone()), ("training (some)".into(), train.iter().take(4).cloned().collect())];
        rows.push(("coordinates".into(), drawn.into_iter().map(|d| d.0).collect()));
        if !quick {
            let model = Model::learn(&train, &held, n, masked_options(name != "town"));
            rows.push(("masked model".into(), (0..SAMPLES).map(|s| model.sample(s, [0; 3], model.size())).collect()));
        }
        report(&rows, n);
        let grid: Vec<Vec<_>> = rows.iter().map(|(_, vols)| vols.iter().take(SAMPLES as usize).flat_map(picture).collect()).collect();
        write_grid(&format!("{dir}/{name}.png"), &grid, 6);
        println!("\nPictures: rows {} (out/exemplar_fcc/{name}.png).", rows.iter().map(|r| r.0.as_str()).collect::<Vec<_>>().join(", "));
    }
}
