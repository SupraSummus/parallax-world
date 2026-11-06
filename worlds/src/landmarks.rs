//! Landmarks: structures standing alone through the valley, a ruined keep
//! on a hilltop in the forest, a watchtower on a rise, a wayside chapel by
//! the path, a pond in a hollow, a farmstead and a windmill in the
//! farmland. A domain of the valley (valley.rs): at most one in each
//! region, chosen by what the valley's raw ground is like there; it
//! shapes the ground under it, bids for its footprint and a clearing
//! round it, so trees and crops keep off, and draws what it wins.
//!
//! A new landmark is a variant of `Landmark` and its `Spec`: where it may
//! stand, its footprint, how it shapes the ground, and its builder: hollow
//! solids with a measured far form (`town::thing`), open frames and boards
//! as they are.

use std::rc::Rc;

use worldgen::column::Column;
use worldgen::frame::{Frame, Part, facing, walls};
use worldgen::generator::{ANY_SIZE, Child, Field, Fill, Kind, Layer, Node, Op, Population, SOLID};
use worldgen::js;
use worldgen::kinds;
use worldgen::noise::{Fx, Rng, ValueNoise, fx, hash64, smoothstep, stream};
use worldgen::plan::{Land, Plan};
use worldgen::units::{Units, m};

use crate::forest::K as F;
use crate::mountains::{self, K as M};
use crate::town::{House, K as T, Style, thing};
use crate::valley::Terrain;

kinds! {
    base: mountains::BASE + mountains::KINDS.len() as u8;
    sand: 2, ["#cdb98a", "#c1ac7c"];
    reed: 5, ["#7f8a3a", "#9b9446"], walk_through;
    lily: 5, ["#4f8a3a"], walk_through;
    sail: 10, ["#e0d2b0", "#d4c5a2"];
    bronze: 3, ["#a87a3a"];
}

/**
 * At most one landmark stands in each region this wide, its cell's
 * centre and all it shapes and bids for inside it: only the region's
 * landmark is asked about a column.
 */
const REGION: i64 = m(224.0);
/** The chance a region has a landmark, and how many places each kind is tried at in it. */
const CHANCE: Fx = fx(0.6);
const TRIES: usize = 8;
/** A landmark's cell, and how far it reaches past it across, up and across: a farmstead's corners, a watchtower's pennant. */
const CELL: i64 = m(16.0);
const REACH: [i64; 3] = [m(20.0), m(30.0), m(20.0)];
/** Plan scales: the bid for land, what each column of a landmark's is. */
pub const BID_SCALE: i64 = m(4.0);
const SPOT_SCALE: i64 = m(1.0);
/** How far round its footprint a landmark eases the ground back to the valley's, and how much the ground may rise under it. */
const TERRACE: Units = Units::m(12.0);
const STEEP: Units = Units::m(2.0);
/** A pond's bowl is this deep, its water this far under its rim, its shore this wide. */
const POND_DEPTH: i64 = m(1.8);
const POND_DROP: i64 = m(0.4);
const POND_SHORE: i64 = m(3.0);
/** How much deeper a roof is than the gable under it (as town.rs's). */
const ROOF: i64 = 4;

/** What a landmark is. */
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Landmark {
    Ruin,
    Watchtower,
    Chapel,
    Pond,
    Farmstead,
    Windmill,
}

pub const LANDMARKS: [Landmark; 6] =
    [Landmark::Ruin, Landmark::Watchtower, Landmark::Chapel, Landmark::Pond, Landmark::Farmstead, Landmark::Windmill];

/** What a place is like, from the valley's ground before landmarks shape it: what a landmark stands by. */
struct Place {
    /** From the nearest town's streets; far if none. */
    town: Units,
    /** From the path's centre line. */
    path: Units,
    /** The ground there over the mean of a ring 24 m round it: up on a rise, down in a hollow. */
    prominence: Units,
}

/** How a landmark shapes the ground under its footprint. */
#[derive(Clone, Copy, PartialEq, Eq)]
enum Shaping {
    /** Levelled to the mean under it, raised by this. */
    Level(i64),
    /** Levelled, and dug into a bowl that holds water: a pond. */
    Bowl,
}

/** Where in its region a landmark is tried. */
#[derive(Clone, Copy, PartialEq, Eq)]
enum Stands {
    Anywhere,
    /** Beside the path, 5 to 10 m off it. */
    ByPath,
    /** In the farmland round a town, 6 to 30 m out from its streets. */
    ByTown,
}

/** A landmark's entry: where it may stand, how often, its footprint and ground, and its builder. */
struct Spec {
    suits: fn(&Place) -> bool,
    /** How often it is chosen of those that suit a place. */
    weight: u32,
    /** Its footprint in its own frame, u along its front and w back from it, lattice units. */
    size: fn(&mut Rng) -> [i64; 2],
    /** Its front (−w) to the path, or any way. */
    faces_path: bool,
    stands: Stands,
    shaping: Shaping,
    /** How far round its footprint it owns the land: trees and crops keep off. */
    clearing: Units,
    build: fn(&Site, &mut Rng) -> Node,
}

impl Landmark {
    fn spec(self) -> Spec {
        match self {
            Landmark::Ruin => Spec {
                suits: |p| p.town > Units::m(300.0) && p.path > Units::m(30.0) && p.prominence >= Units::m(1.0),
                weight: 3,
                size: |r| [m(17.0) + r.below(m(2.0) as u32) as i64; 2],
                faces_path: false,
                stands: Stands::Anywhere,
                shaping: Shaping::Level(0),
                clearing: Units::m(7.0),
                build: ruin,
            },
            Landmark::Watchtower => Spec {
                suits: |p| p.town > Units::m(150.0) && p.path > Units::m(16.0) && p.prominence >= Units::m(0.8),
                weight: 1,
                size: |_| [m(6.0), m(6.0)],
                faces_path: true,
                stands: Stands::Anywhere,
                shaping: Shaping::Level(0),
                clearing: Units::m(5.0),
                build: watchtower,
            },
            Landmark::Chapel => Spec {
                suits: |p| p.town > Units::m(100.0) && p.path > Units::m(11.0) && p.path < Units::m(20.0),
                weight: 2,
                size: |_| [m(7.0), m(12.0)],
                faces_path: true,
                stands: Stands::ByPath,
                shaping: Shaping::Level(0),
                clearing: Units::m(4.0),
                build: chapel,
            },
            Landmark::Pond => Spec {
                suits: |p| p.town > Units::m(60.0) && p.path > Units::m(22.0) && p.prominence <= -Units::m(0.8),
                weight: 2,
                size: |r| {
                    let (a, b) = (m(6.0) + r.below(m(4.0) as u32) as i64, m(5.0) + r.below(m(3.0) as u32) as i64);
                    [2 * (a + POND_SHORE), 2 * (b + POND_SHORE)]
                },
                faces_path: false,
                stands: Stands::Anywhere,
                shaping: Shaping::Bowl,
                clearing: Units::m(4.0),
                build: pond,
            },
            Landmark::Farmstead => Spec {
                suits: |p| p.town > Units::m(10.0) && p.town < Units::m(60.0) && p.path > Units::m(26.0),
                weight: 8,
                size: |r| [m(30.0) + r.below(m(4.0) as u32) as i64, m(24.0) + r.below(m(4.0) as u32) as i64],
                faces_path: true,
                stands: Stands::ByTown,
                shaping: Shaping::Level(0),
                clearing: Units::m(8.0),
                build: farmstead,
            },
            Landmark::Windmill => Spec {
                suits: |p| p.town > Units::m(15.0) && p.town < Units::m(60.0) && p.path > Units::m(20.0),
                weight: 8,
                size: |_| [m(10.0), m(10.0)],
                faces_path: false,
                stands: Stands::ByTown,
                shaping: Shaping::Level(m(1.5)),
                clearing: Units::m(10.0),
                build: windmill,
            },
        }
    }
}

