//! Drawing to a plan, from real places (real.rs): can a model learned on
//! many pieces of the world blend them where a plan says, forest into
//! town, a village along a road, a river through a town?
//!
//! A plan (plan.rs) is a context, per 16 m the shares of what is built,
//! forest, fields and water around, and lines, main roads and water given
//! exactly. Real places hold every mix of shares at their edges, so a model
//! that draws each part of a map from parts of the examples of about its
//! context has seen every blend a plan asks for, without being shown a
//! forest turning into a town as such. Lines hold what shares can't: a
//! road or river long and thin, joined; the models draw the rest around.
//!
//! Synthesis by coordinates and the masked model draw to the plans of
//! held-out places (not their labels), with lines and without, and to
//! plans painted here. The critic (critic.rs) scores how real each looks
//! against real windows of the same context; the plan's error how well
//! its shares were followed; the profile (profile.rs) how far its parts'
//! sizes, widths and distances to each other are from real places'.
//! Pictures to out/blend/.
//!
//! python3 lab/scripts/overture.py first (see there), then
//! cargo run --release --example blend

use std::time::Instant;

use lab::OUT;
use lab::critic::{self, Reference, SCALES};
use lab::exemplar::{Exemplars, Options};
use lab::image::{Image, write_grid};
use lab::masked::{self, Model};
use lab::plan::{Context, Lines, NONE, Plan};
use lab::profile::{self, Profile};
use lab::real;
use lab::towns::{CANOPY, FIELDS, GROUND, LABELS, PAVING, ROOF, STREET, WATER, label_rgb};
use lab::vol::Vol;
use worldgen::noise::{hash64, stream, unit};

const SIDE: usize = 384;
const SAMPLES: usize = 12;
/** Examples synthesis copies from, of the training places' crops. */
const EXAMPLES: usize = 128;
/** Crops the critic learns real windows from, and tests on. */
const CRITIC_REAL: usize = 200;

/** What a context counts: built (roof, wall, street, paving), forest, fields, water; the rest is open ground. */
const GROUPS: [&[u16]; 4] = [&[ROOF, 1, STREET, PAVING], &[CANOPY], &[FIELDS], &[WATER]];
const LINES: [u16; 2] = [STREET, WATER];
/** Levels from which the masked model is given a place's codes, to see which levels' draws lose what is joined. */
const GIVEN: [usize; 2] = [3, 4];
/** Draws of each painted plan by each model. */
const DRAWN: usize = 8;

fn context(map: &Image) -> Context {
    Context::of(map, &GROUPS, 16, 2)
}

/** A map's plan: its context, and `lines` (NONE where none) unless left out. */
fn plan(map: &Image, lines: Option<&Image>) -> Plan {
    Plan { context: context(map), lines: lines.map(|l| Lines::new(l.clone(), &LINES)) }
}

/**
 * A plan painted as a sketch: each metre (x, y) the line `line` gives
 * there, or one of the groups' labels (roof standing for built) or ground,
 * drawn at random by the shares `f` gives; its plan is the sketch's, as a
 * real map's would be.
 */
fn painted(f: &dyn Fn(f32, f32) -> [f32; 4], line: &dyn Fn(f32, f32) -> Option<u16>) -> (Image, Plan) {
    let at = |i: usize| ((i % SIDE) as f32, (i / SIDE) as f32);
    let lines = Image::new(SIDE, SIDE, (0..SIDE * SIDE).map(|i| line(at(i).0, at(i).1).unwrap_or(NONE)).collect());
    let sketch = Image::new(
        SIDE,
        SIDE,
        (0..SIDE * SIDE)
            .map(|i| {
                if lines.data[i] != NONE {
                    return lines.data[i];
                }
                let mut u = unit((hash64(5, (i % SIDE) as i64, (i / SIDE) as i64, 0) >> 32) as u32) as f32;
                for (g, share) in f(at(i).0, at(i).1).iter().enumerate() {
                    if u < *share {
                        return [ROOF, CANOPY, FIELDS, WATER][g];
                    }
                    u -= share;
                }
                GROUND
            })
            .collect(),
    );
    let plan = plan(&sketch, Some(&lines));
    (sketch, plan)
}

