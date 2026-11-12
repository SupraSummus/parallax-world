//! Mountains: ranges along both sides of the valley, what the horizon
//! shows from anywhere in it. Massifs kilometres long take turns along
//! it, some low and wooded to their crests, some towering, snowy; where
//! one side is low the other tends to tower. Each range's face steepens
//! to a sharp crest of peaks and passes, spurs and gullies run down it,
//! side valleys cut into it, and higher ranges may stand behind. Part of
//! the valley's relief, not a domain: every domain stands on the ground
//! they raise. Above a wavering treeline the forest thins out, and a field
//! paints what is there instead: alpine meadows, scree under cliffs, rock
//! on crests and steep faces, snow above a snowline in hollows and on
//! gentle ground, not on windswept tops. Behind the first crest the high
//! country is cut into arêtes and cirques.
//!
//! The ranges are a belt: a few kilometres behind the first crest the
//! land leaves them, west down to the sea, east up to an upland under the
//! treeline. On the valley's relief, so the coast is ragged with islands.

use std::rc::Rc;

use worldgen::column::{Column, lowest_around};
use worldgen::generator::{Field, Layer, Op};
use worldgen::kinds;
use worldgen::noise::{Fbm, Fx, ValueNoise, fx, smoothstep, stream};
use worldgen::plan::Plan;
use worldgen::units::{Units, m};

use crate::fields;
use crate::landmarks::K as L;
use crate::town::K as T;

kinds! {
    base: fields::BASE + fields::KINDS.len() as u8;
    alpine: 1, ["#8fa052", "#9aab5c"];
    scree: 3, ["#a39d92", "#968f84"];
    rock: 3, ["#7e776e", "#6f6961"];
    snow: 14, ["#f1f4f7", "#e3e9ef"];
}

/** The valley's heights with its mountains, lattice units: from the sea's floor up to their highest peaks, and a little over. */
pub const HEIGHTS: (i64, i64) = (m(-90.0), m(1000.0));

/** Where the foothills start off the path, at the nearest and the furthest. */
const FOOT: (Fx, Fx) = (Fx::m(140.0), Fx::m(320.0));
/** The crests' height where the massifs are lowest, and where highest. */
const CRESTS: (Units, Units) = (Units::m(230.0), Units::m(900.0));
/** The highest the ranges raise the ground. */
const TOP: Fx = Fx::m(960.0);
/** The high country behind the first crest is this wide, at the narrowest and the widest; past it the land leaves the ranges over LEAVE. */
const BELT: (Units, Units) = (Units::m(1000.0), Units::m(2200.0));
const LEAVE: Units = Units::m(1500.0);
/**
 * The sea's level: under the valley's floor anywhere (its relief reaches
 * 31 m under 0), so the sea is wherever the ground is under it, west past
 * the ranges, where the lowland falls from LOWLAND, one in SHELF, to DEEP.
 */
pub const SEA: Units = Units::m(-40.0);
const LOWLAND: Fx = Fx::m(10.0);
const SHELF: i64 = 30;
const DEEP: Fx = Fx::m(-75.0);
/** East past the ranges, the upland's lowest, and how much higher its ridges rise. */
const UPLAND: Fx = Fx::m(160.0);
const UPLAND_RIDGES: Units = Units::m(90.0);
/** Each side's noise is read this far off the other's. */
const SIDE: i64 = m(40000.0);

/** 1 where `n` crosses its middle, falling to 0 at its ends: noise's crests. */
fn ridge(n: Fx) -> Fx {
    Fx::ONE - (n - Fx::HALF).abs() * 2
}

/** Column (x, z) turned by about 37°, so what noise draws doesn't line up with its grid, or with the valley. */
fn turned(x: i64, z: i64) -> (i64, i64) {
    ((4 * x - 3 * z) / 5, (3 * x + 4 * z) / 5)
}

/** The mountains of a seed: their relief, and where the trees and the snow end. */
pub struct Ranges {
    /** How far the foothills stand off the path, along it, on either side. */
    feet: ValueNoise<{ m(2600.0) }>,
    /** Higher massifs and lower stretches along the valley, kilometres apart. */
    massifs: Fbm<{ m(3600.0) }, 2>,
    /** How far the first crest stands back from the foothills. */
    reach: ValueNoise<{ m(1900.0) }>,
    /** Peaks and passes along the crests. */
    peaks: Fbm<{ m(1100.0) }, 2>,
    /** Spurs down the faces to the valley, and gullies between them. */
    spurs: Fbm<{ m(700.0) }, 2>,
    /** Side valleys, cut across the ranges. */
    sides: ValueNoise<{ m(2800.0) }>,
    /** The high country behind the first crest, and the upland's ridges past it. */
    behind: Fbm<{ m(1500.0) }, 2>,
    /** How wide the high country is, along each side. */
    belt: ValueNoise<{ m(5000.0) }>,
    /** Crags and ribs over everything. */
    crags: Fbm<{ m(260.0) }, 2>,
    /** The treeline and snowline, wavering over a slope. */
    lines: Fbm<{ m(180.0) }, 2>,
    /** Patches of scree among the rock and the grass. */
    patches: ValueNoise<{ m(14.0) }>,
}

