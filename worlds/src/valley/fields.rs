//! Farmland around the valley's towns: plots of wheat, stubble with
//! haystacks, ploughed and planted rows, meadows, hedges between them.
//! A domain of the valley (valley.rs) that reads only the towns' places
//! and the ground's slope, both coarser than it: it bids for gentle land
//! near towns, and draws only the columns it wins.

use std::sync::Arc;

use ground::column::Column;
use ground::domain::{Domain, Scope};
use ground::plan::{Bid, Bids, Holding, Plan, Sloped};
use ground::{FieldAbout, PopulationOn, Within};
use worldgen::generator::{Layer, Node, Op, Population, Shape, stripe};
use worldgen::kinds;
use worldgen::noise::{Fbm, Fx, Rng, ValueNoise, fx, hash, hash64, smoothstep};
use worldgen::units::{Units, m};

use super::town::{self, Towns};
use crate::forest::K as F;
use crate::path::Path;

kinds! {
    base: town::BASE + town::KINDS.len() as u8;
    wheat: 1, ["#d9b44a", "#e3c25a", "#cfa83e"], walk_through;
    stubble: 1, ["#c8b070", "#bba363", "#d2bb7c"];
    furrow: 2, ["#5e4430", "#6a4d36"];
    tilth: 2, ["#7a5a3e", "#86644a"];
    crop: 5, ["#4f8a34", "#5a9a3c", "#467c2e"], walk_through;
    meadow: 1, ["#7aae48", "#86b850", "#6fa040"];
    // The meadow's grass, in its colours: walked through, the meadow not.
    meadowGrass: 1, ["#7aae48", "#86b850", "#6fa040"], walk_through;
}

/** Plan scale of the slope they bid by. */
pub const SLOPE_SCALE: i64 = m(16.0);

/**
 * The fields' bid for column (x, z) round the valley's towns: up to 0.6
 * at a town's edge, none 90 to 150 m out (by noise), and none where the
 * ground is steep: `slope`, rise over run. And over rectangles
 * (Domain::bids): none beyond 150 m of the towns.
 */
