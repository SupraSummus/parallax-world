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
    /** By material, its colour, linear, of COLOUR_ONE. */
    colours: Vec<[u32; 3]>,
    /** By kind, the mean of its shades' colours: what a site of a coarser level holds of it. */
    means: Vec<[u32; 3]>,
    /** By material, what it glows: its colour, linear, of 255; none if its kind does not glow. */
    glows: Vec<[u32; 3]>,
    seed: u64,
}

impl Palette {
    fn new(kinds: &[KindDef], seed: u64) -> Palette {
        let mut materials = vec![Material { color: "", material_type: 0, walk_through: true, glows: false, medium: false }];
        let shades: Vec<Vec<u8>> = kinds
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
        let linear = |m: &Material, of: f32| {
            let rgb = m.rgb();
            [16, 8, 0].map(|shift| (of * srgb_to_linear(((rgb >> shift) & 255) as f32 / 255.0)).round() as u32)
        };
        let colours: Vec<[u32; 3]> = materials.iter().map(|m| linear(m, COLOUR_ONE as f32)).collect();
        let means = shades
            .iter()
            .map(|ids| {
                [0, 1, 2].map(|c| (ids.iter().map(|&id| colours[id as usize][c]).sum::<u32>() + ids.len() as u32 / 2) / ids.len() as u32)
            })
            .collect();
        let glows = materials.iter().map(|m| if m.glows { linear(m, 255.0) } else { [0; 3] }).collect();
        Palette { materials, shades, colours, means, glows, seed }
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
        // A site of level 0 is one of its kind's shades; a coarser one, all of them.
        let colour = |kind: Kind, id: u8| if s == 1 { palette.colours[id as usize] } else { palette.means[kind as usize] };
        let colours = match (look.faces, faces) {
            (Some(kinds), Some(ids)) => std::array::from_fn(|k| colour(kinds[k], ids[k])),
            _ => [colour(look.kind, id); FACE_KINDS],
        };
        let (medium, glow) = (palette.materials[id as usize].medium, palette.glows[id as usize]);
        let added = Added { op, pick, id, faces, colours, glow, cover: look.cover, medium };
        see(&mut grid, &added, share, x, y, z);
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
 * `medium`, `glow`): as a Fill behind what is there, a Paint in front of
 * it, a Carve taking it out, of both. A medium joins the site's medium,
 * the rest its pieces. What either stops, shows and glows is the sum of
 * what is in it, each by how much of the site's rays it stops there.
 * Pieces covering some faces' rays more than others' (`cover`), unlike
 * through some faces, or put where some did, are added face by face
 * (VoxelGrid's `views`); the site as a whole, to what sees no faces (a
 * host's voxels, a camera colliding), stops and glows as its most.
 * Hosts show a site as one of the materials in it (`seen`), each as
 * often as it is seen there: the thing replaces it as often as it makes
 * up of what they now stop, by its pick.
 */
fn see(grid: &mut VoxelGrid, added: &Added, share: u32, x: i64, y: i64, z: i64) {
    if share == 0 && added.cover.is_none_or(|cover| cover == [0; FACE_KINDS]) {
        return;
    }
    let Some(i) = grid.index(x, y, z) else { return };
    // A carve takes out pieces and medium alike: the medium here, the pieces as any.
    let medium = added.medium && added.op != Op::Carve;
    let old = grid.glow.get(&i).copied().unwrap_or_default();
    if added.op == Op::Carve {
        let kept = (SHARE - share.min(SHARE)) as u64;
        grid.medium.alpha[i] = added.rounded(grid.medium.alpha[i] as u64 * kept);
        grid.medium.colour[i] = grid.medium.colour[i].map(|c| (c as u64 * kept / SHARE as u64) as u16);
    }
    let by_face = !medium && (added.cover.is_some() || added.faces.is_some() || grid.views.contains_key(&i));
    if !by_face {
        let part = if medium { &mut grid.medium } else { &mut grid.pieces };
        let (alpha, colour, glow, seen) = added.to((part.alpha[i], part.colour[i], old), share, 0);
        (part.alpha[i], part.colour[i]) = (alpha, colour);
        if seen {
            match medium {
                true => grid.medium.seen[i] = added.id,
                false => grid.see(i, added.id, added.faces),
            }
        }
        set_glow(grid, i, glow);
        return;
    }
    // Face by face, from what each saw so far; the site as a whole shows what the share as a whole would.
    let (was, was_colour, was_seen) = (grid.pieces.alpha[i], grid.pieces.colour[i], grid.pieces.seen[i]);
    let mut views = grid.views.remove(&i).unwrap_or(crate::grid::Views {
        alpha: [was; FACE_KINDS],
        colour: [was_colour; FACE_KINDS],
        glow: [old; FACE_KINDS],
    });
    let mut shown = grid.faces.get(&i).copied().unwrap_or([was_seen; FACE_KINDS]);
    for k in 0..FACE_KINDS {
        let (alpha, colour, glow, seen) =
            added.to((views.alpha[k], views.colour[k], views.glow[k]), added.cover.map_or(share, |cover| cover[k]), k);
        (views.alpha[k], views.colour[k], views.glow[k]) = (alpha, colour, glow);
        if seen {
            shown[k] = added.faces.map_or(added.id, |faces| faces[k]);
        }
    }
    fn most<T: Ord + Copy>(of: &[[T; 3]; FACE_KINDS]) -> [T; 3] {
        [0, 1, 2].map(|c| of.iter().map(|v| v[c]).max().expect("faces"))
    }
    grid.pieces.alpha[i] = views.alpha.into_iter().max().expect("faces");
    grid.pieces.colour[i] = most(&views.colour);
    let (_, _, _, seen) = added.to((was, was_colour, old), share, 0);
    grid.see(i, if seen { added.id } else { was_seen }, Some(shown));
    let alike = |k: usize| views.alpha[k] == views.alpha[0] && views.colour[k] == views.colour[0] && views.glow[k] == views.glow[0];
    if (0..FACE_KINDS).all(alike) {
        (grid.pieces.alpha[i], grid.pieces.colour[i]) = (views.alpha[0], views.colour[0]);
        set_glow(grid, i, views.glow[0]);
    } else {
        set_glow(grid, i, most(&views.glow));
        grid.views.insert(i, views);
    }
}

/** What `see` adds to a site: a thing's material (its faces' if they differ), its colour through each face, what it glows. */
struct Added {
    op: Op,
    pick: u32,
    id: u8,
    faces: Option<Faces>,
    /** Linear, of COLOUR_ONE, through each face. */
    colours: [[u32; 3]; FACE_KINDS],
    glow: [u32; 3],
    /** How much of each face's rays it covers, if not its share. */
    cover: Option<[u32; FACE_KINDS]>,
    medium: bool,
}

impl Added {
    /**
     * Of what a site (or a face of it) stops, shows and glows, `share` of
     * it (of SHARE) added, seen through face `k`: what it stops, shows and
     * glows after, and whether it is seen.
     */
    fn to(&self, (alpha, colour, glow): (u8, [u16; 3], [u32; 3]), share: u32, k: usize) -> (u8, [u16; 3], [u32; 3], bool) {
        let (alpha, share) = (alpha as u64, share.min(SHARE) as u64);
        let (all, one) = (ALPHA as u64, SHARE as u64);
        // Of SHARE: how much of what was there is still seen, and how much of the site the thing is seen in.
        let (kept, visible) = match self.op {
            Op::Carve => (one - share, 0),
            Op::Fill => (one, (all - alpha) * share / all),
            Op::Paint => (one - share, share),
        };
        let exact = alpha * one + (all - alpha) * share;
        let after = if self.op == Op::Carve { self.rounded(alpha * kept) } else { self.rounded(exact) };
        // As often as it makes up of what the site stops before that rounding: of `now`, a medium of a few
        // SHARE-ths of a step was seen as seldom as rounded up, and nine in ten of its sites that stop rays showed nothing.
        let seen = (self.pick as u64) * exact < visible * all * one;
        let colour =
            [0, 1, 2].map(|c| ((colour[c] as u64 * kept + visible * self.colours[k][c] as u64) / one).min(COLOUR_ONE as u64) as u16);
        let glow = [0, 1, 2].map(|c| (glow[c] as u64 * kept / one + visible * self.glow[c] as u64) as u32);
        (after, colour, glow, seen)
    }

    /**
     * `exact`, in SHARE-ths of a step of alpha, rounded up as often as the
     * remainder, by a coin from the pick (its bits scrambled by an odd
     * factor): cut off, all of a speck's share under one step was lost far away.
     */
    fn rounded(&self, exact: u64) -> u8 {
        let one = SHARE as u64;
        let coin = (self.pick as u64 * 40_503) % one;
        (exact / one + u64::from(exact % one > coin)) as u8
    }
}

/** Sets site `i`'s glow, none if it glows nothing. */
fn set_glow(grid: &mut VoxelGrid, i: usize, glow: [u32; 3]) {
    if glow != [0; 3] {
        grid.glow.insert(i, glow);
    } else if !grid.glow.is_empty() {
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
        let checked = !stands_within(population, &sites, s);
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
        let (n, cells) = (population.cell, cells_reaching(population, &units));
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
 * Whether a population's things may draw in `b` (lattice units) at sites
 * `s` apart, as it says where they stand (Population::stands): each within
 * N and its reach of its cell's centre, its anchor in the cell, so within
 * 2N and its reach of its anchor; and two sites more, as fields are asked
 * (Field::meets): an anchor stands on what the level's sites read.
 */
fn stands_within(population: &Population, b: &Box3, s: i64) -> bool {
    let Some(stands) = &population.stands else { return true };
    let around = population.reach.map(|r| 2 * population.cell + r + 2 * s);
    stands(&Box3 { min: [0, 1, 2].map(|i| b.min[i] - around[i]), max: [0, 1, 2].map(|i| b.max[i] + around[i]) })
}

/** What a box holds throughout, if one thing (`whole`). */
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Whole {
    /** Nothing. */
    Empty,
    /** Solid, and the sites around it too: none of its voxels shows. Of the kind the last field drawing all of it fills. */
    Buried(Kind),
}

/**
 * What box `b` of `level` holds throughout (sites as `generate`'s), as
 * its fields say (Field::meets) and its things are: nothing, or solid
 * wherever a ray could see it from; None if not so. Its layers, in the
 * order they draw, from nothing: one drawing all of it solid makes it
 * that (a carve, empty); one that may draw in it keeps only what it can't
 * change, a carve what is empty, a fill or paint what is buried (as a
 * thing's do: its carves cut only itself); one drawing nothing in it
 * keeps what it is.
 */
pub fn whole(generator: &Generator, b: &Box3, level: u32) -> Option<Whole> {
    let s = 1i64 << level;
    let units = units_of(&b.widen(1), s);
    let mut layers: Vec<&Layer> = generator.layers.iter().collect();
    let key = |layer: &Layer| match layer {
        Layer::Field(f) => (f.order, f.name),
        Layer::Population(p) => (p.order, p.name),
    };
    layers.sort_by(|a, b| key(a).0.total_cmp(&key(b).0).then_with(|| key(a).1.cmp(key(b).1)));
    // What a layer draws in it, and whether that carves; a population's things, which cost the most to ask, as if
    // drawing nothing unless `things`.
    let meets = |layer: &Layer, things: bool| match layer {
        Layer::Field(f) => (f.meets_in(&units, s), f.op == Op::Carve),
        Layer::Population(p) if !stands_within(p, &units, s) => (Meets::Nothing, false),
        Layer::Population(p) if s > p.size => {
            p.far.as_ref().map_or((Meets::Nothing, false), |far| (far.meets_in(&units, s), far.op == Op::Carve))
        }
        // As drawables_in makes them, with what a shape may grow by at this level.
        Layer::Population(p) if things && plan::footprint(s, [0, 0], || reaches(generator, p, &units.widen(2 * s))) => (Meets::Any, false),
        Layer::Population(_) => (Meets::Nothing, false),
    };
    let fold = |things: bool| {
        let mut holds = Some(Whole::Empty);
        for layer in &layers {
            // Things are asked only of what is whole: of what isn't, they change nothing.
            holds = match (meets(layer, things && holds.is_some()), holds) {
                ((Meets::Nothing, _), holds) => holds,
                ((Meets::All(kind), carves), _) if !generator.kinds[kind as usize].medium => {
                    Some(if carves { Whole::Empty } else { Whole::Buried(kind) })
                }
                ((_, true), Some(Whole::Empty)) => Some(Whole::Empty),
                ((_, false), Some(Whole::Buried(kind))) => Some(Whole::Buried(kind)),
                _ => None,
            };
        }
        holds
    };
    // Things only take away from what the rest makes: asked only if that alone is whole.
    fold(false)?;
    fold(true)
}

/**
 * Box `b` of `level` less its bottom and its top that are `whole`, in
 * bricks up from its bottom: the part of it whose voxels may show, all a
 * host need generate; none at its top if all of it is buried, at its
 * bottom if empty. What is whole of a box is of every part of it, so
 * each end is found by halving.
 */
pub fn shown(generator: &Generator, b: &Box3, level: u32) -> Box3 {
    let (bottom, top) = (b.min[1], b.max[1]);
    match whole(generator, b, level) {
        Some(Whole::Buried(_)) => return Box3 { min: [b.min[0], top, b.min[2]], ..*b },
        Some(Whole::Empty) => return Box3 { max: [b.max[0], bottom, b.max[2]], ..*b },
        None => {}
    }
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
 * generated, on what is buried under it, not generated but kept as rock
 * (VoxelGrid::on_buried), its buried bricks kept as one (VoxelGrid::bury).
 * Not `generate`'s grid, so not for checking a box made whole against
 * its parts.
 */
pub fn generate_shown(generator: &Generator, b: &Box3, level: u32) -> VoxelGrid {
    let part = shown(generator, b, level);
    let mut grid = generate(generator, &part, level);
    let under = Box3 { max: [b.max[0], part.min[1], b.max[2]], ..*b };
    if under.min[1] < under.max[1]
        && let Some(Whole::Buried(kind)) = whole(generator, &under, level)
    {
        let s = 1i64 << level;
        let palette = Palette::new(&generator.kinds, generator.seed);
        let id = palette.shade(kind, b.min[0] * s, b.min[1] * s, b.min[2] * s);
        let colour = if s == 1 { palette.colours[id as usize] } else { palette.means[kind as usize] };
        grid = grid.on_buried(&under, id, colour.map(|c| c as u16));
    }
    grid.bury();
    grid
}
