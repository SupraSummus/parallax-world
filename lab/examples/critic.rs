//! The critic (critic.rs) on the towns: first on maps whose verdict is
//! known, to see that it measures what it says (real towns it never saw;
//! the training crops themselves; patchworks of training tiles, which are
//! real up to the tile and wrong across; unseen towns with their streets
//! cut), then on the models: the split grammar, the masked model,
//! synthesis by coordinates. With each, its profile (profile.rs) against
//! the training crops'. A Markdown table,
//! and pictures to out/critic/: each row's samples, and the windows the
//! critic was surest are fake.
//!
//! cargo run --release --example critic [-- quick]
//! (`quick` leaves out the masked model, which takes minutes to learn)

use std::time::Instant;

use lab::OUT;
use lab::critic::{self, Reference, SCALES, Verdict};
use lab::exemplar::{self, Exemplars};
use lab::grammar::Grammar;
use lab::image::{Image, write_grid};
use lab::masked::{self, Model};
use lab::profile::{self, Profile};
use lab::towns::{self, GROUND, LABELS, STREET, TownCrop, label_rgb};
use lab::vol::Vol;
use worldgen::noise::{Rng, stream};

/** Samples per row: half to train the critic on, half to test it with. */
const SAMPLES: u64 = 12;
const TRAIN_SEEDS: u64 = 18;
/** Seeds 19 ..= HELD are the critic's real test maps; the rest up to ALL are real maps scored as if a model's. */
const HELD: u64 = 21;
const ALL: u64 = 24;
const COPY_SIZES: [usize; 3] = [8, 16, 32];

/** Maps of tiles `t` cells wide, each from a random training crop at a random place. */
fn patchwork(train: &[Image], t: usize, seed: u64) -> Image {
    let (w, h) = (train[0].w, train[0].h);
    let mut rng = Rng::new(seed);
    let mut out = Image::filled(w, h, 0);
    for ty in (0..h).step_by(t) {
        for tx in (0..w).step_by(t) {
            let src = &train[rng.below(train.len() as u32) as usize];
            let (sx, sy) = (rng.below((w - t) as u32) as usize, rng.below((h - t) as u32) as usize);
            for y in 0..t.min(h - ty) {
                for x in 0..t.min(w - tx) {
                    out.set(tx + x, ty + y, src.at(sx + x, sy + y));
                }
            }
        }
    }
    out
}

struct Row {
    name: String,
    maps: Vec<Image>,
    verdicts: Vec<Verdict>,
}

