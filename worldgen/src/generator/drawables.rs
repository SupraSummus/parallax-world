//! What is drawn into a box: drawables, each a field, a thing's shapes or
//! its sites, put site by site; and, while checking, where one drew that
//! said it may not (`checking_skipped`).

use super::*;

/**
 * A thing's sites of a level: each what it is of (a kind, and each face's
 * kind, without a `Look`'s covers: a population keeps many, `Measured`),
 * how it meets the world, its share and pick (Put).
 */
pub(super) type Sites = Vec<([i64; 3], Kind, Faces<Kind>, Op, u32, u32)>;

pub(super) enum Draw<'a> {
    /** A field given along its lines or at each site. */
    Field(&'a Field, Lines<'a>),
    /** A field of shapes, those it gives near the box (Contents). */
    Over(&'a Field, &'a [Shape]),
    /** A node's shapes, and whether they are its far ones. */
    Shapes(Vec<Shape>, bool),
    /** A thing measured: its sites, as its population keeps them (Measured). */
    Sites(Arc<Sites>),
}

/** What a field gives: at each site, or spans along each line of sites parallel to `axis` (FieldValues). */
#[derive(Clone, Copy)]
pub(super) enum Lines<'a> {
    At(&'a FieldAt),
    Spans { axis: usize, spans: &'a FieldSpans },
}

pub(super) struct Drawable<'a> {
    pub(super) order: f64,
    pub(super) key: String,
    /** The layer it is of, for its cost. */
    pub(super) layer: &'static str,
    /** The thing it is part of, numbered from 1; 0 for fields. */
    pub(super) thing: u32,
    pub(super) draw: Draw<'a>,
}

/** Draws one drawable at sites `s` apart into `clip`; returns the work. */
pub(super) fn draw(drawable: &Drawable, seed: u64, s: i64, clip: &Box3, floors: Option<&Floors>, put: &mut Put) -> Work {
    match &drawable.draw {
        Draw::Field(field, lines) => draw_field(field, lines, seed, s, clip, floors, put),
        Draw::Over(field, shapes) => draw_field_shapes(field, shapes, seed, s, clip, put),
        Draw::Shapes(shapes, far) => {
            let mut tests = 0;
            for (i, shape) in shapes.iter().enumerate() {
                let seed = hash_text(&format!("{}#{}{i}", drawable.key, if *far { "far" } else { "" }), seed);
                tests += draw_shape(shape, seed, s, clip, put);
            }
            Work { nodes: 1, tests, ..Work::default() }
        }
        Draw::Sites(sites) => {
            for &([x, y, z], kind, faces, op, share, pick) in sites.iter() {
                if clip.holds([x, y, z]) {
                    put(Look { kind, faces: Some(faces.map(|k| (k, share))) }, op, share, pick, x, y, z);
                }
            }
            Work { nodes: 1, tests: sites.len() as u64, ..Work::default() }
        }
    }
}

/** Lines of a field a side that are asked together what they draw (Field::meets): a box's own would reach from the ground into the air. */
const LINES: i64 = 32;
/**
 * Sites a side of a field given at each site that are asked together, a
 * cube: as it is read site by site, a part of a line is skipped as well
 * as all of it, and a body filling a share of space is asked only by
 * its edges.
 */
const SITES: i64 = 16;

/**
 * Draws a field into `clip`, a medium keeping a site with the chance a ray
 * crossing it is stopped; its lines along the floors' axis from `floors`
 * up, if any, one all under them not asked. Returns the work.
 */
