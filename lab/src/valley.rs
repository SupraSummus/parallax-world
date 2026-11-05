//! The data: the valley seen from above, each column's topmost voxel as a
//! class of kinds, and the truth to score against: which domain owns each
//! column (its land, rebuilt from the public API as valley.rs builds it)
//! and how far it is from the nearest town.

use std::rc::Rc;

use worldgen::generator::{Kind, generate};
use worldgen::grid::Box3;
use worldgen::noise::{Fx, fx};
use worldgen::plan::{Land, Plan};
use worldgen::units::Units;
use worlds::fields;
use worlds::forest::{self, GROUND_SCALE, K as F, PATH_SCALE, Relief};
use worlds::town::{self, K as T, TOWNS_SCALE, Towns};

use crate::image::Image;

/** A class of kinds: what a column shows from above, coarser than the 71 kinds. */
pub struct Class {
    pub name: &'static str,
    pub rgb: u32,
    /** The domain whose kinds these are (by index in DOMAINS): an oracle that reads kinds, not the land. */
    pub domain: u8,
}

pub const DOMAINS: [&str; 3] = ["forest", "town", "fields"];
pub const DOMAIN_RGB: [u32; 3] = [0x2e6b30, 0xc0503a, 0xe0c050];

pub const CLASSES: [Class; 21] = [
    Class { name: "ground", rgb: 0x74a83c, domain: 0 },
    Class { name: "litter", rgb: 0x8a5a2b, domain: 0 },
    Class { name: "path", rgb: 0xb09272, domain: 0 },
    Class { name: "rock", rgb: 0x7d7d78, domain: 0 },
    Class { name: "wood", rgb: 0x4b3621, domain: 0 },
    Class { name: "conifer", rgb: 0x1f4a2c, domain: 0 },
    Class { name: "green leaves", rgb: 0x5a8f30, domain: 0 },
    Class { name: "autumn leaves", rgb: 0xc46a1c, domain: 0 },
    Class { name: "undergrowth", rgb: 0x2d5b20, domain: 0 },
    Class { name: "flowers, fungi", rgb: 0xe86a6a, domain: 0 },
    Class { name: "roof", rgb: 0xa8432c, domain: 1 },
    Class { name: "wall", rgb: 0xece6d6, domain: 1 },
    Class { name: "street", rgb: 0x5a5650, domain: 1 },
    Class { name: "paving", rgb: 0xb4ab98, domain: 1 },
    Class { name: "fittings", rgb: 0x2b2d30, domain: 1 },
    Class { name: "water", rgb: 0x3f7fa8, domain: 1 },
    Class { name: "wheat", rgb: 0xe3c25a, domain: 2 },
    Class { name: "stubble", rgb: 0xc8b070, domain: 2 },
    Class { name: "ploughed", rgb: 0x6a4d36, domain: 2 },
    Class { name: "crop rows", rgb: 0x4f8a34, domain: 2 },
    Class { name: "meadow", rgb: 0x9cc858, domain: 2 },
];

/** Each of the valley's kinds' class. */
pub fn class_of_kind(kinds: usize) -> Vec<u8> {
    use worlds::fields::K as P;
    let forest: [(&[F], u8); 10] = [
        (&[F::moss, F::grass, F::tallGrass], 0),
        (&[F::litter, F::needles, F::soil, F::fallenLeaf], 1),
        (&[F::path], 2),
        (&[F::stone, F::mossyStone], 3),
        (&[F::bark, F::spruceBark, F::birchBark, F::birchMark, F::deadwood, F::barkFurrow, F::spruceScale, F::twig, F::cone], 4),
        (&[F::spruce], 5),
        (&[F::oakGreen, F::birchGreen], 6),
        (&[F::oakOrange, F::oakGold, F::birchGold], 7),
        (&[F::fern, F::bracken, F::bush], 8),
        (
            &[
                F::flowerYellow,
                F::flowerViolet,
                F::flowerWhite,
                F::flowerRed,
                F::amanita,
                F::amanitaSpot,
                F::mushroomStem,
                F::bolete,
                F::berry,
            ],
            9,
        ),
    ];
    let town: [(&[T], u8); 6] = [
        (&[T::roofRed, T::roofBrown, T::slate, T::copper], 10),
        (
            &[
                T::plasterOchre,
                T::plasterWhite,
                T::plasterPink,
                T::plasterBlue,
                T::plasterSage,
                T::brick,
                T::trim,
                T::shutterGreen,
                T::shutterBlue,
                T::glass,
                T::door,
                T::churchStone,
            ],
            11,
        ),
        (&[T::cobble], 12),
        (&[T::paving, T::pavingDark, T::slab], 13),
        (&[T::planks, T::counter, T::goods, T::iron, T::lamp], 14),
        (&[T::water], 15),
    ];
    let farm: [(&[P], u8); 5] =
        [(&[P::wheat], 16), (&[P::stubble], 17), (&[P::furrow, P::tilth], 18), (&[P::crop], 19), (&[P::meadow], 20)];
    let mut class = vec![u8::MAX; kinds];
    let mut set = |kind: Kind, c: u8| class[kind as usize] = c;
    forest.iter().for_each(|(ks, c)| ks.iter().for_each(|k| set(forest::k(*k), *c)));
    town.iter().for_each(|(ks, c)| ks.iter().for_each(|k| set(town::k(*k), *c)));
    farm.iter().for_each(|(ks, c)| ks.iter().for_each(|k| set(fields::k(*k), *c)));
    set(forest::k(F::hay), 17);
    assert!(class.iter().all(|c| *c != u8::MAX), "a kind without a class");
    class
}

/**
 * The valley's towns and land, built as valley.rs builds them (it keeps
 * them private): the domains' bids, in DOMAINS' order.
 */
