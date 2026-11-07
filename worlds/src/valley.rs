//! The valley, the flagship world: the forest, with towns along its path,
//! farmland around them, landmarks scattered through it and moorland on
//! its uplands and along stretches of it, between mountain ranges.
//! Domains on one land (plan.rs) that never call each other: each reads
//! only plans coarser than its own, bids for every column, and draws what
//! it wins; the mountains raise the ground they all stand on. New
//! structures join it as landmarks (landmarks.rs), as domains, or as
//! things of one. See "Composing" in docs/generators.md.

use std::rc::Rc;

use worldgen::generator::{Generator, KindDef};
use worldgen::noise::{Fx, fx, stream};
use worldgen::plan::{Land, Plan};
use worldgen::units::{Units, m};
use worldgen::world::{Find, World};

use crate::fields;
use crate::forest::{self, GROUND_SCALE, Ground, PATH_SCALE, Relief};
use crate::landmarks::{self, Landmark, Landmarks};
use crate::moor::{self, Moorland, Uplands};
use crate::mountains::{self, Ranges};
use crate::town::{self, TOWNS_SCALE, Towns};

/** The domains, by their index in the land. */
const FOREST: usize = 0;
const TOWN: usize = 1;
const FIELDS: usize = 2;
const LANDMARKS: usize = 3;
const MOOR: usize = 4;

/**
 * The valley's lie of the land before landmarks shape it: the relief
 * and the ranges along the path, levelled under the towns. Functions of
 * position, read by its plans and by coarse plans that can't read the
 * fine ground plan (where landmarks stand).
 */
pub struct Terrain {
    pub relief: Rc<Relief>,
    pub ranges: Rc<Ranges>,
    pub towns: Rc<Towns>,
}

impl Terrain {
    pub fn new(seed: u64) -> Terrain {
        let relief = Rc::new(Relief::new(seed));
        let towns = Rc::new(Towns::new(seed, relief.clone()));
        Terrain { relief, ranges: Rc::new(Ranges::new(seed)), towns }
    }

    /** The path's centre line at z, with its fraction, bent onto each town's main street. */
    pub fn path_x(&self, z: i64) -> Fx {
        self.towns.street_x(z, self.relief.path_x(z))
    }

    /** The ground at (x, z), with its fraction, `from_path` off the path's centre line. */
    pub fn ground_by(&self, x: i64, z: i64, from_path: Fx) -> Fx {
        self.towns.terraced(x, z, self.relief.ground(x, z, from_path) + self.ranges.height(x, z, from_path))
    }

    /** The ground at (x, z), with its fraction. */
    pub fn ground(&self, x: i64, z: i64) -> Fx {
        self.ground_by(x, z, (Units(x).to_fx() - self.path_x(z)).abs())
    }
}

