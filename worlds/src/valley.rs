//! The valley: the forest, with towns along its path and farmland around
//! them. Three domains on one land (plan.rs) that never call each other:
//! each reads only plans coarser than its own, bids for every column,
//! and draws what it wins. See "Composing" in docs/generators.md.

use std::rc::Rc;

use worldgen::World;
use worldgen::generator::Generator;
use worldgen::noise::{Fx, fx, stream};
use worldgen::plan::{Land, Plan};
use worldgen::units::{Units, m};

use crate::fields;
use crate::forest::{self, GROUND_SCALE, Ground, PATH_SCALE, Relief};
use crate::town::{self, TOWNS_SCALE, Towns};

/** The domains, by their index in the land. */
const FOREST: usize = 0;
const TOWN: usize = 1;
const FIELDS: usize = 2;

/** The valley of `seed`; the camera starts in the forest, on the path to the first town. */
pub fn valley(seed: u64) -> World {
    let relief = Rc::new(Relief::new(seed));
    let towns = Rc::new(Towns::new(seed, relief.clone()));
    // The path, bent onto each town's main street.
    let path = {
        let (relief, towns) = (relief.clone(), towns.clone());
        Rc::new(Plan::new("path", PATH_SCALE, move |_, z| towns.street_x(z, relief.path_x(z))))
    };
    // The valley's ground along the path, levelled under the towns.
    let ground = {
        let (towns, path) = (towns.clone(), path.clone());
        Rc::new(Plan::new("ground", GROUND_SCALE, move |x, z| {
            towns.terraced(x, z, relief.ground(x, z, forest::path_distance(&path, x, z))).round_units().0
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

    let share = {
        let land = land.clone();
        Rc::new(move |x, z| land.share(FOREST, x, z))
    };
    let forest_ground = Ground { ground: ground.clone(), path: path.clone(), share };
    let spawn = forest_ground.on_path(m(-31.0), m(3.0));
    let mut layers = forest::layers(seed, forest_ground, spawn);
    layers.extend(town::layers(towns, land.clone(), TOWN, ground.clone()));
    layers.extend(fields::layers(seed, land, FIELDS, ground, path));
    let kinds = [forest::KINDS, town::KINDS, fields::KINDS].concat();
    // The forest's heights and reach, which hold the towns and fields too.
    World { generator: Generator { kinds, seed: stream(seed, 4), layers }, spawn, heights: forest::HEIGHTS }
}
