//! The contract every world keeps (docs/generators.md, "Checks"), to run
//! in tests: a box generated in parts, alone and in any order, equals it
//! generated whole; each level shows rays from where a camera could be
//! what the level two finer shows; seen from above, what level 0 shows;
//! nothing lies outside its heights, and things keep their reach.

use std::collections::{BTreeMap, BTreeSet};
use std::time::Instant;

use crate::World;
use crate::generator::{Generator, Layer, generate, materials_of, thing_in};
use crate::grid::{Box3, VoxelGrid, is_site};
use crate::noise::{Rng, hash_text};
use crate::picture::{Camera, FACES, first_voxel, nearest_site, picture};

fn b(min_x: i64, min_y: i64, min_z: i64, max_x: i64, max_y: i64, max_z: i64) -> Box3 {
    Box3 { min: [min_x, min_y, min_z], max: [max_x, max_y, max_z] }
}

/** The box of `level`'s sites covering `region` and a margin of two sites. */
fn box_at(r: &Box3, level: u32) -> Box3 {
    let s = 1i64 << level;
    Box3 { min: r.min.map(|v| v.div_euclid(s) - 2), max: r.max.map(|v| -(-v).div_euclid(s) + 2) }
}

fn sites(b: &Box3) -> impl Iterator<Item = (i64, i64, i64)> + '_ {
    (b.min[2]..b.max[2]).flat_map(move |z| {
        (b.min[1]..b.max[1]).flat_map(move |y| {
            let from = b.min[0] + ((b.min[0] + y + z) & 1);
            (from..b.max[0]).step_by(2).map(move |x| (x, y, z))
        })
    })
}

/**
 * How far a level may stray: from the one two finer, in the share of rays
 * through and of materials hit; from level 0, in the materials seen from
 * above (docs/generators.md).
 */
const TOLERANCE: f64 = 0.1;

/** Rays cast at each level, from as many origins. */
const RAYS: usize = 20_000;

/**
 * Levels from 1 up to this one are seen from above, at most. At level 4,
 * sites 2 m apart, a house's edges, of one kind seen from any side, grow
 * up to half a site and show walls from above, 0.11 off at the valley's
 * town's edge ("Media are isotropic" in docs/generators.md).
 */
const FROM_ABOVE: u32 = 3;

/** A box of `region` at `level` generated in eight parts, alone and in any order, equals it generated whole. */
pub fn whole_equals_parts(generator: &Generator, region: &Box3, level: u32) {
    let whole = box_at(region, level);
    let grid = generate(generator, &whole, level);
    assert!(grid.cells.iter().any(|id| *id != 0), "level {level} is empty");
    // Eight parts, split at random points (seeded, so failures reproduce).
    let mut random = Rng::new(7);
    let mut split = |axis: usize, even: bool| {
        let (lo, hi) = (whole.min[axis] as f64, whole.max[axis] as f64);
        let at = lo + (hi - lo) * (0.2 + 0.6 * random.next());
        (if even { 2.0 * (at / 2.0 + 0.5).floor() } else { (at + 0.5).floor() }) as i64
    };
    let (x, y, z) = (split(0, true), split(1, false), split(2, false));
    let mut parts = Vec::new();
    for (min_x, max_x) in [(whole.min[0], x), (x, whole.max[0])] {
        for (min_y, max_y) in [(whole.min[1], y), (y, whole.max[1])] {
            for (min_z, max_z) in [(whole.min[2], z), (z, whole.max[2])] {
                parts.push(b(min_x, min_y, min_z, max_x, max_y, max_z));
            }
        }
    }
    for part in parts.iter().rev() {
        let alone = generate(generator, part, level);
        for (x, y, z) in sites(part) {
            assert_eq!(alone.get(x, y, z), grid.get(x, y, z), "level {level}: part {part:?} differs at ({x}, {y}, {z})");
        }
    }
}

/**
 * The empty sites of a grid a camera could be at: those it could fly to
 * from above, connected to the top of the grid's box through empty sites
 * (voxels sharing a face). Under the ground's shell, inside a closed
 * house or a boulder are not; under a canopy, open to the sky somewhere,
 * is.
 */
struct Open {
    b: Box3,
    /** By point of `b` and a point more on each side, x fastest: 0 not reached yet, 1 open, 2 full or outside. */
    state: Vec<u8>,
}