/** The valley of `seed`; the camera starts in the forest, on the path to the first town. */
pub fn valley(seed: u64) -> World {
    let terrain = Rc::new(Terrain::new(seed));
    let (ranges, towns) = (terrain.ranges.clone(), terrain.towns.clone());
    let landmarks = Rc::new(Landmarks::new(seed, terrain.clone()));
    // No moor from behind the spawn to past the first town: the contract's regions are there.
    let uplands = Rc::new(Uplands::new(seed, terrain.clone(), landmarks.clone(), [m(-200.0), m(500.0)]));
    let moorland = Rc::new(uplands.moorland(stream(seed, 82)));
    let points = points(&terrain, &landmarks, &moorland);
    let path = {
        let terrain = terrain.clone();
        Rc::new(Plan::new("path", PATH_SCALE, move |_, z| terrain.path_x(z)))
    };
    // The valley's ground along the path, rising into the ranges, levelled under the towns, shaped by landmarks and the moor.
    let ground = {
        let (path, landmarks, moorland) = (path.clone(), landmarks.clone(), moorland.clone());
        Rc::new(Plan::new("ground", GROUND_SCALE, move |x, z| {
            let from_path = forest::path_distance(&path, x, z);
            landmarks.shaped(x, z, moorland.shaped(x, z, terrain.ground_by(x, z, from_path))).round_units().0
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
        {
            let landmarks = landmarks.clone();
            Rc::new(Plan::new("landmarks' bid", landmarks::BID_SCALE, move |x, z| landmarks.bid(x, z)))
        },
        Rc::new(Plan::new("moor's bid", town::BID_SCALE, move |x, z| uplands.bid(x, z))),
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
    layers.extend(fields::layers(seed, land.clone(), FIELDS, ground.clone(), path.clone()));
    layers.extend(landmarks::layers(landmarks, land.clone(), LANDMARKS, ground.clone()));
    layers
        .extend(moor::layers(moorland, moor::Ground { ground: ground.clone(), land: Some((land, MOOR)), path: Some(path), bottom: None }));
    layers.extend(mountains::layers(ranges, ground));
    World { generator: Generator { kinds: kinds(), seed: stream(seed, 4), layers }, spawn, heights: mountains::HEIGHTS, points }
}

/** The valley's kinds, each domain's after those of the domains it draws from; the moor alone has them too. */
pub fn kinds() -> Vec<KindDef> {
    [forest::KINDS, town::KINDS, fields::KINDS, mountains::KINDS, landmarks::KINDS, moor::KINDS].concat()
}

/** Where its features are, at any seed: the first town, chapel and windmill, a mountainside, a steading on the moor. */
fn points(terrain: &Rc<Terrain>, landmarks: &Rc<Landmarks>, moorland: &Rc<Moorland>) -> Vec<(&'static str, Find)> {
    let site = |landmark| -> Find {
        let landmarks = landmarks.clone();
        Box::new(move || {
            let s = landmarks.first(landmark);
            [s.centre[0], s.level, s.centre[1]]
        })
    };
    let (towns, moorland) = (terrain.towns.clone(), moorland.clone());
    let (range, floor) = (terrain.clone(), terrain.clone());
    vec![
        (
            "town",
            Box::new(move || {
                let t = towns.first();
                [t.x, t.level, t.z]
            }),
        ),
        ("chapel", site(Landmark::Chapel)),
        ("windmill", site(Landmark::Windmill)),
        ("mountainside", Box::new(move || mountainside(&range))),
        ("steading", Box::new(move || steading(&floor, &moorland))),
    ]
}

/**
 * A mountainside above the treeline, where meadows give way to scree
 * and rock: just behind the crest of the range east of the path, 20 m
 * down from its top, where the crest stands 250 m over the treeline.
 * Looked for every 64 m along the valley, ahead and behind in turn, from
 * 5820 m on, where REGIONS_SEED's is, so its region stays where it was
 * laid out; the first from the start is elsewhere at that seed.
 */
fn mountainside(terrain: &Terrain) -> [i64; 3] {
    (0..400)
        .flat_map(|k| [k, -k - 1])
        .map(|k| m(5820.0) + k * m(64.0))
        .find_map(|z| {
            let path = terrain.path_x(z).round_units().0;
            let out: Vec<(i64, Fx)> = (path..path + m(3000.0)).step_by(m(8.0) as usize).map(|x| (x, terrain.ground(x, z))).collect();
            let (x, top) = out.iter().fold(out[0], |a, b| if b.1 > a.1 { *b } else { a });
            if top < terrain.ranges.treeline(x, z) + Fx::m(250.0) {
                return None;
            }
            let behind = out.iter().skip_while(|(at, _)| *at < x);
            behind.map(|&(x, h)| [x, h.round_units().0, z]).find(|p| Units(p[1]).to_fx() <= top - Fx::m(20.0))
        })
        .expect("a mountainside within 25 km along the valley")
}

/**
 * A steading of the moor on the valley's floor: in the first row of the
 * moor's regions with one within 400 m of the path, the nearest it.
 * From 4.5 km on, where REGIONS_SEED's is, as for the mountainside.
 */
fn steading(terrain: &Terrain, moorland: &Moorland) -> [i64; 3] {
    let path = |z: i64| terrain.path_x(z).round_units().0;
    (0..1000)
        .map(|row| m(4500.0) + row * moor::REGION)
        .find_map(|z| {
            let (i, k) = moor::steading_region(path(z), z);
            let near = (i - 3..=i + 3).filter_map(|i| moorland.steading_in(i, k)).map(|p| ((p[0] - path(p[2])).abs(), p));
            near.filter(|(d, _)| *d < m(400.0)).min().map(|(_, p)| p)
        })
        .expect("a steading by the path within 160 km along the valley")
}
