//! Biomes of biomes: a label image's blocks fitted as a mixture
//! (mixture.rs), the blocks' biomes taken as the next, coarser image, and
//! so on up, while description length still finds more than one biome and
//! there are blocks enough.

use std::collections::HashMap;

use crate::image::Image;
use crate::mixture::{self, Blocks, Fit, Tiles};

pub struct Level {
    /** Cells of the level below per cell of this one, across. */
    pub b: usize,
    pub fit: Fit,
    /** Each number of components tried: its description length, bits. */
    pub bits: Vec<(usize, f64)>,
    /** The biomes: a cell per block of the level below. */
    pub image: Image,
}

pub struct Options {
    /** Block sizes and tiles, finest level first; the last repeats. */
    pub sizes: Vec<usize>,
    pub tiles: Vec<Tiles>,
    pub max_m: usize,
    /** Stop where a level would have fewer blocks than this. */
    pub min_blocks: usize,
    pub restarts: usize,
    pub seed: u64,
    /** Nats a biome scores per side shared with a neighbour's (see mixture::fit); 0 for none. */
    pub smooth: f64,
}

/** One level fitted to `image`: blocks of `b` as `tiles`, the number of biomes by description length. */
pub fn level(image: &Image, b: usize, tiles: Tiles, options: &Options) -> (Level, Blocks) {
    let blocks = mixture::blocks(image, b, tiles, &mut HashMap::new());
    let (fit, fits) = mixture::select(&blocks, options.max_m, options.seed, options.restarts, options.smooth);
    let bits = fits.iter().map(|f| (f.m, f.bits())).collect();
    (Level { b, image: fit.image(&blocks), fit, bits }, blocks)
}

/** Levels over `base`, finest first, until one finds a single biome or blocks run short. */
pub fn learn(base: &Image, options: &Options) -> Vec<Level> {
    let mut levels: Vec<Level> = Vec::new();
    loop {
        let b = options.sizes[levels.len().min(options.sizes.len() - 1)];
        let tiles = options.tiles[levels.len().min(options.tiles.len() - 1)];
        let below = levels.last().map_or(base, |l| &l.image);
        if (below.w / b) * (below.h / b) < options.min_blocks {
            break;
        }
        let (level, _) = level(below, b, tiles, options);
        let single = level.fit.m == 1;
        levels.push(level);
        if single {
            break;
        }
    }
    levels
}