impl Open {
    fn new(grid: &VoxelGrid) -> Open {
        let b = grid.clip();
        let n = [0, 1, 2].map(|i| b.max[i] - b.min[i] + 2);
        let mut state = vec![2u8; (n[0] * n[1] * n[2]) as usize];
        let at = |p: [i64; 3]| ((p[0] - b.min[0] + 1) + n[0] * ((p[1] - b.min[1] + 1) + n[1] * (p[2] - b.min[2] + 1))) as usize;
        for (x, y, z) in sites(&b) {
            if grid.get(x, y, z) == 0 {
                state[at([x, y, z])] = 0;
            }
        }
        let mut stack = Vec::new();
        for z in b.min[2]..b.max[2] {
            for x in b.min[0]..b.max[0] {
                // The top site of the column (x, z): in one of the top two layers.
                let y = b.max[1] - 1 - (x + b.max[1] - 1 + z).rem_euclid(2);
                if y >= b.min[1] && state[at([x, y, z])] == 0 {
                    state[at([x, y, z])] = 1;
                    stack.push(at([x, y, z]));
                }
            }
        }
        // Neighbours by their offset in `state`: the points outside the box are full.
        let faces = FACES.map(|f| f[0] + n[0] * (f[1] + n[1] * f[2]));
        while let Some(i) = stack.pop() {
            for face in faces {
                let q = (i as i64 + face) as usize;
                if state[q] == 0 {
                    state[q] = 1;
                    stack.push(q);
                }
            }
        }
        Open { b, state }
    }

    fn holds(&self, p: [i64; 3]) -> bool {
        let (b, n) = (&self.b, [0, 1, 2].map(|i| self.b.max[i] - self.b.min[i] + 2));
        b.holds(p)
            && is_site(p[0], p[1], p[2])
            && self.state[((p[0] - b.min[0] + 1) + n[0] * ((p[1] - b.min[1] + 1) + n[1] * (p[2] - b.min[2] + 1))) as usize] == 1
    }
}

struct Ray {
    origin: [f64; 3],
    direction: [f64; 3],
    /** How far it goes in the area it is cast in, lattice units. */
    end: f64,
}

/** RAYS rays in random directions from random sites of `area` a camera could be at in `reference` (at `level`), seeded so failures reproduce; and how many origins were tried for them. */
fn rays_from_open(area: &Box3, reference: &VoxelGrid, level: u32) -> (Vec<Ray>, usize) {
    let open = Open::new(reference);
    let s = (1i64 << level) as f64;
    let mut random = Rng::new(11);
    let (mut rays, mut tried) = (Vec::with_capacity(RAYS), 0);
    while rays.len() < RAYS && tried < 50 * RAYS {
        tried += 1;
        let origin: [f64; 3] = std::array::from_fn(|i| area.min[i] as f64 + random.next() * (area.max[i] - area.min[i]) as f64);
        let z = 2.0 * random.next() - 1.0;
        let a = 2.0 * std::f64::consts::PI * random.next();
        let direction = [(1.0 - z * z).sqrt() * a.cos(), (1.0 - z * z).sqrt() * a.sin(), z];
        if !open.holds(nearest_site(origin.map(|v| v / s))) {
            continue;
        }
        // Where it leaves the area.
        let end = (0..3)
            .map(|i| match direction[i] {
                d if d > 0.0 => (area.max[i] as f64 - origin[i]) / d,
                d if d < 0.0 => (area.min[i] as f64 - origin[i]) / d,
                _ => f64::INFINITY,
            })
            .fold(f64::INFINITY, f64::min);
        rays.push(Ray { origin, direction, end });
    }
    (rays, tried)
}

/** How far each ray goes before entering a voxel of `grid` (at `level`), lattice units, and its material; or till it leaves its area, material 0. */
fn first_hits(grid: &VoxelGrid, level: u32, rays: &[Ray]) -> Vec<(f64, u8)> {
    let s = (1i64 << level) as f64;
    rays.iter()
        .map(|ray| match first_voxel(grid, ray.origin.map(|v| v / s), ray.direction, ray.end / s) {
            Some((t, id)) => (t * s, id),
            None => (ray.end, 0),
        })
        .collect()
}