impl Ranges {
    pub fn new(seed: u64) -> Ranges {
        Ranges {
            feet: ValueNoise::new(stream(seed, 60)),
            massifs: Fbm::new(stream(seed, 61)),
            reach: ValueNoise::new(stream(seed, 62)),
            peaks: Fbm::new(stream(seed, 63)),
            spurs: Fbm::new(stream(seed, 64)),
            sides: ValueNoise::new(stream(seed, 65)),
            behind: Fbm::new(stream(seed, 66)),
            belt: ValueNoise::new(stream(seed, 68)),
            crags: Fbm::new(stream(seed, 67)),
            lines: Fbm::new(stream(seed, 70)),
            patches: ValueNoise::new(stream(seed, 71)),
        }
    }

    /** How much the ranges raise column (x, z), `from_path` off the valley's path; with its fraction. */
    pub fn height(&self, x: i64, z: i64, from_path: Fx) -> Fx {
        // Each side's ranges are its own: their noise read far apart.
        let side = if x < 0 { 0 } else { SIDE };
        let foot = FOOT.0 + self.feet.at(z, side) * (FOOT.1 - FOOT.0);
        if from_path <= foot {
            return Fx::ZERO;
        }
        let out = from_path - foot;
        // Along the valley the massifs rise and fall; the crests of the high ones stand further back.
        // Where one side is low the other tends to tower, so the valley is rarely low on both.
        let massif = |at: i64| {
            let own = self.massifs.at(side + at, z);
            let other = self.massifs.at(SIDE - side + at, z);
            let massif = smoothstep(fx(0.36), fx(0.62), own * 3 / 4 + (Fx::ONE - other) / 4);
            CRESTS.0 * Fx::ONE + (CRESTS.1 - CRESTS.0) * massif
        };
        let high = massif(0);
        let run = Fx::m(380.0) + high * fx(0.75) + self.reach.at(side, z) * Units::m(400.0);
        // Peaks along the crest where its noise crosses the middle, passes between.
        let lean = out.round_units().0 / 4;
        let peak = ridge(self.peaks.at(side + lean, z));
        let crest = high * (fx(0.6) + peak * peak * fx(0.4));
        // Up the face to the crest, steepening; behind it, down to the high country.
        let up = if out < run { out / run } else { Fx::ONE };
        let mut h = crest * (up * 2 + up * up + up * up * up) / 4;
        if out > run {
            let (tx, tz) = turned(x, z);
            // Further ranges behind, of their own heights: snowy ones over low wooded ones in front, say.
            let crest = ridge(self.behind.at(tx, tz));
            let behind = massif(out.round_units().0) * (fx(0.25) + crest * crest * fx(0.75));
            // Falling steepest from the crest, so it is sharp.
            let past = Fx::ONE - ((out - run) / Units::m(700.0)).min(Fx::ONE);
            let fall = Fx::ONE - past * past;
            h = h - (h - behind) * fall;
        }
        // Spurs down the face, gullies between them, deepest halfway up.
        let spur = ridge(self.spurs.at(side + lean / 2, z));
        let halfway = up * (Fx::ONE - up) * 4;
        h = h - h * (Fx::ONE - spur) * halfway * fx(0.25);
        // Side valleys, cut across the ranges, deepest at their mouths, rising to passes.
        let across = (self.sides.at(side + lean * 2, z) - Fx::HALF).abs() * 2;
        let cut = Fx::ONE - smoothstep(Fx::ZERO, fx(0.5), across);
        h = h - h * cut * (fx(0.7) - up * fx(0.35));
        // Crags and ribs, more where the ground is higher: sharp ribs, rounded hollows between.
        // Behind the first crest they grow, cutting the high country into arêtes and cirques.
        let (tx, tz) = turned(x, z);
        let inland = smoothstep(run, run + Fx::m(500.0), out);
        let crag = ridge(self.crags.at(tx, tz));
        h = h + (crag * crag * fx(1.5) - Fx::HALF) * h * (fx(0.08) + inland * fx(0.1));
        let h = h.max(Fx::ZERO).min(TOP);
        // Past the high country, the land beyond: down to the sea west, an upland east.
        let past = out - run - (BELT.0.to_fx() + self.belt.at(side, z) * (BELT.1 - BELT.0));
        if past <= Fx::ZERO {
            return h;
        }
        let beyond = if side == 0 { (LOWLAND - past / SHELF).max(DEEP) } else { UPLAND + ridge(self.behind.at(tx, tz)) * UPLAND_RIDGES };
        h + (beyond - h) * smoothstep(Fx::ZERO, LEAVE.to_fx(), past)
    }

    /** Where trees end, by column, lattice units. */
    pub fn treeline(&self, x: i64, z: i64) -> Fx {
        Fx::m(230.0) + self.lines.at(x, z) * Units::m(120.0)
    }

