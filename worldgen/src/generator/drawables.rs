//! What is drawn into a box: drawables, each a field, a thing's shapes or
//! its sites, put site by site; and, while checking, where one drew that
//! said it may not (`checking_skipped`).

use super::*;

/** A thing's sites of a level: each what it is of, how it meets the world, its share and pick (Put). */
pub(super) type Sites = Vec<([i64; 3], Look, Op, u32, u32)>;

pub(super) enum Draw<'a> {
    Field(&'a Field),
    /** A node's shapes, and whether they are its far ones. */
    Shapes(Vec<Shape>, bool),
    /** A thing measured: its sites. */
    Sites(Sites),
}

pub(super) struct Drawable<'a> {
    pub(super) order: f64,
    pub(super) key: String,
    /** The layer it is of, for its cost. */
    pub(super) layer: &'static str,
    /** The thing it is part of, numbered from 1; 0 for fields. */
    pub(super) thing: u32,
    pub(super) draw: Draw<'a>,
    /** Drawn only to see it draws nothing: its population said it stands elsewhere (`checking_skipped`). */
    pub(super) checked: bool,
}

/** Draws one drawable at sites `s` apart into `clip`; returns the work. */
pub(super) fn draw(drawable: &Drawable, seed: u64, s: i64, clip: &Box3, put: &mut Put) -> Work {
    match &drawable.draw {
        Draw::Field(field) => draw_field(field, seed, s, clip, put),
        Draw::Shapes(shapes, far) => {
            let mut tests = 0;
            for (i, shape) in shapes.iter().enumerate() {
                let seed = hash_text(&format!("{}#{}{i}", drawable.key, if *far { "far" } else { "" }), seed);
                tests += draw_shape(shape, seed, s, clip, &mut |share, cover, pick, x, y, z| {
                    put(Look { cover, ..shape.kind.into() }, shape.op, share, pick, x, y, z)
                });
            }
            Work { nodes: 1, tests, ..Work::default() }
        }
        Draw::Sites(sites) => {
            for &([x, y, z], look, op, share, pick) in sites {
                if clip.holds([x, y, z]) {
                    put(look, op, share, pick, x, y, z);
                }
            }
            Work { nodes: 1, tests: sites.len() as u64, ..Work::default() }
        }
    }
}

/** Lines of a field a side that are asked together what they draw (Field::meets): a box's own would reach from the ground into the air. */
const LINES: i64 = 32;

