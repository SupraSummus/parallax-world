//! Farmland around the valley's towns: plots of wheat, stubble with
//! haystacks, ploughed and planted rows, meadows, hedges between them.
//! A domain of the valley (valley.rs) that reads only the towns' places
//! and the ground's slope, both coarser than it: it bids for gentle land
//! near towns, and draws only the columns it wins.

use std::rc::Rc;

use worldgen::column::Column;
use worldgen::domain::{Domain, Scope};
use worldgen::generator::{Field, Layer, Node, Op, Population, Shape, stripe};
use worldgen::kinds;
use worldgen::noise::{Fbm, Fx, Rng, ValueNoise, fx, hash, hash64, smoothstep};
use worldgen::plan::{Holding, Plan, Sloped};
use worldgen::units::{Units, m};

use crate::forest::K as F;
use crate::path::Path;
use crate::town::{self, Towns};

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
 * ground is steep: `slope`, rise over run.
 */
pub fn round_towns(scope: &Scope, towns: Rc<Towns>, slope: Rc<Plan<Fx>>) -> impl Fn(i64, i64) -> Fx + 'static {
    let reach = Fbm::<{ m(160.0) }, 3>::new(scope.seed("reach"));
    move |x, z| {
        let d = towns.distance(x, z).max(Units(0));
        let r = Units::m(90.0) + (reach.at(x, z) * Units::m(60.0)).floor_units();
        if d >= r {
            return -Fx::ONE;
        }
        let near = fx(0.6) * (r - d) / r;
        near - smoothstep(fx(0.12), fx(0.25), slope.at(x, z))
    }
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

/** The fields, a domain bidding `bid` for each column, kept off the forest's `path`. */
pub struct Fields {
    scope: Scope,
    path: Rc<Path>,
    bid: Box<dyn Fn(i64, i64) -> Fx>,
}

impl Fields {
    pub fn new(scope: Scope, path: Rc<Path>, bid: impl Fn(i64, i64) -> Fx + 'static) -> Fields {
        Fields { scope, path, bid: Box::new(bid) }
    }
}

impl Domain for Fields {
    fn bid(&self, x: i64, z: i64) -> Fx {
        (self.bid)(x, z)
    }

    fn layers(self: Rc<Self>, ground: &Rc<Sloped>, hold: Holding) -> Vec<Layer> {
        layers(&self.scope, hold, ground.clone(), self.path.clone())
    }
}

/** What the fields' layers know of a column. */
struct Plots {
    seed: u64,
    hold: Holding,
    /** The forest's path. */
    path: Rc<Path>,
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
        let from_path = Units(x).to_fx() - self.path.x(z);
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
fn layers(scope: &Scope, hold: Holding, ground: Rc<Sloped>, path: Rc<Path>) -> Vec<Layer> {
    let f = Rc::new(Plots { seed: scope.seed("plots"), hold, path, warp: ValueNoise::new(scope.seed("warp")) });
    let dithers = scope.seed("dither");
    let plots = {
        let (f, ground) = (f.clone(), ground.clone());
        let on = ground.clone();
        Field::columns("fields", 0.6, Op::Paint, move |x, z, spacing| {
            let Some(crop) = f.crop(x, z) else { return Vec::new() };
            let dither = hash(dithers, x, z, 0);
            // Half a metre of soil (or stubble) under each, and what grows on it.
            let h = ground.at(x, z);
            let soil = |kind: K| Column::new(h, spacing).surface(kind, m(0.5));
            match crop {
                Crop::Wheat => soil(K::stubble).medium(K::wheat, m(0.9), 0.35),
                Crop::Stubble => soil(K::stubble),
                Crop::Ploughed => soil(if stripe(z, 8, spacing, dither) { K::furrow } else { K::tilth }),
                Crop::Rows if stripe(z, 12, spacing, dither) => soil(K::furrow).medium(K::crop, 2, 0.6),
                Crop::Rows => soil(K::furrow),
                Crop::Meadow => soil(K::meadow).medium(K::meadowGrass, 2, 0.2),
                Crop::Hedge => soil(K::meadow).medium(F::bush, m(1.4), 0.45),
            }
            .spans()
        })
        .about(on, m(0.5), m(1.4))
    };
    // Haystacks on the stubble.
    let haystacks = {
        let seed = scope.seed("haystacks");
        Population::new("haystacks", 3.0, m(6.0))
            .reach(m(2.0))
            .size(m(4.0))
            .on(ground.clone())
            .anchor_along(1, move |line| {
                let [i, k] = line.across;
                Some(line.pick(&mut Rng::new(hash64(seed, i, 0, k)), |x, z| ground.at(x, z)))
            })
            .make(move |[x, y, z], cell| {
                let mut random = cell.random("haystack");
                if f.crop(x as i64, z as i64) != Some(Crop::Stubble) || !random.chance(fx(0.3)) {
                    return None;
                }
                let r = m(1.0) as f64 + random.next() * 4.0;
                Some(Node::of(vec![Shape::ball([x, y + r * 0.5, z], r, F::hay)]))
            })
    };
    vec![Layer::Field(plots), Layer::Population(haystacks)]
}