/** A landmark where it stands: its centre (x, z), the top of the ground under it, its footprint and quarter turns. */
#[derive(Clone, Copy, Debug)]
pub struct Site {
    pub landmark: Landmark,
    pub centre: [i64; 2],
    pub level: i64,
    /** In its own frame, as Spec::size. */
    pub size: [i64; 2],
    pub turns: i64,
    pub seed: u64,
    /** How far round its footprint it owns the land: trees and crops keep off. */
    pub clearing: Units,
}

impl Site {
    /** Its own frame: u along its front, v up from the ground, w back from the front. */
    fn frame(&self) -> Frame {
        Frame::centred([self.centre[0], self.level + 1, self.centre[1]], [self.size[0], 0, self.size[1]], self.turns)
    }

    /** Half its footprint along x and z. */
    pub fn half(&self) -> [i64; 2] {
        if self.turns % 2 == 0 { [self.size[0] / 2, self.size[1] / 2] } else { [self.size[1] / 2, self.size[0] / 2] }
    }

    /** How far column (x, z) is outside its footprint; 0 inside. */
    fn outside(&self, x: i64, z: i64) -> Units {
        let [hx, hz] = self.half();
        let (dx, dz) = (((x - self.centre[0]).abs() - hx).max(0), ((z - self.centre[1]).abs() - hz).max(0));
        Units(((dx * dx + dz * dz) as u64).isqrt() as i64)
    }

    /** The top of a pond's water. */
    fn water(&self) -> Option<i64> {
        (self.landmark == Landmark::Pond).then_some(self.level - POND_DROP)
    }

    /** A pond's bowl: column (x, z)'s distance from its centre over its radii, squared; under one in it. */
    fn bowl(&self, x: i64, z: i64) -> Fx {
        let [hx, hz] = self.half();
        let (a, b) = (hx - POND_SHORE, hz - POND_SHORE);
        let (dx, dz) = (x - self.centre[0], z - self.centre[1]);
        Units(dx * dx).to_fx() / Units(a * a) + Units(dz * dz).to_fx() / Units(b * b)
    }
}

/**
 * The region of column (x, z): REGION wide, region (0, 0) around the
 * origin; each row of them shifted along x by 0.618 of the one before,
 * so what their edges keep landmarks off differs from row to row.
 */
fn region(x: i64, z: i64) -> (i64, i64) {
    let k = (z + REGION / 2).div_euclid(REGION);
    ((x - origin(0, k)[0]).div_euclid(REGION), k)
}

/** Region (i, k)'s least x and z. */
fn origin(i: i64, k: i64) -> [i64; 2] {
    [i * REGION - REGION / 2 - (k * REGION * 618 / 1000).rem_euclid(REGION), k * REGION - REGION / 2]
}

/** What place `centre` is, on `terrain`. */
fn place(terrain: &Terrain, [x, z]: [i64; 2]) -> Place {
    let (r, d) = (m(24.0), m(24.0) * 7 / 10);
    let ring = [(r, 0), (-r, 0), (0, r), (0, -r), (d, d), (d, -d), (-d, d), (-d, -d)].map(|(dx, dz)| terrain.ground(x + dx, z + dz));
    Place {
        town: terrain.towns.distance(x, z),
        path: (Units(x).to_fx() - terrain.path_x(z)).abs().floor_units(),
        prominence: (terrain.ground(x, z) - Fx::mean(ring)).round_units(),
    }
}

/**
 * Whether `site` keeps off the path and out of the towns, stands below
 * the mountains and well over the world's bottom, and its region holds
 * its cell and all it shapes.
 */
fn fits(terrain: &Terrain, site: &Site) -> bool {
    let ([hx, hz], [cx, cz]) = (site.half(), site.centre);
    let reach = TERRACE.0.max(site.clearing.0 + m(2.0));
    let (i, k) = region(cx, cz);
    let [x0, z0] = origin(i, k);
    let inside = |c: i64, h: i64, c0: i64| c - h - reach >= c0 && c + h + reach < c0 + REGION && c - CELL >= c0 && c + CELL < c0 + REGION;
    if !inside(cx, hx, x0) || !inside(cz, hz, z0) {
        return false;
    }
    let off_path = (cz - hz..=cz + hz).step_by(m(1.0) as usize).all(|z| {
        let path = terrain.path_x(z).round_units().0;
        (path - cx).abs() - hx >= m(3.5)
    });
    // Its footprint's edge, every 2 m, and its middle.
    let (nx, nz) = (hx / m(1.0), hz / m(1.0));
    let edge = (-nx..=nx).flat_map(|i| [(i, -nz), (i, nz)]).chain((-nz..=nz).flat_map(|j| [(-nx, j), (nx, j)])).step_by(2).chain([(0, 0)]);
    let mut points = edge.map(|(i, j)| (cx + i * m(1.0), cz + j * m(1.0)));
    let clear = site.clearing + Units::m(4.0);
    off_path
        && points.all(|(x, z)| {
            let ground = terrain.ground(x, z);
            terrain.towns.distance(x, z) > clear && ground < Fx::m(35.0) && ground > Fx::m(-24.0)
        })
}

/** The valley's ground under `site`: at its corners, the middles of its sides and its centre. */
fn under(terrain: &Terrain, site: &Site) -> [Fx; 9] {
    let ([hx, hz], [cx, cz]) = (site.half(), site.centre);
    std::array::from_fn(|n| terrain.ground(cx + (n as i64 % 3 - 1) * hx, cz + (n as i64 / 3 - 1) * hz))
}

/** `site` on the ground: level with the mean of what is under it, and raised as its landmark is. */
fn levelled(terrain: &Terrain, mut site: Site) -> Site {
    let raise = match site.landmark.spec().shaping {
        Shaping::Level(raise) => raise,
        Shaping::Bowl => 0,
    };
    site.level = Fx::mean(under(terrain, &site)).round_units().0 + raise;
    site
}

/** Whether the ground under `site` is even enough to level: it rises at most STEEP, and a quarter of its length, across it. */
fn even(terrain: &Terrain, site: &Site) -> bool {
    let under = under(terrain, site);
    let (lo, hi) = (under.iter().min().expect("nine"), under.iter().max().expect("nine"));
    *hi - *lo <= (STEEP + Units(site.size[0].max(site.size[1]) / 4)).to_fx()
}