fn plans() -> Vec<(&'static str, (Image, Plan))> {
    let s = SIDE as f32;
    let smooth = |e0: f32, e1: f32, x: f32| ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    let road = |d: f32| (d.abs() < 3.5).then_some(STREET);
    vec![
        (
            "forest into town",
            painted(
                &|x, _| {
                    let t = smooth(0.15 * s, 0.85 * s, x);
                    [0.5 * t, 0.9 * (1.0 - t), 0.0, 0.0]
                },
                &|_, y| road(y - s / 2.0),
            ),
        ),
        (
            "town in a forest",
            painted(
                &|x, y| {
                    let t = 1.0 - smooth(100.0, 160.0, ((x - s / 2.0).powi(2) + (y - s / 2.0).powi(2)).sqrt());
                    [0.45 * t, 0.85 * (1.0 - t), 0.0, 0.0]
                },
                &|x, y| road(x - s / 2.0).or(road(y - s / 2.0)),
            ),
        ),
        (
            "village along a road in fields",
            painted(
                &|x, y| {
                    let t = 1.0 - smooth(20.0, 60.0, (x - y).abs() / 2f32.sqrt());
                    [0.25 * t, 0.0, 0.85 * (1.0 - t), 0.0]
                },
                &|x, y| road((x - y) / 2f32.sqrt()),
            ),
        ),
        (
            "river through a town",
            painted(&|_, _| [0.45, 0.0, 0.0, 0.0], &|x, y| road(x - s / 2.0).or(((y - s / 2.0).abs() < 10.0).then_some(WATER))),
        ),
    ]
}