/** Half the sum of the shares' differences: how much of one is not in the other. */
fn difference(a: &BTreeMap<u8, f64>, c: &BTreeMap<u8, f64>) -> f64 {
    let keys: BTreeSet<u8> = a.keys().chain(c.keys()).copied().collect();
    keys.iter().map(|t| (a.get(t).unwrap_or(&0.0) - c.get(t).unwrap_or(&0.0)).abs()).sum::<f64>() / 2.0
}

/** Shares of `counts`' keys in their total. */
fn shares(counts: impl IntoIterator<Item = u8>) -> BTreeMap<u8, f64> {
    let mut shares = BTreeMap::new();
    let mut total = 0.0;
    for key in counts {
        *shares.entry(key).or_insert(0.0) += 1.0;
        total += 1.0;
    }
    shares.values_mut().for_each(|v| *v /= total);
    shares
}

/** Shares by material type, of shares by material id: `types` is each id's. */
fn by_type(types: &[u8], shares: &BTreeMap<u8, f64>) -> BTreeMap<u8, f64> {
    let mut by_type = BTreeMap::new();
    shares.iter().for_each(|(id, v)| *by_type.entry(types[*id as usize]).or_insert(0.0) += v);
    by_type
}

/** What a margin measures. */
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Measure {
    /** Of the rays that get 2 sites of the level far, the share that gets 4 more. */
    Through,
    /** Of those that hit something, the materials they hit. */
    Hit,
    /** What rays straight down hit first. */
    FromAbove,
}

/** How far a level of a world strays from another in a region, by one measure. */
#[derive(Clone, Debug)]
pub struct Margin {
    pub region: Box3,
    pub measure: Measure,
    pub level: u32,
    /** The level it is measured against: two finer, or 0 from above. */
    pub against: u32,
    /** How far apart: in the share through, or how much of one's materials are not in the other's. */
    pub difference: f64,
    /** The share of rays through, at `against` and at `level` (Through only). */
    pub through: [f64; 2],
    /** The share of each material id, at `against` and at `level` (Hit and FromAbove). */
    pub materials: [BTreeMap<u8, f64>; 2],
}

impl Margin {
    pub fn broken(&self) -> bool {
        self.difference >= TOLERANCE
    }
}