fn draw_field(field: &Field, lines: &Lines, seed: u64, s: i64, clip: &Box3, floors: Option<&Floors>, put: &mut Put) -> Work {
    let mut work = Work::default();
    let op = field.op;
    let field_seed = hash_text(field.name, seed);
    // A medium's share of a site, from its extinction.
    let medium = |e: f64| share(1.0 - js::exp(-e * CHORD * s as f64));
    let share_of = |extinction: Option<f64>| extinction.map_or(SHARE, medium);
    let picked = |x: i64, y: i64, z: i64| to_pick(hash(field_seed, x * s, y * s, z * s));
    // Block by block of lines along an axis, those it says it draws nothing in skipped; while checking, drawn to see
    // they don't. A field's spans lie along their own; one given at each site reads plans where each site is, so it
    // is the same along any, and is drawn along the floors', if any: what is buried under them is not asked.
    let axis = match lines {
        Lines::At(_) => floors.map_or(0, |floors| floors.axis),
        Lines::Spans { axis, .. } => *axis,
    };
    let ([a, b], (lo, hi)) = (across(axis), (clip.min, clip.max));
    // Where line (u, v) reads dithered plans, in the s × s lines it stands for (Plan::dithered): the same for every
    // field along it, so a line's fields read one point. Along a row of lines the offsets across it are a Weyl
    // sequence (the golden ratio's) from a phase hashed per row, so any run of them is spread evenly: a street along
    // the row is read by as many lines as it covers, give or take one, not by chance. None at level 0.
    let bits = s.trailing_zeros();
    let offset = |u: i64, v: i64| -> [i64; 3] {
        let mut offset = [0; 3];
        if bits > 0 {
            let weyl = |n: i64, row: i64, axis: usize| {
                let phase = hash64(seed, 0x0f00_7041, row, (axis as i64) << 8 | bits as i64);
                ((n as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15).wrapping_add(phase) >> (64 - bits)) as i64 - s / 2
            };
            (offset[a], offset[b]) = (weyl(v, u, a), weyl(u, v, b));
        }
        offset
    };
    let (bottom, top) = (lo[axis] * s, (hi[axis] - 1) * s);
    // Blocks of lines across, each all along them or, given at each site, in parts along them too.
    let (side, along) = match lines {
        Lines::At(_) => (SITES, SITES),
        Lines::Spans { .. } => (LINES, hi[axis] - lo[axis]),
    };
    let mut site = [0i64; 3];
    // A medium's site's parts, kept from site to site.
    let mut parts: Vec<Part> = Vec::new();
    let (mut drew, mut unlike) = (None, None);
    let mut record = |_: Look, _: Op, share: u32, _: u32, x: i64, y: i64, z: i64| drew = drew.or((share > 0).then_some([x, y, z]));
    let blocks = (lo[a]..hi[a]).step_by(side as usize).flat_map(|u| (lo[b]..hi[b]).step_by(side as usize).map(move |v| (u, v)));
    for (bu, bv, bw) in blocks.flat_map(|(u, v)| (lo[axis]..hi[axis]).step_by(along.max(1) as usize).map(move |w| (u, v, w))) {
        let (to_u, to_v, to_w) = ((bu + side).min(hi[a]), (bv + side).min(hi[b]), (bw + along).min(hi[axis]));
        let mut block = Box3 { min: [0; 3], max: [0; 3] };
        (block.min[a], block.max[a], block.min[b], block.max[b]) = (bu * s, (to_u - 1) * s + 1, bv * s, (to_v - 1) * s + 1);
        (block.min[axis], block.max[axis]) = (bw * s, (to_w - 1) * s + 1);
        let meets = field.meets_in(&block, s);
        if meets == Meets::Nothing && !checking() {
            continue;
        }
        let put: &mut Put = if meets == Meets::Nothing { &mut record } else { &mut *put };
        // All of a kind, not asked; while checking, asked and held to it.
        let all = match meets {
            Meets::All(kind) if !checking() => Some(kind),
            _ => None,
        };
        for u in bu..to_u {
            for v in bv..to_v {
                site[a] = u;
                site[b] = v;
                // The line's first site in the block: along the floors' axis, its floor.
                let first = match floors {
                    Some(floors) if axis == floors.axis => floors.floor(site).max(bw),
                    _ => bw,
                };
                if first >= to_w {
                    continue;
                }
                match lines {
                    Lines::At(at) => {
                        let mut w = first + ((first + u + v) & 1);
                        while w < to_w {
                            site[axis] = w;
                            let [x, y, z] = site;
                            let value = all.map(|kind| (kind, None)).or_else(|| {
                                work.reads += 1;
                                sites::footprint(s, [0; 3], || at(x * s, y * s, z * s))
                            });
                            if let Meets::All(kind) = meets
                                && value != Some((kind, None))
                            {
                                unlike = unlike.or(Some((site, kind)));
                            }
                            if let Some((kind, extinction)) = value {
                                put(kind.into(), op, share_of(extinction), picked(x, y, z), x, y, z);
                            }
                            w += 2;
                        }
                    }
                    Lines::Spans { spans, .. } => {
                        let spans = match all {
                            // Past the sites, so that the top ones are whole too.
                            Some(kind) => vec![Span::solid(DEEP, top + 2 * s, kind)],
                            None => {
                                work.reads += 1;
                                sites::footprint(s, offset(u, v), || spans(u * s, v * s))
                            }
                        };
                        if let Meets::All(kind) = meets {
                            // Checking: it is all `kind` as it says, a solid span of it through and nothing else in it.
                            let through = spans
                                .iter()
                                .any(|span| span.kind == kind && span.holds == Holds::Solid && span.from < bottom && span.to > top);
                            let alone = spans
                                .iter()
                                .all(|span| span.from >= span.to || span.from > top || (span.kind == kind && span.holds == Holds::Solid));
                            if !(through && alone) && unlike.is_none() {
                                site[axis] = lo[axis];
                                unlike = Some((site, kind));
                            }
                        }
                        // Solid spans cover the line's sites together, as they lie along it.
                        let (solid, thin): (Vec<&Span>, Vec<&Span>) = spans.iter().partition(|span| span.holds == Holds::Solid);
                        cover(&solid, axis, [u, v], (first, hi[axis]), s, op, &mut |look, share, w| {
                            site[axis] = w;
                            put(look, op, share, picked(site[0], site[1], site[2]), site[0], site[1], site[2]);
                        });
                        // A medium is in every site its span reaches, whole or in part.
                        for span in thin {
                            let Holds::Medium(e) = span.holds else { unreachable!("solid spans are covered") };
                            let first = ((span.from - s).div_euclid(s) + 1).max(first);
                            let mut w = first + ((first + u + v) & 1);
                            while w * s < span.to + s && w < hi[axis] {
                                let at = |end: i64| ((end - w * s) as f64 / s as f64).clamp(-1.0, 1.0);
                                let own = (at(span.from), at(span.to));
                                site[axis] = w;
                                let pick = picked(site[0], site[1], site[2]);
                                if own == (-1.0, 1.0) {
                                    put(span.kind.into(), op, medium(e), pick, site[0], site[1], site[2]);
                                } else if own.0 < own.1 {
                                    parts.clear();
                                    parts.extend(solid.iter().map(|other| {
                                        ((at(other.from), at(other.to)), if other.from < span.from { Lies::Under } else { Lies::Over })
                                    }));
                                    parts.push((own, Lies::It));
                                    parts.retain(|((from, to), _)| from < to);
                                    parts.sort_by(|((p, _), _), ((q, _), _)| p.total_cmp(q));
                                    let [down, up, along] = held(&parts, e * CHORD * s as f64);
                                    let kind = span.kind;
                                    let faces = Faces::along(axis, [(kind, up), (kind, along), (kind, down)]);
                                    // A see-through kind has one share for every face (VoxelGrid's `medium`): its most across the line.
                                    put(Look { kind, faces: Some(faces) }, op, down.max(up), pick, site[0], site[1], site[2]);
                                }
                                w += 2;
                            }
                        }
                    }
                }
            }
        }
    }
    if let Some(site) = drew {
        broke(field.name, site, s, clip, "draws where it says it may not");
    }
    if let Some((site, kind)) = unlike {
        broke(field.name, site, s, clip, &format!("isn't all kind {kind}, as it says"));
    }
    work
}

