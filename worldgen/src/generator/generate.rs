//! Generating a box of a world at a level: its fields' and its
//! populations' things' drawables (forms.rs) drawn in order (drawables.rs);
//! what a box holds throughout (`whole`, `shown`).

use super::*;

#[derive(Clone, Debug, PartialEq)]
pub struct Material {
    pub color: &'static str,
    pub material_type: u8,
    /** Its kind's KindDef::walk_through. */
    pub walk_through: bool,
    /** Its kind's KindDef::glows. */
    pub glows: bool,
    /** Its kind's KindDef::medium. */
    pub medium: bool,
}

impl Material {
    /** The colour '#rrggbb' as 0xrrggbb; 0 for none (material 0, empty). */
    pub fn rgb(&self) -> u32 {
        self.color.strip_prefix('#').and_then(|hex| u32::from_str_radix(hex, 16).ok()).unwrap_or(0)
    }
}

/** Shades of the kinds' colours, as material ids picked by a hash of the position in lattice units. */
struct Palette {
    materials: Vec<Material>,
    shades: Vec<Vec<u8>>,
    /** By material, what it glows: its colour, linear, of 255; none if its kind does not glow. */
    glows: Vec<[u32; 3]>,
    seed: u64,
}

impl Palette {
    fn new(kinds: &[KindDef], seed: u64) -> Palette {
        let mut materials = vec![Material { color: "", material_type: 0, walk_through: true, glows: false, medium: false }];
        let shades = kinds
            .iter()
            .map(|k| {
                k.colors
                    .iter()
                    .map(|color| {
                        materials.push(Material {
                            color,
                            material_type: k.kind_type,
                            walk_through: k.walk_through,
                            glows: k.glows,
                            medium: k.medium,
                        });
                        (materials.len() - 1) as u8
                    })
                    .collect()
            })
            .collect();
        assert!(materials.len() <= 256, "{} materials: ids are bytes, so at most 255 and none", materials.len());
        let glows = materials
            .iter()
            .map(|m| {
                let rgb = m.rgb();
                let linear = |shift: u32| (255.0 * srgb_to_linear(((rgb >> shift) & 255) as f32 / 255.0)).round() as u32;
                if m.glows { [linear(16), linear(8), linear(0)] } else { [0; 3] }
            })
            .collect();
        Palette { materials, shades, glows, seed }
    }

    fn shade(&self, kind: Kind, x: i64, y: i64, z: i64) -> u8 {
        let shades = &self.shades[kind as usize];
        shades[((hash(self.seed, x, y, z) as u64 * shades.len() as u64) >> 32) as usize]
    }
}

/** The materials of the grids `generate` makes: ids are the same in all of them. */
pub fn materials_of(generator: &Generator) -> Vec<Material> {
    Palette::new(&generator.kinds, generator.seed).materials
}

/**
 * The voxels of `b` at `level`: sites 2^level apart, site (x, y, z) at
 * 2^level · (x, y, z) in lattice units, the box in the level's units. The
 * grid may reach one site further in x (its rows start and end on an even x).
 */
