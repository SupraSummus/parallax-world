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
                tests +=
                    draw_shape(shape, seed, s, clip, &mut |share, pick, x, y, z| put(shape.kind.into(), shape.op, share, pick, x, y, z));
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

/** Lines of a field's spans a side that are asked together whether they may draw (About): a box's own would reach from the ground into the air. */
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
    match &field.values {
        FieldValues::At(at) => {
            for z in clip.min[2]..clip.max[2] {
                for y in clip.min[1]..clip.max[1] {
                    let mut x = clip.min[0] + ((clip.min[0] + y + z) & 1);
                    work.reads += 0.max((clip.max[0] - x + 1) / 2) as u64;
                    while x < clip.max[0] {
                        if let Some((kind, extinction)) = plan::footprint(s, offset(x, z), || at(x * s, y * s, z * s)) {
                            put(kind.into(), op, share_of(extinction), picked(x, y, z), x, y, z);
                        }
                        x += 2;
                    }
                }
            }
        }
        FieldValues::Spans { axis, spans } => {
            let axis = *axis;
            let (lo, hi) = (clip.min, clip.max);
            let others: Vec<usize> = (0..3).filter(|i| *i != axis).collect();
            let (a, b) = (others[0], others[1]);
            let tie = Tie::new(s);
            let mut site = [0i64; 3];
            assert!(axis == 1 || field.about.is_none(), "field {}: spans along y only are about a plan of heights", field.name);
            // Block by block of lines, those that may not meet the clip skipped; while checking, drawn to see they don't.
            let (mut drew, mut unlike) = (None, None);
            let mut record = |_: Look, _: Op, share: u32, _: u32, x: i64, y: i64, z: i64| drew = drew.or((share > 0).then_some([x, y, z]));
            for (bu, bv) in (lo[a]..hi[a]).step_by(LINES as usize).flat_map(|u| (lo[b]..hi[b]).step_by(LINES as usize).map(move |v| (u, v)))
            {
                let (to_u, to_v) = ((bu + LINES).min(hi[a]), (bv + LINES).min(hi[b]));
                let (bottom, top) = (lo[axis] * s, (hi[axis] - 1) * s);
                let meets = field
                    .about
                    .as_ref()
                    .map_or(Meets::Any, |about| about.meets([bu * s, bv * s], [(to_u - 1) * s, (to_v - 1) * s], [bottom, top], s));
                if meets == Meets::Nothing && !checking() {
                    continue;
                }
                let put: &mut Put = if meets == Meets::Nothing { &mut record } else { &mut *put };
                for u in bu..to_u {
                    for v in bv..to_v {
                        site[a] = u;
                        site[b] = v;
                        let (spans, about) = match meets {
                            // Under its ground: all of a kind, not asked.
                            Meets::All(kind) if !checking() => (vec![Span::solid(DEEP, top + s, kind)], None),
                            _ => {
                                // A line along x or z reads plans at one z or x: dithered across it only.
                                let mut moved = offset(site[0], site[2]);
                                if axis != 1 {
                                    moved[axis / 2] = 0;
                                }
                                work.reads += 1;
                                (plan::footprint(s, moved, || spans(u * s, v * s)), field.about.as_ref())
                            }
                        };
                        // What it is about, held to it where asked.
                        if let Some((about, slack)) = about.and_then(|about| Some((about, about.slack(s)?))) {
                            let g = about.of.at(u * s, v * s);
                            let (low, high) = (g - about.below - slack, g + about.above + slack);
                            let under = |span: &Span| about.under == Some(span.kind) && span.holds == Holds::Solid;
                            for span in spans.iter().filter(|span| span.from < span.to) {
                                assert!(
                                    (span.from >= low || under(span)) && span.to <= high,
                                    "field {}: span [{}, {}) at column ({}, {}) reaches past {} under and {} over its ground {g}, \
                                 give or take {slack} at sites {s} apart (Field::about); see \"Rules\" in {DOCS}",
                                    field.name,
                                    span.from,
                                    span.to,
                                    u * s,
                                    v * s,
                                    about.below,
                                    about.above
                                );
                            }
                            assert!(
                                about.under.is_none()
                                    || bottom >= low
                                    || spans.iter().any(|span| under(span) && span.from < bottom && span.to >= low),
                                "field {}: at column ({}, {}), kind {:?} doesn't reach up from the bottom to {low}, {} under its ground {g}, \
                                 give or take {slack} at sites {s} apart (Field::under); see \"Rules\" in {DOCS}",
                                field.name,
                                u * s,
                                v * s,
                                about.under,
                                about.below,
                            );
                        }
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
                        for span in spans {
                            // s is a power of two: shifting is dividing, rounded down.
                            let first = (-((-span.from) >> s.trailing_zeros())).max(lo[axis]);
                            let mut w = first + ((first + u + v) & 1);
                            let (op, share) = match span.holds {
                                Holds::Solid => (op, SHARE),
                                Holds::Medium(e) => (op, medium(e)),
                                Holds::Coat(c) => (Op::Paint, share(c)),
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
            if let Some(site) = drew {
                broke(field.name, site, s, clip, "draws where it says it may not");
            }
            if let Some((site, kind)) = unlike {
                broke(field.name, site, s, clip, &format!("isn't all kind {kind} under its ground, as it says"));
            }
        }
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