/** Where a part of a voxel lies to the medium `held` is asked about, along their line: under it, over it, or it. */
#[derive(Clone, Copy, PartialEq)]
enum Lies {
    Under,
    It,
    Over,
}

/** A span's part of a voxel, −1 to 1 along its line, and where it lies to the medium `held` is asked about. */
type Part = ((f64, f64), Lies);

/**
 * What a medium holds of a site (of SHARE) through its faces: those the
 * line comes in by from above, those from below, and those along it.
 * `parts` are the line's spans' parts of the voxel, the medium's among
 * them, none empty, in order of where they start, and `depth` how much
 * the medium stops along a mean chord's length.
 *
 * It stops the chords that cross it, each as far as it runs in it: across
 * the line as a ray along the line does (`crossed`), along the line all
 * its way, as it stays in the medium. Chords that end in the solids
 * behind it, short of it, show it as a ray along the line does too: a
 * ground shows only by its top, which the medium is on. Chords that meet
 * the solids in front of it show none. Along the line, in front is over it.
 */
fn held(parts: &[Part], depth: f64) -> [u32; 3] {
    let own = parts.iter().find_map(|&(part, lies)| (lies == Lies::It).then_some(part)).expect("the medium among its parts");
    let stops = |length: f64| 1.0 - js::exp(-depth * length);
    let (crossing, length) = crossed(own.0, own.1);
    let stopped = stops(length);
    // Of the chords crossing planes across the line, and those within them: the share that meets any part, the
    // share that crosses the medium, and what it stops of those.
    let ways = [true, false].map(|across| {
        let meet = slabs(across, parts.iter().map(|p| p.0), false);
        if across { (meet, crossing, stopped) } else { (meet, slabs(false, [own], false), stops(1.0)) }
    });
    // Those crossing it, as far as each runs in it; the rest that meet it or behind it, as far as it lies along the line.
    let on = |across: bool, front: Lies| {
        let (meet, crossing, stops) = ways[usize::from(!across)];
        let in_front = slabs(across, parts.iter().filter(|p| p.1 == front).map(|p| p.0), false);
        share(crossing * stops + (meet - in_front - crossing).max(0.0) * stopped)
    };
    [on(true, Lies::Over), on(true, Lies::Under), on(false, Lies::Over)]
}