pub fn generate(generator: &Generator, b: &Box3, level: u32) -> VoxelGrid {
    let s = 1i64 << level;
    let mut grid = VoxelGrid::new(b);
    if (0..3).any(|d| b.max[d] <= b.min[d]) {
        // No sites, so nothing to make, not even things near.
        return grid;
    }
    let palette = Palette::new(&generator.kinds, generator.seed);
    let clip = grid.clip();
    let mut put = |look: Look, op: Op, share: u32, pick: u32, x: i64, y: i64, z: i64| {
        let shade = |kind: Kind| palette.shade(kind, x * s, y * s, z * s);
        let id = shade(look.kind);
        let faces = look.faces.map(|faces| faces.map(shade));
        let medium = palette.materials[id as usize].medium;
        see(&mut grid, op, share, pick, (id, faces, medium), palette.glows[id as usize], x, y, z);
    };
    let drawables = drawables_in(generator, &clip, s);
    let mut at = 0;
    while at < drawables.len() {
        // A thing's drawables, together: those of fields one by one.
        let thing = drawables[at].thing;
        let end = if thing == 0 { at + 1 } else { (at..drawables.len()).find(|i| drawables[*i].thing != thing).unwrap_or(drawables.len()) };
        let group = &drawables[at..end];
        at = end;
        if group[0].checked {
            let mut drew = None;
            for drawable in group {
                draw(drawable, generator.seed, s, &clip, &mut |_, _, share, _, x, y, z| drew = drew.or((share > 0).then_some([x, y, z])));
            }
            if let Some(site) = drew {
                broke(&group[0].key, site, s, &clip, "draws where it says it may not");
            }
            continue;
        }
        let carves = group.iter().any(|d| matches!(&d.draw, Draw::Shapes(shapes, _) if shapes.iter().any(|s| s.op == Op::Carve)));
        let mut work = Work::default();
        if !carves {
            for drawable in group {
                work.add(&draw(drawable, generator.seed, s, &clip, &mut put));
            }
        } else {
            // It cuts only itself: drawn aside, then over the world.
            let mut own = Own::new(&thing_box(group, s).intersect(&clip));
            for drawable in group {
                work.add(&draw(drawable, generator.seed, s, &clip, &mut |look, op, share, pick, x, y, z| {
                    own.put(look, op, share, pick, x, y, z)
                }));
            }
            own.each(&mut put);
        }
        if thing != 0 {
            work.largest = work.tests;
        }
        cost::add(group[0].layer, work);
    }
    // One coin per site: its pieces shown as often as they stop rays.
    let coins = hash_text("shown", generator.seed);
    grid.show(|x, y, z| ((hash(coins, x * s, y * s, z * s) as u64 * ALPHA as u64) >> 32) as u8);
    grid
}

/**
 * Adds a thing's `share` of site (x, y, z) to it (VoxelGrid's `pieces`,
 * `medium`, `glow`), of material `id` (its faces' if they differ) glowing
 * `glow`: as a Fill behind what is there, a Paint in front of it, a Carve
 * taking it out, of both. A `medium` joins the site's medium, the rest
 * its pieces. What either shows is one of what is in it, each as often
 * as it is seen there: the thing replaces it as often as it makes up of
 * what they now stop, by its pick.
 */
#[allow(clippy::too_many_arguments)]
fn see(
    grid: &mut VoxelGrid,
    op: Op,
    share: u32,
    pick: u32,
    (id, faces, medium): (u8, Option<Faces>, bool),
    glow: [u32; 3],
    x: i64,
    y: i64,
    z: i64,
) {
    if share == 0 {
        return;
    }
    let Some(i) = grid.index(x, y, z) else { return };
    let part = if medium { &mut grid.medium } else { &mut grid.pieces };
    let (alpha, share) = (part.alpha[i] as u64, share.min(SHARE) as u64);
    let (all, one) = (ALPHA as u64, SHARE as u64);
    let old = grid.glow.get(&i).copied().unwrap_or_default();
    // Of SHARE: how much of what was there is still seen, and how much of the site the thing is seen in.
    let (kept, visible) = match op {
        Op::Carve => (one - share, 0),
        Op::Fill => (one, (all - alpha) * share / all),
        Op::Paint => (one - share, share),
    };
    // In SHARE-ths of a step of alpha, rounded up as often as the remainder,
    // by a coin from the pick (its bits scrambled by an odd factor):
    // cut off, all of a speck's share under one step was lost far away.
    let coin = (pick as u64 * 40_503) % one;
    let rounded = |exact: u64| (exact / one + u64::from(exact % one > coin)) as u8;
    let exact = alpha * one + (all - alpha) * share;
    if op == Op::Carve {
        for part in [&mut grid.pieces, &mut grid.medium] {
            part.alpha[i] = rounded(part.alpha[i] as u64 * kept);
        }
    } else {
        part.alpha[i] = rounded(exact);
    }
    // As often as it makes up of what the site stops before that rounding: of `now`, a medium of a few
    // SHARE-ths of a step was seen as seldom as rounded up, and nine in ten of its sites that stop rays showed nothing.
    if (pick as u64) * exact < visible * all * one {
        match medium {
            true => grid.medium.seen[i] = id,
            false => grid.see(i, id, faces),
        }
    }
    if old == [0; 3] && glow == [0; 3] {
        return;
    }
    let glow = [0, 1, 2].map(|c| (old[c] as u64 * kept / one + visible * glow[c] as u64) as u32);
    if glow != [0; 3] {
        grid.glow.insert(i, glow);
    } else if old != [0; 3] {
        grid.glow.remove(&i);
    }
}