fn main() {
    let quick = std::env::args().any(|a| a == "quick");
    let dir = format!("{OUT}/critic");
    std::fs::create_dir_all(&dir).expect("out dir");
    let n = LABELS.len();
    let all = towns::gather(&(1..=ALL).collect::<Vec<_>>(), -2..=2, 2, &format!("{OUT}/towns"));
    let by = |r: std::ops::RangeInclusive<u64>| -> Vec<&TownCrop> { all.iter().filter(|t| r.contains(&t.seed)).collect() };
    let train = towns::training(&by(1..=TRAIN_SEEDS));
    let held = towns::training(&by(TRAIN_SEEDS + 1..=HELD));
    // One crop a town where crops stand for a model's samples: a town's crops split between training and testing the critic would let it learn the town.
    let fresh = towns::held_out(&by(HELD + 1..=ALL));
    let held_one = towns::held_out(&by(TRAIN_SEEDS + 1..=HELD));
    eprintln!("{} training crops, {} held out for the critic, {} more real", train.len(), held.len(), fresh.len());
    let (w, h) = (train[0].w, train[0].h);

    let mut rows: Vec<(String, Vec<Image>)> = vec![
        ("real, unseen (calibration)".into(), fresh.iter().take(SAMPLES as usize).cloned().collect()),
        (
            "training crops (calibration)".into(),
            train.iter().step_by(train.len() / SAMPLES as usize).take(SAMPLES as usize).cloned().collect(),
        ),
        ("patchwork of 32 m tiles (calibration)".into(), (0..SAMPLES).map(|s| patchwork(&train, 32, s)).collect()),
        ("patchwork of 128 m tiles (calibration)".into(), (0..SAMPLES).map(|s| patchwork(&train, 128, s)).collect()),
        (
            "real, unseen, streets cut every 16 m (calibration)".into(),
            fresh.iter().take(SAMPLES as usize).enumerate().map(|(i, m)| profile::cut(m, STREET, GROUND, 16, 6, i as u64)).collect(),
        ),
    ];

    let start = Instant::now();
    let mut grammar = Grammar::learn(&train, n, 32, 1);
    (grammar.shared, grammar.matching, grammar.snap) = (true, true, lab::grammar::SNAP);
    rows.push(("split grammar".into(), (0..SAMPLES).map(|s| grammar.sample(s, (0, 0, w, h)).image).collect()));
    eprintln!("grammar in {:.0} s", start.elapsed().as_secs_f64());

    let start = Instant::now();
    let examples: Vec<Image> = train.iter().step_by(train.len().div_ceil(64)).cloned().collect();
    let ex = Exemplars::new(
        &examples,
        n,
        exemplar::Options {
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
        },
    );
    rows.push(("coordinates".into(), (0..SAMPLES).map(|s| ex.sample(s, (0, 0, w, h)).0).collect()));
    eprintln!("coordinates in {:.0} s", start.elapsed().as_secs_f64());

    if !quick {
        let start = Instant::now();
        let vols: Vec<Vol> = train.iter().map(Vol::of_image).collect();
        let options = masked::Options {
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
        };
        let model = Model::learn(&vols, &[], n, options);
        rows.push(("masked model".into(), (0..SAMPLES).map(|s| model.sample(s, [0; 3], model.size()).to_image()).collect()));
        eprintln!("masked model in {:.0} s", start.elapsed().as_secs_f64());
    }

    // The critic of each scale, per row; each scale's real maps made once.
    let mut verdicts: Vec<Vec<Verdict>> = rows.iter().map(|_| Vec::new()).collect();
    for (i, scale) in SCALES.iter().enumerate() {
        let start = Instant::now();
        let reference = Reference::new(&train, &held, n, *scale, None);
        for ((_, maps), v) in rows.iter().zip(&mut verdicts) {
            v.push(reference.judge(maps, None, stream(7, i as u64)));
        }
        eprintln!("critics of {} m in {:.0} s", scale.extent(), start.elapsed().as_secs_f64());
    }
    let rows: Vec<Row> = rows.into_iter().zip(verdicts).map(|((name, maps), verdicts)| Row { name, maps, verdicts }).collect();

    // Copies of the training maps, a window size at a time (each set is large).
    let mut copied = vec![Vec::new(); rows.len()];
    let mut real_copied = Vec::new();
    for k in COPY_SIZES {
        let set = critic::window_set(&train, k);
        real_copied.push(critic::found(&held, k, &set, 2));
        for (r, row) in rows.iter().enumerate() {
            copied[r].push(critic::found(&row.maps, k, &set, 2));
        }
    }

    println!("## The critic on the towns\n");
    println!(
        "{} training crops (seeds 1–{TRAIN_SEEDS}), the critic tested on {} real ones it never saw (seeds {}–{HELD}). \
         {SAMPLES} maps a row of {w} × {h} m.\n",
        train.len(),
        held.len(),
        TRAIN_SEEDS + 1
    );
    println!(
        "AUC: how well a critic tells the row's windows from real ones (0.5 not at all, 1 always), windows of {}. \
         Copied: busy windows found verbatim in the training crops, k × k m; real towns held out: {}. \
         Repeated: busy 16 m windows found in another map of the row. \
         Profile: how many doublings, on average, the sizes of parts, their widths, and the distances between labels \
         moved from the training crops' beyond the critic's test maps', each family's mean weighed by the labels' shares.\n",
        SCALES.iter().map(|s| format!("{} m", s.extent())).collect::<Vec<_>>().join(", "),
        COPY_SIZES.iter().zip(&real_copied).map(|(k, c)| format!("{:.0}% at {k}", 100.0 * c)).collect::<Vec<_>>().join(", ")
    );
    let real_profile = Profile::of(&train, n);
    let baseline = Profile::of(&held, n).differences(&real_profile);
    let families = |maps: &[Image]| -> String {
        let mut diffs = Profile::of(maps, n).differences(&real_profile);
        profile::beyond(&mut diffs, &baseline);
        profile::by_family(&diffs).iter().map(|f| format!("{f:.2}")).collect::<Vec<_>>().join(" | ")
    };
    println!(
        "| maps | {} | {} | repeated | {} |",
        SCALES.iter().map(|s| format!("AUC {} m", s.extent())).collect::<Vec<_>>().join(" | "),
        COPY_SIZES.iter().map(|k| format!("copied {k}")).collect::<Vec<_>>().join(" | "),
        profile::FAMILIES.join(" | ")
    );
    println!("|---|{}", "---|".repeat(SCALES.len() + COPY_SIZES.len() + 1 + profile::FAMILIES.len()));
    // Its profile is the baseline's own, so none: the unseen towns' row is the profiles' bar.
    println!(
        "| real, the critic's test maps | | | | {} | {:.0}% |{}",
        real_copied.iter().map(|c| format!("{:.0}%", 100.0 * c)).collect::<Vec<_>>().join(" | "),
        100.0 * critic::repeated(&held_one, 16, 2),
        " |".repeat(profile::FAMILIES.len())
    );
    for (r, row) in rows.iter().enumerate() {
        println!(
            "| {} | {} | {} | {:.0}% | {} |",
            row.name,
            row.verdicts.iter().map(|v| format!("{:.2}", v.auc)).collect::<Vec<_>>().join(" | "),
            copied[r].iter().map(|c| format!("{:.0}%", 100.0 * c)).collect::<Vec<_>>().join(" | "),
            100.0 * critic::repeated(&row.maps, 16, 2),
            families(&row.maps)
        );
    }

    // Pictures: two maps a row; then per scale above the finest, the windows surest fake.
    let maps: Vec<Vec<_>> = rows.iter().map(|r| r.maps.iter().take(3).map(|m| (m.w, m.h, m.to_rgb(&label_rgb, 1))).collect()).collect();
    write_grid(&format!("{dir}/towns.png"), &maps, 6);
    for (i, s) in SCALES.iter().enumerate().skip(1) {
        let e = s.extent();
        let zoom = (256 / e).max(1);
        let grid: Vec<Vec<_>> = rows
            .iter()
            .map(|r| {
                r.verdicts[i]
                    .worst
                    .iter()
                    .map(|(m, x, y)| (e * zoom, e * zoom, r.maps[*m].crop(*x, *y, e, e).to_rgb(&label_rgb, zoom)))
                    .collect()
            })
            .collect();
        write_grid(&format!("{dir}/worst-{e}.png"), &grid, 6);
    }
    println!(
        "\nPictures (out/critic/): towns.png, three maps a row; worst-64.png, worst-256.png, the windows each row's critic was surest are fake. Rows: {}.",
        rows.iter().map(|r| r.name.as_str()).collect::<Vec<_>>().join(", ")
    );
}