/**
 * Puts the sites `s` apart of a line along `axis`, at `line` across it,
 * within [lo, hi), as its solid `spans` cover each: the share of the
 * voxel's chords along each face's direction that they stop, or as a
 * carve frees (`slabs`). A face across the axis sees the span its rays
 * meet first, the others the one that fills most of the site. At every
 * level alike: a site holds what its part of the spans looks like, whole
 * only if they hold all of its voxel.
 */
fn cover(spans: &[&Span], axis: usize, line: [i64; 2], (lo, hi): (i64, i64), s: i64, op: Op, put: &mut dyn FnMut(Look, u32, i64)) {
    let (Some(from), Some(to)) = (spans.iter().map(|span| span.from).min(), spans.iter().map(|span| span.to).max()) else { return };
    // The sites whose voxels, s each way along the axis, meet [from, to].
    let first = ((from - s).div_euclid(s) + 1).max(lo);
    let mut w = first + ((first + line[0] + line[1]) & 1);
    // Kept from site to site: a line has thousands.
    let mut parts: Vec<(f64, f64, Kind)> = Vec::new();
    let mut ends: Vec<(f64, f64)> = Vec::new();
    while w * s < to + s && w < hi {
        let at = |end: i64| ((end - w * s) as f64 / s as f64).clamp(-1.0, 1.0);
        // Each span's part of the voxel, −1 to 1 along the axis, in order.
        parts.clear();
        parts.extend(spans.iter().map(|span| (at(span.from), at(span.to), span.kind)).filter(|p| p.0 < p.1));
        // Whole in one span, as most sites under the ground: all of it, to every face. So are the sites after it, up
        // to where that span ends or the next starts: put without being measured.
        if op != Op::Carve
            && let [(-1.0, 1.0, kind)] = parts[..]
        {
            let top = (w + 1) * s;
            let ends = spans.iter().filter(|span| span.from < span.to).map(|span| if span.from >= top { span.from } else { span.to });
            let until = ends.filter(|&end| end >= top).min().expect("the span it is whole in");
            while w < hi && (w + 1) * s <= until {
                put(kind.into(), SHARE, w);
                w += 2;
            }
            continue;
        }
        parts.sort_by(|p, q| p.0.total_cmp(&q.0));
        if let (Some(bottom), Some(top)) = (parts.first(), parts.iter().max_by(|p, q| p.1.total_cmp(&q.1))) {
            let most = parts.iter().max_by(|p, q| (p.1 - p.0).total_cmp(&(q.1 - q.0))).expect("a part").2;
            ends.clear();
            ends.extend(parts.iter().map(|p| (p.0, p.1)));
            let [across, along] = [true, false].map(|across| share(slabs(across, ends.iter().copied(), op == Op::Carve)));
            let faces = Faces::along(axis, [(bottom.2, across), (most, along), (top.2, across)]);
            put(Look { kind: most, faces: Some(faces) }, across.max(along), w);
        }
        w += 2;
    }
}