/** The sites of level spacing `s` a thing's drawables may draw at: its shapes' bounds and two sites more, and its measured sites. */
fn thing_box(group: &[Drawable], s: i64) -> Box3 {
    let boxes: Vec<Box3> = group
        .iter()
        .flat_map(|d| match &d.draw {
            Draw::Shapes(shapes, _) => shapes
                .iter()
                .map(|shape| Box3 {
                    min: shape.bounds.min.map(|v| v.div_euclid(s) - 2),
                    max: shape.bounds.max.map(|v| -(-v).div_euclid(s) + 2),
                })
                .collect(),
            Draw::Sites(sites) => sites.iter().map(|(p, ..)| Box3 { min: *p, max: p.map(|v| v + 1) }).collect(),
            Draw::Field(_) => Vec::new(),
        })
        .collect();
    bounds_of_shapes(&boxes)
}

/** What draws into `clip` (sites s apart), in drawing order: by layer, then key. */
fn drawables_in<'a>(generator: &'a Generator, clip: &Box3, s: i64) -> Vec<Drawable<'a>> {
    // The lattice units the sites of `clip` cover, with what a shape may grow by at this level.
    let sites = units_of(clip, s);
    let units = sites.widen(s);
    let mut drawables = Vec::new();
    let mut things = 0;
    for layer in &generator.layers {
        let population = match layer {
            Layer::Field(field) => {
                // Asked where it may draw line by line (draw_field).
                let (order, key, layer) = (field.order, field.name.to_string(), field.name);
                drawables.push(Drawable { order, key, layer, thing: 0, draw: Draw::Field(field), checked: false });
                continue;
            }
            Layer::Population(p) => p,
        };
        let checked = !stands_within(population, &sites);
        if checked && !checking() {
            continue;
        }
        if s > population.size {
            if let Some(far) = &population.far {
                let (order, layer) = (population.order, population.name);
                let (key, draw) = (population.name.to_string(), Draw::Field(far));
                drawables.push(Drawable { order, key, layer, thing: 0, draw, checked });
            }
            continue;
        }
        let n = population.cell;
        // Cells whose reach meets `units`: centres within N + reach of it.
        let cells = [0, 1, 2].map(|axis| {
            let reach = population.reach[axis];
            (-(-(units.min[axis] - n - reach)).div_euclid(n), (units.max[axis] - 1 + n + reach).div_euclid(n))
        });
        // Its things read the ground on the level's sites, as its fields do (Sloped::at).
        let work = plan::footprint(s, [0, 0], || {
            each_thing(generator, population, cells, |cell, _, node| {
                let (key, bounds, reach) = (cell.key(), bounds_of(&node), reach_of(population, cell));
                assert!(bounds.inside(&reach), "{}", past_reach(&key, &bounds, &reach, n, population.reach));
                things += 1;
                let (order, layer, seed) = (population.order, population.name, generator.seed);
                let at = Visit { s, units: &units, order, layer, thing: things, seed, checked };
                visit(&node, key, bounds, &at, &mut drawables);
            })
        });
        cost::add(population.name, work);
    }
    drawables.sort_by(|a, b| a.order.total_cmp(&b.order).then_with(|| a.key.cmp(&b.key)));
    drawables
}

/** Why a thing within `bounds` is past its cell's `reach` (N and the population's reach past its centre): along which axes, by how much, what would hold it. */
fn past_reach(key: &str, bounds: &Box3, reach: &Box3, n: i64, population: [i64; 3]) -> String {
    let centre = [0, 1, 2].map(|i| (reach.min[i] + reach.max[i]) / 2);
    let needed = [0, 1, 2].map(|i| population[i].max(centre[i] - n - bounds.min[i]).max(bounds.max[i] - centre[i] - n));
    let axes: Vec<String> = (0..3)
        .filter(|i| needed[*i] > population[*i])
        .map(|i| format!("{} by {}", ["x", "y", "z"][i], needed[i] - population[i]))
        .collect();
    format!(
        "{key} reaches past its population's reach along {}: its bounds are {bounds:?}, its cell's {reach:?} \
         (N = {n} and reach {population:?} from its centre); reach {needed:?} would hold this one, \
         but derive it from the largest possible thing; see \"Populations\" in {DOCS}",
        axes.join(", ")
    )
}

/**
 * Whether a population's things may draw in `b` (lattice units): anywhere,
 * unless they stand on a plan of heights (Population::on): each within N
 * and its reach of its cell's centre, its anchor in the cell and at most
 * N off the plan, so within 2N and its reach of where it stands.
 */
