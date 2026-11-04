//! What the examples share: the area studied, and how a segmentation is
//! scored and described against the truth.

use std::time::Instant;

use crate::image::Image;
use crate::stats;
use crate::valley::{self, Area, CLASSES, DOMAINS, Valley};

/** Where the examples write. */
pub const OUT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/out");

/**
 * The valley of seed 42 at level 3 (a column a metre wide), 1.5 by 2.3 km:
 * its first two towns (z = 200 m and −1069 m, on x = 0), their fields,
 * and the forest around.
 */
pub const AREA: Area = Area { level: 3, x0: -768, z0: -1536, nx: 1536, nz: 2304 };

/** The valley over AREA, generated once (on two threads) and cached in OUT. */
pub fn data() -> Valley {
    let start = Instant::now();
    let a = AREA;
    let cache = format!("{OUT}/valley-{}-{}-{}-{}-{}.bin", a.level, a.x0, a.z0, a.nx, a.nz);
    let v = valley::valley(42, AREA, 2, &cache);
    eprintln!("data: {} × {} columns in {:.1} s", a.nx, a.nz, start.elapsed().as_secs_f64());
    v
}

pub fn class_rgb(l: u16) -> u32 {
    CLASSES[l as usize].rgb
}

pub fn domain_rgb(l: u16) -> u32 {
    valley::DOMAIN_RGB[l as usize]
}

/** Each column's domain as its class says (the kinds' owner), not the land. */
pub fn domain_of_classes(classes: &Image) -> Image {
    Image::new(classes.w, classes.h, classes.data.iter().map(|c| CLASSES[*c as usize].domain as u16).collect())
}

/** Prints a confusion matrix of biomes (rows) against domains, with each biome's commonest classes and town distance. */
pub fn describe(biomes: &Image, scale: usize, v: &Valley) {
    let m = biomes.labels();
    let mut domain = vec![[0u64; 3]; m];
    let mut class = vec![vec![0u64; CLASSES.len()]; m];
    let mut distance = vec![Vec::new(); m];
    for y in 0..biomes.h * scale {
        for x in 0..biomes.w * scale {
            let b = biomes.at(x / scale, y / scale) as usize;
            let i = y * v.area.nx + x;
            domain[b][v.owner.data[i] as usize] += 1;
            class[b][v.class.data[i] as usize] += 1;
            if x % scale == scale / 2 && y % scale == scale / 2 {
                distance[b].push(v.town_distance[i]);
            }
        }
    }
    println!("| biome | blocks | {} | median m to town | commonest classes |", DOMAINS.join(" | "));
    println!("|---|---|---|---|---|---|---|");
    for b in 0..m {
        let total: u64 = domain[b].iter().sum();
        let mut top: Vec<usize> = (0..CLASSES.len()).collect();
        top.sort_by_key(|c| std::cmp::Reverse(class[b][*c]));
        let top: Vec<String> =
            top.iter().take(4).map(|c| format!("{} {:.0}%", CLASSES[*c].name, 100.0 * class[b][*c] as f64 / total as f64)).collect();
        distance[b].sort_unstable();
        let median = distance[b].get(distance[b].len() / 2).copied().unwrap_or(0);
        let shares: Vec<String> = domain[b].iter().map(|d| format!("{:.0}%", 100.0 * *d as f64 / total as f64)).collect();
        println!(
            "| {b} | {} | {} | {} | {} |",
            biomes.data.iter().filter(|l| **l as usize == b).count(),
            shares.join(" | "),
            if median.abs() >= 10_000 { "none near".into() } else { median.to_string() },
            top.join(", ")
        );
    }
}

/** The adjusted Rand index of `predicted` against `truth`, per cell. */
pub fn ari(predicted: &Image, truth: &Image) -> f64 {
    stats::ari(&stats::confusion(&predicted.data, &truth.data, predicted.labels(), truth.labels().max(3)))
}

/**
 * Biomes as domains: each biome taken for the domain most of its cells
 * have. The adjusted Rand index of that against the truth, and the F1
 * score of the town and of the fields (forest is most of the land, so
 * purity says little).
 */
pub fn as_domains(predicted: &Image, truth: &Image) -> (f64, f64, f64) {
    let c = stats::confusion(&predicted.data, &truth.data, predicted.labels(), 3);
    let domain: Vec<u16> = c.iter().map(|row| (0..3).max_by_key(|d| row[*d]).unwrap_or(0) as u16).collect();
    let merged: Vec<u16> = predicted.data.iter().map(|b| domain[*b as usize]).collect();
    let m = stats::confusion(&merged, &truth.data, 3, 3);
    let f1 = |d: usize| {
        let hit = m[d][d] as f64;
        let (said, is) = (m[d].iter().sum::<u64>() as f64, m.iter().map(|r| r[d]).sum::<u64>() as f64);
        if hit == 0.0 { 0.0 } else { 2.0 * hit / (said + is) }
    };
    (stats::ari(&m), f1(1), f1(2))
}