fn main() {
    let dir = format!("{OUT}/blend");
    std::fs::create_dir_all(&dir).expect("out dir");
    let n = LABELS.len();
    let places = real::places();
    assert!(places.len() >= 10, "real places in out/real/: run lab/scripts/overture.py");
    let (train_p, held_p): (Vec<_>, Vec<_>) = places.iter().enumerate().partition(|(i, _)| !real::held_out(*i));
    // Crops of the places' maps, and of their lines alike.
    let crops = |ps: &[(usize, &real::Place)]| -> Vec<(Image, Image)> {
        let maps = real::crops(&ps.iter().map(|(_, p)| &p.map).collect::<Vec<_>>(), SIDE, 4);
        let lines = real::crops(&ps.iter().map(|(_, p)| &p.lines).collect::<Vec<_>>(), SIDE, 4);
        maps.into_iter().zip(lines).collect()
    };
    let (train, held) = (crops(&train_p), crops(&held_p));
    let every =
        |v: &[(Image, Image)], k: usize| -> Vec<(Image, Image)> { v.iter().step_by((v.len() / k).max(1)).take(k).cloned().collect() };
    let maps = |v: &[(Image, Image)]| -> Vec<Image> { v.iter().map(|c| c.0.clone()).collect() };
    // Of each held-out place's 16 crops, the sixth is scored as if a model's (so it is not among the critic's real ones), the eleventh's plan drawn to.
    let real_row: Vec<Image> = held.iter().skip(5).step_by(16).take(SAMPLES).map(|c| c.0.clone()).collect();
    let targets: Vec<(Image, Image)> = held.iter().skip(10).step_by(16).take(SAMPLES).cloned().collect();
    let critic_real = maps(&every(&train, CRITIC_REAL));
    let critic_held =
        maps(&every(&held.iter().enumerate().filter(|(i, _)| i % 16 != 5).map(|(_, c)| c.clone()).collect::<Vec<_>>(), CRITIC_REAL));
    let (real_ctx, held_ctx): (Vec<Context>, Vec<Context>) =
        (critic_real.iter().map(context).collect(), critic_held.iter().map(context).collect());
    let references: Vec<Reference> =
        SCALES.iter().map(|s| Reference::new(&critic_real, &critic_held, n, *s, Some((&real_ctx, &held_ctx)))).collect();
    println!("## Drawing to a plan, from real places\n");
    println!(
        "{} places to learn from ({} crops of {SIDE} m), {} held out ({} crops). Synthesis copies from {EXAMPLES} crops.\n",
        train_p.len(),
        train.len(),
        held_p.len(),
        held.len()
    );

    // Each model learns from plans with lines and without.
    let start = Instant::now();
    let examples = every(&train, EXAMPLES);
    let options = Options {
        levels: 6,
        passes: 3,
        jitter: vec![0.5, 0.5, 0.4, 0.3, 0.2, 0.0, 0.0],
        clusters: 64,
        pool: 4,
        position: 0.0,
        mixed: true,
        guide: 8.0,
        lines: 4.0,
        seed: 1,
    };
    let exemplars = |with_lines: bool| {
        let plans = examples.iter().map(|(m, l)| plan(m, with_lines.then_some(l))).collect();
        Exemplars::new(&maps(&examples), n, options.clone()).with_plans(plans)
    };
    let (coords, coords_lines) = (exemplars(false), exemplars(true));
    eprintln!("indexed in {:.0} s", start.elapsed().as_secs_f64());
    let vols: Vec<Vol> = train.iter().map(|c| Vol::of_image(&c.0)).collect();
    let masked_model = |with_lines: bool| {
        let start = Instant::now();
        let plans: Vec<Plan> = train.iter().map(|(m, l)| plan(m, with_lines.then_some(l))).collect();
        let options = masked::Options {
            levels: 6,
            codes: 32,
            subdivisions: 4,
            radius: [2, 0, 2],
            steps: 16,
            hidden: 64,
            examples: 300_000,
            epochs: 2,
            position: [false; 3],
            augment: false,
            seed: 1,
        };
        let model = Model::learn_guided(&vols, &[], n, options, &plans, &[]);
        eprintln!("masked model in {:.0} s", start.elapsed().as_secs_f64());
        model
    };
    let (masked, masked_lines) = (masked_model(false), masked_model(true));
    type Draw<'a> = Box<dyn Fn(u64, &Plan) -> Image + 'a>;
    let models: Vec<(&str, bool, Draw)> = vec![
        ("coordinates", false, Box::new(|s, p| coords.sample_guided(s, (0, 0, SIDE, SIDE), Some(p)).0)),
        ("coordinates, lines", true, Box::new(|s, p| coords_lines.sample_guided(s, (0, 0, SIDE, SIDE), Some(p)).0)),
        ("masked", false, Box::new(|s, p| masked.sample_guided(s, [0; 3], masked.size(), Some(p)).to_image())),
        ("masked, lines", true, Box::new(|s, p| masked_lines.sample_guided(s, [0; 3], masked_lines.size(), Some(p)).to_image())),
    ];

    // Profiles against the critic's real crops, beyond how far its held-out ones are from them.
    let real_profile = Profile::of(&critic_real, n);
    let baseline = Profile::of(&critic_held, n).differences(&real_profile);
    let judged = |maps: &[Image]| {
        let mut diffs = Profile::of(maps, n).differences(&real_profile);
        profile::beyond(&mut diffs, &baseline);
        diffs
    };
    let cut: Vec<Image> = real_row.iter().enumerate().map(|(i, m)| profile::cut(m, STREET, GROUND, 16, 6, i as u64)).collect();
    let mut rows: Vec<(String, Vec<Image>, Vec<Context>)> = vec![
        ("real, held out (calibration)".into(), real_row.clone(), real_row.iter().map(context).collect()),
        ("real, streets cut every 16 m (calibration)".into(), cut, real_row.iter().map(context).collect()),
    ];
    let unguided: Vec<Image> = (0..SAMPLES as u64).map(|s| coords.sample(s, (0, 0, SIDE, SIDE)).0).collect();
    rows.push(("coordinates, unguided".into(), unguided.clone(), unguided.iter().map(context).collect()));
    for (name, with_lines, draw) in &models {
        let plans: Vec<Plan> = targets.iter().map(|(m, l)| plan(m, with_lines.then_some(l))).collect();
        let drawn = plans.iter().enumerate().map(|(s, p)| draw(s as u64, p)).collect();
        rows.push((name.to_string(), drawn, plans.into_iter().map(|p| p.context).collect()));
    }
    // The held-out places' own codes from a level up, the levels below drawn: which levels' draws lose what is joined?
    for from in GIVEN {
        let drawn = targets
            .iter()
            .enumerate()
            .map(|(s, (m, _))| masked.sample_given(s as u64, Some(&plan(m, None)), &Vol::of_image(m), from).to_image())
            .collect();
        rows.push((format!("masked, given from {} m", 1 << from), drawn, targets.iter().map(|(m, _)| context(m)).collect()));
    }

    println!(
        "Each model draws to the plans of held-out places; unguided, it draws what it likes. \
         Plan error: the mean difference between the shares (built, forest, fields, water) of what was drawn, by 16 m, \
         and those asked for. AUC: a critic against real windows of the context it was drawn to (or has), \
         so it judges how the plan was filled in. Profile: how many doublings, on average, the sizes of parts, \
         their widths, and the distances between labels moved from real places' beyond how far real places \
         differ from each other, each family's mean weighed by the labels' shares; and the distributions that moved most.\n"
    );
    println!(
        "| maps | plan error | {} | copied 16 | copied 32 | {} | moved most |",
        SCALES.iter().map(|s| format!("AUC {} m", s.extent())).collect::<Vec<_>>().join(" | "),
        profile::FAMILIES.join(" | ")
    );
    println!("|---|---|{}---|---|{}---|", "---|".repeat(SCALES.len()), "---|".repeat(profile::FAMILIES.len()));
    let sets: Vec<_> = [16, 32].iter().map(|k| critic::window_set(&maps(&train), *k)).collect();
    let name = |l: usize| LABELS[l].name;
    for (row, maps, plans) in &rows {
        let error = maps.iter().zip(plans).map(|(m, t)| context(m).error(t)).sum::<f64>() / maps.len() as f64;
        let aucs: Vec<String> =
            references.iter().enumerate().map(|(i, r)| format!("{:.2}", r.judge(maps, Some(plans), stream(9, i as u64)).auc)).collect();
        let mut diffs = judged(maps);
        let families = profile::by_family(&diffs);
        // Labels of a hundredth of the land or more, so a few stray cells of a rare one don't lead.
        diffs.retain(|d| d.weight >= 0.01);
        diffs.sort_by(|a, b| b.moved.total_cmp(&a.moved));
        let most: Vec<String> = diffs
            .iter()
            .take(3)
            .map(|d| match d.other {
                Some(b) => format!("{} near {} {:.1}", name(d.label), name(b), d.moved),
                None => format!("{} {} {:.1}", name(d.label), d.family, d.moved),
            })
            .collect();
        println!(
            "| {row} | {error:.3} | {} | {:.0}% | {:.0}% | {} | {} |",
            aucs.join(" | "),
            100.0 * critic::found(maps, 16, &sets[0], 2),
            100.0 * critic::found(maps, 32, &sets[1], 2),
            families.iter().map(|f| format!("{f:.2}")).collect::<Vec<_>>().join(" | "),
            most.join(", ")
        );
        eprintln!("{row} scored");
    }
    let grid: Vec<Vec<_>> =
        rows.iter().map(|(_, maps, _)| maps.iter().take(4).map(|m| (m.w, m.h, m.to_rgb(&label_rgb, 1))).collect()).collect();
    write_grid(&format!("{dir}/held.png"), &grid, 6);

    // Painted plans, by the models that take lines; enough draws for the critic to learn from and test on.
    println!(
        "\nPainted plans, roads and a river among their lines. Gap: how far the real windows the critic set \
         against the draws were from the plan, in shares: a plan unlike any real place's has a large one. \
         Sizes: as above, against all real places, so the plan's own mix counts too.\n"
    );
    let painted_models: Vec<&(&str, bool, Draw)> = models.iter().filter(|m| m.1).collect();
    println!(
        "| plan | gap | {} |",
        painted_models.iter().map(|m| format!("{0}: plan error | {0}: AUC 64 m | {0}: sizes", m.0)).collect::<Vec<_>>().join(" | ")
    );
    println!("|---|---|{}", "---|---|---|".repeat(painted_models.len()));
    let mut grid = Vec::new();
    for (name, (sketch, plan)) in plans() {
        let plans = vec![plan.context.clone(); DRAWN];
        let mut gap = 0.0;
        let mut cells = Vec::new();
        let mut shown = vec![sketch.clone()];
        for (_, _, draw) in &painted_models {
            let drawn: Vec<Image> = (0..DRAWN as u64).map(|s| draw(s, &plan)).collect();
            let error = drawn.iter().map(|m| context(m).error(&plan.context)).sum::<f64>() / drawn.len() as f64;
            let verdict = references[1].judge(&drawn, Some(&plans), 11);
            gap = verdict.gap;
            let sizes = profile::by_family(&judged(&drawn))[0];
            cells.push(format!("{error:.3} | {:.2} | {sizes:.2}", verdict.auc));
            shown.extend(drawn.into_iter().take(2));
        }
        println!("| {name} | {gap:.3} | {} |", cells.join(" | "));
        grid.push(shown.iter().map(|m| (m.w, m.h, m.to_rgb(&label_rgb, 1))).collect());
    }
    write_grid(&format!("{dir}/painted.png"), &grid, 6);
    println!(
        "\nPictures (out/blend/): held.png, four maps a row ({}); painted.png, each plan's sketch, then two draws by each of {}.",
        rows.iter().map(|r| r.0.as_str()).collect::<Vec<_>>().join(", "),
        painted_models.iter().map(|m| m.0).collect::<Vec<_>>().join(", ")
    );
}