/** Shares by kind: its type, its index in Generator::kinds and its name. */
pub type KindShares = BTreeMap<(u8, usize, &'static str), f64>;

/**
 * A world's margins in its regions, to print: `{}` breaks down the broken
 * ones by material type and kind, `{:#}` all of them.
 */
#[derive(Clone)]
pub struct Report {
    pub name: String,
    pub margins: Vec<Margin>,
    /** Each material id's kind (index in Generator::kinds), its name and type; none for id 0. */
    materials: Vec<Option<(usize, &'static str, u8)>>,
}

impl Report {
    fn new(name: &str, generator: &Generator) -> Report {
        // Kinds have consecutive material ids, one per colour, after 0 (none): see materials_of.
        let mut materials = vec![None];
        for (i, kind) in generator.kinds.iter().enumerate() {
            materials.extend(kind.colors.iter().map(|_| Some((i, kind.name, kind.kind_type))));
        }
        Report { name: name.to_string(), margins: Vec::new(), materials }
    }

    /** Only the broken margins. */
    pub fn broken(&self) -> Report {
        Report { margins: self.margins.iter().filter(|m| m.broken()).cloned().collect(), ..self.clone() }
    }

    /** Shares by material type, and by kind, from shares by material id. */
    pub fn by_type_and_kind(&self, shares: &BTreeMap<u8, f64>) -> (BTreeMap<u8, f64>, KindShares) {
        let (mut types, mut kinds) = (BTreeMap::new(), BTreeMap::new());
        for (&id, &share) in shares {
            let Some(Some((kind, name, kind_type))) = self.materials.get(id as usize) else { continue };
            *types.entry(*kind_type).or_insert(0.0) += share;
            *kinds.entry((*kind_type, *kind, *name)).or_insert(0.0) += share;
        }
        (types, kinds)
    }

    /** The shares of a margin's materials by type, then within each by kind, largest differences first. */
    fn breakdown(&self, f: &mut std::fmt::Formatter, m: &Margin) -> std::fmt::Result {
        let [(ta, ka), (tc, kc)] = [&m.materials[0], &m.materials[1]].map(|s| self.by_type_and_kind(s));
        let get = |map: &BTreeMap<u8, f64>, t: u8| map.get(&t).copied().unwrap_or(0.0);
        let mut types: Vec<u8> = ta.keys().chain(tc.keys()).copied().collect::<BTreeSet<_>>().into_iter().collect();
        types.sort_by(|a, b| (get(&tc, *b) - get(&ta, *b)).abs().total_cmp(&(get(&tc, *a) - get(&ta, *a)).abs()));
        for t in types {
            let (a, c) = (get(&ta, t), get(&tc, t));
            if a.max(c) < 0.005 {
                continue;
            }
            let mut kinds: Vec<_> = ka.keys().chain(kc.keys()).filter(|k| k.0 == t).copied().collect::<BTreeSet<_>>().into_iter().collect();
            let kget = |map: &KindShares, k| map.get(&k).copied().unwrap_or(0.0);
            kinds.sort_by(|x, y| (kget(&kc, *y) - kget(&ka, *y)).abs().total_cmp(&(kget(&kc, *x) - kget(&ka, *x)).abs()));
            let kinds: Vec<String> = kinds
                .iter()
                .filter(|k| kget(&ka, **k).max(kget(&kc, **k)) >= 0.005)
                .map(|k| format!("{} {:.3} -> {:.3}", k.2, kget(&ka, *k), kget(&kc, *k)))
                .collect();
            writeln!(f, "      type {t:>3}: {a:.3} -> {c:.3} ({:+.3}): {}", c - a, kinds.join(", "))?;
        }
        Ok(())
    }
}

impl std::fmt::Display for Report {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        let mut region = None;
        for m in &self.margins {
            if region != Some(m.region) {
                let r = m.region;
                writeln!(f, "{} in x {}..{}, y {}..{}, z {}..{}:", self.name, r.min[0], r.max[0], r.min[1], r.max[1], r.min[2], r.max[2])?;
                region = Some(r);
            }
            let (what, values) = match m.measure {
                Measure::Through => ("rays through", format!(": {:.3} -> {:.3}", m.through[0], m.through[1])),
                Measure::Hit => ("materials hit", String::new()),
                Measure::FromAbove => ("from above", String::new()),
            };
            let broken = if m.broken() { format!(", breaks {TOLERANCE}") } else { String::new() };
            writeln!(f, "  level {} against {}, {what}{values}: {:.3} apart{broken}", m.level, m.against, m.difference)?;
            if m.measure != Measure::Through && (m.broken() || f.alternate()) {
                self.breakdown(f, m)?;
            }
        }
        Ok(())
    }
}

/**
 * What rays from where a camera could be see of `region` at each level
 * from 1 to `levels`: as many get through, and of those that hit
 * something, as many hit each material type, as at the level two finer,
 * within TOLERANCE.
 */
fn levels_against_finer(generator: &Generator, region: &Box3, levels: u32, report: &mut Report) {
    let types: Vec<u8> = materials_of(generator).iter().map(|m| m.material_type).collect();
    // Grids generated, by level and box: a level's is the reference of the level two coarser.
    let mut grids: Vec<(u32, Box3, VoxelGrid)> = Vec::new();
    let grid = |grids: &mut Vec<(u32, Box3, VoxelGrid)>, level: u32, area: &Box3| -> usize {
        let b = box_at(area, level);
        grids.iter().position(|(l, g, _)| *l == level && *g == b).unwrap_or_else(|| {
            grids.push((level, b, generate(generator, &b, level)));
            grids.len() - 1
        })
    };
    let mut origins: Option<(u32, Box3, Vec<Ray>)> = None;
    for level in 1..=levels {
        let (si, finer) = (1i64 << level, level.saturating_sub(2));
        let s = si as f64;
        // At least 16 sites of the level across, around the region's centre.
        let grow = |i: usize| 0.max(8 * si - (region.max[i] - region.min[i]) / 2);
        let area = Box3 { min: [0, 1, 2].map(|i| region.min[i] - grow(i)), max: [0, 1, 2].map(|i| region.max[i] + grow(i)) };
        let reference = grid(&mut grids, finer, &area);
        if origins.as_ref().is_none_or(|(l, a, _)| *l != finer || *a != area) {
            let (rays, tried) = rays_from_open(&area, &grids[reference].2, finer);
            assert!(rays.len() >= RAYS / 2, "level {level}: a camera could be at only {} of {tried} points", rays.len());
            origins = Some((finer, area, rays));
        }
        let rays = &origins.as_ref().expect("made").2;
        let fine = first_hits(&grids[reference].2, finer, rays);
        let coarse_grid = grid(&mut grids, level, &area);
        let coarse = first_hits(&grids[coarse_grid].2, level, rays);
        // A level is seen from afar: of the rays that got 2 of its sites far, how many get 4 more,
        // and what those that stop stop on.
        let (from, d) = (2.0 * s, 4.0 * s);
        let through = |hits: &[(f64, u8)]| {
            let seen: Vec<_> = hits.iter().filter(|(t, id)| *t > from && !(*id == 0 && *t <= from + d)).collect();
            seen.iter().filter(|(t, _)| *t > from + d).count() as f64 / seen.len() as f64
        };
        let hit = |hits: &[(f64, u8)]| shares(hits.iter().filter(|(t, id)| *t > from && *id != 0).map(|(_, id)| *id));
        let (a, c) = (hit(&fine), hit(&coarse));
        let margin = |measure, difference, through, materials| Margin {
            region: *region,
            measure,
            level,
            against: finer,
            difference,
            through,
            materials,
        };
        let through = [through(&fine), through(&coarse)];
        report.margins.push(margin(Measure::Through, (through[1] - through[0]).abs(), through, Default::default()));
        report.margins.push(margin(Measure::Hit, difference(&by_type(&types, &a), &by_type(&types, &c)), [0.0; 2], [a, c]));
        // Only the grids the coming levels read: the coarse one, and the one after it.
        grids.retain(|(l, ..)| *l + 1 >= level);
    }
}

/**
 * What a camera straight above `region` sees at each level from 1 to
 * `levels` (at most FROM_ABOVE): the materials of what each ray down
 * hits first, as at level 0, within TOLERANCE. Unlike levels two apart,
 * level 0 catches a drift, a little a level, such as walls taking over
 * from roofs.
 */
fn from_above_against_level_0(world: &World, region: &Box3, levels: u32, report: &mut Report) {
    let types: Vec<u8> = materials_of(&world.generator).iter().map(|m| m.material_type).collect();
    // A ray every 2 units.
    let width = ((region.max[0] - region.min[0]) / 2).clamp(16, 1024) as usize;
    let camera = Camera::top([region.min[0] as f64, region.min[2] as f64], [region.max[0] as f64, region.max[2] as f64], width);
    let seen = |level: u32| shares(picture(&world.generator, &camera, level, world.heights_at(level)).material);
    let reference = seen(0);
    for level in 1..=levels.min(FROM_ABOVE) {
        let here = seen(level);
        report.margins.push(Margin {
            region: *region,
            measure: Measure::FromAbove,
            level,
            against: 0,
            difference: difference(&by_type(&types, &reference), &by_type(&types, &here)),
            through: [0.0; 2],
            materials: [reference.clone(), here],
        });
    }
}

/**
 * How far each level of `world`, named `name` (with its seed, say), up
 * to `levels` strays in each of `regions` from the level two finer (rays
 * from where a camera could be) and from level 0 (from above).
 */
pub fn margins(name: &str, world: &World, regions: &[Box3], levels: u32) -> Report {
    let mut report = Report::new(name, &world.generator);
    for region in regions {
        levels_against_finer(&world.generator, region, levels, &mut report);
        from_above_against_level_0(world, region, levels, &mut report);
    }
    report
}

/** Sites of each level above and below a world's heights that `heights_hold` generates. */
const BEYOND: i64 = 16;

/** The kind of material `id` of `generator`, for messages. */
fn kind_of(generator: &Generator, id: u8) -> &'static str {
    let mut first = 1;
    for kind in &generator.kinds {
        first += kind.colors.len();
        if (id as usize) < first {
            return kind.name;
        }
    }
    "none"
}

