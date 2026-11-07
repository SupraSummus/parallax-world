//! Writes the split grammar with shared splits (grammar.rs), learned from
//! the towns as `shared` learns it, as the rule table the learned world
//! draws from (worlds/src/learned.rs); with `--check`, only says whether
//! the committed table is the one learned now.
//!
//! cargo run --release --example export [-- --check]

use lab::OUT;
use lab::grammar::{self, Grammar};
use lab::towns::{self, FIELDS, LABELS, TownCrop, WATER};

const TRAIN_SEEDS: u64 = 18;
const KINDS: usize = 32;
const TABLE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../worlds/data/town-grammar.bin");
/** A rule is texture, which the world draws as forest, where less than this of it is town, and less than FIELDS_SHARE fields. */
const TOWN_SHARE: f64 = 0.02;
const FIELDS_SHARE: f64 = 0.5;

/** Which rules (the grammar's nodes) are texture: next to no town (the labels up to WATER), and not mostly fields. */
fn texture(grammar: &Grammar) -> Vec<bool> {
    grammar
        .nodes
        .iter()
        .map(|nd| {
            let area = (nd.w * nd.h) as f64;
            let town = nd.hist[..=WATER as usize].iter().sum::<u32>() as f64;
            town < TOWN_SHARE * area && (nd.hist[FIELDS as usize] as f64) < FIELDS_SHARE * area
        })
        .collect()
}

fn main() {
    let check = std::env::args().any(|a| a == "--check");
    let all = towns::gather(&(1..=TRAIN_SEEDS).collect::<Vec<_>>(), -2..=2, 2, &format!("{OUT}/towns"));
    let train: Vec<&TownCrop> = all.iter().collect();
    let grammar = Grammar::learn(&towns::training(&train), LABELS.len(), KINDS, 1);
    let texture = texture(&grammar);
    let table = grammar::export(&grammar, &texture);
    println!(
        "{} towns, {} rules learned ({} texture), table of {} bytes",
        train.len(),
        grammar.nodes.len(),
        texture.iter().filter(|t| **t).count(),
        table.len()
    );
    if check {
        let same = std::fs::read(TABLE).is_ok_and(|t| t == table);
        println!("{TABLE}: {}", if same { "the same" } else { "differs" });
        std::process::exit(if same { 0 } else { 1 });
    }
    std::fs::write(TABLE, &table).expect("table written");
    println!("wrote {TABLE}");
}
