//! The masked model on the FCC lattice (masked_fcc.rs): the forest, a
//! town, caves and islands at level 3 (sites a metre apart), each with the
//! engine's overlapping cells and with an octree-like partition of them,
//! to see whether sites shared by two or six coarse cells help. For each,
//! held-out bits per level, statistics of samples against held-out
//! volumes, seams (how much more labels change between neighbours of
//! different owners than of one), and whether a window drawn alone is
//! that window of the whole. Pictures to out/masked_fcc/.
//!
//! cargo run --release --example masked_fcc [-- scenes...] [shared|owned]
//! (scenes: forest town caves islands; all and both by default)

use std::time::Instant;

use lab::OUT;
use lab::fcc_stats::{picture, report};
use lab::image::write_grid;
use lab::masked_fcc::{Cells, Model, Options};
use lab::scenes::{self, LABELS3};
use lab::vol::Vol;

const SAMPLES: u64 = 4;

fn options(cells: Cells, augment: bool) -> Options {
    Options {
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
        cells,
        seed: 1,
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cells: Vec<Cells> = match (args.iter().any(|a| a == "shared"), args.iter().any(|a| a == "owned")) {
        (true, false) => vec![Cells::Shared],
        (false, true) => vec![Cells::Owned],
        _ => vec![Cells::Shared, Cells::Owned],
    };
    let mut names: Vec<&str> = args.iter().map(|a| a.as_str()).filter(|a| !matches!(*a, "shared" | "owned")).collect();
    if names.is_empty() {
        names = vec!["forest", "town", "caves", "islands"];
    }
    let dir = format!("{OUT}/masked_fcc");
    std::fs::create_dir_all(&dir).expect("out dir");
    let n = LABELS3.len();
    for name in names {
        let start = Instant::now();
        let (train, held) = scenes::lattice(name);
        eprintln!(
            "{name}: {} to learn from, {} held out, {:?} points, in {:.1} s",
            train.len(),
            held.len(),
            train[0].n,
            start.elapsed().as_secs_f64()
        );
        let mut rows: Vec<(String, Vec<Vol>)> =
            vec![("held out".into(), held.clone()), ("training (some)".into(), train.iter().take(4).cloned().collect())];
        for c in &cells {
            let start = Instant::now();
            let model = Model::learn(&train, &held, n, options(*c, name != "town"));
            println!("\n## {name}, cells {c:?}\n");
            println!(
                "{} volumes of {:?} points to learn from, {} held out. Model: {} parameters, learned in {:.0} s. {}\n",
                train.len(),
                train[0].n,
                held.len(),
                model.parameters(),
                start.elapsed().as_secs_f64(),
                model.describe()
            );
            println!(
                "Held-out bits a site, level 0 (labels) to the top: {}\n",
                model.bits.iter().map(|b| format!("{b:.3}")).collect::<Vec<_>>().join(", ")
            );
            let start = Instant::now();
            let samples: Vec<Vol> = (0..SAMPLES).map(|s| model.sample(s, [0; 3], model.size())).collect();
            println!("Sampling: {:.1} s a volume.\n", start.elapsed().as_secs_f64() / SAMPLES as f64);
            let size = model.size();
            let (at, w) = (size.map(|v| v / 3), size.map(|v| v / 4));
            let same = model.sample(0, at, w) == samples[0].crop(at, w);
            println!("Window {w:?} at {at:?} drawn alone is that window of the whole: {same}.\n");
            assert!(same, "locality");
            rows.push((format!("cells {c:?}"), samples));
        }
        println!("\n## {name}: samples against the held-out volumes\n");
        report(&rows, n);
        let grid: Vec<Vec<_>> = rows.iter().map(|(_, vols)| vols.iter().take(SAMPLES as usize).flat_map(picture).collect()).collect();
        let tag = if cells.len() == 1 { format!("-{:?}", cells[0]).to_lowercase() } else { String::new() };
        write_grid(&format!("{dir}/{name}{tag}.png"), &grid, 6);
        println!("\nPictures: rows {} (out/masked_fcc/{name}{tag}.png).", rows.iter().map(|r| r.0.as_str()).collect::<Vec<_>>().join(", "));
    }
}
