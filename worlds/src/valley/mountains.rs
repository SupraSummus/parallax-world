//! Mountains: ranges along both sides of the valley, winding with its
//! course (course.rs), what the horizon shows from anywhere in it.
//! Massifs kilometres long take turns along it, some low and wooded to
//! their crests, some towering, snowy; where one side is low the other
//! tends to tower. Each range's face steepens
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
//!
//! At their feet, every few kilometres, a rock spur stands out onto the
//! valley's floor or a gorge cuts into it (`Landforms`), what the railway
//! along the valley must tunnel through or bridge.

use std::sync::Arc;

use worldgen::column::{Column, lowest_around};
use worldgen::domain::{Domain, Scope};
use worldgen::generator::{Field, Layer, Meets, Op};
use worldgen::kinds;
use worldgen::noise::{Fbm, Fx, Rng, ValueNoise, fx, hash64, smoothstep};
use worldgen::plan::{self, Holding, Sloped};
use worldgen::units::{Units, m};

use super::course::COURSE_SLOPE;
use super::fields;
use super::landmarks::K as L;
use super::town::{self, Towns};
use crate::SURFACE;

kinds! {
    base: fields::BASE + fields::KINDS.len() as u8;
    alpine: 1, ["#8fa052", "#9aab5c"];
    scree: 3, ["#a39d92", "#968f84"];
    rock: 3, ["#7e776e", "#6f6961"];
    snow: 14, ["#f1f4f7", "#e3e9ef"];
    sea: 13, ["#2478a0"], walk_through, medium;
    seaTop: 13, ["#9cc4d6"], walk_through, medium;
}

/** The valley's heights with its mountains, lattice units: from the sea's floor up to their highest peaks, and a little over. */
pub const HEIGHTS: (i64, i64) = (m(-90.0), m(1000.0));

/** Where the foothills start off the valley's course, at the nearest and the furthest. */
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
/**
 * The sea is see-through: a medium whose colour is dark in red, so it stops
 * red first (worldgen's `stops_faster`). Its floor shows greener, then bluer,
 * the deeper it is, to some 10 m down.
 * Its top SEA_TOP is another, lighter and denser, so that where the sea
 * ends shows: little from above, most along the water.
 */
