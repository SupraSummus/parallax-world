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
                tests += draw_shape(shape, seed, s, clip, &mut |share, cover, rays, pick, x, y, z| {
                    put(Look { cover, rays, ..shape.kind.into() }, shape.op, share, pick, x, y, z)
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
    // A medium's share of a site, from its extinction: what it stops of the rays along the site's lines.
    // The same medium throughout a span: its share worked out once.
    let last = std::cell::Cell::new((f64::NAN, 0));
    let medium = |e: f64| {
        if last.get().0 != e {
            last.set((e, share(grid::medium_stops(e, s as f64, None))));
        }
        last.get().1
    };
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
        let mut centres = std::collections::HashMap::new();
        let mut columns: std::collections::HashMap<(i64, i64), Vec<Span>> = std::collections::HashMap::new();
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
                            // What the field gives at a point (lattice units, in halves of the spacing from the site's
                            // middle, nearest a whole unit), each read once, as the site's column reads it.
                            let mut read = |halves: [i64; 3], work: &mut Work| {
                                let p = [0, 1, 2].map(|i| {
                                    let twice = 2 * site[i] * s + halves[i] * s;
                                    twice.div_euclid(2) + i64::from(twice.rem_euclid(2) == 1 && twice.div_euclid(2) & 1 == 1)
                                });
                                all.map(|kind| (kind, None)).or_else(|| {
                                    *centres.entry(p).or_insert_with(|| {
                                        work.reads += 1;
                                        plan::footprint(s, offset(x, z), || at(p[0], p[1], p[2]))
                                    })
                                })
                            };
                            let value = read([0; 3], &mut work);
                            if let Meets::All(kind) = meets
                                && value != Some((kind, None))
                            {
                                unlike = unlike.or(Some((site, kind)));
                            }
                            // Where it gives the same at its neighbours' middles and its voxel's corners, the site is all
                            // of it; else each of its lines reads it.
                            let neighbours = VOXEL_FACES.iter().map(|f| f.map(|v| 2 * v as i64));
                            let axes = (0..3).flat_map(|i| [-2, 2].map(|v| std::array::from_fn(|j| if j == i { v } else { 0 })));
                            let corners = (0..8).map(|c| [0, 1, 2].map(|i| if c >> i & 1 == 1 { 1 } else { -1 }));
                            let edge = neighbours.chain(axes).chain(corners).any(|h| read(h, &mut work) != value);
                            if !edge {
                                if let Some((kind, extinction)) = value {
                                    put(kind.into(), op, share_of(extinction), picked(x, y, z), x, y, z);
                                }
                            } else {
                                // Each of its lines read along it as far apart as the lines are, nearest a whole
                                // unit, halves to even (no way favoured): each value where it is read.
                                let lines = grid::lines();
                                let mut parts: Vec<((Kind, Option<f64>), grid::Segments)> = Vec::new();
                                for (l, &(axis, q, [t0, t1])) in lines.lines.iter().enumerate() {
                                    let l = l as u16;
                                    let n = ((t1 - t0) * grid::ACROSS as f64).ceil().max(1.0) as usize;
                                    let step = (t1 - t0) / n as f64;
                                    for j in 0..n {
                                        let t = t0 + (j as f64 + 0.5) * step;
                                        let p = [0, 1, 2].map(|i| {
                                            ((site[i] * s) as f64 + s as f64 * (q[i] + t * lines.dirs[axis][i])).round_ties_even() as i64
                                        });
                                        work.reads += 1;
                                        let Some(v) = plan::footprint(s, offset(x, z), || at(p[0], p[1], p[2])) else { continue };
                                        let at = parts.iter().position(|(w, _)| *w == v).unwrap_or_else(|| {
                                            parts.push((v, Vec::new()));
                                            parts.len() - 1
                                        });
                                        let segments = &mut parts[at].1;
                                        // Its ends shared with its neighbours', so a piece and the next join.
                                        let (from, to) = ((t0 + j as f64 * step) as f32, (t0 + (j + 1) as f64 * step) as f32);
                                        match segments.last_mut() {
                                            Some(last) if last.0 == l && last.2 >= from => last.2 = to,
                                            _ => segments.push((l, from, to)),
                                        }
                                    }
                                }
                                for ((kind, extinction), segments) in parts {
                                    let pick = picked(x, y, z);
                                    match extinction {
                                        None => {
                                            let filled = share(grid::filled(&segments));
                                            put(Look { rays: Some(keep(segments)), ..kind.into() }, op, filled, pick, x, y, z);
                                        }
                                        Some(e) => {
                                            put(kind.into(), op, share(grid::medium_stops(e, s as f64, Some(&segments))), pick, x, y, z)
                                        }
                                    }
                                }
                            }
                            w += 2;
                        }
                    }
                    Lines::Spans(spans_of) => {
                        let spans = match all {
                            Some(kind) => vec![Span::solid(DEEP, top + s, kind)],
                            None => {
                                // A line along x or z reads plans at one z or x: dithered across it only.
                                let mut moved = offset(site[0], site[2]);
                                if axis != 1 {
                                    moved[axis / 2] = 0;
                                }
                                // Along y, as its neighbours read it, once.
                                let mut read = || {
                                    work.reads += 1;
                                    plan::footprint(s, moved, || spans_of(u * s, v * s))
                                };
                                if axis == 1 { columns.entry((u, v)).or_insert_with(read).clone() } else { read() }
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
                        if axis == 1 {
                            let mut column = |c: (i64, i64), work: &mut Work| -> Vec<Span> {
                                if all.is_some() {
                                    return spans.clone();
                                }
                                columns
                                    .entry(c)
                                    .or_insert_with(|| {
                                        work.reads += 1;
                                        plan::footprint(s, offset(c.0, c.1), || spans_of(c.0 * s, c.1 * s))
                                    })
                                    .clone()
                            };
                            // The nine columns a site's voxel reaches into, its own in the middle.
                            let around: Vec<((i64, i64), Vec<Span>)> = (-1..=1)
                                .flat_map(|i| (-1..=1).map(move |j| (u + i, v + j)))
                                .map(|c| (c, if c == (u, v) { spans.clone() } else { column(c, &mut work) }))
                                .collect();
                            let solid = |span: &&Span| span.holds == Holds::Solid && span.from < span.to;
                            // Where any of them changes: a site with none in its reach is each column's throughout.
                            let mut cuts: Vec<i64> =
                                around.iter().flat_map(|(_, spans)| spans.iter().filter(solid).flat_map(|sp| [sp.from, sp.to])).collect();
                            cuts.sort_unstable();
                            cuts.dedup();
                            // Between two cuts each column is one kind or none: all nine one kind, all none, or mixed.
                            let between = |y: i64| {
                                let mut kinds = around
                                    .iter()
                                    .map(|(_, spans)| spans.iter().filter(solid).find(|sp| sp.from <= y && sp.to > y).map(|sp| sp.kind));
                                let first = kinds.next().flatten();
                                if kinds.all(|k| k == first) { Ok(first) } else { Err(()) }
                            };
                            let (mut w, mut at) = (lo[axis] + ((lo[axis] + u + v) & 1), 0);
                            let mut known: Option<(usize, Result<Option<Kind>, ()>)> = None;
                            while w < hi[axis] {
                                site[axis] = w;
                                let [x, y, z] = site;
                                let (y0, y1) = (w * s - s, w * s + s);
                                while at < cuts.len() && cuts[at] <= y0 {
                                    at += 1;
                                }
                                let state = if cuts.get(at).is_none_or(|&c| c >= y1) {
                                    let state = match known {
                                        Some((k, state)) if k == at => state,
                                        _ => between(y0),
                                    };
                                    known = Some((at, state));
                                    state
                                } else {
                                    Err(())
                                };
                                match state {
                                    Ok(Some(kind)) => put(kind.into(), op, SHARE, picked(x, y, z), x, y, z),
                                    // Nothing up to the next cut: on to the first site reaching it.
                                    Ok(None) => match cuts.get(at) {
                                        Some(&c) => {
                                            let next = (c - s).div_euclid(s) + 1;
                                            w = w.max(next + ((next + u + v) & 1)) - 2;
                                        }
                                        None => break,
                                    },
                                    Err(()) => {
                                        for (kind, segments) in span_segments(&around, site, s) {
                                            let filled = share(grid::filled(&segments));
                                            put(Look { rays: Some(keep(segments)), ..kind.into() }, op, filled, picked(x, y, z), x, y, z);
                                        }
                                    }
                                }
                                w += 2;
                            }
                        }
                        for span in spans.iter().filter(|span| axis != 1 || span.holds != Holds::Solid) {
                            // s is a power of two: shifting is dividing, rounded down.
                            let first = (-((-span.from) >> s.trailing_zeros())).max(lo[axis]);
                            let mut w = first + ((first + u + v) & 1);
                            let (op, share) = match span.holds {
                                Holds::Solid => (op, SHARE),
                                Holds::Medium(e) => (op, medium(e)),
                                Holds::Coat(c) => (Op::Paint, share(c)),
                            };
                            while w * s < span.to && w < hi[axis] {
                                site[axis] = w;
                                put(span.kind.into(), op, share, picked(site[0], site[1], site[2]), site[0], site[1], site[2]);
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
 * Of each of a site's lines (grid's `lines`), its pieces in the columns
 * around the site's, along y, sites s apart (each s × s about its sites'
 * line): from and to along it, and which column, of the nine by x then
 * z, its own the middle. The same for every site: columns are where
 * sites are.
 */
fn column_pieces() -> &'static Vec<Vec<(f64, f64, usize)>> {
    static PIECES: std::sync::OnceLock<Vec<Vec<(f64, f64, usize)>>> = std::sync::OnceLock::new();
    PIECES.get_or_init(|| {
        let lines = grid::lines();
        lines
            .lines
            .iter()
            .map(|&(axis, q, [t0, t1])| {
                let d = lines.dirs[axis];
                let at = |t: f64, i: usize| q[i] + t * d[i];
                // Where the line crosses from column to column, along x and z: half a spacing past a column's line.
                let mut cuts = vec![t0, t1];
                for i in [0, 2] {
                    if d[i].abs() > 1e-12 {
                        let [a, b] = [at(t0, i), at(t1, i)];
                        for k in (a.min(b) - 0.5).ceil() as i64..=(a.max(b) - 0.5).floor() as i64 {
                            cuts.push((k as f64 + 0.5 - q[i]) / d[i]);
                        }
                    }
                }
                cuts.retain(|t| (t0..=t1).contains(t));
                cuts.sort_by(f64::total_cmp);
                cuts.windows(2)
                    .filter(|pair| pair[1] > pair[0])
                    .map(|pair| {
                        let mid = (pair[0] + pair[1]) / 2.0;
                        let [i, j] = [0, 2].map(|i| at(mid, i).round() as i64);
                        assert!(i.abs() <= 1 && j.abs() <= 1, "a voxel within the columns around its site's");
                        (pair[0], pair[1], ((i + 1) * 3 + j + 1) as usize)
                    })
                    .collect()
            })
            .collect()
    })
}

/**
 * Where on the lines of `site` (sites `s` apart, grid's `lines`) the
 * solid spans along y of the columns around it lie, by kind: each line
 * through the columns it crosses, each column s × s around its sites'
 * line, in each the part of the line whose height is in a span of it.
 */
fn span_segments(around: &[((i64, i64), Vec<Span>)], site: [i64; 3], s: i64) -> Vec<(Kind, grid::Segments)> {
    let lines = grid::lines();
    let (centre, s) = (site.map(|v| (v * s) as f64), s as f64);
    // Of each column (around's order: by x, then z, its own the middle), its solid spans the voxel's heights reach,
    // in spacings from the site.
    let near: Vec<Vec<(f64, f64, Kind)>> = around
        .iter()
        .map(|(_, spans)| {
            let (from, to) = (centre[1] - s, centre[1] + s);
            let reach = |sp: &&Span| (sp.from as f64) < to && (sp.to as f64) > from;
            let solid = |sp: &&Span| sp.holds == Holds::Solid && sp.from < sp.to;
            let rel = |y: i64| (y as f64 - centre[1]) / s;
            spans.iter().filter(solid).filter(reach).map(|sp| (rel(sp.from), rel(sp.to), sp.kind)).collect()
        })
        .collect();
    let mut kinds: Vec<(Kind, grid::Segments)> = Vec::new();
    for (l, pieces) in column_pieces().iter().enumerate() {
        let (axis, q, _) = lines.lines[l];
        let dy = lines.dirs[axis][1];
        let per = 1.0 / dy;
        for &(ta, tb, column) in pieces {
            for &(from, to, kind) in &near[column] {
                // from ≤ y(t) < to along the part of the line in this column.
                let [a, b] = if dy.abs() < 1e-12 {
                    if (from..to).contains(&q[1]) { [ta, tb] } else { continue }
                } else {
                    let [u, w] = [from, to].map(|y| (y - q[1]) * per);
                    [u.min(w).max(ta), u.max(w).min(tb)]
                };
                if a < b {
                    let at = kinds.iter().position(|(k, _)| *k == kind).unwrap_or_else(|| {
                        kinds.push((kind, Vec::with_capacity(2 * lines.lines.len())));
                        kinds.len() - 1
                    });
                    let (l, a, b) = (l as u16, a as f32, b as f32);
                    // Joined to the last where they touch, as across columns of one span they mostly do.
                    match kinds[at].1.last_mut() {
                        Some(last) if last.0 == l && last.2 >= a && last.1 <= a => last.2 = last.2.max(b),
                        _ => kinds[at].1.push((l, a, b)),
                    }
                }
            }
        }
    }
    // By line, each line's in order, those touching joined.
    for (_, segments) in &mut kinds {
        if !segments.is_sorted_by(|a, b| (a.0, a.1) <= (b.0, b.1)) {
            segments.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.total_cmp(&b.1)));
        }
        segments.dedup_by(|next, kept| {
            let touch = kept.0 == next.0 && kept.2 >= next.1;
            if touch {
                kept.2 = kept.2.max(next.2);
            }
            touch
        });
    }
    kinds
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
        work.tests += draw_shape(shape, picks, s, clip, &mut |share, cover, rays, pick, x, y, z| {
            put(Look { cover, rays, ..shape.kind.into() }, shape.op, share, pick, x, y, z)
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