/** Where a level `level` box of `grid` has a voxel, in lattice units, and its kind; how many it has. */
fn voxels(generator: &Generator, grid: &VoxelGrid, level: u32) -> Option<([i64; 3], &'static str, usize)> {
    let b = grid.clip();
    let full: Vec<_> = sites(&b).filter(|&(x, y, z)| grid.get(x, y, z) != 0).collect();
    let &(x, y, z) = full.first()?;
    Some(([x, y, z].map(|v| v << level), kind_of(generator, grid.get(x, y, z)), full.len()))
}

/**
 * Hosts generate a world only within its heights, so nothing may lie
 * outside them: at each level up to one past `levels`, in each region
 * and a host tile's width around spawn, BEYOND sites above and below
 * the heights are empty. Returns where they are not.
 */
pub fn heights_hold(world: &World, regions: &[Box3], levels: u32) -> Vec<String> {
    let mut broken = Vec::new();
    for level in 0..=levels + 1 {
        let (low, high) = world.heights_at(level);
        // A host's tile: 128 sites wide.
        let half = 64 << level;
        let [x, _, z] = world.spawn;
        let spawn = Box3 { min: [x - half, 0, z - half], max: [x + half, 0, z + half] };
        for (i, area) in regions.iter().chain([&spawn]).enumerate() {
            let b = box_at(area, level);
            for (from, to, side) in [(high, high + BEYOND, "above"), (low - BEYOND, low, "below")] {
                let slab = Box3 { min: [b.min[0], from, b.min[2]], max: [b.max[0], to, b.max[2]] };
                if let Some((at, kind, n)) = voxels(&world.generator, &generate(&world.generator, &slab, level), level) {
                    let place = if i == regions.len() { "around spawn".to_string() } else { format!("in {area:?}") };
                    broken.push(format!(
                        "level {level}, {place}: {n} voxels {side} the heights {:?} (World::heights), which hosts don't generate, \
                         such as {kind} at {at:?} lattice units",
                        world.heights
                    ));
                }
            }
        }
    }
    broken
}