const SEA_STOPS: f64 = 0.8 / m(6.0) as f64;
const SEA_TOP: i64 = m(0.25);
const SEA_TOP_STOPS: f64 = 0.4 / SEA_TOP as f64;
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
    pub fn new(scope: &Scope) -> Ranges {
        Ranges {
            feet: ValueNoise::new(scope.seed("feet")),
            massifs: Fbm::new(scope.seed("massifs")),
            reach: ValueNoise::new(scope.seed("reach")),
            peaks: Fbm::new(scope.seed("peaks")),
            spurs: Fbm::new(scope.seed("spurs")),
            sides: ValueNoise::new(scope.seed("sides")),
            behind: Fbm::new(scope.seed("behind")),
            belt: ValueNoise::new(scope.seed("belt")),
            crags: Fbm::new(scope.seed("crags")),
            lines: Fbm::new(scope.seed("lines")),
            patches: ValueNoise::new(scope.seed("patches")),
        }
    }

    /** How much the ranges raise column (x, z), `across` from the valley's course (west less than 0); with its fraction. */
    pub fn height(&self, x: i64, z: i64, across: Fx) -> Fx {
        // Each side's ranges are its own: their noise read far apart.
        let side = if across < Fx::ZERO { 0 } else { SIDE };
        let foot = FOOT.0 + self.feet.at(z, side) * (FOOT.1 - FOOT.0);
        if across.abs() <= foot {
            return Fx::ZERO;
        }
        let out = across.abs() - foot;
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

/** Each side's landforms at the ranges' feet: one or none in each stretch this long along the valley, within it by MARGIN. */
pub const APART: i64 = m(3000.0);
const MARGIN: i64 = m(150.0);
/** Where across from the course a spur's nose starts and a gorge is at its full depth: past the towns, short of the railway's band. */
pub const NOSE: i64 = m(195.0);
const _: () = assert!(NOSE > town::ACROSS + (COURSE_SLOPE * town::LONG as f64) as i64);
/** A spur's nose rises over this across; its crest behind rises one in RIDGE, to meet the face. */
const CLIFF: Fx = Fx::m(30.0);
const RIDGE: i64 = 5;
/** A gorge fades out on the floor over this, inside NOSE; it and a spur, up the face, between FAR's two (across from the course), so neither stands past the ranges. */
const FADE: Fx = Fx::m(70.0);
const FAR: (Fx, Fx) = (Fx::m(700.0), Fx::m(1000.0));
/** A gorge's bed, flat, its share of its half width either side of its middle; steep walls from there. */
const BED: Fx = fx(0.4);

/** A landform at the ranges' feet, at z along the valley: its height or depth, and its half width along z. */
#[derive(Clone, Copy)]
pub enum Form {
    Spur { z: i64, high: Fx, half: Fx },
    Gorge { z: i64, deep: Fx, half: Fx },
}

/**
 * Landforms at the ranges' feet along each side, one or none every APART,
 * none by a town: rock spurs, ridges down the face whose steep noses
 * stand out onto the valley's floor, and gorges that side streams cut down
 * the face and into the floor, fading out on it. Both reach across from
 * NOSE out past the railway's band, so the railway, which can't go round
 * or over them, tunnels through a spur and bridges a gorge.
 */
pub struct Landforms {
    seed: u64,
    towns: Arc<Towns>,
}

impl Landforms {
    pub fn new(scope: &Scope, towns: Arc<Towns>) -> Landforms {
        Landforms { seed: scope.seed("forms"), towns }
    }

    /** The form in the stretch of side `side` (±1) that z is in, if any: two in five a spur, two a gorge. */
    pub fn form(&self, side: i64, z: i64) -> Option<Form> {
        let stretch = z.div_euclid(APART);
        let mut random = Rng::new(hash64(self.seed, stretch, side, 0));
        let at = stretch * APART + MARGIN + i64::from(random.below((APART - 2 * MARGIN) as u32));
        // Clear of a town and the ground it levels round it (120 m), a form's half width, and room.
        if self.towns.near(at).is_some_and(|t| (t.z - at).abs() < town::LONG + m(260.0)) {
            return None;
        }
        let between = |lo: f64, hi: f64, random: &mut Rng| Fx::m(lo) + Units::m(hi - lo).to_fx() * random.fraction();
        match random.below(5) {
            0 | 1 => Some(Form::Spur { z: at, high: between(30.0, 55.0, &mut random), half: between(60.0, 90.0, &mut random) }),
            2 | 3 => Some(Form::Gorge { z: at, deep: between(20.0, 32.0, &mut random), half: between(30.0, 45.0, &mut random) }),
            _ => None,
        }
    }

    /**
     * What the landforms add to column (x, z) `across` from the valley's
     * course, where the ranges raise it `h` (a gorge's depth subtracted):
     * a spur stands over the ranges, a gorge cuts into them and the floor.
     */
    pub fn at(&self, z: i64, across: Fx, h: Fx) -> Fx {
        let out = across.abs() - Units(NOSE).to_fx();
        if out <= -FADE || across.abs() >= FAR.1 {
            return Fx::ZERO;
        }
        let side = if across < Fx::ZERO { -1 } else { 1 };
        let far = Fx::ONE - smoothstep(FAR.0, FAR.1, across.abs());
        match self.form(side, z) {
            Some(Form::Spur { z: at, high, half }) if out > Fx::ZERO => {
                let crest = (high * smoothstep(Fx::ZERO, CLIFF, out) + out / RIDGE) * far;
                let top = crest * (Fx::ONE - smoothstep(Fx::ZERO, half, Units((z - at).abs()).to_fx()));
                // Nothing where it has faded, over the sea's floor too.
                (top - h.max(Fx::ZERO)).max(Fx::ZERO)
            }
            Some(Form::Gorge { z: at, deep, half }) => {
                -deep * smoothstep(-FADE, Fx::ZERO, out) * far * (Fx::ONE - smoothstep(half * BED, half, Units((z - at).abs()).to_fx()))
            }
            _ => Fx::ZERO,
        }
    }
}

/** Part of the relief, so holding no land: drawn over all of it. */
impl Domain for Ranges {
    fn holds_land(&self) -> bool {
        false
    }

    fn layers(self: Arc<Self>, ground: &Arc<Sloped>, _: Holding) -> Vec<Layer> {
        layers(self, ground.clone())
    }
}

/**
 * What covers the ground above the treeline: alpine meadows, then a belt
 * of scree and rock, snow above the snowline where it is not too steep to
 * lie (lower in hollows, bared on tops); rock on crests and steep faces,
 * scree under them; painted over whatever the domains under it drew, down
 * the faces they show.
 */
fn layers(ranges: Arc<Ranges>, ground: Arc<Sloped>) -> Vec<Layer> {
    let bounds = ground.clone();
    let sea = sea(ground.clone());
    let surface = Field::columns("mountains", 0.8, Op::Paint, move |x, z| {
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
        let far = plan::spacing().max(m(12.0));
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
        Column::new(h).surface(kind, SURFACE).sides(under, lowest_around(|x, z| ground.at(x, z), x, z), m(1.5)).spans()
    })
    .about(bounds, SURFACE, 1);
    vec![Layer::Field(surface), Layer::Field(sea)]
}

/** How much of ground `height` units high is dry land: none under the sea, all from a little over it. */
pub fn dry(height: i64) -> Fx {
    smoothstep(SEA.to_fx() + Fx::m(0.5), SEA.to_fx() + Fx::m(2.0), Units(height).to_fx())
}

/** The sea: water up to its level on sand, its top apart (SEA_TOP), and a beach of sand a little over; nothing higher. */
fn sea(ground: Arc<Sloped>) -> Field {
    Field::columns("sea", 0.85, Op::Paint, move |x, z| {
        let h = ground.at(x, z);
        if h > (SEA + Units::m(1.5)).0 {
            return Vec::new();
        }
        let deep = (SEA.0 - h).max(0);
        let top = deep.min(SEA_TOP);
        Column::new(h).surface(L::sand, m(1.0)).medium(K::sea, deep - top, SEA_STOPS).medium(K::seaTop, top, SEA_TOP_STOPS).spans()
    })
    .meets(|b, _| Meets::under((SEA + Units::m(1.5)).0 + 1, b))
}
