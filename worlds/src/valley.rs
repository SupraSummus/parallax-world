//! The valley, the flagship world: the forest, with towns along its path,
//! farmland around them, landmarks scattered through it, moorland on its
//! uplands and along stretches of it, and a railway up one side, between
//! mountain ranges; past them the sea to the west, an upland to the east.
//! Domains on one land (plan.rs) that never call each other: each reads
//! only plans coarser than its own, bids for every column, and draws what
//! it wins; the mountains raise the ground they all stand on. New
//! structures join it as landmarks (landmarks.rs), as domains, or as
//! things of one. See "Composing" in docs/generators.md.

use std::rc::Rc;

use worldgen::generator::{Generator, KindDef};
use worldgen::noise::{Fx, fx, stream};
use worldgen::plan::{Bid, Land, Plan, Sloped};
use worldgen::units::{Units, m};
use worldgen::world::{Find, World};

use crate::fields;
use crate::forest::{self, GROUND_SCALE, Ground, Relief};
use crate::landmarks::{self, Landmark, Landmarks};
use crate::moor::{self, Moorland, Uplands};
use crate::mountains::{self, Ranges};
use crate::path::Path;
use crate::railway::{self, Railway, Structure};
use crate::town::{self, Towns};

/** The domains, by their index in the land. */
const FOREST: usize = 0;
const TOWN: usize = 1;
const FIELDS: usize = 2;
const LANDMARKS: usize = 3;
const MOOR: usize = 4;
const RAILWAY: usize = 5;

/**
 * The most the valley's ground rises or falls from a column to the next,
 * in units (Sloped): the mountains' faces and the railway's cuttings are
 * its steepest, a few units.
 */
const SLOPE: i64 = 8;

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
    let railway = Rc::new(Railway::new(seed, terrain.clone()));
    let landmarks = Rc::new(Landmarks::new(seed, terrain.clone(), railway.clone()));
    // No moor from behind the spawn to past the first town: the contract's regions are there.
    let uplands = Rc::new(Uplands::new(seed, terrain.clone(), landmarks.clone(), railway.clone(), [m(-200.0), m(500.0)]));
    let moorland = Rc::new(uplands.moorland(stream(seed, 82)));
    let points = points(&terrain, &landmarks, &moorland, &railway);
    let path = {
        let terrain = terrain.clone();
        Rc::new(Path::new(move |z| terrain.path_x(z)))
    };
    // The valley's ground along the path, rising into the ranges, levelled under the towns, cut and filled by the railway,
    // shaped by landmarks and the moor.
    let ground = {
        let (path, landmarks, moorland, railway) = (path.clone(), landmarks.clone(), moorland.clone(), railway.clone());
        let plan = Plan::new("ground", GROUND_SCALE, move |x, z| {
            let ground = railway.shaped(x, z, terrain.ground_by(x, z, path.across(x, z)));
            landmarks.shaped(x, z, moorland.shaped(x, z, ground)).round_units().0
        });
        Rc::new(Sloped::new(Rc::new(plan), SLOPE))
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
    let bids: Vec<Bid> = vec![
        Box::new(|_, _| Fx::ZERO),
        {
            let towns = towns.clone();
            Box::new(move |x, z| towns.bid(x, z))
        },
        Box::new(fields::bid(seed, towns.clone(), slope)),
        {
            let landmarks = landmarks.clone();
            Box::new(move |x, z| landmarks.bid(x, z))
        },
        Box::new(move |x, z| uplands.bid(x, z)),
        {
            let railway = railway.clone();
            Box::new(move |x, z| railway.bid(x, z))
        },
    ];
    let land = Rc::new(Land::new(town::BID_SCALE, fx(0.125), bids));

    // Thinning out to the treeline, and to the sea.
    let share = {
        let (land, ranges, ground) = (land.clone(), ranges.clone(), ground.clone());
        Rc::new(move |x, z| {
            let h = ground.at(x, z);
            land.share(FOREST, x, z) * ranges.below_treeline(x, z, h) * mountains::dry(h)
        })
    };
    let forest_ground = Ground { ground: ground.clone(), path: path.clone(), share };
    let spawn = forest_ground.on_path(m(-31.0), m(3.0));
    let mut layers = forest::layers(seed, forest_ground, spawn);
    layers.extend(town::layers(towns, land.clone(), TOWN, ground.clone()));
    layers.extend(fields::layers(seed, land.clone(), FIELDS, ground.clone(), path.clone()));
    layers.extend(landmarks::layers(landmarks, land.clone(), LANDMARKS, ground.clone()));
    let moor = moor::Ground { ground: ground.clone(), land: Some((land.clone(), MOOR)), path: Some(path), under: false };
    layers.extend(moor::layers(moorland, moor));
    layers.extend(mountains::layers(ranges, ground.clone()));
    layers.extend(railway::layers(railway, land, RAILWAY, ground));
    World { generator: Generator { kinds: kinds(), seed: stream(seed, 4), layers }, spawn, heights: mountains::HEIGHTS, points }
}

/** The valley's kinds, each domain's after those of the domains it draws from; the moor alone has them too. */
pub fn kinds() -> Vec<KindDef> {
    [forest::KINDS, town::KINDS, fields::KINDS, mountains::KINDS, landmarks::KINDS, moor::KINDS, railway::KINDS].concat()
}

/**
 * Where its features are, at any seed: the first town, chapel and
 * windmill, a mountainside, the coast, a steading on the moor; the
 * railway's first station, bridge and tunnel.
 */
fn points(terrain: &Rc<Terrain>, landmarks: &Rc<Landmarks>, moorland: &Rc<Moorland>, railway: &Rc<Railway>) -> Vec<(&'static str, Find)> {
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
        ("coast", {
            let terrain = terrain.clone();
            Box::new(move || coast(&terrain))
        }),
        ("steading", Box::new(move || steading(&floor, &moorland))),
        ("station", {
            let railway = railway.clone();
            Box::new(move || railway.first_station())
        }),
        ("bridge", {
            let railway = railway.clone();
            Box::new(move || railway.first(Structure::Bridge))
        }),
        ("tunnel", {
            let railway = railway.clone();
            Box::new(move || railway.first(Structure::Tunnel))
        }),
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

/** The coast west of the start: the last dry ground going west along z = 0, every 8 m. */
fn coast(terrain: &Terrain) -> [i64; 3] {
    let ground = |x: i64| terrain.ground(x, 0).round_units().0;
    (0..m(40_000.0) / m(8.0))
        .map(|k| -k * m(8.0))
        .find(|&x| ground(x - m(8.0)) <= mountains::SEA.0)
        .map(|x| [x, ground(x), 0])
        .expect("a coast within 40 km west of the start")
}

/**
 * A steading of the moor on the valley's floor, within 400 m of the
 * path: of those reaching the first row of points that any reaches, the
 * nearest the path. Rows and points are as far apart as a steading
 * reaches (moor::NEAR), so none falls between. From 4.5 km on, where
 * REGIONS_SEED's is, as for the mountainside.
 */
fn steading(terrain: &Terrain, moorland: &Moorland) -> [i64; 3] {
    let path = |z: i64| terrain.path_x(z).round_units().0;
    (m(4500.0)..m(164_500.0))
        .step_by(moor::NEAR as usize)
        .find_map(|z| {
            let near = (-8..=8).filter_map(|i| moorland.steading_near(path(z) + i * moor::NEAR, z)).map(|p| ((p[0] - path(p[2])).abs(), p));
            near.filter(|(d, _)| *d < m(400.0)).min().map(|(_, p)| p)
        })
        .expect("a steading by the path within 160 km along the valley")
}
