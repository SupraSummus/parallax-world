//! Gathers the towns (towns.rs) and draws them.
//!
//! cargo run --release --example gather [seeds]

use std::time::Instant;

use lab::experiment::OUT;
use lab::towns::{self, CROP, label_rgb};

fn main() {
    let seeds: u64 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(1);
    let start = Instant::now();
    let crops = towns::gather(&(1..=seeds).collect::<Vec<_>>(), -2..=2, 2, &format!("{OUT}/towns"));
    eprintln!("{} towns in {:.1} s", crops.len(), start.elapsed().as_secs_f64());
    let pictures: Vec<_> = crops.iter().take(6).map(|c| (CROP, CROP, c.crop(0, 0, false).to_rgb(&label_rgb, 1))).collect();
    lab::image::write_side_by_side(&format!("{OUT}/towns.png"), &pictures, 8);
}