fn stands_within(population: &Population, b: &Box3) -> bool {
    let Some(ground) = &population.on else { return true };
    let (n, [rx, ry, rz]) = (population.cell, population.reach);
    let (around, along) = (2 * n + rx.max(rz), 3 * n + ry);
    let Some((lo, hi)) = ground.range([b.min[0] - around, b.min[2] - around], [b.max[0] + around, b.max[2] + around]) else { return true };
    b.min[1] <= hi + along && b.max[1] > lo - along
}

/** What a box holds throughout, if one thing (`whole`). */
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Whole {
    /** Nothing. */
    Empty,
    /** Solid, and the sites around it too: none of its voxels shows. */
    Buried,
}

/**
 * What box `b` of `level` holds throughout (sites as `generate`'s), as
 * its layers say (About, Population::on): nothing, or solid wherever a ray
 * could see it from. The last field drawing all of it decides; a layer
 * drawing in it after keeps only that: a carve what is empty, a fill or
 * paint what is buried (as a thing's: its carves cut only itself). None
 * if not so.
 */
pub fn whole(generator: &Generator, b: &Box3, level: u32) -> Option<Whole> {
    let s = 1i64 << level;
    let units = units_of(&b.widen(1), s);
    let (min, max, ys) = ([units.min[0], units.min[2]], [units.max[0] - 1, units.max[2] - 1], [units.min[1], units.max[1] - 1]);
    // What each draws in it, and whether that carves, in the order they draw (drawables_in).
    let mut layers: Vec<(f64, &str, Meets, bool)> = generator
        .layers
        .iter()
        .map(|layer| match layer {
            Layer::Field(f) => (f.order, f.name, f.about.as_ref().map_or(Meets::Any, |a| a.meets(min, max, ys, s)), f.op == Op::Carve),
            Layer::Population(p) => {
                let meets = if stands_within(p, &units) { Meets::Any } else { Meets::Nothing };
                (p.order, p.name, meets, p.far.as_ref().is_some_and(|far| far.op == Op::Carve))
            }
        })
        .collect();
    layers.sort_by(|a, b| a.0.total_cmp(&b.0).then_with(|| a.1.cmp(b.1)));
    let kept = |carves: bool| if carves { Whole::Empty } else { Whole::Buried };
    let last = layers.iter().rposition(|&(.., meets, _)| matches!(meets, Meets::All(kind) if !generator.kinds[kind as usize].medium));
    let whole = last.map_or(Whole::Empty, |i| kept(layers[i].3));
    let after = &layers[last.map_or(0, |i| i + 1)..];
    after.iter().all(|&(.., meets, carves)| meets == Meets::Nothing || kept(carves) == whole).then_some(whole)
}

/**
 * Box `b` of `level` less its bottom and its top that are `whole`, in
 * bricks up from its bottom: the part of it whose voxels may show, all a
 * host need generate. What is whole of a box is of every part of it, so
 * each end is found by halving.
 */
pub fn shown(generator: &Generator, b: &Box3, level: u32) -> Box3 {
    let (bottom, top) = (b.min[1], b.max[1]);
    let at = |k: i64| (bottom + k * BRICK).min(top);
    let whole_in = |y0: i64, y1: i64| {
        y0 >= y1 || whole(generator, &Box3 { min: [b.min[0], y0, b.min[2]], max: [b.max[0], y1, b.max[2]] }, level).is_some()
    };
    let bricks: Vec<i64> = (0..=(top - bottom + BRICK - 1).div_euclid(BRICK).max(0)).collect();
    let low = bricks[1..].partition_point(|&k| whole_in(bottom, at(k)));
    let high = low + bricks[low..].partition_point(|&k| !whole_in(at(k), top));
    Box3 { min: [b.min[0], at(low as i64), b.min[2]], max: [b.max[0], at(high as i64), b.max[2]] }
}

/**
 * Box `b` of `level` as hosts make it: what may show of it (`shown`)
 * generated, its buried bricks kept as one (VoxelGrid::bury). Not
 * `generate`'s grid, so not for checking a box made whole against its parts.
 */
pub fn generate_shown(generator: &Generator, b: &Box3, level: u32) -> VoxelGrid {
    let mut grid = generate(generator, &shown(generator, b, level), level);
    grid.bury();
    grid
}
