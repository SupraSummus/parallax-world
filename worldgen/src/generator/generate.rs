//! Generating a box of a world at a level: its fields' and its
//! populations' things' drawables (forms.rs) drawn in order (drawables.rs);
//! what a box holds throughout (`whole`, `shown`).

use std::cell::OnceCell;

use super::*;
use crate::grid::Faces;

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
        let hex = |color: &str| {
            let rgb = color.strip_prefix('#').and_then(|hex| u32::from_str_radix(hex, 16).ok()).expect("a colour #rrggbb");
            [16, 8, 0].map(|shift| linear((rgb >> shift) as u8) as u32)
        };
        let shades: Vec<Vec<[u32; 3]>> = kinds.iter().map(|k| k.colors.iter().map(|c| hex(c)).collect()).collect();
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
    let rgb = |colour: &[u32; 3]| colour.map(|c| srgb(c as u16));
    let colours: Vec<(usize, [u8; 3])> = (0..generator.kinds.len())
        .flat_map(|k| palette.shades[k].iter().chain([&palette.means[k]]).map(move |c| (k, c)).collect::<Vec<_>>())
        .map(|(k, c)| (k, rgb(c)))
        .collect();
    let of = move |look: fcc::Look| {
        let at = look.to_le_bytes();
        let far = |c: &[u8; 3]| (0..3).map(|i| c[i].abs_diff(at[i])).max().expect("channels");
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
    made(&Contents::of(generator, &VoxelGrid::new(b).clip(), level), b, None)
}

