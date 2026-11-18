//! Generating a box of a world at a level: its fields' and its
//! populations' things' drawables (forms.rs) drawn in order (drawables.rs);
//! what a box holds throughout (`whole`, `shown`).

use super::*;

/** The kinds' colours: of each, its shades, and their mean. */
struct Palette {
    /** By kind, its shades' colours, linear, of COLOUR_ONE. */
    shades: Vec<Vec<[u32; 3]>>,
    /** By kind, the mean of its shades: what a site of a coarser level holds of it. */
    means: Vec<[u32; 3]>,
    seed: u64,
}

impl Palette {
    fn new(kinds: &[KindDef], seed: u64) -> Palette {
        let linear = |color: &str| {
            let rgb = color.strip_prefix('#').and_then(|hex| u32::from_str_radix(hex, 16).ok()).expect("a colour #rrggbb");
            [16, 8, 0].map(|shift| (COLOUR_ONE as f32 * srgb_to_linear(((rgb >> shift) & 255) as f32 / 255.0)).round() as u32)
        };
        let shades: Vec<Vec<[u32; 3]>> = kinds.iter().map(|k| k.colors.iter().map(|c| linear(c)).collect()).collect();
        let means = shades
            .iter()
            .map(|of| [0, 1, 2].map(|c| (of.iter().map(|s| s[c]).sum::<u32>() + of.len() as u32 / 2) / of.len() as u32))
            .collect();
        Palette { shades, means, seed }
    }

    /**
     * What a site of `kind` at (x, y, z) (lattice units) of a level of
     * spacing `s` holds of it: at level 0 one of its shades, by a hash of
     * the position; coarser, their mean.
     */
    fn colour(&self, kind: Kind, s: i64, [x, y, z]: [i64; 3]) -> [u32; 3] {
        let shades = &self.shades[kind as usize];
        match s {
            1 => shades[((hash(self.seed, x, y, z) as u64 * shades.len() as u64) >> 32) as usize],
            _ => self.means[kind as usize],
        }
    }
}

/**
 * The kind a look is the colour of: one of its shades, as a site of level
 * 0 of it alone, or their mean, as a coarser one, to within a few steps
 * (what stops few rays is unpremultiplied from little); None if none's,
 * as where kinds mix. For messages, tests and stats: grids hold what
 * sites look like, not what they are made of.
 */
pub fn kind_of<'a>(generator: &'a Generator) -> impl Fn(fcc::Look) -> Option<&'a KindDef> + 'a {
    let palette = Palette::new(&generator.kinds, generator.seed);
    let rgb = |colour: &[u32; 3]| look_of(colour.map(|c| c as u16), ALPHA, ALPHA).to_le_bytes();
    let colours: Vec<(usize, [u8; 4])> = (0..generator.kinds.len())
        .flat_map(|k| palette.shades[k].iter().chain([&palette.means[k]]).map(move |c| (k, c)).collect::<Vec<_>>())
        .map(|(k, c)| (k, rgb(c)))
        .collect();
    let of = move |look: fcc::Look| {
        let at = look.to_le_bytes();
        let far = |c: &[u8; 4]| (0..3).map(|i| c[i].abs_diff(at[i])).max().expect("channels");
        let (k, c) = colours.iter().min_by_key(|(_, c)| far(c))?;
        (look != 0 && far(c) <= KIND_STEPS).then_some(*k)
    };
    // Many sites share a look: each is matched once.
    let known = RefCell::new(std::collections::HashMap::new());
    move |look| known.borrow_mut().entry(look).or_insert_with(|| of(look)).map(|k| &generator.kinds[k])
}

/** How many steps of sRGB a look may be from a kind's colour and still be of it (`kind_of`). */
const KIND_STEPS: u8 = 3;

/**
 * The voxels of `b` at `level`: sites 2^level apart, site (x, y, z) at
 * 2^level · (x, y, z) in lattice units, the box in the level's units. The
 * grid may reach one site further in x (its rows start and end on an even x).
 */
pub fn generate(generator: &Generator, b: &Box3, level: u32) -> VoxelGrid {
    made(generator, b, level, None)
}

