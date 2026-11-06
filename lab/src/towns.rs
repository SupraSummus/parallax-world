//! The town's data: many towns, each a square crop of the valley around
//! it, from several seeds and along the path, at a metre a column; cached
//! in out/towns/.
//!
//! The 21 classes are folded into a few labels that tell a town's shapes
//! apart (LABELS): what a town builds, and the rest by what it looks like
//! from above (canopy, low growth, fields).

use worldgen::units::m;

use crate::image::Image;
use crate::valley::{self, Area};

/**
 * A crop's side, metres: the largest town (3 blocks a side, 5 rows) is
 * 254 × 228 m, 266 × 240 m with its wall. Gathered MARGIN wider each
 * side, to be cut at a shift.
 */
pub const CROP: usize = 384;
pub const MARGIN: usize = 32;

/** Towns are this far apart along the path (worlds' town.rs, private there). */
const SPACING: i64 = m(1200.0);

pub struct Label {
    pub name: &'static str,
    pub rgb: u32,
}

pub const LABELS: [Label; 8] = [
    Label { name: "roof", rgb: 0xa8432c },
    Label { name: "wall", rgb: 0xece6d6 },
    Label { name: "street", rgb: 0x5a5650 },
    Label { name: "paving", rgb: 0xb4ab98 },
    Label { name: "fittings, water", rgb: 0x2b4d70 },
    Label { name: "canopy", rgb: 0x2e5a2a },
    Label { name: "ground", rgb: 0x86b04c },
    Label { name: "fields", rgb: 0xe0c050 },
];
pub const ROOF: u16 = 0;
pub const STREET: u16 = 2;
pub const PAVING: u16 = 3;

/** A valley class's label. */
pub fn label_of_class(class: u16) -> u16 {
    match class {
        10 => 0,
        11 => 1,
        12 => 2,
        13 => 3,
        14 | 15 => 4,
        4..=7 => 5,
        16..=20 => 7,
        _ => 6,
    }
}

pub fn label_rgb(l: u16) -> u32 {
    LABELS[l as usize].rgb
}

/** A town in the data: its seed, number along the path, and the land around it, in labels, MARGIN wider than a crop each side. */
pub struct TownCrop {
    pub seed: u64,
    pub n: i64,
    pub labels: Image,
}

impl TownCrop {
    /** The crop with its centre (dx, dz) from the town's, mirrored along x if `mirror`. */
    pub fn crop(&self, dx: i64, dz: i64, mirror: bool) -> Image {
        let at = |d: i64| (MARGIN as i64 + d).clamp(0, 2 * MARGIN as i64) as usize;
        let c = self.labels.crop(at(dx), at(dz), CROP, CROP);
        if !mirror {
            return c;
        }
        Image::new(CROP, CROP, (0..CROP * CROP).map(|i| c.at(CROP - 1 - i % CROP, i / CROP)).collect())
    }
}

/** A crop's shift from its town's centre, number `i`: anywhere in ±MARGIN, so a town is not aligned to the cells of any level. */
pub fn shift(t: &TownCrop, i: i64) -> (i64, i64) {
    let h = worldgen::noise::hash64(t.seed, t.n, i, 77);
    let m = 2 * MARGIN as u64;
    ((h % m) as i64 - MARGIN as i64, ((h >> 20) % m) as i64 - MARGIN as i64)
}

/** The crops to learn from: each town's at two shifts, as it is and mirrored. */
pub fn training(towns: &[&TownCrop]) -> Vec<Image> {
    towns
        .iter()
        .flat_map(|t| (0..2).flat_map(move |i| [false, true].map(|m| (i, m))).map(move |(i, m)| (t, i, m)))
        .map(|(t, i, m)| {
            let (dx, dz) = shift(t, i);
            t.crop(dx, dz, m)
        })
        .collect()
}

/** The held-out crops: each town's at a shift of its own. */
pub fn held_out(towns: &[&TownCrop]) -> Vec<Image> {
    towns
        .iter()
        .map(|t| {
            let (dx, dz) = shift(t, 9);
            t.crop(dx, dz, false)
        })
        .collect()
}

/** The towns of `seeds` numbered `ns` along the path (some numbers have none), generated on `threads` threads, cached in `dir`. */
pub fn gather(seeds: &[u64], ns: std::ops::RangeInclusive<i64>, threads: usize, dir: &str) -> Vec<TownCrop> {
    let mut wanted = Vec::new();
    for &seed in seeds {
        let (towns, _) = valley::land(seed);
        for n in ns.clone() {
            if let Some(t) = towns.near(n * SPACING) {
                wanted.push((seed, n, t.x / m(1.0), t.z / m(1.0)));
            }
        }
    }
    let next = std::sync::atomic::AtomicUsize::new(0);
    let mut crops: Vec<(usize, TownCrop)> = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..threads)
            .map(|_| {
                let (wanted, next) = (&wanted, &next);
                scope.spawn(move || {
                    let mut out = Vec::new();
                    loop {
                        let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        let Some(&(seed, n, x, z)) = wanted.get(i) else { break };
                        let side = CROP + 2 * MARGIN;
                        let half = (side / 2) as i64;
                        let area = Area { level: 3, x0: x - half, z0: z - half, nx: side, nz: side };
                        let v = valley::valley(seed, area, 1, &format!("{dir}/town-{seed}-{n}.bin"));
                        let labels = Image::new(side, side, v.class.data.iter().map(|c| label_of_class(*c)).collect());
                        out.push((i, TownCrop { seed, n, labels }));
                    }
                    out
                })
            })
            .collect();
        handles.into_iter().flat_map(|h| h.join().expect("a gathering thread")).collect()
    });
    crops.sort_by_key(|(i, _)| *i);
    crops.into_iter().map(|(_, c)| c).collect()
}