/**
 * The landmark of region (i, k), if it has one: the kinds in an order
 * drawn by weight, each tried at a few places in the region (beside the
 * path if it stands by it), the first that suits one and fits it.
 * The first region's is a chapel by the path ahead of the camera.
 */
fn site_of(seed: u64, terrain: &Terrain, i: i64, k: i64) -> Option<Site> {
    let mut random = Rng::new(hash64(seed, i, k, 0));
    if (i, k) == (0, 0) {
        return Some(levelled(terrain, first(terrain, &mut random)));
    }
    if !random.chance(CHANCE) {
        return None;
    }
    let [x0, z0] = origin(i, k);
    let mut kinds: Vec<Landmark> = LANDMARKS.to_vec();
    while !kinds.is_empty() {
        let total: u32 = kinds.iter().map(|l| l.spec().weight).sum();
        let mut pick = random.below(total);
        let n = kinds
            .iter()
            .position(|l| {
                let weight = l.spec().weight;
                if pick < weight {
                    return true;
                }
                pick -= weight;
                false
            })
            .expect("a kind");
        let landmark = kinds.swap_remove(n);
        let spec = landmark.spec();
        for _ in 0..TRIES {
            let size = (spec.size)(&mut random);
            let Some([x, z]) = candidate(terrain, spec.stands, (x0, z0), size, &mut random) else { break };
            if region(x, z) != (i, k) {
                continue;
            }
            let turns = if spec.faces_path { facing(terrain.path_x(z).round_units().0 - x, 0) } else { random.below(4) as i64 };
            let site = Site { landmark, centre: [x, z], level: 0, size, turns, seed: random.next_u64(), clearing: spec.clearing };
            if fits(terrain, &site) && even(terrain, &site) && (spec.suits)(&place(terrain, site.centre)) {
                return Some(levelled(terrain, site));
            }
        }
    }
    None
}

/** A place in the region from (x0, z0) to try a landmark of `size` at, as it `stands`; none if it can't stand in it. */
fn candidate(terrain: &Terrain, stands: Stands, (x0, z0): (i64, i64), size: [i64; 2], random: &mut Rng) -> Option<[i64; 2]> {
    let side = if random.chance(Fx::HALF) { 1 } else { -1 };
    match stands {
        Stands::Anywhere => Some([x0 + below(random, REGION), z0 + below(random, REGION)]),
        Stands::ByPath => {
            let z = z0 + below(random, REGION);
            Some([terrain.path_x(z).round_units().0 + side * (size[1] / 2 + m(5.0) + below(random, m(5.0))), z])
        }
        Stands::ByTown => {
            let t = terrain.towns.near(z0 + REGION / 2)?;
            let (half_z, out) = (t.half_z() + m(40.0), size[0].max(size[1]) / 2 + m(6.0) + below(random, m(24.0)));
            Some([t.x + side * (t.half_x() + out), t.z - half_z + below(random, 2 * half_z)])
        }
    }
}

/** The first region's chapel, beside the path 33 m ahead of the camera, its front to the path, nearer the camera's line of sight. */
fn first(terrain: &Terrain, random: &mut Rng) -> Site {
    let landmark = Landmark::Chapel;
    let size = (landmark.spec().size)(random);
    let z = m(2.0);
    // On the side the camera, on the path at z = -31 m, looks across to.
    let side = if terrain.path_x(m(-31.0)) > terrain.path_x(z) { 1 } else { -1 };
    // Across the path, clear of it where it bends nearest.
    let near = (z - size[0] / 2..=z + size[0] / 2)
        .step_by(m(1.0) as usize)
        .map(|z| terrain.path_x(z).round_units().0 * side)
        .max()
        .expect("a step");
    let x = side * (near + size[1] / 2 + m(5.0));
    // A wider clearing than a chapel's, so that the trees between hide less of it.
    Site { landmark, centre: [x, z], level: 0, size, turns: facing(-side, 0), seed: random.next_u64(), clearing: Units::m(16.0) }
}

/** What a column round a landmark is. */
#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum Spot {
    #[default]
    Off,
    Grass,
    Trodden,
    Sand,
    Gravel,
    Mud,
}

/** The valley's landmarks, by region. */
pub struct Landmarks {
    terrain: Rc<Terrain>,
    sites: Plan<Option<Site>>,
    /** Patches of gravel and of a yard's grass; clumps of reeds. */
    patches: ValueNoise<{ m(3.0) }>,
    reeds: ValueNoise<{ m(2.5) }>,
}

impl Landmarks {
    pub fn new(seed: u64, terrain: Rc<Terrain>) -> Landmarks {
        let sites = {
            let (terrain, seed) = (terrain.clone(), stream(seed, 70));
            Plan::new("landmarks", REGION, move |i, k| site_of(seed, &terrain, i, k))
        };
        Landmarks { terrain, sites, patches: ValueNoise::new(stream(seed, 71)), reeds: ValueNoise::new(stream(seed, 72)) }
    }

    /** The landmark of region (i, k), if any. */
    pub fn of_region(&self, i: i64, k: i64) -> Option<Site> {
        self.sites.at(i, k)
    }

    /** The landmark of column (x, z)'s region, if any. */
    pub fn site(&self, x: i64, z: i64) -> Option<Site> {
        let (i, k) = region(x, z);
        self.of_region(i, k)
    }

    /** The valley's `ground` at (x, z), with its fraction, shaped by the landmark there: levelled under it, easing back over TERRACE. */
    pub fn shaped(&self, x: i64, z: i64, ground: Fx) -> Fx {
        let Some(s) = self.site(x, z) else { return ground };
        let d = s.outside(x, z);
        if d >= TERRACE {
            return ground;
        }
        let weight = Fx::ONE - smoothstep(Fx::ZERO, TERRACE.to_fx(), d.to_fx());
        let level = ground + (Units(s.level).to_fx() - ground) * weight;
        match s.landmark.spec().shaping {
            Shaping::Bowl => level - (Fx::ONE - s.bowl(x, z)).max(Fx::ZERO) * Units(POND_DEPTH),
            Shaping::Level(_) => level,
        }
    }

    /** Its bid for column (x, z), BID_SCALE: ahead over a landmark's footprint and clearing, never on the path. */
    pub fn bid(&self, x: i64, z: i64) -> Fx {
        let most = fx(4.0);
        let Some(s) = self.site(x, z) else { return -most };
        if (Units(x).to_fx() - self.terrain.path_x(z)).abs() < Fx::m(2.4) {
            return -most;
        }
        ((s.clearing - s.outside(x, z)).to_fx() / Units::m(2.0)).clamp(-most, most)
    }