/** `generate` of `b`, whose grid's sites are some of those of `contents`, nothing drawn under `floors`, if any: what is buried there is not made. */
fn made(contents: &Contents, b: &Box3, floors: Option<&Floors>) -> VoxelGrid {
    let (generator, s) = (contents.generator, contents.s);
    let mut grid = VoxelGrid::new(b);
    if (0..3).any(|d| b.max[d] <= b.min[d]) {
        // No sites, so nothing to make, not even things near.
        return grid;
    }
    let palette = Palette::new(&generator.kinds, generator.seed);
    let clip = grid.clip();
    let mut put = |look: Look, op: Op, share: u32, pick: u32, x: i64, y: i64, z: i64| {
        // Things and lines across the floors' axis are drawn whole and kept from the floors up.
        if floors.is_some_and(|floors| floors.under([x, y, z])) {
            return;
        }
        let colour = |kind: Kind| palette.colour(kind, s, [x, y, z].map(|v| v * s));
        let kind = &generator.kinds[look.kind as usize];
        if !see_whole(&mut grid, &look, kind, op, share, [x, y, z], || colour(look.kind)) {
            see(&mut grid, &Added::of(&look, &generator.kinds, op, share, pick, colour), x, y, z);
        }
    };
    let drawables = drawables_in(contents, &clip);
    let mut at = 0;
    while at < drawables.len() {
        // A thing's drawables, together: those of fields one by one.
        let thing = drawables[at].thing;
        let end = if thing == 0 { at + 1 } else { (at..drawables.len()).find(|i| drawables[*i].thing != thing).unwrap_or(drawables.len()) };
        let group = &drawables[at..end];
        at = end;
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
    grid
}

/**
 * Adds a thing's `share` of site (x, y, z) to it (VoxelGrid's `pieces`,
 * `medium`, `glow`): as a Fill behind what is there, apart from it; a
 * Paint in front of it, lying on it, and past it only where it covers
 * more; a Carve taking it out, of both. A medium joins the site's medium,
 * the rest its pieces; painted, it lies on the pieces and takes their
 * place, as far as it covers them. What either stops, shows and glows
 * is the sum of
 * what is in it, each by how much of the site's rays it stops there.
 * Pieces covering some faces' rays more than others', unlike through
 * some faces, or put where some did, are added face by face (`pieces`,
 * VoxelGrid's `views`); the site as a whole, to what sees no faces (a
 * host's voxels, a camera colliding), stops and glows as its most.
 * What stops a walking body adds its share to the site's `solid`,
 * wherever in it, in front or behind; a carve takes out of it too.
 * Few sites are in `glow` or `views` (grid's Few): most are added to at
 * a glance.
 */
fn see(grid: &mut VoxelGrid, added: &Added, x: i64, y: i64, z: i64) {
    let share = added.share;
    if share == 0 && added.faces.each().iter().all(|&(_, cover)| cover == 0) {
        return;
    }
    let Some(i) = grid.index(x, y, z) else { return };
    let (solid, all, one) = (grid.solid[i] as u64, ALPHA as u64, SHARE as u64);
    let share64 = share.min(SHARE) as u64;
    grid.solid[i] = match added.op {
        Op::Carve => added.rounded(solid * (one - share64)),
        _ if added.solid => added.rounded(solid * one + (all - solid) * share64),
        _ => solid as u8,
    };
    // A carve takes out pieces and medium alike: the medium here, the pieces as any.
    if added.op == Op::Carve {
        let kept = (SHARE - share.min(SHARE)) as u64;
        let (stops, shows) = grid.medium.get(i);
        grid.medium.set(i, stops.map(|a| added.rounded(a as u64 * kept)), shows.map(|c| (c as u64 * kept / SHARE as u64) as u16));
    } else if let Some(faster) = added.medium {
        let mut share = share.min(SHARE);
        if added.op == Op::Paint && grid.pieces.get(i).0 > 0 {
            // It lies on the pieces, as any paint on what is there, and takes their place: as much of them as it
            // covers through the face that sees most of it, through every face alike, as a site shows its pieces whole
            // or not at all. The site's medium is a share of the rays the pieces let through, so the rest of them.
            let (all, one) = (ALPHA as u64, SHARE as u64);
            let most = (0..FACE_KINDS).max_by_key(|&k| added.faces.get(k).1).expect("faces");
            let seen = grid.views.face(i, most).map_or(grid.pieces.get(i).0, |sight| sight.0) as u64;
            let covered = (share as u64 * all).checked_div(seen).map_or(one, |of| of.min(one));
            let faces = Faces::all((added.sight, covered as u32));
            let under = Added { op: Op::Carve, share: covered as u32, faces, medium: None, solid: false, ..*added };
            pieces(grid, &under, i);
            let left = all * one - seen * (one - covered);
            share = (share as u64 * all * one).checked_div(left).map_or(SHARE, |of| of.min(one) as u32);
        }
        let (stops, shows, glow) = added.joins(grid.medium.get(i), grid.glow.get(i).copied().unwrap_or_default(), faster, share);
        grid.medium.set(i, stops, shows);
        set_glow(grid, i, glow);
        return;
    }
    pieces(grid, added, i);
}

/**
 * How many times as fast as the channel it stops least a medium of
 * `colour` (linear, of COLOUR_ONE) stops each: where its share of a site
 * lets `t` through, it lets t^this. Its colour is what it sends on of the
 * light it stops, and it scatters every channel alike: so it stops each
 * the more, the darker its colour there. The law's one place: what sees
 * a site takes what its medium holds (`Added::joins`).
 */
fn stops_faster(colour: [u32; 3]) -> [f64; 3] {
    let sends = colour.map(|c| (c as f64 / COLOUR_ONE as f64).max(1.0 / 255.0));
    let most = js::max(sends);
    sends.map(|c| most / c)
}

/** A medium's share of a site (of SHARE) in each channel: `share` in the one it stops least, the others `faster` (`stops_faster`). */
fn shares(share: u32, faster: [f64; 3]) -> [u64; 3] {
    let through = 1.0 - share as f64 / SHARE as f64;
    faster.map(|faster| if faster == 1.0 { share as u64 } else { ((1.0 - js::pow(through, faster)) * SHARE as f64) as u64 })
}

/**
 * `see`, of site `i`'s pieces: `added` to each face as it covers it,
 * from what that face saw so far, its glow too; each class of faces that
 * saw alike and are added to alike once. The site as a whole stops,
 * shows and glows as its most.
 */
fn pieces(grid: &mut VoxelGrid, added: &Added, i: usize) {
    // Alike through every face where it has no views, or no glows apart (VoxelGrid's `glows`).
    let views = grid.views.get(i).unwrap_or(Faces::all(grid.pieces.get(i)));
    let glow = grid.glow.get(i).copied().unwrap_or_default();
    let glows = grid.glows.get(i).map_or(Faces::all(glow), |glows| Faces::by(|k| glows[k]));
    let after = views.zip_with(&glows, |s, g| (s, g)).zip_with(&added.faces, |(sight, glow), face| added.to(sight, glow, face));
    let shown = after.map(|(alpha, colour, _)| (alpha, colour));
    fn most<T: Ord + Copy + Default>(of: impl Iterator<Item = [T; 3]>) -> [T; 3] {
        of.fold([T::default(); 3], |most, c| std::array::from_fn(|i| most[i].max(c[i])))
    }
    grid.pieces.set(i, shown.each().iter().map(|s| s.0).max().expect("a sight"), most(shown.each().iter().map(|s| s.1)));
    set_glow(grid, i, most(after.each().iter().map(|a| a.2)));
    let unlike = after.each().len() > 1;
    let lit = after.each().iter().any(|a| a.2 != [0; 3]);
    grid.views.set(i, unlike.then_some(&shown));
    grid.glows.set(i, (unlike && lit).then(|| std::array::from_fn(|k| after.get(k).2)));
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
    look: &Look,
    kind: &KindDef,
    op: Op,
    share: u32,
    [x, y, z]: [i64; 3],
    own: impl FnOnce() -> [u32; 3],
) -> bool {
    if share < SHARE || look.faces.is_some() || kind.glows || (kind.medium && op != Op::Carve) {
        return false;
    }
    let Some(i) = grid.index(x, y, z) else { return true };
    if grid.glow.get(i).is_some() || grid.views.has(i) {
        return false;
    }
    // A fill is behind what is there: what a carve left of colour, if anything.
    let behind = match op {
        Op::Carve => {
            grid.pieces.set(i, 0, [0; 3]);
            grid.medium.set(i, [0; 3], [0; 3]);
            grid.solid[i] = 0;
            return true;
        }
        Op::Paint => [0; 3],
        Op::Fill => match grid.pieces.get(i) {
            (0, colour) => colour,
            _ => return false,
        },
    };
    let own = own();
    grid.pieces.set(i, ALPHA, std::array::from_fn(|c| (behind[c] as u32 + own[c]).min(COLOUR_ONE as u32) as u16));
    if !kind.walk_through {
        grid.solid[i] = ALPHA;
    }
    true
}

/** What is seen of a kind: its colour (linear, of COLOUR_ONE), and what it glows. */
type Sight = ([u16; 3], [u16; 3]);

/** What a thing shows and glows through a face, and how much of the face's rays it covers (of SHARE). */
type Face = (Sight, u32);

/** What `see` adds to a site: its share of it, what it is through each face, whether a medium, whether it stops a walking body. */
#[derive(Clone, Copy)]
struct Added {
    op: Op,
    pick: u32,
    /** Of SHARE, how much of the site's rays it stops: of each face's, as `faces` cover them. */
    share: u32,
    /** What its own kind shows and glows. */
    sight: Sight,
    faces: Faces<Face>,
    /** If a medium, how many times as fast as its share it stops each channel (`stops_faster`). */
    medium: Option<[f64; 3]>,
    solid: bool,
}

impl Added {
    /** What a put of `share` of `look`, its kinds of `kinds`, adds: `colour` gives each kind's colour at its site. */
    fn of(look: &Look, kinds: &[KindDef], op: Op, share: u32, pick: u32, colour: impl Fn(Kind) -> [u32; 3]) -> Added {
        // What glows gives light of its colour.
        let sight = |kind: Kind| {
            let own = colour(kind);
            let glow = if kinds[kind as usize].glows {
                own.map(|c| ((c * 255 + COLOUR_ONE as u32 / 2) / COLOUR_ONE as u32) as u16)
            } else {
                [0; 3]
            };
            (own.map(|c| c as u16), glow)
        };
        let own = sight(look.kind);
        let faces = match look.faces {
            None => Faces::all((own, share)),
            Some(faces) => faces.map(|(kind, cover)| (if kind == look.kind { own } else { sight(kind) }, cover)),
        };
        let kind = &kinds[look.kind as usize];
        Added {
            op,
            pick,
            share,
            sight: own,
            faces,
            medium: kind.medium.then(|| stops_faster(own.0.map(u32::from))),
            solid: !kind.walk_through,
        }
    }

    /**
     * Of what a site's face stops and shows, `sight`, and glows, what it
     * stops, shows and glows once `face` is added to it.
     */
    fn to(&self, (alpha, colour): crate::grid::Sight, glow: [u32; 3], ((own, glows), share): Face) -> (u8, [u16; 3], [u32; 3]) {
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
        let colour = std::array::from_fn(|c| ((colour[c] as u64 * kept + visible * own[c] as u64) / one).min(COLOUR_ONE as u64) as u16);
        let glow = std::array::from_fn(|c| (glow[c] as u64 * kept / one + visible * glows[c] as u64) as u32);
        (after, colour, glow)
    }

    /**
     * A site's medium, what it stops of each channel (of ALPHA) and shows,
     * and its `glow`, once `share` (of SHARE) of the site of this one joins
     * it: of each channel its own share, `faster`. None of a site's media
     * hides another: this one stops its share of what the others let
     * through, behind them, or, painted, they of what it lets through.
     */
    fn joins(&self, (stops, shows): ([u8; 3], [u16; 3]), glow: [u32; 3], faster: [f64; 3], share: u32) -> ([u8; 3], [u16; 3], [u32; 3]) {
        let (all, one) = (ALPHA as u64, SHARE as u64);
        // Of SHARE: how much of what was there is still seen, and how much of the site this is seen in, where `stopped` (of ALPHA) is.
        let seen = |share: u64, stopped: u8| match self.op {
            Op::Paint => (one - share, share),
            _ => (one, (all - stopped as u64) * share / all),
        };
        let shares = shares(share, faster);
        let mut after = (stops, shows);
        for c in 0..3 {
            let share = shares[c];
            let (kept, visible) = seen(share, stops[c]);
            after.0[c] = self.rounded(stops[c] as u64 * one + (all - stops[c] as u64) * share);
            after.1[c] = ((shows[c] as u64 * kept + visible * self.sight.0[c] as u64) / one).min(COLOUR_ONE as u64) as u16;
        }
        let (kept, visible) = seen(share as u64, crate::grid::least(stops));
        (after.0, after.1, std::array::from_fn(|c| (glow[c] as u64 * kept / one + visible * self.sight.1[c] as u64) as u32))
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
    grid.glow.set(i, (glow != [0; 3]).then_some(glow));
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
            Draw::Field(..) | Draw::Over(..) => Vec::new(),
        })
        .collect();
    bounds_of_shapes(&boxes)
}