/** Columns of cells of each population that `things_hold` looks in, at most. */
const COLUMNS: usize = 4096;
/** How far apart the columns it looks in are, at most, lattice units across (4 km); or 64 cells. */
const AROUND: i64 = 32_000;
/** Things whose bounds leave the heights that it generates, at most, of each population. */
const OUTSIDE: usize = 16;

/**
 * Things of each population of `world` in up to COLUMNS columns of cells
 * spread over a square AROUND spawn, of all heights: each keeps within
 * its population's reach (which `generate` checks only where it draws), and
 * where its bounds leave the world's heights, nothing is generated
 * outside them (at level 0). Returns where they do not.
 */
pub fn things_hold(world: &World) -> Vec<String> {
    let mut broken = Vec::new();
    let (h0, h1) = world.heights;
    for layer in &world.generator.layers {
        let Layer::Population(population) = layer else { continue };
        let (n, reach) = (population.cell, population.reach);
        let half = (AROUND / 2 / n).max(32);
        let [ci, _, ck] = world.spawn.map(|v| v.div_euclid(n));
        let side = (2 * half + 1) as usize;
        let mut random = Rng::new(hash_text(population.name, 13));
        let columns: Vec<(i64, i64)> = if side * side <= COLUMNS {
            (0..side * side).map(|c| (ci - half + (c % side) as i64, ck - half + (c / side) as i64)).collect()
        } else {
            (0..COLUMNS).map(|_| (ci - half + random.below(side as u32) as i64, ck - half + random.below(side as u32) as i64)).collect()
        };
        // Cells whose reach meets the heights.
        let (j0, j1) = ((h0 - n - reach[1]).div_euclid(n), (h1 + n + reach[1]).div_euclid(n) + 1);
        let (mut made, mut past, mut needed, mut example, mut outside) = (0, 0, reach, None, 0);
        for (i, k) in columns {
            for j in (j0..=j1).filter(|j| (i + j + k) & 1 == 0) {
                let Some((key, bounds, allowed)) = thing_in(&world.generator, population, i, j, k) else { continue };
                made += 1;
                if !bounds.inside(&allowed) {
                    past += 1;
                    let over = |a: usize| 0.max(allowed.min[a] - bounds.min[a]).max(bounds.max[a] - allowed.max[a]);
                    needed = [0, 1, 2].map(|a| needed[a].max(reach[a] + over(a)));
                    example.get_or_insert(format!("{key}: bounds {bounds:?}, reach allows {allowed:?}"));
                }
                if (bounds.min[1] < h0 || bounds.max[1] > h1) && outside < OUTSIDE {
                    outside += 1;
                    for (from, to, side) in [(h1, bounds.max[1] + 2, "above"), (bounds.min[1] - 2, h0, "below")] {
                        if from >= to {
                            continue;
                        }
                        let b = Box3 { min: [bounds.min[0] - 2, from, bounds.min[2] - 2], max: [bounds.max[0] + 2, to, bounds.max[2] + 2] };
                        if let Some((at, kind, n)) = voxels(&world.generator, &generate(&world.generator, &b, 0), 0) {
                            broken.push(format!(
                                "{key}: {n} voxels {side} the heights {:?} (World::heights), which hosts don't generate, \
                                 such as {kind} at {at:?} lattice units",
                                world.heights
                            ));
                        }
                    }
                }
            }
        }
        if past > 0 {
            broken.push(format!(
                "{past} of the {made} {} sampled reach past their population's reach {reach:?}; it needs at least {needed:?} \
                 (see \"Populations\" in docs/generators.md); such as {}",
                population.name,
                example.expect("one")
            ));
        }
    }
    broken
}