    /** What column (x, z) is of the landmark there. */
    fn spot(&self, x: i64, z: i64) -> Spot {
        let Some(s) = self.site(x, z) else { return Spot::Off };
        let d = s.outside(x, z);
        if d > s.clearing + Units::m(2.0) {
            return Spot::Off;
        }
        let (dx, dz) = (x - s.centre[0], z - s.centre[1]);
        let [hx, hz] = s.half();
        let patch = self.patches.at(x, z);
        match s.landmark {
            Landmark::Ruin if d == Units(0) && patch > fx(0.62) => Spot::Gravel,
            Landmark::Watchtower if dx.abs().max(dz.abs()) < m(2.6) => Spot::Trodden,
            Landmark::Chapel => {
                // A path from its door to the valley's.
                let [ux, _, uz] = s.frame().site([s.size[0] / 2, 0, 0]);
                let path = self.terrain.path_x(z).round_units().0;
                let along = (x - ux) * (path - ux) >= 0 && (x - ux).abs() <= (path - ux).abs();
                if (z - uz).abs() <= m(0.8) && along { Spot::Trodden } else { Spot::Grass }
            }
            Landmark::Pond => {
                let bowl = s.bowl(x, z);
                if bowl < fx(0.7) {
                    Spot::Mud
                } else if bowl < fx(1.5) {
                    if patch > fx(0.65) { Spot::Gravel } else { Spot::Sand }
                } else {
                    Spot::Grass
                }
            }
            Landmark::Farmstead if dx.abs() < hx - m(1.0) && dz.abs() < hz - m(1.0) && patch < fx(0.6) => Spot::Trodden,
            Landmark::Windmill if dx * dx + dz * dz < m(4.6) * m(4.6) => Spot::Trodden,
            _ => Spot::Grass,
        }
    }
}

/** The landmarks' layers on `ground`, where domain `me` of `land` owns the land: their surface, ponds' water, the landmarks. */
pub fn layers(landmarks: Rc<Landmarks>, land: Rc<Land>, me: usize, ground: Rc<Plan<i64>>) -> Vec<Layer> {
    let spots = {
        let landmarks = landmarks.clone();
        Rc::new(Plan::dithered("landmark spots", SPOT_SCALE, move |x, z| landmarks.spot(x, z)))
    };
    let surface = {
        let ground = ground.clone();
        Field::columns("landmarks' ground", 0.55, Op::Paint, move |x, z, spacing| {
            let spot = spots.at(x, z);
            if spot == Spot::Off || !land.owns(me, x, z) {
                return Vec::new();
            }
            let kind: Kind = match spot {
                Spot::Trodden => F::path.into(),
                Spot::Sand => K::sand.into(),
                Spot::Gravel => M::scree.into(),
                Spot::Mud => F::soil.into(),
                _ => F::grass.into(),
            };
            Column::new(ground.at(x, z), spacing).surface(kind, m(0.5)).spans()
        })
    };
    // Ponds' water up to its level, reeds round their edges while sites are close enough to show them thin.
    let water = {
        let (landmarks, ground) = (landmarks.clone(), ground.clone());
        Field::columns("ponds", 0.7, Op::Paint, move |x, z, spacing| {
            let Some(s) = landmarks.site(x, z) else { return Vec::new() };
            let Some(level) = s.water() else { return Vec::new() };
            let bowl = s.bowl(x, z);
            if bowl >= fx(1.3) {
                return Vec::new();
            }
            let mut column = Column::new(ground.at(x, z), spacing);
            if bowl < Fx::ONE {
                column = column.water_to(T::water, level);
            }
            if bowl > fx(0.62) && landmarks.reeds.at(x, z) > fx(0.58) {
                column = column.medium(K::reed, m(1.2), 0.6);
            }
            column.spans()
        })
    };
    let built = {
        let anchor = landmarks.clone();
        Population::new("landmarks", 1.6, CELL)
            .reach(REACH)
            .anchor(move |c| anchor.site(c.centre[0], c.centre[2]).map(|s| [s.centre[0] as f64, s.level as f64, s.centre[1] as f64]))
            .make(move |_, c| landmarks.site(c.centre[0], c.centre[2]).map(|s| (s.landmark.spec().build)(&s, &mut Rng::new(s.seed))))
    };
    vec![Layer::Field(surface), Layer::Field(water), Layer::Population(built)]
}

// ---------------------------------------------------------------------------
// Builders: each a landmark in its own frame (Site::frame), u along its front, v up from the ground, w back from the front.

fn below(random: &mut Rng, n: i64) -> i64 {
    random.below(n as u32) as i64
}

/**
 * An arched opening in a wall (its own frame: u along it, v up, w into
 * it), `half` either side of u, its sides from v up `tall`, a round head
 * over them; cut `deep`, and `kind` set in at the back if any.
 */
fn arch(wall: &mut Part, u: i64, v: i64, half: i64, tall: i64, deep: i64, kind: Option<Kind>) {
    let (uf, top, r) = (u as f64, (v + tall) as f64, half as f64);
    wall.cut([u - half, v, 0], [u + half, v + tall, deep]);
    wall.lathe([uf, top, 0.0], [uf, top, deep as f64], move |_| r, 0, SOLID, Op::Carve);
    if let Some(kind) = kind {
        wall.paint([u - half, v, deep], [u + half, v + tall, deep + 1], kind);
        wall.lathe([uf, top, deep as f64], [uf, top, (deep + 1) as f64], move |_| r, kind, SOLID, Op::Paint);
    }
}

/**
 * A gable roof over a box `length` along u, `depth` along w, its walls
 * `top` high: a gable of `wall` under it, the roof `rise` over the eaves
 * along both sides, its ridge along u (0) or w (2).
 */
fn roof(part: &mut Part, [length, depth]: [i64; 2], top: i64, rise: i64, ridge: usize, (wall, roof): (Kind, Kind)) {
    let across = 2 - ridge;
    let (width, eaves) = (if across == 0 { length } else { depth }, m(0.5));
    // Flush with the gable ends: a roof over them would cover them, a gable being solid.
    let box_of = |rise: f64, eaves: i64, from: i64| {
        let (mut min, mut max) = ([0.0, from as f64, 0.0], [length as f64, from as f64 + rise, depth as f64]);
        (min[across], max[across]) = (-eaves as f64, (width + eaves) as f64);
        (min, max)
    };
    let (min, max) = box_of(rise as f64 * width as f64 / (width + 2 * eaves) as f64, 0, top - ROOF);
    part.gable(min, max, ridge, wall, Op::Fill);
    let (min, max) = box_of(rise as f64, eaves, top);
    part.gable(min, max, ridge, roof, Op::Fill);
    // Eaves boarded along both sides: a thin wedge's edge would leave gaps.
    for (from, to) in [(-eaves, 0), (width, width + eaves)] {
        let (mut min, mut max) = ([0, top - 2, 0], [length, top + 1, depth]);
        (min[across], max[across]) = (from, to);
        part.add(min, max, roof);
    }
}

/**
 * A ruined keep: walls standing to broken heights, mossy along their
 * tops, an arched gate and windows, a breach; a round tower at a corner
 * with what is left of its battlements; an arch of a hall gone; rubble.
 * Thick and open, it is drawn as it is at every level: no far form.
 */
