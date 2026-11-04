//! Regimes: a whole valley-like area, each part refined by the model its
//! kind calls for.
//!
//! Top: settlements are sites of a plan, as the valley's towns are (a
//! site every SPACING along a line, jittered), each the centre of a box a
//! town crop wide. A plan rather than a parse of whole areas: the plane
//! has no edge to parse from, and a dozen areas would be a dozen examples
//! of one rule (towns.rs and the report say more).
//!
//! In a site's box the split grammar (grammar.rs, shared splits on) draws
//! the town and its fields as rectangles, down to rectangles whose rule
//! is texture (no town, not mostly fields: canopy and ground), where it
//! stops. Everywhere else, and
//! in those rectangles, a texture model fills in the pixels: W0's (synth.rs),
//! learned on countryside away from towns, its 16 m biomes sampled as
//! usual but, in a box, those of the place in the training crop each
//! block's rule was drawn from (the grammar says what grows where, the
//! texture model how it looks). Texture is a function of position, so it runs on across
//! rectangles' edges and the forest is not made of blocks.
//!
//! Each part is local, so a window alone is the same window of the whole.

use worldgen::noise::{hash64, stream};

use crate::grammar::{Grammar, Leaves, TEXTURE};
use crate::hierarchy::{self, Options};
use crate::image::Image;
use crate::mixture;
use crate::synth::{self, Report, Window};
use crate::towns::{self, CROP, LABELS};
use crate::valley::{self, Area};

/** Sites: one per SPACING along z on x = 0, jittered by up to JITTER each way. */
pub const SPACING: i64 = 1200;
pub const JITTER: i64 = 150;
/** A rule is texture where less than this of it is town, and less than FIELDS fields. */
const TOWN: f64 = 0.02;
const FIELDS: f64 = 0.5;

pub struct Regimes {
    pub grammar: Grammar,
    /** Which rules (the grammar's nodes) are texture. */
    pub texture: Vec<bool>,
    /** The countryside's texture model, and its finest biomes' block side. */
    pub model: synth::Model,
    pub block: usize,
    /** Each training crop's finest biomes (a cell per block), by the countryside's. */
    pub crop_biomes: Vec<Image>,
    pub report: String,
}

/** A site: its number along z, centre, and the seed its town is drawn with. */
#[derive(Clone, Copy, Debug)]
pub struct Site {
    pub j: i64,
    pub x: i64,
    pub y: i64,
    pub seed: u64,
}

impl Site {
    /** Its box's first column and row in the area. */
    pub fn origin(&self) -> (i64, i64) {
        (self.x - CROP as i64 / 2, self.y - CROP as i64 / 2)
    }
}

/** The sites of `seed` whose boxes meet the window (x0, y0) w × h. */
pub fn sites(seed: u64, (x0, y0, w, h): (i64, i64, usize, usize)) -> Vec<Site> {
    let half = CROP as i64 / 2;
    let (j0, j1) = ((y0 - half - JITTER).div_euclid(SPACING), (y0 + h as i64 + half + JITTER).div_euclid(SPACING));
    (j0..=j1)
        .map(|j| {
            let jitter = |n: i64| (hash64(seed, j, n, 41) % (2 * JITTER as u64 + 1)) as i64 - JITTER;
            Site { j, x: jitter(0), y: j * SPACING + jitter(1), seed: hash64(seed, j, 0, 42) }
        })
        .filter(|s| {
            let (sx, sy) = s.origin();
            sx < x0 + w as i64 && sy < y0 + h as i64 && sx + CROP as i64 > x0 && sy + CROP as i64 > y0
        })
        .collect()
}

impl Regimes {
    /**
     * The regimes of `grammar` (learned from town crops `crops`) and a
     * texture model of `countryside`, with W0's `options`.
     */
    pub fn learn(mut grammar: Grammar, countryside: &Image, options: &Options) -> Regimes {
        let n = grammar.n;
        assert_eq!(countryside.labels(), n, "the countryside shows every label, so its tokens number as the towns' do");
        let mut levels = hierarchy::learn(countryside, options);
        while levels.last().is_some_and(|l| l.fit.m == 1) {
            levels.pop();
        }
        let block = levels[0].b;
        let report =
            format!("texture model: biomes {}", levels.iter().map(|l| format!("{} of {}", l.fit.m, l.b)).collect::<Vec<_>>().join(", "));
        let images: Vec<(usize, &Image)> = levels.iter().map(|l| (l.b, &l.image)).collect();
        let model = synth::Model::new(countryside, &images, 8, true);

        // Rules are texture where they have next to no town, and not mostly fields.
        let texture: Vec<bool> = grammar
            .nodes
            .iter()
            .map(|nd| {
                let area = (nd.w * nd.h) as f64;
                (nd.hist[..=4].iter().sum::<u32>() as f64) < TOWN * area && (nd.hist[7] as f64) < FIELDS * area
            })
            .collect();

        // Each training crop's finest biomes, by the countryside's.
        let fit = &levels[0].fit;
        let crop_biomes: Vec<Image> = grammar
            .crops
            .iter()
            .map(|c| {
                let blocks = mixture::blocks_of(c, n, block, mixture::Tiles::Labels, &mut Default::default());
                Image::new(blocks.nx, blocks.ny, fit.classify(&blocks))
            })
            .collect();
        grammar.shared = true;
        grammar.texture = texture.clone();
        Regimes { grammar, texture, model, block, crop_biomes, report }
    }