/** The height of `world`'s surface over `region`: the median of what rays straight down hit first, at level 2. */
fn surface(world: &World, region: &Box3) -> Option<f64> {
    const LEVEL: u32 = 2;
    let width = ((region.max[0] - region.min[0]) / 8).clamp(16, 256) as usize;
    let camera = Camera::top([region.min[0] as f64, region.min[2] as f64], [region.max[0] as f64, region.max[2] as f64], width);
    let heights = world.heights_at(LEVEL);
    let top = ((heights.1 + 1) << LEVEL) as f64;
    let mut hits: Vec<f64> =
        picture(&world.generator, &camera, LEVEL, heights).depth.iter().filter(|d| d.is_finite()).map(|d| top - *d as f64).collect();
    hits.sort_by(f64::total_cmp);
    hits.get(hits.len() / 2).copied()
}

/**
 * `regions`, chosen in `from`, moved up or down as far as `to`'s surface
 * over each is higher or lower: for a world of another seed, whose
 * ground lies higher or lower.
 */
pub fn follow_surface(regions: &[Box3], from: &World, to: &World) -> Vec<Box3> {
    regions
        .iter()
        .map(|r| {
            let d = match (surface(from, r), surface(to, r)) {
                (Some(a), Some(b)) => (b - a).round() as i64,
                _ => 0,
            };
            Box3 { min: [r.min[0], r.min[1] + d, r.min[2]], max: [r.max[0], r.max[1] + d, r.max[2]] }
        })
        .collect()
}

/**
 * The whole contract for `world`, named `name` (with its seed, say), in
 * each of `regions` (between them, a bit of everything in it), up to the
 * level `levels` it keeps what a camera sees to. Prints its margins (see
 * them with --nocapture); panics where it breaks it.
 */
pub fn check(name: &str, world: &World, regions: &[Box3], levels: u32) {
    let start = Instant::now();
    for region in regions {
        // Up to one level past the checked ones: where things may be drawn as media (a forest's trees).
        for level in [0, 2, levels + 1] {
            whole_equals_parts(&world.generator, region, level);
        }
    }
    println!("{name}: parts {:.1} s", start.elapsed().as_secs_f64());
    check_content(name, world, regions, levels);
}

/**
 * The contract but for parts: what a world's seed changes, its content.
 * For more seeds of a world `check` checks at one.
 */
pub fn check_content(name: &str, world: &World, regions: &[Box3], levels: u32) {
    let start = Instant::now();
    let report = margins(name, world, regions, levels);
    let rays = start.elapsed().as_secs_f64();
    let mut broken = heights_hold(world, regions, levels);
    let heights = start.elapsed().as_secs_f64() - rays;
    broken.extend(things_hold(world));
    let things = start.elapsed().as_secs_f64() - rays - heights;
    println!("{report}{name}: margins {rays:.1} s, heights {heights:.1} s, things {things:.1} s");
    let margins = report.broken();
    assert!(
        margins.margins.is_empty() && broken.is_empty(),
        "{name} breaks the contract:\n{}{}",
        if margins.margins.is_empty() { String::new() } else { margins.to_string() },
        broken.join("\n")
    );
}