pub fn land(seed: u64) -> (Rc<Towns>, Land) {
    let relief = Rc::new(Relief::new(seed));
    let towns = Rc::new(Towns::new(seed, relief.clone()));
    let path = {
        let (relief, towns) = (relief.clone(), towns.clone());
        Rc::new(Plan::new("path", PATH_SCALE, move |_, z| towns.street_x(z, relief.path_x(z))))
    };
    let ground = {
        let (towns, path) = (towns.clone(), path.clone());
        Rc::new(Plan::new("ground", GROUND_SCALE, move |x, z| {
            towns.terraced(x, z, relief.ground(x, z, forest::path_distance(&path, x, z))).round_units().0
        }))
    };
    let slope = {
        let step = 16;
        Rc::new(Plan::sampled("slope", fields::SLOPE_SCALE, step, move |x, z| {
            let rise = (ground.at(x + step, z) - ground.at(x - step, z)).abs() + (ground.at(x, z + step) - ground.at(x, z - step)).abs();
            Units(rise).to_fx() / Units(2 * step)
        }))
    };
    let bids = vec![
        Rc::new(Plan::new("forest's bid", TOWNS_SCALE, |_, _| Fx::ZERO)),
        {
            let towns = towns.clone();
            Rc::new(Plan::new("town's bid", town::BID_SCALE, move |x, z| towns.bid(x, z)))
        },
        Rc::new(fields::bid(seed, towns.clone(), slope)),
    ];
    (towns, Land::new(town::BID_SCALE, fx(0.125), bids))
}

/** An area of columns at a level: its first column (level units) and size. */
#[derive(Clone, Copy, Debug)]
pub struct Area {
    pub level: u32,
    pub x0: i64,
    pub z0: i64,
    pub nx: usize,
    pub nz: usize,
}

/** The valley over an area: per column (x fastest, z rows), its class, owner domain and metres from the nearest town. */
pub struct Valley {
    pub area: Area,
    pub class: Image,
    pub owner: Image,
    pub town_distance: Vec<i32>,
}

/** The valley of `seed` over `area`, generated on `threads` threads (each its own world), or read from `cache` if there. */
pub fn valley(seed: u64, area: Area, threads: usize, cache: &str) -> Valley {
    let n = area.nx * area.nz;
    if let Ok(bytes) = std::fs::read(cache)
        && bytes.len() == n * 6
    {
        let image = |at: usize| Image::new(area.nx, area.nz, bytes[at * n..(at + 1) * n].iter().map(|b| *b as u16).collect());
        let town_distance = bytes[2 * n..].chunks(4).map(|c| i32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect();
        return Valley { area, class: image(0), owner: image(1), town_distance };
    }
    const TILE: usize = 64;
    let rows: Vec<usize> = (0..area.nz).step_by(TILE).collect();
    let parts: Vec<Vec<(usize, u8, u8, i32)>> = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..threads)
            .map(|t| {
                let rows = &rows;
                scope.spawn(move || {
                    let world = worlds::valley::valley(seed);
                    let generator = &world.generator;
                    let class = class_of_kind(generator.kinds.len());
                    let mut material_class = vec![0u8];
                    for (kind, def) in generator.kinds.iter().enumerate() {
                        material_class.extend(def.colors.iter().map(|_| class[kind]));
                    }
                    let (towns, land) = land(seed);
                    let (min_y, max_y) = world.heights_at(area.level);
                    let s = 1i64 << area.level;
                    let mut out = Vec::new();
                    for &tz in rows.iter().skip(t).step_by(threads) {
                        for tx in (0..area.nx).step_by(TILE) {
                            let (x0, z0) = (area.x0 + tx as i64, area.z0 + tz as i64);
                            let b = Box3 { min: [x0, min_y, z0], max: [x0 + TILE as i64, max_y, z0 + TILE as i64] };
                            let grid = generate(generator, &b, area.level);
                            for j in tz..(tz + TILE).min(area.nz) {
                                for i in tx..(tx + TILE).min(area.nx) {
                                    let (x, z) = (area.x0 + i as i64, area.z0 + j as i64);
                                    let id = (min_y..max_y)
                                        .rev()
                                        .filter(|y| (x + y + z) & 1 == 0)
                                        .map(|y| grid.get(x, y, z))
                                        .find(|id| *id != 0);
                                    let owner = (0..DOMAINS.len()).find(|d| land.owns(*d, x * s, z * s)).unwrap_or(0) as u8;
                                    let metres = (towns.distance(x * s, z * s).0 / 8).clamp(-10_000, 10_000) as i32;
                                    out.push((j * area.nx + i, material_class[id.unwrap_or(0) as usize], owner, metres));
                                }
                            }
                        }
                    }
                    out
                })
            })
            .collect();
        handles.into_iter().map(|h| h.join().expect("a generating thread")).collect()
    });
    let (mut class, mut owner, mut town_distance) = (vec![0u8; n], vec![0u8; n], vec![0i32; n]);
    for (at, c, o, d) in parts.into_iter().flatten() {
        (class[at], owner[at], town_distance[at]) = (c, o, d);
    }
    let mut bytes = [class.as_slice(), owner.as_slice()].concat();
    bytes.extend(town_distance.iter().flat_map(|d| d.to_le_bytes()));
    if let Some(dir) = std::path::Path::new(cache).parent() {
        std::fs::create_dir_all(dir).expect("cache directory");
    }
    std::fs::write(cache, bytes).expect("cache written");
    let image = |v: Vec<u8>| Image::new(area.nx, area.nz, v.into_iter().map(|b| b as u16).collect());
    Valley { area, class: image(class), owner: image(owner), town_distance }
}