pub fn round_towns(scope: &Scope, towns: Arc<Towns>, slope: Arc<Plan<Fx>>) -> (Bid, Bids) {
    let reach = Fbm::<{ m(160.0) }, 3>::new(scope.seed("reach"));
    let (least, more) = (Units::m(90.0), Units::m(60.0));
    let bids: Bids = {
        let towns = towns.clone();
        Box::new(move |min, max| (-Fx::ONE, if towns.least_distance(min, max) >= least + more { -Fx::ONE } else { fx(0.6) }))
    };
    let bid: Bid = Box::new(move |x, z| {
        let d = towns.distance(x, z).max(Units(0));
        let r = least + (reach.at(x, z) * more).floor_units();
        if d >= r {
            return -Fx::ONE;
        }
        let near = fx(0.6) * (r - d) / r;
        near - smoothstep(fx(0.12), fx(0.25), slope.at(x, z))
    });
    (bid, bids)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Crop {
    Wheat,
    Stubble,
    Ploughed,
    Rows,
    Meadow,
    Hedge,
}

const CROPS: [Crop; 6] = [Crop::Wheat, Crop::Wheat, Crop::Stubble, Crop::Ploughed, Crop::Rows, Crop::Meadow];

/** The fields, a domain bidding `bid` for each column and `bids` over rectangles (Domain::bids), kept off the forest's `path`. */
pub struct Fields {
    scope: Scope,
    path: Arc<Path>,
    bid: Bid,
    bids: Bids,
}

impl Fields {
    pub fn new(scope: Scope, path: Arc<Path>, (bid, bids): (Bid, Bids)) -> Fields {
        Fields { scope, path, bid, bids }
    }
}

impl Domain for Fields {
    fn bid(&self, x: i64, z: i64) -> Fx {
        (self.bid)(x, z)
    }

    fn bids(&self, min: [i64; 2], max: [i64; 2]) -> (Fx, Fx) {
        (self.bids)(min, max)
    }

    fn layers(self: Arc<Self>, ground: &Arc<Sloped>, hold: Holding) -> Vec<Layer> {
        layers(&self.scope, hold, ground.clone(), self.path.clone())
    }
}

/** What the fields' layers know of a column. */
struct Plots {
    seed: u64,
    hold: Holding,
    /** The forest's path. */
    path: Arc<Path>,
    warp: ValueNoise<{ m(64.0) }>,
}

impl Plots {
    /**
     * The crop of column (x, z), if the fields own it and it is off the
     * path: plots in bands across the valley and strips along it, warped,
     * a hedge on some of their edges.
     */
    fn crop(&self, x: i64, z: i64) -> Option<Crop> {
        if !self.hold.owns(x, z) || self.path.by(x, z, Units::m(1.6).to_fx()) {
            return None;
        }
        let from_path = self.path.across(x, z);
        let side = from_path.signum();
        let across = from_path.abs().floor_units() + (self.warp.at(z, 0) * Units::m(12.0)).floor_units();
        let along = Units(z) + (self.warp.at(x, 1) * Units::m(12.0)).floor_units();
        let (plot, strip) = (Units::m(40.0), Units::m(28.0));
        let edge = |v: Units, w: Units| v.rem_euclid(w) < Units::m(1.0);
        let h = hash(self.seed, side, across.div_euclid(plot), along.div_euclid(strip));
        if (edge(across, plot) && h & 1 == 0) || (edge(along, strip) && h & 2 == 0) {
            return Some(Crop::Hedge);
        }
        Some(CROPS[(h >> 8) as usize % CROPS.len()])
    }
}

/** The fields' layers on `ground`, along `path`, where they own the land. */
fn layers(scope: &Scope, hold: Holding, ground: Arc<Sloped>, path: Arc<Path>) -> Vec<Layer> {
    let region = hold.region();
    let f = Arc::new(Plots { seed: scope.seed("plots"), hold, path, warp: ValueNoise::new(scope.seed("warp")) });
    let dithers = scope.seed("dither");
    let plots = {
        let (f, ground) = (f.clone(), ground.clone());
        let on = ground.clone();
        ground::columns("fields", 0.6, Op::Paint, move |x, z| {
            let Some(crop) = f.crop(x, z) else { return Vec::new() };
            let dither = hash(dithers, x, z, 0);
            // Half a metre of soil (or stubble) under each, and what grows on it.
            let h = ground.at(x, z);
            let soil = |kind: K| Column::new(h).surface(kind, m(0.5));
            match crop {
                Crop::Wheat => soil(K::stubble).medium(K::wheat, m(0.9), 0.35),
                Crop::Stubble => soil(K::stubble),
                Crop::Ploughed => soil(if stripe(z, 8, dither) { K::furrow } else { K::tilth }),
                Crop::Rows if stripe(z, 12, dither) => soil(K::furrow).medium(K::crop, 2, 0.6),
                Crop::Rows => soil(K::furrow),
                Crop::Meadow => soil(K::meadow).medium(K::meadowGrass, 2, 0.2),
                Crop::Hedge => soil(K::meadow).medium(F::bush, m(1.4), 0.45),
            }
            .spans()
        })
        .about(on, m(0.5), m(1.4))
        .within(region.clone())
    };
    // Haystacks on the stubble.
    let haystacks = {
        let (seed, chance) = (scope.seed("haystacks"), fx(0.3));
        Population::new("haystacks", 3.0, m(6.0))
            .reach(m(2.0))
            .chance(chance)
            .size(m(4.0))
            .on(ground.clone())
            .within(region)
            .anchor_along(1, move |line| {
                let [i, k] = line.across;
                Some(line.pick(&mut Rng::new(hash64(seed, i, 0, k)), |x, z| ground.at(x, z)))
            })
            .make(move |[x, y, z], cell| {
                if !cell.chance(chance) || f.crop(x as i64, z as i64) != Some(Crop::Stubble) {
                    return None;
                }
                let mut random = cell.random("haystack");
                let r = m(1.0) as f64 + random.next() * 4.0;
                Some(Node::of(vec![Shape::ball([x, y + r * 0.5, z], r, F::hay)]))
            })
    };
    vec![Layer::Field(plots), Layer::Population(haystacks)]
}