    /** Where snow lies from, by column, lattice units: higher where the trees end higher. */
    fn snowline(&self, x: i64, z: i64) -> Fx {
        Fx::m(540.0) + self.lines.at(x, z) * Units::m(100.0)
    }

    /** How much of column (x, z), its ground `height` units high, is below the treeline: the forest's share is times this. */
    pub fn below_treeline(&self, x: i64, z: i64, height: i64) -> Fx {
        let line = self.treeline(x, z);
        Fx::ONE - smoothstep(line - Fx::m(30.0), line, Units(height).to_fx())
    }
}

/**
 * What covers the ground above the treeline: alpine meadows, then a belt
 * of scree and rock, snow above the snowline where it is not too steep to
 * lie (lower in hollows, bared on tops); rock on crests and steep faces,
 * scree under them; painted over whatever the domains under it drew, down
 * the faces they show.
 */
pub fn layers(ranges: Rc<Ranges>, ground: Rc<Plan<i64>>) -> Vec<Layer> {
    let sea = sea(ground.clone());
    let surface = Field::columns("mountains", 0.8, Op::Paint, move |x, z, spacing| {
        let h = ground.at(x, z);
        let height = Units(h).to_fx();
        let treeline = ranges.treeline(x, z);
        if height < treeline - Fx::m(30.0) {
            return Vec::new();
        }
        // Rise over run to the ground a metre off at any spacing, as smoothed to the spacing it would be gentler, and grassier
        // far away; and to the ground further off, for cliffs nearby.
        let around = |step: i64| [(step, 0), (-step, 0), (0, step), (0, -step)].map(|(dx, dz)| ground.at(x + dx, z + dz));
        let near = around(m(1.0));
        let slope =
            |heights: [i64; 4], step: i64| Units(heights.map(|n| (n - h).abs()).into_iter().max().unwrap_or(0)).to_fx() / Units(step);
        let steep = slope(near, m(1.0));
        let far = spacing.max(m(12.0));
        let wide = around(far);
        let cliffs = slope(wide, far);
        // How far the ground around lies over this column: snow stays in hollows and gullies, crests are bare.
        let hollow = Units(wide.iter().sum::<i64>() / 4 - h).to_fx();
        let (tx, tz) = turned(x, z);
        let patch = ranges.patches.at(tx, tz);
        let above = height - treeline;
        // Scree gathers in hollows and gullies, rock shows on ribs, give or take a patch.
        let lie = hollow + (patch - Fx::HALF) * Units::m(3.0);
        // Snow lies lower in hollows; under it a belt of bare scree and rock, then meadows.
        let snowline = ranges.snowline(x, z) + patch * Units::m(40.0) - hollow.max(Fx::ZERO) * 6;
        let bare = (snowline - Fx::m(100.0) + patch * Units::m(50.0)).min(treeline + Fx::m(230.0) + patch * Units::m(80.0));
        // The steepest it lies on: steeper in hollows, gentler the more the ground stands over what is around, so the wind
        // bares tops.
        let holds = if hollow > Fx::ZERO { fx(0.95) } else { fx(0.75) + hollow / Units::m(3.0) };
        let kind = if height >= snowline && steep < holds {
            K::snow
        } else if steep >= fx(1.05) || (above > Fx::m(60.0) && hollow < -Fx::m(2.5)) {
            K::rock
        } else if cliffs >= fx(1.0) || steep >= fx(0.75) {
            if lie > -Fx::m(0.5) { K::scree } else { K::rock }
        } else if height >= bare {
            if lie > -Fx::m(1.0) { K::scree } else { K::rock }
        } else if above >= patch * Units::m(30.0) - Fx::m(20.0) {
            if patch < fx(0.12) { K::scree } else { K::alpine }
        } else {
            return Vec::new();
        };
        let under = if kind == K::alpine { K::scree } else { K::rock };
        // As deep as the ground under it (the forest's terrain), so it paints all the sides that shows.
        Column::new(h, spacing).surface(kind, m(1.0)).sides(under, lowest_around(|x, z| ground.at(x, z), x, z, spacing), m(1.5)).spans()
    });
    vec![Layer::Field(surface), Layer::Field(sea)]
}

/** How much of ground `height` units high is dry land: none under the sea, all from a little over it. */
pub fn dry(height: i64) -> Fx {
    smoothstep(SEA.to_fx() + Fx::m(0.5), SEA.to_fx() + Fx::m(2.0), Units(height).to_fx())
}

/** The sea: water up to its level on sand, and a beach of it a little over. */
fn sea(ground: Rc<Plan<i64>>) -> Field {
    Field::columns("sea", 0.85, Op::Paint, move |x, z, spacing| {
        let h = ground.at(x, z);
        if h > (SEA + Units::m(1.5)).0 {
            return Vec::new();
        }
        Column::new(h, spacing).surface(L::sand, m(1.0)).water_to(T::water, SEA.0).spans()
    })
}