    /** The biome under (x, y) of a site's box: of the place in its rule's training crop that the grammar drew it from. */
    fn biome_at(&self, site: &Site, x: i64, y: i64) -> u16 {
        let (sx, sy) = site.origin();
        let d = self.grammar.sample(site.seed, ((x - sx) as usize, (y - sy) as usize, 1, 1), Leaves::Pixels);
        let b = &self.crop_biomes[d.source[0] as usize];
        let (u, v) = d.at[0];
        b.at((u as usize / self.block).min(b.w - 1), (v as usize / self.block).min(b.h - 1))
    }

    /**
     * The area of `seed` over the window (x0, y0) w × h, alone; with
     * `regimes` off, the grammar fills its boxes whole (W1's way, shared
     * splits on), the texture model only outside them.
     */
    pub fn sample(&mut self, seed: u64, (x0, y0, w, h): (i64, i64, usize, usize), regimes: bool) -> Image {
        self.grammar.texture = if regimes { self.texture.clone() } else { Vec::new() };
        let mut report = Report::default();
        let window = Window::new(x0, y0, w, h);
        let texture_seed = stream(seed, 1);
        // The finest biomes: sampled, but in a box those under the rules the grammar drew there.
        let pw = synth::parents_window(&self.model, 0, window);
        let mut biomes = synth::sample(&self.model, texture_seed, 1, pw, &mut report);
        let b = self.block as i64;
        let all = sites(seed, (pw.x0 * b, pw.y0 * b, pw.w * self.block, pw.h * self.block));
        for j in 0..pw.h {
            for i in 0..pw.w {
                let (cx, cy) = ((pw.x0 + i as i64) * b + b / 2, (pw.y0 + j as i64) * b + b / 2);
                let inside = all.iter().find(|s| {
                    let (sx, sy) = s.origin();
                    (sx..sx + CROP as i64).contains(&cx) && (sy..sy + CROP as i64).contains(&cy)
                });
                if let Some(site) = inside {
                    biomes.set(i, j, self.biome_at(site, cx, cy));
                }
            }
        }
        let mut out = synth::refine(&self.model, texture_seed, 0, window, &biomes, &mut report);
        // The boxes: the grammar's pixels wherever it did not leave texture.
        for site in sites(seed, (x0, y0, w, h)) {
            let (sx, sy) = site.origin();
            let (ax, ay) = (sx.max(x0), sy.max(y0));
            let (bx, by) = ((sx + CROP as i64).min(x0 + w as i64), (sy + CROP as i64).min(y0 + h as i64));
            let (bw, bh) = ((bx - ax) as usize, (by - ay) as usize);
            let drawn = self.grammar.sample(site.seed, ((ax - sx) as usize, (ay - sy) as usize, bw, bh), Leaves::Pixels);
            for v in 0..bh {
                for u in 0..bw {
                    let l = drawn.image.at(u, v);
                    if l != TEXTURE {
                        out.set((ax - x0) as usize + u, (ay - y0) as usize + v, l);
                    }
                }
            }
        }
        out
    }
}

/** A real area of `seed`, side × side at a metre a column, centred on its town nearest z = 0, in labels; cached in `dir`. */
pub fn area(seed: u64, side: usize, threads: usize, dir: &str) -> Image {
    let (towns_, _) = valley::land(seed);
    let unit = worldgen::units::m(1.0);
    let (x, z) = towns_.near(0).map_or((0, 0), |t| (t.x / unit, t.z / unit));
    let half = (side / 2) as i64;
    let a = Area { level: 3, x0: x - half, z0: z - half, nx: side, nz: side };
    let v = valley::valley(seed, a, threads, &format!("{dir}/area-{seed}-{side}.bin"));
    Image::new(side, side, v.class.data.iter().map(|c| towns::label_of_class(*c)).collect())
}

/** The countryside of areas centred on their towns: their tiles a crop wide, but for the middle ones (the town and its fields), in a mosaic `across` tiles wide. */
pub fn countryside(areas: &[Image], across: usize) -> Image {
    let mut tiles = Vec::new();
    for a in areas {
        let n = a.w / CROP;
        for j in 0..n {
            for i in 0..n {
                let middle = |k: usize| (n / 2 - 1..=n / 2).contains(&k);
                if !(middle(i) && middle(j)) {
                    tiles.push(a.crop(i * CROP, j * CROP, CROP, CROP));
                }
            }
        }
    }
    let down = tiles.len() / across;
    let mut out = Image::filled(across * CROP, down * CROP, 0);
    for (t, tile) in tiles.iter().take(across * down).enumerate() {
        for y in 0..CROP {
            for x in 0..CROP {
                out.set((t % across) * CROP + x, (t / across) * CROP + y, tile.at(x, y));
            }
        }
    }
    assert!(LABELS.len() >= out.labels());
    out
}