/** `generate`, nothing drawn under `floors`, if any: what is buried there is not made. */
fn made(generator: &Generator, b: &Box3, level: u32, floors: Option<&Floors>) -> VoxelGrid {
    let s = 1i64 << level;
    let mut grid = VoxelGrid::new(b);
    if (0..3).any(|d| b.max[d] <= b.min[d]) {
        // No sites, so nothing to make, not even things near.
        return grid;
    }
    let palette = Palette::new(&generator.kinds, generator.seed);
    let clip = grid.clip();
    let mut odd = Vec::new();
    let mut put = |look: Look, op: Op, share: u32, pick: u32, x: i64, y: i64, z: i64| {
        // Things and lines across y are drawn whole and kept from the floors up.
        if floors.is_some_and(|floors| y < floors.at(x, z)) {
            return;
        }
        let colour = |kind: Kind| palette.colour(kind, s, [x, y, z].map(|v| v * s));
        let kind = &generator.kinds[look.kind as usize];
        if !see_whole(&mut grid, &odd, &look, kind, op, share, [x, y, z], || colour(look.kind)) {
            see(&mut grid, &mut odd, &Added::of(&look, kind, op, pick, colour), share, x, y, z);
        }
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
                draw(drawable, generator.seed, s, &clip, None, &mut |_, _, share, _, x, y, z| {
                    drew = drew.or((share > 0).then_some([x, y, z]))
                });
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
                work.add(&draw(drawable, generator.seed, s, &clip, floors, &mut put));
            }
        } else {
            // It cuts only itself: drawn aside, then over the world.
            let mut own = Own::new(&thing_box(group, s).intersect(&clip));
            for drawable in group {
                work.add(&draw(drawable, generator.seed, s, &clip, floors, &mut |look, op, share, pick, x, y, z| {
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
 * `medium`, `glow`): as a Fill behind what is there, apart from it; a
 * Paint in front of it, lying on it, and past it only where it covers
 * more; a Carve taking it out, of both. A medium joins the site's medium,
 * the rest its pieces. What either stops, shows and glows is the sum of
 * what is in it, each by how much of the site's rays it stops there.
 * Pieces covering some faces' rays more than others' (`cover`), unlike
 * through some faces, or put where some did, are added face by face
 * (VoxelGrid's `views`); the site as a whole, to what sees no faces (a
 * host's voxels, a camera colliding), stops and glows as its most.
 * What stops a walking body adds its share to the site's `solid`,
 * wherever in it, in front or behind; a carve takes out of it too.
 * `odd` marks, by index in `shown`, the sites that may be in `glow` or
 * `views`: few are, so most are added to without a lookup.
 */
fn see(grid: &mut VoxelGrid, odd: &mut Vec<bool>, added: &Added, share: u32, x: i64, y: i64, z: i64) {
    if share == 0 && added.cover.is_none_or(|cover| cover == [0; FACE_KINDS]) {
        return;
    }
    let Some(i) = grid.index(x, y, z) else { return };
    if odd.len() <= i {
        odd.resize(grid.shown.len(), false);
    }
    let (solid, all, one) = (grid.solid[i] as u64, ALPHA as u64, SHARE as u64);
    let share64 = share.min(SHARE) as u64;
    grid.solid[i] = match added.op {
        Op::Carve => added.rounded(solid * (one - share64)),
        _ if added.solid => added.rounded(solid * one + (all - solid) * share64),
        _ => solid as u8,
    };
    // A carve takes out pieces and medium alike: the medium here, the pieces as any.
    let medium = added.medium && added.op != Op::Carve;
    let old = if odd[i] { grid.glow.get(&i).copied().unwrap_or_default() } else { [0; 3] };
    if added.op == Op::Carve {
        let kept = (SHARE - share.min(SHARE)) as u64;
        grid.medium.alpha[i] = added.rounded(grid.medium.alpha[i] as u64 * kept);
        grid.medium.colour[i] = grid.medium.colour[i].map(|c| (c as u64 * kept / SHARE as u64) as u16);
    }
    let by_face = !medium && (added.cover.is_some() || added.faces.is_some() || odd[i] && grid.views.contains_key(&i));
    if !by_face {
        let part = if medium { &mut grid.medium } else { &mut grid.pieces };
        let (alpha, colour, glow) = added.to((part.alpha[i], part.colour[i], old), share, 0);
        (part.alpha[i], part.colour[i]) = (alpha, colour);
        set_glow(grid, odd, i, glow);
        return;
    }
    // Face by face, from what each saw so far; the site as a whole stops and glows as its most.
    odd[i] = true;
    let (was, was_colour) = (grid.pieces.alpha[i], grid.pieces.colour[i]);
    let mut views = grid.views.remove(&i).unwrap_or(crate::grid::Views {
        alpha: [was; FACE_KINDS],
        colour: [was_colour; FACE_KINDS],
        glow: [old; FACE_KINDS],
    });
    for k in 0..FACE_KINDS {
        let (alpha, colour, glow) =
            added.to((views.alpha[k], views.colour[k], views.glow[k]), added.cover.map_or(share, |cover| cover[k]), k);
        (views.alpha[k], views.colour[k], views.glow[k]) = (alpha, colour, glow);
    }
    fn most<T: Ord + Copy>(of: &[[T; 3]; FACE_KINDS]) -> [T; 3] {
        [0, 1, 2].map(|c| of.iter().map(|v| v[c]).max().expect("faces"))
    }
    grid.pieces.alpha[i] = views.alpha.into_iter().max().expect("faces");
    grid.pieces.colour[i] = most(&views.colour);
    let alike = |k: usize| views.alpha[k] == views.alpha[0] && views.colour[k] == views.colour[0] && views.glow[k] == views.glow[0];
    if (0..FACE_KINDS).all(alike) {
        (grid.pieces.alpha[i], grid.pieces.colour[i]) = (views.alpha[0], views.colour[0]);
        set_glow(grid, odd, i, views.glow[0]);
    } else {
        set_glow(grid, odd, i, most(&views.glow));
        grid.views.insert(i, views);
    }
}

/**
 * `see`, for most sites: one put whole (painted, or filled where nothing
 * is) or carved out whole, alike through every face, by a kind that
 * neither glows nor is a medium, where nothing is in `glow` or `views`.
 * Set outright, without `see`'s sums, nor its colour (`own`) for a carve.
 * False, and nothing done, if not so.
 */
#[allow(clippy::too_many_arguments)]
fn see_whole(
    grid: &mut VoxelGrid,
    odd: &[bool],
    look: &Look,
    kind: &KindDef,
    op: Op,
    share: u32,
    [x, y, z]: [i64; 3],
    own: impl FnOnce() -> [u32; 3],
) -> bool {
    if share < SHARE || look.cover.is_some() || look.faces.is_some() || kind.glows || (kind.medium && op != Op::Carve) {
        return false;
    }
    let Some(i) = grid.index(x, y, z) else { return true };
    if odd.get(i).is_some_and(|odd| *odd) {
        return false;
    }
    // A fill is behind what is there: what a carve left of colour, if anything.
    let behind = match op {
        Op::Carve => {
            (grid.pieces.alpha[i], grid.pieces.colour[i], grid.medium.alpha[i], grid.medium.colour[i], grid.solid[i]) =
                (0, [0; 3], 0, [0; 3], 0);
            return true;
        }
        Op::Paint => [0; 3],
        Op::Fill if grid.pieces.alpha[i] == 0 => grid.pieces.colour[i],
        Op::Fill => return false,
    };
    let own = own();
    grid.pieces.colour[i] = std::array::from_fn(|c| (behind[c] as u32 + own[c]).min(COLOUR_ONE as u32) as u16);
    grid.pieces.alpha[i] = ALPHA;
    if !kind.walk_through {
        grid.solid[i] = ALPHA;
    }
    true
}

/** What `see` adds to a site: a thing's colour through each face, what it glows, whether a medium, whether it stops a walking body. */
struct Added {
    op: Op,
    pick: u32,
    /** Linear, of COLOUR_ONE. */
    colour: [u32; 3],
    /** Its colour through each face, where they differ. */
    faces: Option<[[u32; 3]; FACE_KINDS]>,
    glow: [u32; 3],
    /** How much of each face's rays it covers, if not its share. */
    cover: Option<[u32; FACE_KINDS]>,
    medium: bool,
    solid: bool,
}

impl Added {
    /** What a put of `look`, of `kind`, adds: `colour` gives each kind's colour at its site. */
    fn of(look: &Look, kind: &KindDef, op: Op, pick: u32, colour: impl Fn(Kind) -> [u32; 3]) -> Added {
        let own = colour(look.kind);
        let faces = look.faces.map(|kinds| kinds.map(&colour)).filter(|faces| faces.iter().any(|c| *c != own));
        // What glows gives light of its colour.
        let glow = if kind.glows { own.map(|c| (c * 255 + COLOUR_ONE as u32 / 2) / COLOUR_ONE as u32) } else { [0; 3] };
        Added { op, pick, colour: own, faces, glow, cover: look.cover, medium: kind.medium, solid: !kind.walk_through }
    }

    /**
     * Of what a site (or a face of it) stops, shows and glows, `share` of
     * it (of SHARE) added, seen through face `k`: what it stops, shows and
     * glows after.
     */
    fn to(&self, (alpha, colour, glow): (u8, [u16; 3], [u32; 3]), share: u32, k: usize) -> (u8, [u16; 3], [u32; 3]) {
        let (alpha, share) = (alpha as u64, share.min(SHARE) as u64);
        let (all, one) = (ALPHA as u64, SHARE as u64);
        // Of SHARE: how much of what was there is still seen, and how much of the site the thing is seen in.
        let (kept, visible) = match self.op {
            Op::Carve => (one - share, 0),
            Op::Fill => (one, (all - alpha) * share / all),
            // On what is there, as much of it as the paint covers of the site.
            Op::Paint => (one - (share * all).checked_div(alpha).map_or(one, |painted| painted.min(one)), share),
        };
        let after = match self.op {
            Op::Carve => self.rounded(alpha * kept),
            Op::Fill => self.rounded(alpha * one + (all - alpha) * share),
            Op::Paint => self.rounded((alpha * one).max(all * share)),
        };
        let own = self.faces.map_or(self.colour, |faces| faces[k]);
        let colour = std::array::from_fn(|c| ((colour[c] as u64 * kept + visible * own[c] as u64) / one).min(COLOUR_ONE as u64) as u16);
        let glow = std::array::from_fn(|c| (glow[c] as u64 * kept / one + visible * self.glow[c] as u64) as u32);
        (after, colour, glow)
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
fn set_glow(grid: &mut VoxelGrid, odd: &mut [bool], i: usize, glow: [u32; 3]) {
    if glow != [0; 3] {
        grid.glow.insert(i, glow);
        odd[i] = true;
    } else if odd[i] {
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
    // The lattice units the sites of `clip` cover, with what a shape may reach by at this level: a speck, the sites around it.
    let sites = units_of(clip, s);
    let units = sites.widen(2 * s);
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
            // Things are asked only of what is empty: what is buried or not whole, they leave so.
            holds = match (meets(layer, things && holds == Some(Whole::Empty)), holds) {
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
 * generated, and of that only each block of columns from its `Floors`
 * up; what is buried under them, and under it all, not generated but
 * kept as rock (VoxelGrid::on_rock), its buried bricks kept as one
 * (VoxelGrid::bury). Not `generate`'s grid, so not for checking a box
 * made whole against its parts.
 */
pub fn generate_shown(generator: &Generator, b: &Box3, level: u32) -> VoxelGrid {
    let part = shown(generator, b, level);
    let under = Box3 { max: [b.max[0], part.min[1], b.max[2]], ..*b };
    let rock = match whole(generator, &under, level) {
        Some(Whole::Buried(kind)) if under.min[1] < under.max[1] => Some(kind),
        _ => None,
    };
    let all = if rock.is_some() { Box3 { min: b.min, ..part } } else { part };
    let floors = Floors::of(generator, &VoxelGrid::new(&all).clip(), part.min[1], rock, level);
    // Nothing buried, nothing is kept from being drawn; all buried, nothing is made, not even things near.
    let under = floors.kind.map(|_| &floors);
    let mut grid = if part.min[1] < part.max[1] { made(generator, &all, level, under) } else { VoxelGrid::new(&all) };
    if let Some(kind) = floors.kind {
        let s = 1i64 << level;
        let colour = Palette::new(&generator.kinds, generator.seed).colour(kind, s, b.min.map(|v| v * s));
        grid.on_rock(|x, z| floors.at(x, z), colour.map(|c| c as u16));
    }
    grid.bury();
    grid
}

/**
 * Columns a side of the blocks `Floors` are found for. No fewer: a
 * ground's range over a smaller block is sampled between the level's
 * columns (Sloped::range), each made for it.
 */
const FLOOR: i64 = 2 * BRICK;

/**
 * How far up from its bottom each block of a box's columns, FLOOR a
 * side, is buried (`whole`), in bricks: hosts make none of that
 * (`generate_shown`), so a tile on a slope costs its surface, not the
 * rock under it.
 */
pub(super) struct Floors {
    min: [i64; 3],
    /** Blocks along x and z. */
    blocks: [i64; 2],
    /** By block, z-major: the first of its sites along y that isn't buried under. */
    y: Vec<i64>,
    /** The kind what is buried is of, the last found; none if nothing is. */
    kind: Option<Kind>,
}

impl Floors {
    /**
     * Those of box `b` of `level`, known buried up to `from`, in `rock`
     * if that is above its bottom. Found by quarters: what is buried of
     * a block is of each part of it, so each is asked from its parent's
     * floor up.
     */
    fn of(generator: &Generator, b: &Box3, from: i64, rock: Option<Kind>, level: u32) -> Floors {
        let blocks = [0, 2].map(|a| (b.max[a] - b.min[a] + FLOOR - 1).div_euclid(FLOOR).max(0));
        let mut floors = Floors { min: b.min, blocks, y: vec![b.min[1]; (blocks[0] * blocks[1]) as usize], kind: rock };
        let y = floors.rise(generator, b, level, [[0, 0], blocks], from);
        floors.quarter(generator, b, level, [[0, 0], blocks], y);
        floors
    }

    /** How far up from `from` blocks `lo` to `hi` (x, z; exclusive) of `b` are buried together, brick by brick. */
    fn rise(&mut self, generator: &Generator, b: &Box3, level: u32, [lo, hi]: [[i64; 2]; 2], from: i64) -> i64 {
        let along = |a: usize, block: i64| (b.min[2 * a] + block * FLOOR).min(b.max[2 * a]);
        let mut y = from;
        while y < b.max[1] {
            let top = (y + BRICK).min(b.max[1]);
            let layer = Box3 { min: [along(0, lo[0]), y, along(1, lo[1])], max: [along(0, hi[0]), top, along(1, hi[1])] };
            let Some(Whole::Buried(kind)) = whole(generator, &layer, level) else { break };
            (self.kind, y) = (Some(kind), top);
        }
        y
    }

    /**
     * Sets the floors of blocks `lo` to `hi` of `b`, buried up to `y`
     * together, then of its quarters, unless none is buried higher: where
     * no rock is near, few blocks are asked.
     */
    fn quarter(&mut self, generator: &Generator, b: &Box3, level: u32, [lo, hi]: [[i64; 2]; 2], y: i64) {
        for z in lo[1]..hi[1] {
            self.y[(z * self.blocks[0] + lo[0]) as usize..(z * self.blocks[0] + hi[0]) as usize].fill(y);
        }
        let mid = [0, 1].map(|a| (lo[a] + hi[a] + 1) / 2);
        let quarters = [[lo, mid], [[mid[0], lo[1]], [hi[0], mid[1]]], [[lo[0], mid[1]], [mid[0], hi[1]]], [mid, hi]];
        // Those with blocks: of a block alone, itself.
        let quarters: Vec<[[i64; 2]; 2]> = quarters.into_iter().filter(|[lo, hi]| lo[0] < hi[0] && lo[1] < hi[1]).collect();
        if quarters.len() < 2 {
            return;
        }
        let floors: Vec<i64> = quarters.iter().map(|q| self.rise(generator, b, level, *q, y)).collect();
        if floors.iter().any(|&floor| floor > y) {
            for (q, floor) in quarters.into_iter().zip(floors) {
                self.quarter(generator, b, level, q, floor);
            }
        }
    }

    /** The first site of column (x, z) (sites) that isn't buried under: the lowest of all, off the box. */
    #[inline]
    pub(super) fn at(&self, x: i64, z: i64) -> i64 {
        let (i, k) = ((x - self.min[0]).div_euclid(FLOOR), (z - self.min[2]).div_euclid(FLOOR));
        if i < 0 || i >= self.blocks[0] || k < 0 || k >= self.blocks[1] {
            return i64::MIN;
        }
        self.y[(k * self.blocks[0] + i) as usize]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_site_put_whole_outright_is_as_its_sums_make_it() {
        let kind = |walk_through, glows, medium| KindDef { name: "", kind_type: 0, colors: &[], walk_through, glows, medium };
        let kinds = [kind(false, false, false), kind(true, false, false), kind(false, true, false), kind(true, false, true)];
        let (plain, sheet) = (Look::from(0), Look { cover: Some([SHARE / 3; FACE_KINDS]), ..Look::from(0) });
        let colour = |_: Kind| [40_000, 20_000, 10_000];
        // What is in the site before, each put as `see` adds it: nothing, pieces in part, whole, cut down to a
        // trace, to nothing but their colour, unlike through its faces, a lamp, mist.
        let before: [&[(usize, &Look, Op, u32)]; 8] = [
            &[],
            &[(0, &plain, Op::Fill, SHARE / 2)],
            &[(0, &plain, Op::Fill, SHARE)],
            &[(0, &plain, Op::Fill, SHARE), (0, &plain, Op::Carve, SHARE - 300)],
            &[(0, &plain, Op::Fill, SHARE / 200), (0, &plain, Op::Carve, SHARE / 2)],
            &[(0, &sheet, Op::Paint, SHARE / 3)],
            &[(2, &plain, Op::Fill, SHARE / 2)],
            &[(3, &plain, Op::Fill, SHARE / 2), (0, &plain, Op::Fill, SHARE / 4)],
        ];
        let mut outright = 0;
        for (puts, pick) in before.iter().flat_map(|puts| [0, 21_845, 43_690].map(|pick| (puts, pick))) {
            let (mut grid, mut odd) = (VoxelGrid::new(&Box3::around([0; 3], 1)), Vec::new());
            for &(k, look, op, share) in *puts {
                see(&mut grid, &mut odd, &Added::of(look, &kinds[k], op, pick, colour), share, 0, 0, 0);
            }
            for (kind, look, op, share) in kinds.iter().flat_map(|kind| {
                [&plain, &sheet].into_iter().flat_map(move |look| {
                    [Op::Fill, Op::Paint, Op::Carve].into_iter().flat_map(move |op| [SHARE, SHARE - 1].map(|share| (kind, look, op, share)))
                })
            }) {
                let (mut whole, mut summed) = (grid.clone(), (grid.clone(), odd.clone()));
                if !see_whole(&mut whole, &odd, look, kind, op, share, [0; 3], || colour(0)) {
                    continue;
                }
                see(&mut summed.0, &mut summed.1, &Added::of(look, kind, op, pick, colour), share, 0, 0, 0);
                let held = |g: VoxelGrid| (g.pieces.alpha, g.pieces.colour, g.medium.alpha, g.medium.colour, g.solid, g.glow, g.views);
                assert_eq!(held(whole), held(summed.0), "after {} puts, pick {pick}, share {share}", puts.len());
                outright += 1;
            }
        }
        assert!(outright > 60, "only {outright} put outright");
    }
}