fn ruin(s: &Site, random: &mut Rng) -> Node {
    let mut shapes = Vec::new();
    let mut part = Part::new(s.frame(), &mut shapes);
    let size = s.size[0];
    let keep = m(10.0) + below(random, m(1.5));
    let o = (size - keep) / 2;
    let t = m(1.25);
    let stone: Kind = F::stone.into();
    let moss: Kind = F::mossyStone.into();
    let breach = 1 + 2 * below(random, 2) as usize;
    {
        let mut k = part.within(Frame::at([o, 0, o]));
        for (i, (frame, length)) in walls(keep, keep).into_iter().enumerate() {
            let mut wall = k.within(frame);
            // The gate's wall and the back one high enough for their openings, the others lower.
            let base = match i {
                0 => m(4.2),
                2 => m(5.0),
                _ => m(2.0) + below(random, m(2.5)),
            };
            wall.add([0, -m(1.0), 0], [length, base, t], stone);
            // Broken tops: stretches of 1.5 to 3 m, each standing to a height of its own, mossy on top.
            let mut u = 0;
            while u < length {
                let wide = (m(1.5) + below(random, m(1.5))).min(length - u);
                let top = base + below(random, m(3.0));
                wall.add([u, base, 0], [u + wide, top, t], stone);
                wall.paint([u, top - 3, 0], [u + wide, top, t], moss);
                u += wide;
            }
            wall.cells([0, 0, -1], [length, m(0.9), t + 1], moss, Fill::Const(0.4), Op::Paint);
            match i {
                0 => {
                    // The gate: an arch of dressed stone.
                    let c = length / 2;
                    wall.paint([c - m(1.2), 0, 0], [c + m(1.2), m(2.2), t], T::churchStone);
                    wall.lathe(
                        [c as f64, m(2.2) as f64, 0.0],
                        [c as f64, m(2.2) as f64, t as f64],
                        |_| m(1.2) as f64,
                        T::churchStone,
                        SOLID,
                        Op::Paint,
                    );
                    arch(&mut wall, c, 0, m(0.9), m(2.2), t, None);
                }
                2 => {
                    for u in [length / 3, 2 * length / 3] {
                        arch(&mut wall, u, m(2.6), 4, m(1.0), t, None);
                    }
                }
                _ if i == breach => {
                    let c = length / 2 + below(random, m(2.0)) - m(1.0);
                    wall.cut([c - m(1.3), m(0.7), 0], [c + m(1.3), m(10.0), t]);
                    wall.cut([c - m(0.6), m(0.2), 0], [c + m(0.6), m(0.7), t]);
                }
                _ => {}
            }
        }
        // The tower on the back corner: round, a little wider at its foot, slits, the stumps of its merlons.
        let (c, r) = ((keep - t / 2) as f64, m(2.4) as f64);
        let top = m(10.5) + below(random, m(3.0));
        k.lathe([c, -m(1.0) as f64, c], [c, top as f64, c], move |h| r + 3.0 * (1.0 - h), stone, SOLID, Op::Fill);
        k.lathe([c, (top - 3) as f64, c], [c, top as f64, c], move |_| r + 1.0, moss, SOLID, Op::Paint);
        for j in 0..8 {
            if !random.chance(fx(0.7)) {
                continue;
            }
            let a = j as f64 * std::f64::consts::PI / 4.0;
            let (u, w) = ((c + (r - 3.0) * js::cos(a)) as i64, (c + (r - 3.0) * js::sin(a)) as i64);
            let high = top + m(0.5) + below(random, m(0.5));
            k.add([u - 3, top, w - 3], [u + 3, high, w + 3], stone);
            k.paint([u - 3, high - 2, w - 3], [u + 3, high, w + 3], moss);
        }
        let ci = c as i64;
        for (v, (du, dw)) in [(m(4.0), (1, 0)), (m(7.0), (0, 1)), (m(5.5), (1, 0))] {
            let (u, w) = (ci + du * (r as i64 + 1), ci + dw * (r as i64 + 1));
            let (half_u, half_w) = (1 + du * 3, 1 + dw * 3);
            k.cut([u - half_u, v, w - half_w], [u + half_u, v + m(1.0), w + half_w]);
        }
    }
    // An arch of a hall gone, by the keep's left side: two piers under a round head.
    let (u0, w0, pier, gap) = (m(1.0), o + m(1.0) + below(random, m(1.5)), m(0.9), m(2.4));
    let arch_top = m(3.6) + below(random, m(0.6));
    part.add([u0, -m(0.8), w0], [u0 + pier, arch_top, w0 + 2 * pier + gap], stone);
    part.cut([u0, 0, w0 + pier], [u0 + pier, m(2.0), w0 + pier + gap]);
    let (head, mid) = (m(2.0) as f64, (w0 + pier + gap / 2) as f64);
    part.lathe([u0 as f64, head, mid], [(u0 + pier) as f64, head, mid], move |_| (gap / 2) as f64, 0, SOLID, Op::Carve);
    part.paint([u0, arch_top - 3, w0], [u0 + pier, arch_top, w0 + 2 * pier + gap], moss);
    // Rubble round the walls' feet, half sunk.
    for _ in 0..20 + below(random, 10) {
        let (u, w) = ((m(0.5) + below(random, size - m(1.0))) as f64, (m(0.5) + below(random, size - m(1.0))) as f64);
        let r = 2.0 + random.next() * 4.0;
        let kind = if random.chance(fx(0.4)) { moss } else { stone };
        part.ellipsoid([u, r * 0.3 - 1.0, w], [r * (1.0 + random.next() * 0.5), r * 0.7, r * (1.0 + random.next() * 0.5)], kind, Op::Fill);
    }
    for _ in 0..3 {
        let (u, w) = (o + keep + m(0.5) + below(random, m(2.0)), o + below(random, keep));
        let (a, b) = (m(0.5) + below(random, m(0.5)), m(0.4) + below(random, m(0.4)));
        part.add([u, -2, w], [u + a, b, w + b], stone);
    }
    Node::of(shapes)
}

/**
 * A wooden lookout tower over the treetops: four posts leaning in,
 * braced crosswise in tiers, a planked platform with a railing, a ladder
 * up through a hatch in it, a roof on the posts and a pennant. Rods and
 * boards, drawn as they are at every level.
 */