/**
 * Draws a field's shapes, those it gives near a box holding `clip` (sites
 * s apart), into it, each as a thing's shape is drawn (draw_shape), but
 * over the world, in the order given, with picks of their own (from their
 * bounds), so whichever box asked; returns the work.
 */
fn draw_field_shapes(field: &Field, shapes: &[Shape], seed: u64, s: i64, clip: &Box3, put: &mut Put) -> Work {
    // Those reaching it as what it meets is asked (Field::meets_in): two sites wider each way.
    let units = units_of(clip, s).widen(2 * s);
    let field_seed = hash_text(field.name, seed);
    let mut work = Work::default();
    for shape in shapes.iter().filter(|shape| shape.bounds.overlaps(&units)) {
        assert!(
            shape.op == field.op,
            "field {}: a shape drawn other than by its field's op (Field::shapes); see \"Model\" in {DOCS}",
            field.name
        );
        let b = &shape.bounds;
        let picks = hash64(hash64(field_seed, b.min[0], b.min[1], b.min[2]), b.max[0], b.max[1], b.max[2]);
        work.shapes += 1;
        work.tests += draw_shape(shape, picks, s, clip, put);
    }
    work
}

thread_local! {
    /** While `checking_skipped` runs: what drew where it said it may not. */
    static CHECKING: RefCell<Option<Vec<String>>> = const { RefCell::new(None) };
}

/**
 * Runs `f`, what it generates that says it may not draw in a box (a
 * field's lines about their ground) drawn there all the same: returns
 * where it drew.
 */
pub fn checking_skipped<R>(f: impl FnOnce() -> R) -> (R, Vec<String>) {
    let outer = CHECKING.replace(Some(Vec::new()));
    let value = f();
    (value, CHECKING.replace(outer).unwrap_or_default())
}

/** Whether `checking_skipped` runs. */
pub(super) fn checking() -> bool {
    CHECKING.with_borrow(Option::is_some)
}

/** Keeps, while checking, that what `key` names, at `site`, sites `s` apart, in `clip`, does other than it says (`what`). */
pub(super) fn broke(key: &str, site: [i64; 3], s: i64, clip: &Box3, what: &str) {
    let message = format!("{key} {what}: at site {site:?}, sites {s} apart, in {:?} (lattice units)", units_of(clip, s));
    CHECKING.with_borrow_mut(|broken| broken.as_mut().expect("checking").push(message));
}

/** The lattice units of the sites `s` apart of `clip`. */
pub(super) fn units_of(clip: &Box3, s: i64) -> Box3 {
    Box3 { min: clip.min.map(|v| v * s), max: clip.max.map(|v| (v - 1) * s + 1) }
}
