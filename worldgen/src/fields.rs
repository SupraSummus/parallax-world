//! Farmland around the valley's towns: plots of wheat, stubble with
//! haystacks, ploughed and planted rows, meadows, hedges between them.
//! A domain of the valley (valley.rs) that reads only the towns' places
//! and the ground's slope, both coarser than it: it bids for gentle land
//! near towns, and draws only the columns it wins.

use std::rc::Rc;

use crate::forest::{self, K as F, m};
use crate::generator::{Field, FieldValues, Kind, Layer, Node, Op, Population, SOLID, Span, ball, stripe};
use crate::kinds;
use crate::noise::{Fbm, Fixed, ONE, Rng, ValueNoise, fixed, hash, hash64, smoothstep, stream};
use crate::plan::{Land, Plan};
use crate::town::{self, Towns};

kinds! {
    base: town::BASE + town::KINDS.len() as u8;
    wheat: 1, ["#d9b44a", "#e3c25a", "#cfa83e"];
    stubble: 1, ["#c8b070", "#bba363", "#d2bb7c"];
    furrow: 2, ["#5e4430", "#6a4d36"];
    tilth: 2, ["#7a5a3e", "#86644a"];
    crop: 5, ["#4f8a34", "#5a9a3c", "#467c2e"];
    meadow: 1, ["#7aae48", "#86b850", "#6fa040"];
}

/** Plan scale of the fields' bid, and of the slope they bid by. */
pub const BID_SCALE: i64 = m(4.0);
pub const SLOPE_SCALE: i64 = m(16.0);

/**
 * The fields' bid for column (x, z): up to 0.6 at a town's edge, none
 * 90 to 150 m out (by noise), and none where the ground is steep:
 * `slope`, rise over run as a fraction of ONE.
 */
pub fn bid(seed: u64, towns: Rc<Towns>, slope: Rc<Plan<Fixed>>) -> Plan<Fixed> {
    let reach = Fbm::<{ m(160.0) }, 3>::new(stream(seed, 40));
    Plan::new("fields' bid", BID_SCALE, move |x, z| {
        let d = towns.distance(x, z).max(0);
        let r = m(90.0) + reach.at(x, z) * m(60.0) / ONE;
        if d >= r {
            return -ONE;
        }
        let near = fixed(0.6) * (r - d) / r;
        near - smoothstep(fixed(0.12), fixed(0.25), slope.at(x, z))
    })
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

struct Fields {
    seed: u64,
    land: Rc<Land>,
    me: usize,
    path: Rc<Plan<Fixed>>,
    warp: ValueNoise<{ m(64.0) }>,
}

impl Fields {
    /**
     * The crop of column (x, z), if the fields own it and it is off the
     * path: plots in bands across the valley and strips along it, warped,
     * a hedge on some of their edges.
     */
    fn crop(&self, x: i64, z: i64) -> Option<Crop> {
        let from_path = x * ONE - self.path.at(0, z);
        if from_path.abs() < m(1.6) * ONE || !self.land.owns(self.me, x, z) {
            return None;
        }
        let side = from_path.signum();
        let across = (from_path.abs() >> 16) + self.warp.at(z, 0) * m(12.0) / ONE;
        let along = z + self.warp.at(x, 1) * m(12.0) / ONE;
        let (band, strip) = (across.div_euclid(m(40.0)), along.div_euclid(m(28.0)));
        let edge = |v: i64, w: i64| v.rem_euclid(w) < m(1.0);
        let h = hash(stream(self.seed, 41), side, band, strip);
        if (edge(across, m(40.0)) && h & 1 == 0) || (edge(along, m(28.0)) && h & 2 == 0) {
            return Some(Crop::Hedge);
        }
        Some(CROPS[(h >> 8) as usize % CROPS.len()])
    }
}

/** A medium from `from`, `height` tall: at least two sites tall, as dense along a ray through it as it is (its extinction times height kept). */
fn medium(from: i64, height: i64, kind: Kind, extinction: f64, spacing: i64) -> Span {
    let tall = height.max(2 * spacing);
    Span { from, to: from + tall, kind, extinction: Some(extinction * height as f64 / tall as f64) }
}

/** The fields' layers on `ground`, along `path`, where domain `me` of `land` owns the land. */
pub fn layers(seed: u64, land: Rc<Land>, me: usize, ground: Rc<Plan<i64>>, path: Rc<Plan<Fixed>>) -> Vec<Layer> {
    let f = Rc::new(Fields { seed, land, me, path, warp: ValueNoise::new(stream(seed, 42)) });
    let plots = {
        let (f, ground) = (f.clone(), ground.clone());
        Field {
            name: "fields",
            order: 0.6,
            op: Op::Paint,
            values: FieldValues::Spans {
                axis: 1,
                spans: Box::new(move |x, z, spacing| {
                    let Some(crop) = f.crop(x, z) else { return Vec::new() };
                    let h = ground.at(x, z);
                    let dither = hash(stream(f.seed, 43), x, z, 0);
                    let surface = |kind: Kind| Span { from: h + 1 - m(0.5).max(2 * spacing), to: h + 1, kind, extinction: None };
                    let above = h + 1;
                    match crop {
                        Crop::Wheat => vec![surface(k(K::stubble)), medium(above, m(0.9), k(K::wheat), 0.35, spacing)],
                        Crop::Stubble => vec![surface(k(K::stubble))],
                        Crop::Ploughed => vec![surface(k(if stripe(z, 8, spacing, dither) { K::furrow } else { K::tilth }))],
                        Crop::Rows => {
                            let mut spans = vec![surface(k(K::furrow))];
                            if stripe(z, 12, spacing, dither) {
                                spans.push(medium(above, 2, k(K::crop), 0.6, spacing));
                            }
                            spans
                        }
                        Crop::Meadow => vec![surface(k(K::meadow)), medium(above, 2, k(K::meadow), 0.2, spacing)],
                        Crop::Hedge => vec![surface(k(K::meadow)), medium(above, m(1.4), forest::k(F::bush), 0.45, spacing)],
                    }
                }),
            },
        }
    };
    // Haystacks on the stubble.
    let cell = m(6.0);
    let haystacks = Population {
        name: "haystacks",
        order: 3.0,
        cell,
        reach: [m(2.0); 3],
        size: m(4.0),
        anchor: {
            let (ground, seed) = (ground.clone(), stream(seed, 44));
            Box::new(move |cell_at| {
                let [cx, _, cz] = cell_at.centre;
                let mut pick = Rng::new(hash64(seed, cell_at.i, cell_at.j, cell_at.k));
                let (x, z) = (cx - cell + pick.below(2 * cell as u32) as i64, cz - cell + pick.below(2 * cell as u32) as i64);
                Some([x as f64, ground.at(x, z) as f64, z as f64])
            })
        },
        make: Box::new(move |[x, y, z], cell| {
            let mut random = cell.random("haystack");
            if f.crop(x as i64, z as i64) != Some(Crop::Stubble) || !random.chance(fixed(0.3)) {
                return None;
            }
            let r = m(1.0) as f64 + random.next() * 4.0;
            Some(Node::of(vec![ball([x, y + r * 0.5, z], r, forest::k(F::hay), SOLID, Op::Fill)]))
        }),
        far: None,
    };
    vec![Layer::Field(plots), Layer::Population(haystacks)]
}