fn watchtower(s: &Site, random: &mut Rng) -> Node {
    let mut shapes = Vec::new();
    let mut part = Part::new(s.frame(), &mut shapes);
    let (wood, deck): (Kind, Kind) = (F::deadwood.into(), T::planks.into());
    let c = s.size[0] / 2;
    let cf = c as f64;
    let floor = m(20.0) + below(random, m(3.0));
    let top = floor + m(2.2);
    // Half the square of the posts' axes at height v: 2.2 m at the foot, 1.4 m at the top.
    let half = move |v: i64| m(2.2) as f64 - (m(0.8) * v) as f64 / top as f64;
    let corner = |q: usize, v: i64| {
        let (su, sw) = [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)][q];
        [cf + su * half(v), v as f64, cf + sw * half(v)]
    };
    for q in 0..4 {
        part.capsule(corner(q, -m(0.6)), corner(q, top), 1.6, wood, Op::Fill);
    }
    // Braced crosswise between each pair of posts in tiers, a girt over each.
    let tiers = 5;
    for t in 0..tiers {
        let (v0, v1) = (m(0.3) + (floor - m(0.3)) * t / tiers, m(0.3) + (floor - m(0.3)) * (t + 1) / tiers);
        for q in 0..4 {
            let r = (q + 1) % 4;
            part.capsule(corner(q, v0), corner(r, v1), 1.1, wood, Op::Fill);
            part.capsule(corner(r, v0), corner(q, v1), 1.1, wood, Op::Fill);
            part.capsule(corner(q, v1), corner(r, v1), 1.3, wood, Op::Fill);
        }
    }
    // The platform on joists, a railing round it; the hatch over the ladder at the front.
    let h = half(floor) as i64;
    let (e0, e1) = (c - h - m(0.6), c + h + m(0.6));
    for w in [c - h - 1, c + h - 1] {
        part.add([e0, floor - 3, w], [e1, floor, w + 3], wood);
    }
    part.add([e0, floor, e0], [e1, floor + 2, e1], deck);
    let ladder = c - h + 4;
    part.cut([c - 5, floor - 3, ladder - 3], [c + 5, floor + 2, ladder + 6]);
    for rail in [m(0.5), m(1.0)] {
        let v = floor + rail;
        part.add([e0, v, e0], [e1, v + 2, e0 + 2], deck);
        part.add([e0, v, e1 - 2], [e1, v + 2, e1], deck);
        part.add([e0, v, e0], [e0 + 2, v + 2, e1], deck);
        part.add([e1 - 2, v, e0], [e1, v + 2, e1], deck);
    }
    for (u, w) in [(e0, e0), (e1 - 2, e0), (e0, e1 - 2), (e1 - 2, e1 - 2), (c - 1, e0), (c - 1, e1 - 2), (e0, c - 1), (e1 - 2, c - 1)] {
        part.add([u, floor + 2, w], [u + 2, floor + m(1.0) + 2, w + 2], wood);
    }
    // A ladder straight up the front, inside the posts.
    for u in [c - 4, c + 2] {
        part.add([u, -2, ladder], [u + 2, floor + m(1.0), ladder + 2], wood);
    }
    for v in (m(0.35)..floor).step_by(m(0.35) as usize) {
        part.add([c - 4, v, ladder], [c + 4, v + 1, ladder + 2], wood);
    }
    // A roof over the posts' tops, its ridge along the front.
    let kind: Kind = if random.chance(Fx::HALF) { T::thatch.into() } else { T::roofBrown.into() };
    let (r0, r1) = (cf - half(top) - m(0.8) as f64, cf + half(top) + m(0.8) as f64);
    let ridge = top + m(1.8);
    part.gable([r0, top as f64, r0], [r1, ridge as f64, r1], 0, kind, Op::Fill);
    // A pennant on a pole at the ridge.
    part.capsule([cf, (ridge - 4) as f64, cf], [cf, (ridge + m(1.6)) as f64, cf], 0.8, T::iron, Op::Fill);
    let fv = ridge + m(1.0);
    part.add([c + 1, fv, c - 1], [c + 1 + m(0.9), fv + m(0.5), c + 1], T::faluRed);
    Node::of(shapes)
}

/**
 * A wayside chapel: a stone nave, its gable to the path with an arched
 * door and a round window, lancets down its sides, an apse at the back,
 * a bell-cote with its bell over the front gable. Solid, measured far away.
 */
fn chapel(s: &Site, random: &mut Rng) -> Node {
    let mut shapes = Vec::new();
    let mut part = Part::new(s.frame(), &mut shapes);
    let stone: Kind = T::churchStone.into();
    let roof_kind: Kind = if random.chance(fx(0.6)) { T::slate.into() } else { T::roofRed.into() };
    let (width, top, rise) = (m(5.0), m(4.0), m(3.0));
    let length = s.size[1] - m(3.6);
    let mut nave = part.within(Frame::at([(s.size[0] - width) / 2, 0, m(1.0)]));
    nave.add([-2, -m(1.5), -2], [width + 2, 2, length + 2], F::stone);
    nave.add([0, 0, 0], [width, top, length], stone);
    roof(&mut nave, [width, length], top, rise, 2, (stone, roof_kind));
    let c = width / 2;
    let [(front, _), (right, side), _, (left, _)] = walls(width, length);
    {
        let mut wall = nave.within(front);
        // A door under a round arch of darker stone, steps before it, a round window over it.
        wall.paint([c - 7, 0, 0], [c + 7, m(2.0), 1], F::stone);
        wall.lathe([c as f64, m(2.0) as f64, 0.0], [c as f64, m(2.0) as f64, 1.0], |_| 7.0, F::stone, SOLID, Op::Paint);
        arch(&mut wall, c, 0, 5, m(2.0), 3, Some(T::door.into()));
        wall.add([c - 9, -3, -m(0.9)], [c + 9, 1, 0], F::stone);
        let v = (top + 7) as f64;
        wall.lathe([c as f64, v, 0.0], [c as f64, v, 2.0], |_| 3.5, 0, SOLID, Op::Carve);
        wall.lathe([c as f64, v, 2.0], [c as f64, v, 3.0], |_| 3.5, T::glass, SOLID, Op::Paint);
    }
    for frame in [right, left] {
        let mut wall = nave.within(frame);
        for j in 0..3 {
            arch(&mut wall, side * (2 * j + 1) / 6, m(1.4), 2, m(1.4), 2, Some(T::glass.into()));
        }
    }
    // The apse: a half round at the back, lower, under a cone.
    let (cu, back, r) = (c as f64, (length - 1) as f64, (width / 2 - 3) as f64);
    let eaves = (top - m(0.6)) as f64;
    nave.lathe([cu, -m(1.5) as f64, back], [cu, eaves, back], move |_| r, stone, SOLID, Op::Fill);
    nave.lathe(
        [cu, eaves - 2.0, back],
        [cu, eaves + m(1.8) as f64, back],
        move |t| (r + 3.0) * (1.0 - t) + 1.0,
        roof_kind,
        SOLID,
        Op::Fill,
    );
    // The bell-cote on the front gable: a pier, an arch through it and its bell, a little roof and a cross.
    let apex = top + rise;
    let cap = apex + m(1.6);
    nave.add([c - 6, apex - m(1.0), -1], [c + 6, cap, 6], stone);
    arch(&mut nave.within(Frame::at([0, 0, -1])), c, apex + 1, 4, m(0.7), 7, None);
    nave.lathe([cu, (apex + 9) as f64, 2.5], [cu, (apex + 3) as f64, 2.5], |t| 1.2 + 1.8 * t * t, K::bronze, SOLID, Op::Fill);
    nave.gable([(c - 8) as f64, cap as f64, -2.0], [(c + 8) as f64, (cap + 6) as f64, 7.0], 2, roof_kind, Op::Fill);
    nave.add([c - 1, cap + 5, 2], [c + 1, cap + 5 + m(1.0), 4], T::iron);
    nave.add([c - 3, cap + 9, 2], [c + 3, cap + 11, 4], T::iron);
    thing(shapes)
}

/**
 * A pond: its bowl and water are the ground's and the ponds' field; this
 * is its jetty from the front shore out over the water, on posts, and
 * lily pads on the water while sites are close enough to show them.
 */
