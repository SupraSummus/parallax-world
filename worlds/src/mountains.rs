//! Mountains: ranges along both sides of the valley, rising a few hundred
//! metres off its path to peaks a couple of hundred metres high, which is
//! what the horizon shows from anywhere in it. Part of the valley's relief,
//! not a domain: every domain stands on the ground they raise. Above the
//! treeline the forest thins out, and a field paints what is there
//! instead: alpine grass, then scree and rock, snow on the peaks.

use std::rc::Rc;

use worldgen::column::Column;
use worldgen::generator::{Field, Op};
use worldgen::kinds;
use worldgen::noise::{Fbm, Fx, ValueNoise, fx, smoothstep, stream};
use worldgen::plan::Plan;
use worldgen::units::{Units, m};

use crate::fields;

kinds! {
    base: fields::BASE + fields::KINDS.len() as u8;
    alpine: 1, ["#8fa052", "#9aab5c"];
    scree: 3, ["#a39d92", "#968f84"];
    rock: 3, ["#7e776e", "#6f6961"];
    snow: 14, ["#f1f4f7", "#e3e9ef"];
}

/** The valley's heights with its mountains, lattice units: up to their peaks, and a little over for what stands on them. */
pub const HEIGHTS: (i64, i64) = (m(-36.0), m(256.0));

/** Where the ranges' foothills start, off the path, and how far out they reach their full height. */
const FOOT: Fx = Fx::m(220.0);
const FULL: Fx = Fx::m(900.0);

/** The mountains of a seed: their relief, and where the trees and the snow end. */
pub struct Ranges {
    /** Crests where this crosses its middle, a few hundred metres apart. */
    ridges: Fbm<{ m(1100.0) }, 4>,
    /** The ranges' bulk under the crests. */
    bulk: Fbm<{ m(1800.0) }, 3>,
    /** Higher massifs and lower saddles along the valley, kilometres apart. */
    massifs: ValueNoise<{ m(3200.0) }>,
    /** The treeline and snowline, wavering over a slope. */
    lines: Fbm<{ m(180.0) }, 2>,
    /** Patches of scree among the rock and the grass. */
    patches: ValueNoise<{ m(14.0) }>,
}

impl Ranges {
    pub fn new(seed: u64) -> Ranges {
        Ranges {
            ridges: Fbm::new(stream(seed, 60)),
            bulk: Fbm::new(stream(seed, 61)),
            massifs: ValueNoise::new(stream(seed, 62)),
            lines: Fbm::new(stream(seed, 63)),
            patches: ValueNoise::new(stream(seed, 64)),
        }
    }

    /** How much the ranges raise column (x, z), `from_path` off the valley's path; with its fraction. */
    pub fn height(&self, x: i64, z: i64, from_path: Fx) -> Fx {
        let rise = smoothstep(FOOT, FULL, from_path);
        if rise == Fx::ZERO {
            return Fx::ZERO;
        }
        let crest = Fx::ONE - (self.ridges.at(x, z) - Fx::HALF).abs() * 2;
        let peaks = smoothstep(fx(0.45), Fx::ONE, crest);
        let massif = fx(0.4) + self.massifs.at(x, z);
        let bulk = smoothstep(fx(0.3), fx(0.7), self.bulk.at(x, z));
        rise * massif * (bulk * Units::m(70.0) + peaks * peaks * Units::m(110.0) + rise * Units::m(20.0))
    }

    /** Where trees end, by column, lattice units. */
    fn treeline(&self, x: i64, z: i64) -> Fx {
        Fx::m(75.0) + self.lines.at(x, z) * Units::m(25.0)
    }

    /** Where snow lies from, by column, lattice units: higher where the trees end higher. */
    fn snowline(&self, x: i64, z: i64) -> Fx {
        Fx::m(125.0) + self.lines.at(x, z) * Units::m(40.0)
    }

    /** How much of column (x, z), its ground `height` units high, is below the treeline: the forest's share is times this. */
    pub fn below_treeline(&self, x: i64, z: i64, height: i64) -> Fx {
        let line = self.treeline(x, z);
        Fx::ONE - smoothstep(line - Fx::m(20.0), line, Units(height).to_fx())
    }
}

/**
 * What covers the ground above the treeline: alpine grass, then scree and
 * rock, snow above the snowline but where it is too steep to lie; painted
 * over whatever the domains under it drew.
 */
pub fn layers(ranges: Rc<Ranges>, ground: Rc<Plan<i64>>) -> Vec<worldgen::generator::Layer> {
    let surface = Field::columns("mountains", 0.8, Op::Paint, move |x, z, spacing| {
        let h = ground.at(x, z);
        let height = Units(h).to_fx();
        let treeline = ranges.treeline(x, z);
        if height < treeline - Fx::m(10.0) {
            return Vec::new();
        }
        // Rise over run to the sites around, at least a metre off.
        let step = spacing.max(m(1.0));
        let rise = [(step, 0), (-step, 0), (0, step), (0, -step)].map(|(dx, dz)| (ground.at(x + dx, z + dz) - h).abs()).into_iter().max();
        let slope = Units(rise.unwrap_or(0)).to_fx() / Units(step);
        let patch = ranges.patches.at(x, z);
        let kind = if height >= ranges.snowline(x, z) + patch * Units::m(8.0) && slope < fx(1.1) {
            K::snow
        } else if slope >= fx(0.9) || height >= treeline + Fx::m(30.0) + patch * Units::m(20.0) {
            if patch < fx(0.4) { K::scree } else { K::rock }
        } else if height >= treeline + patch * Units::m(10.0) - Fx::m(10.0) {
            if patch < fx(0.25) { K::scree } else { K::alpine }
        } else {
            return Vec::new();
        };
        Column::new(h, spacing).surface(kind, m(1.0)).spans()
    });
    vec![worldgen::generator::Layer::Field(surface)]
}