/** Draws a field into `clip`, a medium keeping a site with the chance a ray crossing it is stopped; returns the work. */
fn draw_field(field: &Field, seed: u64, s: i64, clip: &Box3, put: &mut Put) -> Work {
    let mut work = Work::default();
    let op = field.op;
    let field_seed = hash_text(field.name, seed);
    // A medium's share of a site, from its extinction.
    let medium = |e: f64| share(1.0 - js::exp(-e * CHORD * s as f64));
    let share_of = |extinction: Option<f64>| extinction.map_or(SHARE, medium);
    let picked = |x: i64, y: i64, z: i64| to_pick(hash(field_seed, x * s, y * s, z * s));
    // Where column (i, j) of sites reads dithered plans, in the s × s columns it stands for (Plan::dithered): the same
    // for every field, so a column's fields read one point. Along a row of columns the offsets across it are a Weyl
    // sequence (the golden ratio's) from a phase hashed per row, so any run of them is spread evenly: a street along
    // the row is read by as many columns as it covers, give or take one, not by chance. None at level 0.
    let bits = s.trailing_zeros();
    let offset = |i: i64, j: i64| -> [i64; 2] {
        if bits == 0 {
            return [0, 0];
        }
        let weyl = |n: i64, row: i64, axis: i64| {
            let phase = hash64(seed, 0x0f00_7041, row, axis << 8 | bits as i64);
            ((n as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15).wrapping_add(phase) >> (64 - bits)) as i64 - s / 2
        };
        [weyl(j, i, 0), weyl(i, j, 2)]
    };
    // Block by block of lines along an axis (a field's spans', or y for one given at each site), those it says it
    // draws nothing in skipped; while checking, drawn to see they don't.
    /** What a field gives along its lines. */
    enum Lines<'a> {
        At(&'a FieldAt),
        Spans(&'a FieldSpans),
    }
    let (axis, lines) = match &field.values {
        FieldValues::At(at) => (1, Lines::At(at)),
        FieldValues::Spans { axis, spans } => (*axis, Lines::Spans(spans)),
        FieldValues::Shapes(near) => return draw_field_shapes(field, near, seed, s, clip, put),
    };
    let ([a, b], (lo, hi)) = (across(axis), (clip.min, clip.max));
    let (bottom, top) = (lo[axis] * s, (hi[axis] - 1) * s);
    let tie = Tie::new(s);
    let mut site = [0i64; 3];
    let (mut drew, mut unlike) = (None, None);
    let mut record = |_: Look, _: Op, share: u32, _: u32, x: i64, y: i64, z: i64| drew = drew.or((share > 0).then_some([x, y, z]));
    for (bu, bv) in (lo[a]..hi[a]).step_by(LINES as usize).flat_map(|u| (lo[b]..hi[b]).step_by(LINES as usize).map(move |v| (u, v))) {
        let (to_u, to_v) = ((bu + LINES).min(hi[a]), (bv + LINES).min(hi[b]));
        let mut block = Box3 { min: [0; 3], max: [0; 3] };
        (block.min[a], block.max[a], block.min[b], block.max[b]) = (bu * s, (to_u - 1) * s + 1, bv * s, (to_v - 1) * s + 1);
        (block.min[axis], block.max[axis]) = (bottom, top + 1);
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
                match lines {
                    Lines::At(at) => {
                        let mut w = lo[axis] + ((lo[axis] + u + v) & 1);
                        while w < hi[axis] {
                            site[axis] = w;
                            let [x, y, z] = site;
                            let value = all.map(|kind| (kind, None)).or_else(|| {
                                work.reads += 1;
                                plan::footprint(s, offset(x, z), || at(x * s, y * s, z * s))
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
                    Lines::Spans(spans) => {
                        let spans = match all {
                            // Past the sites, so that the top ones are whole too.
                            Some(kind) => vec![Span::solid(DEEP, top + 2 * s, kind)],
                            None => {
                                // A line along x or z reads plans at one z or x: dithered across it only.
                                let mut moved = offset(site[0], site[2]);
                                if axis != 1 {
                                    moved[axis / 2] = 0;
                                }
                                work.reads += 1;
                                plan::footprint(s, moved, || spans(u * s, v * s))
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
                        cover(&solid, axis, [u, v], (lo[axis], hi[axis]), s, op, &mut |look, share, w| {
                            site[axis] = w;
                            put(look, op, share, picked(site[0], site[1], site[2]), site[0], site[1], site[2]);
                        });
                        // A medium or a coat, thickened to hold a site (Column), is in the sites whose middles it holds.
                        for span in thin {
                            // s is a power of two: shifting is dividing, rounded down.
                            let first = (-((-span.from) >> s.trailing_zeros())).max(lo[axis]);
                            let mut w = first + ((first + u + v) & 1);
                            let (op, share) = match span.holds {
                                Holds::Medium(e) => (op, medium(e)),
                                Holds::Coat(c) => (Op::Paint, share(c)),
                                Holds::Solid => unreachable!("solid spans are covered"),
                            };
                            while w * s <= span.to && w < hi[axis] {
                                site[axis] = w;
                                if tie.within(axis, site.map(|c| c * s), span.from, span.to, 1) {
                                    put(span.kind.into(), op, share, picked(site[0], site[1], site[2]), site[0], site[1], site[2]);
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
        // Whole in one span, as most sites under the ground: all of it, to every face.
        if op != Op::Carve
            && let [(-1.0, 1.0, kind)] = parts[..]
        {
            put(kind.into(), SHARE, w);
            w += 2;
            continue;
        }
        parts.sort_by(|p, q| p.0.total_cmp(&q.0));
        if let (Some(bottom), Some(top)) = (parts.first(), parts.iter().max_by(|p, q| p.1.total_cmp(&q.1))) {
            let most = parts.iter().max_by(|p, q| (p.1 - p.0).total_cmp(&(q.1 - q.0))).expect("a part").2;
            let faces: [Kind; FACE_KINDS] = std::array::from_fn(|k| match VOXEL_FACES[k][axis] {
                1 => top.2,
                -1 => bottom.2,
                _ => most,
            });
            ends.clear();
            ends.extend(parts.iter().map(|p| (p.0, p.1)));
            let [across, along] = [true, false].map(|across| share(slabs(across, &ends, op == Op::Carve)));
            let cover = (across != along).then(|| std::array::from_fn(|k| if VOXEL_FACES[k][axis] == 0 { along } else { across }));
            put(Look { kind: most, faces: faces.iter().any(|&k| k != most).then_some(faces), cover }, across.max(along), w);
        }
        w += 2;
    }
}

/**
 * Draws a field's shapes into `clip` (sites s apart), each as a thing's
 * shape is drawn (draw_shape), but over the world, in the order `near`
 * gives them, with picks of their own (from their bounds), so whichever
 * box asked; returns the work.
 */
fn draw_field_shapes(field: &Field, near: &FieldShapes, seed: u64, s: i64, clip: &Box3, put: &mut Put) -> Work {
    // Asked as what it meets is (Field::meets_in): two sites wider each way.
    let units = units_of(clip, s).widen(2 * s);
    let field_seed = hash_text(field.name, seed);
    let mut work = Work::default();
    for shape in plan::footprint(s, [0, 0], || near(&units)).iter().filter(|shape| shape.bounds.overlaps(&units)) {
        assert!(
            shape.op == field.op,
            "field {}: a shape drawn other than by its field's op (Field::shapes); see \"Model\" in {DOCS}",
            field.name
        );
        let b = &shape.bounds;
        let picks = hash64(hash64(field_seed, b.min[0], b.min[1], b.min[2]), b.max[0], b.max[1], b.max[2]);
        work.shapes += 1;
        work.tests += draw_shape(shape, picks, s, clip, &mut |share, cover, pick, x, y, z| {
            put(Look { cover, ..shape.kind.into() }, shape.op, share, pick, x, y, z)
        });
    }
    work
}

thread_local! {
    /** While `checking_skipped` runs: what drew where it said it may not. */
    static CHECKING: RefCell<Option<Vec<String>>> = const { RefCell::new(None) };
}

/**
 * Runs `f`, what it generates that says it may not draw in a box (a
 * field's lines about their ground, a population standing elsewhere)
 * drawn there all the same: returns where it drew.
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