fn pond(s: &Site, random: &mut Rng) -> Node {
    let mut jetty = Vec::new();
    let mut part = Part::new(s.frame(), &mut jetty);
    let [su, sw] = s.size;
    let (cu, rb) = (su / 2, sw / 2 - POND_SHORE);
    let (w0, w1) = (POND_SHORE - m(2.0), POND_SHORE + m(4.5).min(rb - m(1.5)));
    let wood: Kind = F::deadwood.into();
    part.add([cu - 5, 0, w0], [cu + 5, 2, w1], T::planks);
    let mut w = w0;
    while w < w1 {
        part.paint([cu - 5, 1, w], [cu + 5, 2, w + 1], wood);
        w += 4;
    }
    let mut w = w0 + 1;
    while w < w1 {
        for u in [cu - 6, cu + 4] {
            part.add([u, -m(2.2), w], [u + 2, 3, w + 2], wood);
        }
        w += m(1.5);
    }
    let mut lilies = Vec::new();
    let mut part = Part::new(s.frame(), &mut lilies);
    let (ra, rb) = ((su / 2 - POND_SHORE) as f64, rb as f64);
    let water = (-POND_DROP - 1) as f64;
    for _ in 0..6 + below(random, 8) {
        let (a, d) = (random.next() * 2.0 * std::f64::consts::PI, 0.2 + random.next() * 0.45);
        let (u, w) = (cu as f64 + ra * d * js::cos(a), (sw / 2) as f64 + rb * d * js::sin(a));
        let r = 2.0 + random.next() * 2.0;
        part.lathe([u, water, w], [u, water + 2.0, w], move |_| r, K::lily, SOLID, Op::Paint);
    }
    Node::group(vec![Child::new("jetty", ANY_SIZE, Node::of(jetty)), Child::new("lilies", m(0.5), Node::of(lilies))])
}

/**
 * A farmstead: a farmhouse at the front (the towns' house), a barn across
 * the back of the yard, haystacks and a well in it, a fence round it with
 * a gate to the front.
 */
fn farmstead(s: &Site, random: &mut Rng) -> Node {
    let [su, sw] = s.size;
    let mut children = Vec::new();
    let (length, depth) = (m(10.0) + below(random, m(2.0)), m(7.0) + below(random, m(1.5)));
    let house = House { thatched: random.chance(fx(0.4)), ..House::new(length, depth, 1 + below(random, 2), Style::of(s.seed)) };
    let mut shapes = Vec::new();
    house.build(&mut Part::new(s.frame().then(Frame::at([m(2.5), 0, m(3.0)])), &mut shapes), random);
    children.push(Child::new("house", ANY_SIZE, thing(shapes)));
    let (length, depth) = (m(13.0) + below(random, m(3.0)), m(8.0));
    let mut shapes = Vec::new();
    barn(&mut Part::new(s.frame().then(Frame::at([su - m(2.5) - length, 0, sw - m(2.5) - depth])), &mut shapes), length, depth, random);
    children.push(Child::new("barn", ANY_SIZE, thing(shapes)));
    let mut shapes = Vec::new();
    well(&mut Part::new(s.frame().then(Frame::at([su - m(4.5), 0, m(7.0)])), &mut shapes), random);
    children.push(Child::new("well", ANY_SIZE, thing(shapes)));
    // Haystacks behind the house: domes of hay round a pole.
    let mut shapes = Vec::new();
    let mut part = Part::new(s.frame(), &mut shapes);
    for i in 0..2 + below(random, 2) {
        let (u, w) = ((m(3.5) + i * m(3.4)) as f64, (sw - m(5.0) - below(random, m(2.0))) as f64);
        let (r, h) = (m(1.4) as f64 + random.next() * m(0.4) as f64, m(2.6) as f64 + random.next() * m(0.6) as f64);
        part.lathe([u, -2.0, w], [u, h, w], move |t| r * (1.0 - t * t * t), F::hay, SOLID, Op::Fill);
        part.capsule([u, h - 4.0, w], [u, h + m(0.6) as f64, w], 1.0, F::deadwood, Op::Fill);
    }
    children.push(Child::new("haystacks", ANY_SIZE, Node::of(shapes)));
    // The fence: posts every 2 m and two rails, the gate's gap at the front.
    let mut shapes = Vec::new();
    let mut part = Part::new(s.frame(), &mut shapes);
    let (a, b) = ([m(1.0), m(1.0)], [su - m(1.0), sw - m(1.0)]);
    let gate = (su - m(11.0), su - m(8.0));
    let wood: Kind = F::deadwood.into();
    for (from, to, along) in [(a, [b[0], a[1]], 0), (a, [a[0], b[1]], 2), ([b[0] - 2, a[1]], [b[0] - 2, b[1]], 2), ([a[0], b[1] - 2], b, 0)]
    {
        let across = 2 - along;
        let ends = if along == 0 { (from[0], to[0]) } else { (from[1], to[1]) };
        let at = if along == 0 { from[1] } else { from[0] };
        let gap = if along == 0 && at == a[1] { Some(gate) } else { None };
        let segments = match gap {
            Some((g0, g1)) => vec![(ends.0, g0), (g1, ends.1)],
            None => vec![ends],
        };
        let box_at = |from: i64, to: i64, v0: i64, v1: i64| {
            let (mut min, mut max) = ([0, v0, 0], [0, v1, 0]);
            (min[along], max[along], min[across], max[across]) = (from, to, at, at + 2);
            (min, max)
        };
        for (s0, s1) in segments {
            for v in [m(0.45), m(0.95)] {
                let (min, max) = box_at(s0, s1, v, v + 2);
                part.add(min, max, wood);
            }
            let posts = (s0..s1 - 2).step_by(m(2.0) as usize).chain([s1 - 2]);
            for p in posts {
                let (min, max) = box_at(p, p + 2, -m(0.4), m(1.25));
                part.add(min, max, wood);
            }
        }
    }
    children.push(Child::new("fence", ANY_SIZE, Node::of(shapes)));
    Node::group(children)
}

/** A barn in its frame, `length` along its front and `depth` back: red boards on a stone footing, timbered corners, a steep roof, doors to the yard, a hayloft. */
fn barn(part: &mut Part, length: i64, depth: i64, random: &mut Rng) {
    let top = m(4.0);
    let (boards, timber): (Kind, Kind) = (T::faluRed.into(), F::bark.into());
    let roof_kind: Kind = if random.chance(Fx::HALF) { T::thatch.into() } else { T::roofBrown.into() };
    part.add([-1, -m(1.0), -1], [length + 1, 2, depth + 1], F::stone);
    part.add([0, 0, 0], [length, top, depth], boards);
    for (u, w) in [(0, 0), (length - 3, 0), (0, depth - 3), (length - 3, depth - 3)] {
        part.paint([u, 0, w], [u + 3, top, w + 3], timber);
    }
    part.paint([0, top - 3, 0], [length, top, depth], timber);
    roof(part, [length, depth], top, m(3.6), 0, (boards, roof_kind));
    // Double doors in the front, planked, braced across.
    let c = length / 2;
    let (half, tall) = (m(1.6), m(3.2));
    part.cut([c - half, 0, 0], [c + half, tall, 2]);
    part.paint([c - half, 0, 2], [c + half, tall, 3], T::planks);
    part.paint([c - 1, 0, 2], [c + 1, tall, 3], timber);
    part.paint([c - half, tall / 2 - 1, 2], [c + half, tall / 2 + 1, 3], timber);
    // A hayloft door high in the gable end, hay inside.
    let [_, (gable, _), _, _] = walls(length, depth);
    let mut end = part.within(gable);
    end.cut([depth / 2 - 6, top + 2, 0], [depth / 2 + 6, top + 2 + m(1.2), 3]);
    end.paint([depth / 2 - 6, top + 2, 3], [depth / 2 + 6, top + 2 + m(1.2), 4], F::hay);
}

