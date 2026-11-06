//! The valley, the flagship world: the forest, with towns along its path
//! and farmland around them, between mountain ranges. Domains on one land
//! (plan.rs) that never call each other: each reads only plans coarser
//! than its own, bids for every column, and draws what it wins; the
//! mountains raise the ground they all stand on. New structures join it
//! as domains or as things of one. See "Composing" in docs/generators.md.

use std::rc::Rc;

use worldgen::World;
use worldgen::generator::Generator;
use worldgen::noise::{Fx, fx, stream};
use worldgen::plan::{Land, Plan};
use worldgen::units::{Units, m};

use crate::fields;
use crate::forest::{self, GROUND_SCALE, Ground, PATH_SCALE, Relief};
use crate::mountains::{self, Ranges};
use crate::town::{self, TOWNS_SCALE, Towns};

/** The domains, by their index in the land. */
const FOREST: usize = 0;
const TOWN: usize = 1;
const FIELDS: usize = 2;

/** The valley of `seed`; the camera starts in the forest, on the path to the first town. */
pub fn valley(seed: u64) -> World {
    let relief = Rc::new(Relief::new(seed));
    let ranges = Rc::new(Ranges::new(seed));
    let towns = Rc::new(Towns::new(seed, relief.clone()));
    // The path, bent onto each town's main street.
    let path = {
        let (relief, towns) = (relief.clone(), towns.clone());
        Rc::new(Plan::new("path", PATH_SCALE, move |_, z| towns.street_x(z, relief.path_x(z))))
    };
    // The valley's ground along the path, rising into the ranges, levelled under the towns.
    let ground = {
        let (towns, path, ranges) = (towns.clone(), path.clone(), ranges.clone());
        Rc::new(Plan::new("ground", GROUND_SCALE, move |x, z| {
            let from_path = forest::path_distance(&path, x, z);
            towns.terraced(x, z, relief.ground(x, z, from_path) + ranges.height(x, z, from_path)).round_units().0
        }))
    };
    let slope = {
        let ground = ground.clone();
        let step = 16;
        Rc::new(Plan::sampled("slope", fields::SLOPE_SCALE, step, move |x, z| {
            let rise = (ground.at(x + step, z) - ground.at(x - step, z)).abs() + (ground.at(x, z + step) - ground.at(x, z - step)).abs();
            Units(rise).to_fx() / Units(2 * step)
        }))
    };
    // The forest bids nothing: it has what the others don't outbid it for.
    let bids = vec![
        Rc::new(Plan::new("forest's bid", TOWNS_SCALE, |_, _| Fx::ZERO)),
        {
            let towns = towns.clone();
            Rc::new(Plan::new("town's bid", town::BID_SCALE, move |x, z| towns.bid(x, z)))
        },
        Rc::new(fields::bid(seed, towns.clone(), slope)),
    ];
    let land = Rc::new(Land::new(town::BID_SCALE, fx(0.125), bids));

    // Thinning out to the treeline.
    let share = {
        let (land, ranges, ground) = (land.clone(), ranges.clone(), ground.clone());
        Rc::new(move |x, z| land.share(FOREST, x, z) * ranges.below_treeline(x, z, ground.at(x, z)))
    };
    let forest_ground = Ground { ground: ground.clone(), path: path.clone(), share };
    let spawn = forest_ground.on_path(m(-31.0), m(3.0));
    let mut layers = forest::layers(seed, forest_ground, spawn);
    layers.extend(town::layers(towns, land.clone(), TOWN, ground.clone()));
    layers.extend(fields::layers(seed, land, FIELDS, ground.clone(), path));
    layers.extend(mountains::layers(ranges, ground));
    let kinds = [forest::KINDS, town::KINDS, fields::KINDS, mountains::KINDS].concat();
    World { generator: Generator { kinds, seed: stream(seed, 4), layers }, spawn, heights: mountains::HEIGHTS }
}