/** What draws into `clip`, some of the sites of `contents`, in drawing order: by layer, then key. */
fn drawables_in<'a>(contents: &'a Contents, clip: &Box3) -> Vec<Drawable<'a>> {
    let (generator, s) = (contents.generator, contents.s);
    // The lattice units the sites of `clip` cover, with what a shape may reach by at this level: a speck, the sites around it.
    let sites = units_of(clip, s);
    assert!(sites.inside(&contents.units), "{clip:?} is not of the sites its contents are of");
    let units = sites.widen(2 * s);
    let mut drawables = Vec::new();
    let mut things = 0;
    for laid in &contents.layers {
        let (order, layer) = (laid.order, laid.name);
        let mut work = Work::default();
        let draw = match &laid.form {
            // Asked where it may draw line by line (draw_field).
            Form::Field(f, lines) => Some(Draw::Field(f, *lines)),
            Form::Shapes { field, near, shapes } => Some(Draw::Over(field, contents.shapes(near, shapes))),
            Form::Things(p, made) => {
                let made = contents.made(p, made);
                // Its things are visited as they were made, on the level's sites; those reaching `units` draw.
                sites::footprint(s, [0; 3], || {
                    for thing in &made.things {
                        things += 1;
                        let at = Visit { s, units: &units, order, layer, thing: things, seed: generator.seed, measured: &p.measured };
                        visit(&thing.node, thing.name.clone(), thing.bounds, &at, &mut drawables);
                    }
                });
                work = made.work;
                None
            }
        };
        if let Some(draw) = draw {
            drawables.push(Drawable { order, key: layer.to_string(), layer, thing: 0, draw });
        }
        // What it looked up, where it stands and what is in it, it did.
        cost::add(layer, work);
    }
    drawables.sort_by(|a, b| a.order.total_cmp(&b.order).then_with(|| a.key.cmp(&b.key)));
    drawables
}