/** A well: a ring of stone with water down in it, two posts and a little roof. */
fn well(part: &mut Part, random: &mut Rng) {
    let (r, rim) = (m(0.8) as f64, m(0.8));
    part.lathe([0.0, -2.0, 0.0], [0.0, rim as f64, 0.0], move |_| r, F::stone, SOLID, Op::Fill);
    part.lathe([0.0, m(0.4) as f64, 0.0], [0.0, (rim + 1) as f64, 0.0], move |_| r - 2.0, 0, SOLID, Op::Carve);
    part.lathe([0.0, (m(0.4) - 2) as f64, 0.0], [0.0, m(0.4) as f64, 0.0], move |_| r - 2.0, T::water, SOLID, Op::Paint);
    for u in [-m(0.9), m(0.9) - 2] {
        part.add([u, 0, -1], [u + 2, m(2.0), 1], F::deadwood);
    }
    part.capsule([-m(0.9) as f64, m(1.7) as f64, 0.0], [m(0.9) as f64, m(1.7) as f64, 0.0], 1.0, F::deadwood, Op::Fill);
    let roof_kind: Kind = if random.chance(Fx::HALF) { T::thatch.into() } else { T::roofRed.into() };
    part.gable([-m(1.2) as f64, m(2.0) as f64, -m(0.8) as f64], [m(1.2) as f64, m(2.7) as f64, m(0.8) as f64], 0, roof_kind, Op::Fill);
}

/**
 * A windmill: a round whitewashed tower tapering on a stone footing, a
 * stage round it, a door and windows, a conical cap, and four sails on
 * its windshaft in front of the cap. The tower is measured far away; the
 * sails, boards, are drawn as they are.
 */
fn windmill(s: &Site, random: &mut Rng) -> Node {
    let mut shapes = Vec::new();
    let mut part = Part::new(s.frame(), &mut shapes);
    let (cu, cw) = (s.size[0] / 2, s.size[1] / 2);
    let (u, w) = (cu as f64, cw as f64);
    let (r0, r1) = (m(3.1) as f64, m(2.3) as f64);
    let h = m(10.0) + below(random, m(1.5));
    part.lathe([u, -m(1.2) as f64, w], [u, m(1.0) as f64, w], move |_| r0 + 2.0, T::churchStone, SOLID, Op::Fill);
    part.lathe([u, 0.0, w], [u, h as f64, w], move |t| r0 + (r1 - r0) * t, T::plasterWhite, SOLID, Op::Fill);
    part.lathe([u, m(3.6) as f64, w], [u, (m(3.6) + 2) as f64, w], move |_| r0 + m(1.0) as f64, T::planks, SOLID, Op::Fill);
    // Its door to the front; windows round it at heights of their own.
    let front = cw - r0 as i64;
    part.cut([cu - 5, 0, front - 3], [cu + 5, m(2.1), front + 5]);
    part.paint([cu - 5, 0, front + 5], [cu + 5, m(2.1), front + 6], T::door);
    for (v, (du, dw)) in [(m(6.0), (1, 0)), (m(4.8), (-1, 0)), (m(7.5), (0, 1)), (m(7.0), (0, -1))] {
        // Cut two units either way of the wall's face, three across; its glass a unit in behind.
        let r = (r0 + (r1 - r0) * v as f64 / h as f64) as i64;
        let (pu, pw) = (cu + du * r, cw + dw * r);
        let cut = |p: i64, d: i64| if d == 0 { (p - 3, p + 3) } else { (p - 2, p + 2) };
        let glass = |p: i64, d: i64| match d {
            1 => (p - 3, p - 2),
            -1 => (p + 2, p + 3),
            _ => (p - 3, p + 3),
        };
        let ((u0, u1), (w0, w1)) = (cut(pu, du), cut(pw, dw));
        part.cut([u0, v, w0], [u1, v + 7, w1]);
        let ((u0, u1), (w0, w1)) = (glass(pu, du), glass(pw, dw));
        part.paint([u0, v, w0], [u1, v + 7, w1], T::glass);
    }
    // The cap, its windshaft out of the front, a hub.
    let cap: Kind = if random.chance(Fx::HALF) { T::thatch.into() } else { T::roofBrown.into() };
    part.lathe([u, (h - 2) as f64, w], [u, (h + m(2.6)) as f64, w], move |t| (r1 + 4.0) * (1.0 - t) + 1.0, cap, SOLID, Op::Fill);
    let hub_v = h + m(0.9);
    let hub_w = cw - r1 as i64 - m(1.4);
    part.capsule([u, hub_v as f64, w], [u, hub_v as f64, hub_w as f64], 2.0, F::deadwood, Op::Fill);
    part.ball([u, hub_v as f64, hub_w as f64], 3.5, F::deadwood, Op::Fill);
    // Four sails in a cross, each a stock with its cloth on one side, barred across.
    let mut sails = Vec::new();
    let mut part = Part::new(s.frame(), &mut sails);
    let length = m(8.5).min(hub_v - m(1.8));
    let (w0, w1) = (hub_w - 5, hub_w - 3);
    let rect = |part: &mut Part, a: [i64; 2], b: [i64; 2], w: (i64, i64), kind: Kind, paint: bool| {
        let (min, max) = ([a[0].min(b[0]), a[1].min(b[1]), w.0], [a[0].max(b[0]), a[1].max(b[1]), w.1]);
        if paint {
            part.paint(min, max, kind);
        } else {
            part.add(min, max, kind);
        }
    };
    let (stock, cloth, bars): (Kind, Kind, Kind) = (F::deadwood.into(), K::sail.into(), T::planks.into());
    for (du, dv) in [(0, 1), (1, 0), (0, -1), (-1, 0)] {
        let (su, sv) = (dv, -du);
        let at = |along: i64, side: i64| [cu + du * along + su * side, hub_v + dv * along + sv * side];
        rect(&mut part, at(0, -1), at(length, 1), (w0 - 1, w1), stock, false);
        let (inner, outer) = (m(1.6), m(1.6) + m(1.5));
        rect(&mut part, at(inner, 1), at(length, outer), (w0, w1), cloth, false);
        let mut along = inner;
        while along < length {
            rect(&mut part, at(along, 1), at(along + 1, outer), (w0, w1), bars, true);
            along += m(1.0);
        }
        rect(&mut part, at(inner, outer - 1), at(length, outer), (w0, w1), bars, true);
    }
    Node::group(vec![Child::new("tower", ANY_SIZE, thing(shapes)), Child::new("sails", ANY_SIZE, Node::of(sails))])
}