/** Why a thing within `bounds`, anchored at `key`, is past its population's `reach` from there: along which axes, by how much. */
fn past_reach(name: &str, bounds: &Box3, key: [i64; 3], reach: [i64; 3]) -> String {
    let needed = [0, 1, 2].map(|i| reach[i].max(key[i] - bounds.min[i]).max(bounds.max[i] - 1 - key[i]));
    let axes: Vec<String> =
        (0..3).filter(|i| needed[*i] > reach[*i]).map(|i| format!("{} by {}", ["x", "y", "z"][i], needed[i] - reach[i])).collect();
    format!(
        "{name} reaches past its population's reach {reach:?} from its anchor along {}: its bounds are {bounds:?}; \
         reach {needed:?} would hold this one, but derive it from the largest possible thing; see \"Populations\" in {DOCS}",
        axes.join(", ")
    )
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
    Contents::of(generator, &b.widen(1), level).whole(b)
}

/**
 * What the layers of a box at a level draw in it: for `whole` of it and
 * of its parts, as hosts ask of many (`shown`, `Floors`), and for drawing
 * it. What a layer is at the level is decided here alone (`Laid`); a
 * field's shapes and a population's things, which cost the most to ask,
 * are asked once, of all of it: each is the same whichever box asks (see
 * "Model" in docs/generators.md), so a part has those of them reaching it.
 */
pub(super) struct Contents<'a> {
    pub(super) generator: &'a Generator,
    pub(super) s: i64,
    /** Its layers, in the order they draw. */
    layers: Vec<Laid<'a>>,
    /** The lattice units of the sites it is of. */
    units: Box3,
}

/** A layer as it is at a level, if it draws anything there. */
struct Laid<'a> {
    order: f64,
    name: &'static str,
    form: Form<'a>,
}

/** What a layer draws at a level: a field (its own, or a population's far field past its things' size), or things. */
enum Form<'a> {
    /** Given along its lines or at each site. */
    Field(&'a Field, Lines<'a>),
    /** A field's shapes, those `near` gives (FieldValues::Shapes). */
    Shapes { field: &'a Field, near: &'a Near<'a>, shapes: OnceCell<Vec<Shape>> },
    /** A population's things. */
    Things(&'a Population, OnceCell<Made>),
}

/** The things a population makes reaching a box, and what making them took. */
struct Made {
    things: Vec<Thing>,
    work: Work,
}

/** What gives a field's shapes near a box (FieldValues::Shapes), borrowed for as long as the contents are. */
type Near<'a> = dyn Fn(&Box3) -> Vec<Shape> + Send + Sync + 'a;

/** A thing made: its name and its bounds, which its population's reach of its anchor holds. */
struct Thing {
    name: String,
    bounds: Box3,
    node: Node,
}

impl<'a> Contents<'a> {
    /**
     * Those of the sites `sites` (a box of `level`'s) and of the parts of
     * them asked about: drawn into (a grid's clip), or whole of a box with
     * a site round it, as `whole` asks.
     */
    pub(super) fn of(generator: &'a Generator, sites: &Box3, level: u32) -> Contents<'a> {
        let s = 1i64 << level;
        let field = |field: &'a Field| match &field.values {
            FieldValues::At(at) => Form::Field(field, Lines::At(at.as_ref())),
            FieldValues::Spans { axis, spans } => Form::Field(field, Lines::Spans { axis: *axis, spans: spans.as_ref() }),
            FieldValues::Shapes(near) => Form::Shapes { field, near: near.as_ref(), shapes: OnceCell::new() },
        };
        let mut layers: Vec<Laid> = generator
            .layers
            .iter()
            .filter_map(|layer| match layer {
                Layer::Field(f) => Some(Laid { order: f.order, name: f.name, form: field(f) }),
                Layer::Population(p) => {
                    let form = if s > p.size { field(p.far.as_ref()?) } else { Form::Things(p, OnceCell::new()) };
                    Some(Laid { order: p.order, name: p.name, form })
                }
            })
            .collect();
        layers.sort_by(|a, b| a.order.total_cmp(&b.order).then_with(|| a.name.cmp(b.name)));
        Contents { generator, s, layers, units: units_of(sites, s) }
    }

    /**
     * What box `b`, a part of theirs, holds throughout, as its fields say
     * (Field::meets) and its things are (`whole`).
     */
    pub(super) fn whole(&self, b: &Box3) -> Option<Whole> {
        let (generator, s) = (self.generator, self.s);
        let units = units_of(&b.widen(1), s);
        assert!(units.inside(&self.units), "whole: {b:?} with a site round it is not of the sites its contents are of");
        // What a layer draws in it, and whether that carves; a population's things, which cost the most to ask, as if
        // drawing nothing unless `things`. A shape is asked with what it may grow by at this level, as drawing asks it.
        let reaching = units.widen(2 * s);
        let meets = |laid: &Laid, things: bool| match &laid.form {
            Form::Field(f, _) => (f.meets_in(&units, s), f.op == Op::Carve),
            Form::Shapes { field, near, shapes } => {
                // As it says, and nothing where none of its shapes reaches.
                let said = field.meets_in(&units, s);
                let reached = said != Meets::Nothing && self.shapes(near, shapes).iter().any(|shape| shape.bounds.overlaps(&reaching));
                (if reached { said } else { Meets::Nothing }, field.op == Op::Carve)
            }
            Form::Things(p, made) if things => {
                let reached = self.made(p, made).things.iter().any(|thing| thing.bounds.overlaps(&reaching));
                (if reached { Meets::Any } else { Meets::Nothing }, false)
            }
            Form::Things(..) => (Meets::Nothing, false),
        };
        let fold = |things: bool| {
            let mut holds = Some(Whole::Empty);
            for laid in &self.layers {
                // Things are asked only of what is empty: what is buried or not whole, they leave so.
                holds = match (meets(laid, things && holds == Some(Whole::Empty)), holds) {
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

    /** The shapes `near` gives near the box, once asked, with what a shape may grow by at this level. */
    fn shapes<'c>(&self, near: &Near, shapes: &'c OnceCell<Vec<Shape>>) -> &'c [Shape] {
        shapes.get_or_init(|| sites::footprint(self.s, [0; 3], || near(&self.units.widen(2 * self.s))))
    }

    /**
     * The things `p` makes reaching the box, once made, as drawing makes
     * them: on the level's sites, as they read the ground (Sloped::at),
     * each within its population's reach.
     */
    fn made<'c>(&self, p: &Population, made: &'c OnceCell<Made>) -> &'c Made {
        made.get_or_init(|| {
            let mut things = Vec::new();
            let all = anchors_reaching(p, &self.units.widen(2 * self.s));
            let work = sites::footprint(self.s, [0; 3], || {
                each_thing(p, &all, |anchor, node| {
                    let (name, bounds, reach) = (anchor.name(), bounds_of(&node), anchor.reach(p.reach));
                    assert!(bounds.inside(&reach), "{}", past_reach(&name, &bounds, anchor.key, p.reach));
                    things.push(Thing { name, bounds, node });
                })
            });
            Made { things, work }
        })
    }
}

/**
 * Box `b` of `level` less its ends along `up` that are `whole`, in bricks
 * up from its bottom: the part of it whose voxels may show, all a host
 * need generate; none at its top if all of it is buried, at its bottom if
 * empty. What is whole of a box is of every part of it, so each end is
 * found by halving.
 */
pub fn shown(generator: &Generator, b: &Box3, level: u32, up: usize) -> Box3 {
    shown_of(&Contents::of(generator, &b.widen(1), level), b, up)
}

/** `shown`, of box `b` whose `contents` they are. */
fn shown_of(contents: &Contents, b: &Box3, up: usize) -> Box3 {
    let (bottom, top) = (b.min[up], b.max[up]);
    match contents.whole(b) {
        Some(Whole::Buried(_)) => return b.slab(up, top, top),
        Some(Whole::Empty) => return b.slab(up, bottom, bottom),
        None => {}
    }
    let at = |k: i64| (bottom + k * BRICK).min(top);
    let whole_in = |y0: i64, y1: i64| y0 >= y1 || contents.whole(&b.slab(up, y0, y1)).is_some();
    let bricks: Vec<i64> = (0..=(top - bottom + BRICK - 1).div_euclid(BRICK).max(0)).collect();
    let low = bricks[1..].partition_point(|&k| whole_in(bottom, at(k)));
    let high = low + bricks[low..].partition_point(|&k| !whole_in(at(k), top));
    b.slab(up, at(low as i64), at(high as i64))
}

/**
 * Box `b` of `level` as hosts make it, rock under it along `up`: what may
 * show of it (`shown`) generated, and of that only each block of lines
 * along `up` from its `Floors` up; what is buried under them, and under
 * it all, not generated but kept as rock (VoxelGrid::on_rock), its buried
 * bricks kept as one (VoxelGrid::bury). Not `generate`'s grid, so not for
 * checking a box made whole against its parts.
 */
pub fn generate_shown(generator: &Generator, b: &Box3, level: u32, up: usize) -> VoxelGrid {
    // Its parts are asked what they hold throughout of what it holds, asked once.
    let contents = Contents::of(generator, &b.widen(1), level);
    let part = shown_of(&contents, b, up);
    let under = b.slab(up, b.min[up], part.min[up]);
    let rock = match contents.whole(&under) {
        Some(Whole::Buried(kind)) if under.min[up] < under.max[up] => Some(kind),
        _ => None,
    };
    let all = if rock.is_some() { part.slab(up, b.min[up], part.max[up]) } else { part };
    let floors = Floors::of(&contents, &VoxelGrid::new(&all).clip(), up, part.min[up], rock);
    // Nothing buried, nothing is kept from being drawn; all buried, nothing is made, not even things near.
    let under = floors.kind.map(|_| &floors);
    let mut grid = if part.min[up] < part.max[up] { made(&contents, &all, under) } else { VoxelGrid::new(&all) };
    if let Some(kind) = floors.kind {
        let s = 1i64 << level;
        let colour = Palette::new(&generator.kinds, generator.seed).colour(kind, s, b.min.map(|v| v * s));
        grid.on_rock(up, |site| floors.floor(site), colour.map(|c| c as u16));
    }
    grid.bury();
    grid
}

/**
 * Lines a side of the blocks `Floors` are found for. No fewer: a
 * ground's range over a smaller block is sampled between the level's
 * columns (Sloped::range), each made for it.
 */
const FLOOR: i64 = 2 * BRICK;

/**
 * How far up from its bottom along an axis each block of a box's lines
 * along it, FLOOR a side, is buried (`whole`), in bricks: hosts make none
 * of that (`generate_shown`), so a tile on a slope costs its surface, not
 * the rock under it.
 */
pub(super) struct Floors {
    /** The axis they are found along, up from the box's bottom. */
    pub(super) axis: usize,
    min: [i64; 3],
    /** Blocks along the axes across it, in axis order. */
    blocks: [i64; 2],
    /** By block, the second axis across major: the first of its sites along the axis that isn't buried under. */
    floor: Vec<i64>,
    /** The kind what is buried is of, the last found; none if nothing is. */
    kind: Option<Kind>,
}

impl Floors {
    /**
     * Those of box `b` of `level` along `axis`, known buried up to `from`,
     * in `rock` if that is above its bottom. Found by quarters: what is
     * buried of a block is of each part of it, so each is asked from its
     * parent's floor up.
     */
    fn of(contents: &Contents, b: &Box3, axis: usize, from: i64, rock: Option<Kind>) -> Floors {
        let blocks = across(axis).map(|a| (b.max[a] - b.min[a] + FLOOR - 1).div_euclid(FLOOR).max(0));
        let floor = vec![b.min[axis]; (blocks[0] * blocks[1]) as usize];
        let mut floors = Floors { axis, min: b.min, blocks, floor, kind: rock };
        let up = floors.rise(contents, b, [[0, 0], blocks], from);
        floors.quarter(contents, b, [[0, 0], blocks], up);
        floors
    }

    /** How far up from `from` blocks `lo` to `hi` (exclusive) of `b` are buried together, brick by brick. */
    fn rise(&mut self, contents: &Contents, b: &Box3, [lo, hi]: [[i64; 2]; 2], from: i64) -> i64 {
        let [a, c] = across(self.axis);
        let at = |a: usize, block: i64| (b.min[a] + block * FLOOR).min(b.max[a]);
        let blocks = b.slab(a, at(a, lo[0]), at(a, hi[0])).slab(c, at(c, lo[1]), at(c, hi[1]));
        let mut up = from;
        while up < b.max[self.axis] {
            let top = (up + BRICK).min(b.max[self.axis]);
            let Some(Whole::Buried(kind)) = contents.whole(&blocks.slab(self.axis, up, top)) else { break };
            (self.kind, up) = (Some(kind), top);
        }
        up
    }

    /**
     * Sets the floors of blocks `lo` to `hi` of `b`, buried up to `up`
     * together, then of its quarters, unless none is buried higher: where
     * no rock is near, few blocks are asked.
     */
    fn quarter(&mut self, contents: &Contents, b: &Box3, [lo, hi]: [[i64; 2]; 2], up: i64) {
        for k in lo[1]..hi[1] {
            self.floor[(k * self.blocks[0] + lo[0]) as usize..(k * self.blocks[0] + hi[0]) as usize].fill(up);
        }
        let mid = [0, 1].map(|a| (lo[a] + hi[a] + 1) / 2);
        let quarters = [[lo, mid], [[mid[0], lo[1]], [hi[0], mid[1]]], [[lo[0], mid[1]], [mid[0], hi[1]]], [mid, hi]];
        // Those with blocks: of a block alone, itself.
        let quarters: Vec<[[i64; 2]; 2]> = quarters.into_iter().filter(|[lo, hi]| lo[0] < hi[0] && lo[1] < hi[1]).collect();
        if quarters.len() < 2 {
            return;
        }
        let floors: Vec<i64> = quarters.iter().map(|q| self.rise(contents, b, *q, up)).collect();
        if floors.iter().any(|&floor| floor > up) {
            for (q, floor) in quarters.into_iter().zip(floors) {
                self.quarter(contents, b, q, floor);
            }
        }
    }

    /** The first site of the line through `site` along the axis that isn't buried under: the lowest of all, off the box. */
    #[inline]
    pub(super) fn floor(&self, site: [i64; 3]) -> i64 {
        let [a, c] = across(self.axis);
        let (i, k) = ((site[a] - self.min[a]).div_euclid(FLOOR), (site[c] - self.min[c]).div_euclid(FLOOR));
        if i < 0 || i >= self.blocks[0] || k < 0 || k >= self.blocks[1] {
            return i64::MIN;
        }
        self.floor[(k * self.blocks[0] + i) as usize]
    }

    /** Whether `site` is buried under them. */
    #[inline]
    pub(super) fn under(&self, site: [i64; 3]) -> bool {
        site[self.axis] < self.floor(site)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_site_put_whole_outright_is_as_its_sums_make_it() {
        let kind = |walk_through, glows, medium| KindDef { name: "", kind_type: 0, colors: &[], walk_through, glows, medium };
        let kinds = [kind(false, false, false), kind(true, false, false), kind(false, true, false), kind(true, false, true)];
        let (plain, sheet) = (Look::from(0), Look { kind: 0, faces: Some(Faces::all((0, SHARE / 3))) });
        let colour = |_: Kind| [40_000, 20_000, 10_000];
        // Each look is of kind 0: of the one kind `of` is given.
        let from_ref = std::slice::from_ref::<KindDef>;
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
            let mut grid = VoxelGrid::new(&Box3::around([0; 3], 1));
            for &(k, look, op, share) in *puts {
                see(&mut grid, &Added::of(look, from_ref(&kinds[k]), op, share, pick, colour), 0, 0, 0);
            }
            for (kind, look, op, share) in kinds.iter().flat_map(|kind| {
                [&plain, &sheet].into_iter().flat_map(move |look| {
                    [Op::Fill, Op::Paint, Op::Carve].into_iter().flat_map(move |op| [SHARE, SHARE - 1].map(|share| (kind, look, op, share)))
                })
            }) {
                let (mut whole, mut summed) = (grid.clone(), grid.clone());
                if !see_whole(&mut whole, look, kind, op, share, [0; 3], || colour(0)) {
                    continue;
                }
                see(&mut summed, &Added::of(look, from_ref(kind), op, share, pick, colour), 0, 0, 0);
                fn few<T: Copy>(few: &crate::grid::Few<T>) -> Vec<(usize, T)> {
                    few.iter().map(|(i, value)| (i, *value)).collect()
                }
                let held = |g: VoxelGrid| {
                    let few = (few(&g.glow), g.views.iter().collect::<Vec<_>>(), few(&g.glows));
                    (g.pieces.alpha, g.pieces.colour, g.medium.alpha, g.medium.colour, g.solid, few)
                };
                assert_eq!(held(whole), held(summed), "after {} puts, pick {pick}, share {share}", puts.len());
                outright += 1;
            }
        }
        assert!(outright > 60, "only {outright} put outright");
    }

    #[test]
    fn a_site_glows_through_each_face_as_the_kind_it_shows_there() {
        let kind = |glows| KindDef { name: "", kind_type: 0, colors: &[], walk_through: false, glows, medium: false };
        let kinds = [kind(false), kind(true)];
        // A lamp through the even faces, what does not glow through the others: whichever of the two the site is of.
        let faces: [Kind; FACE_KINDS] = std::array::from_fn(|k| (k % 2 == 0) as Kind);
        for of in [0, 1] {
            let mut grid = VoxelGrid::new(&Box3::around([0; 3], 1));
            let look = Look { kind: of, faces: Some(Faces::by(|k| (faces[k], SHARE))) };
            see(&mut grid, &Added::of(&look, &kinds, Op::Fill, SHARE, 0, |_| [40_000; 3]), 0, 0, 0);
            let glows: [bool; FACE_KINDS] = std::array::from_fn(|k| grid.light(0, 0, 0, k).2 != [0; 3]);
            assert_eq!(glows, faces.map(|kind| kind == 1), "the site of kind {of}");
        }
    }
}
