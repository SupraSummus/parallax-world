//! A node's forms, as drawables: near, its shapes and its children's
//! (`near`), or far, its shapes for far or sites measured from its near
//! form (`measure`; `Shadows` for a population's medium), as its spacing
//! calls for (`visit`); and a node drawn alone (`alone`), as measuring
//! draws it.

use super::*;

/** Where a node is drawn: at sites `s` apart, for the lattice units `units`, in its layer's `order`, as part of `thing`. */
pub(super) struct Visit<'u> {
    pub(super) s: i64,
    pub(super) units: &'u Box3,
    pub(super) order: f64,
    pub(super) layer: &'static str,
    pub(super) thing: u32,
    pub(super) seed: u64,
    pub(super) checked: bool,
}

/** Adds a node's drawables and its children's, as far as they reach `at.units` and are drawn at its spacing. */
pub(super) fn visit(node: &Node, key: String, bounds: Box3, at: &Visit, drawables: &mut Vec<Drawable<'_>>) {
    let Visit { s, units, order, layer, thing, seed, checked } = *at;
    if !bounds.overlaps(units) {
        return;
    }
    let draw = match node.far.as_ref().filter(|far| s * ONE > far.detail.to_bits()) {
        Some(Far { detail, form: FarForm::Measured }) => Draw::Sites(measure(node, &key, layer, bounds, detail.to_bits(), s, seed)),
        Some(Far { form: FarForm::Shapes(shapes), .. }) => Draw::Shapes(shapes.to_vec(), true),
        None => return near(node, key, bounds, at, drawables),
    };
    drawables.push(Drawable { order, key, layer, thing, draw, checked });
}

/** Adds a node's near form, whatever the spacing: its shapes, and its children's drawables (`visit`). */
fn near(node: &Node, key: String, bounds: Box3, at: &Visit, drawables: &mut Vec<Drawable<'_>>) {
    let Visit { s, units, order, layer, thing, checked, .. } = *at;
    drawables.push(Drawable { order, key: key.clone(), layer, thing, draw: Draw::Shapes(node.shapes.get().to_vec(), false), checked });
    let reaching;
    let children: &[Child] = match &node.children {
        None => &[],
        Some(Children::Fixed(children)) => children,
        Some(Children::Reaching(children)) => {
            reaching = children(units);
            &reaching
        }
    };
    let mut names = std::collections::HashSet::new();
    for child in children {
        assert!(names.insert(&child.name), "{key} has two children named {}; see \"Things\" in {DOCS}", child.name);
        let child_bounds = bounds_of(&child.node);
        assert!(child_bounds.inside(&bounds), "{key}/{} reaches past its parent; see \"Things\" in {DOCS}", child.name);
        if s <= child.size {
            visit(&child.node, format!("{key}/{}", child.name), child_bounds, at, drawables);
        }
    }
}

/** How a node's drawables are added: `visit`, or `near` for its near form. */
type Form = fn(&Node, String, Box3, &Visit, &mut Vec<Drawable>);

/**
 * A thing's sites, drawn aside from the world: in a box of a level's
 * sites, what it puts at each since it last carved it, put on the world
 * as they came (`each`).
 */
pub(super) struct Own {
    b: Box3,
    n: [i64; 3],
    /** By site, its last put in `puts`; NONE if none. */
    cells: Vec<u32>,
    /** Puts: each what it is of, how it meets the world, its share and pick (Put), and the site's put before it. */
    puts: Vec<(Look, Op, u32, u32, u32)>,
}

const NONE: u32 = u32::MAX;

impl Own {
    pub(super) fn new(b: &Box3) -> Own {
        let n = [0, 1, 2].map(|i| (b.max[i] - b.min[i]).max(0));
        Own { b: *b, n, cells: vec![NONE; (n[0] * n[1] * n[2]) as usize], puts: Vec::new() }
    }

    fn index(&self, x: i64, y: i64, z: i64) -> usize {
        assert!(self.b.holds([x, y, z]), "a thing drew at ({x}, {y}, {z}), outside its bounds {:?}", self.b);
        let [x, y, z] = [x - self.b.min[0], y - self.b.min[1], z - self.b.min[2]];
        (x + self.n[0] * (y + self.n[1] * z)) as usize
    }

    /** Within the thing only: a Carve, where it keeps a voxel, takes out what the thing put there. */
    #[allow(clippy::too_many_arguments)]
    pub(super) fn put(&mut self, look: Look, op: Op, share: u32, pick: u32, x: i64, y: i64, z: i64) {
        let i = self.index(x, y, z);
        if op == Op::Carve {
            if pick < share {
                self.cells[i] = NONE;
            }
            return;
        }
        self.puts.push((look, op, share, pick, self.cells[i]));
        self.cells[i] = (self.puts.len() - 1) as u32;
    }

    /** The puts at index `i` of `cells`, last first. */
    fn puts_at(&self, i: usize) -> impl Iterator<Item = &(Look, Op, u32, u32, u32)> {
        std::iter::successors(self.puts.get(self.cells[i] as usize), |p| self.puts.get(p.4 as usize))
    }

    /** What the thing has at (x, y, z), as `of`. */
    fn at(&self, x: i64, y: i64, z: i64) -> Option<(Kind, Op)> {
        self.of(self.index(x, y, z))
    }

    /**
     * What the thing has at index `i` of `cells`, its voxels kept by chance:
     * its site's kind (measured, its faces are measured anew) and how it meets the world.
     */
    fn of(&self, i: usize) -> Option<(Kind, Op)> {
        // Last first: the last Paint kept, else the first Fill kept.
        let mut fill = None;
        for &(look, op, ..) in self.puts_at(i).filter(|p| p.3 < p.2) {
            match op {
                Op::Paint => return Some((look.kind, op)),
                _ => fill = Some((look.kind, op)),
            }
        }
        fill
    }

    /** Puts what the thing put, as it came. */
    pub(super) fn each(&self, visit: &mut Put) {
        for i in 0..self.cells.len() {
            let puts: Vec<_> = self.puts_at(i).collect();
            let j = i as i64;
            let (x, y, z) =
                (j % self.n[0] + self.b.min[0], (j / self.n[0]) % self.n[1] + self.b.min[1], j / (self.n[0] * self.n[1]) + self.b.min[2]);
            for &&(look, op, share, pick, _) in puts.iter().rev() {
                visit(look, op, share, pick, x, y, z);
            }
        }
    }
}

/** A node of `layer` drawn alone, whole, at sites `f` apart, its drawables added by `form`. */
fn alone(node: &Node, key: &str, layer: &'static str, bounds: Box3, f: i64, seed: u64, form: Form) -> Own {
    let units = bounds.widen(f);
    let mut drawables = Vec::new();
    form(node, key.to_string(), bounds, &Visit { s: f, units: &units, order: 0.0, layer, thing: 1, seed, checked: false }, &mut drawables);
    let fine = Box3 { min: bounds.min.map(|v| v.div_euclid(f) - 2), max: bounds.max.map(|v| -(-v).div_euclid(f) + 2) };
    let mut own = Own::new(&fine);
    let mut work = Work::default();
    for drawable in &drawables {
        work.add(&draw(drawable, seed, f, &fine, &mut |look, op, share, pick, x, y, z| own.put(look, op, share, pick, x, y, z)));
    }
    cost::add(layer, work);
    own
}

/**
 * The pairs of neighbouring columns along an axis that meet an n[0] × n[1]
 * box of them from (0, 0), to `pair`. Seen along an axis, a voxel is a
 * square turned 45° round its column (|u| + |v| ≤ 1, columns 1 apart),
 * twice a column's area: the voxels of the columns whose u + v is even
 * cover the plane once, those of the odd ones once more. So a ray along an
 * axis meets the voxels of two neighbouring columns, one of each, and half
 * a column's area of rays those of each pair: what a thing hides along the
 * axis is half a column's area for each pair either of which holds one of
 * its sites. Counted by columns, a sheet one site thick hid half what it
 * does, and a lone voxel half its shadow.
 */
fn pairs(n: [i64; 2], mut pair: impl FnMut([i64; 2], [i64; 2])) {
    for i in -1..n[0] {
        for j in 0..n[1] {
            pair([i, j], [i + 1, j]);
        }
    }
    for i in 0..n[0] {
        for j in -1..n[1] {
            pair([i, j], [i, j + 1]);
        }
    }
}

/**
 * Columns along an axis, an n[0] × n[1] box of them from (0, 0), each
 * holding a site or not: what a thing hides along the axis.
 */
#[derive(Clone)]
struct Columns {
    n: [i64; 2],
    held: Vec<bool>,
}

impl Columns {
    fn new(n: [i64; 2]) -> Columns {
        Columns { n, held: vec![false; (n[0] * n[1]) as usize] }
    }

    /** Whether column `p` holds a site: none outside the box. */
    fn holds(&self, p: [i64; 2]) -> bool {
        (0..2).all(|k| (0..self.n[k]).contains(&p[k])) && self.held[(p[0] * self.n[1] + p[1]) as usize]
    }

    fn set(&mut self, p: [i64; 2]) {
        self.held[(p[0] * self.n[1] + p[1]) as usize] = true;
    }

    /** Those any of `planes` holds, of the box of the first (none if there are none). */
    fn union<'a>(planes: impl IntoIterator<Item = &'a Columns>) -> Columns {
        let mut planes = planes.into_iter();
        let mut out = planes.next().cloned().unwrap_or_else(|| Columns::new([0, 0]));
        for plane in planes {
            out.held.iter_mut().zip(&plane.held).for_each(|(o, h)| *o |= h);
        }
        out
    }

    /** Rows, each a box one column deep, one after another along the box's first axis. */
    fn stacked<'a>(rows: impl ExactSizeIterator<Item = &'a Columns>) -> Columns {
        let height = rows.len() as i64;
        let held: Vec<bool> = rows.flat_map(|row| row.held.iter().copied()).collect();
        Columns { n: [height, held.len() as i64 / height.max(1)], held }
    }

    /** How many pairs of neighbouring columns (`pairs`) meeting the box either of which holds a site. */
    fn pairs_held(&self) -> u32 {
        let mut count = 0;
        pairs(self.n, |p, q| count += u32::from(self.holds(p) || self.holds(q)));
        count
    }

    /** What the sites on them hide along the axis, in columns' areas (`pairs`). */
    fn shadow(&self) -> f64 {
        self.pairs_held() as f64 / 2.0
    }
}

/**
 * A layer of a thing across an axis: what it hides seen along the axis, the columns along each of the other two
 * it holds (one deep), and how many of its sites are of each kind.
 */
struct Slice {
    across: Columns,
    along: [Columns; 2],
    kinds: Vec<(Kind, u32)>,
}

/**
 * What slabs of things hide, measured: things drawn alone at a level,
 * and for each slab of them across an axis what it adds to their shadow,
 * on average over directions and the things. A population's far form,
 * from things made at the origin, is as dense as that (extinction =
 * things per volume · their mean shadow), each slab a medium of its own.
 */
pub struct Shadows {
    f: i64,
    axis: usize,
    /** By thing, its first site along each axis, and its layers across the axis from the first. */
    things: Vec<([i64; 3], Vec<Slice>)>,
}

/**
 * How much of what a slab adds to a thing's shadow across it, and of its
 * shadow along the other axes, it shows on average over directions. A
 * convex body's mean shadow is a quarter of its area (Cauchy): for a
 * cylinder, half its shadow along its axis and π/4 of that across it,
 * for a ball half of each. Media are isotropic: denser, the canopy hides
 * more from above than its trees; thinner, less among them; these keep
 * both best ("Levels" in docs/generators.md).
 */
const ACROSS: f64 = 0.5;
const ALONG: f64 = 2.0 / 3.0;

impl Shadows {
    pub fn new(nodes: &[Node], level: u32, axis: usize) -> Shadows {
        let f = 1i64 << level;
        let (a, b) = ((axis + 1) % 3, (axis + 2) % 3);
        let things = nodes
            .iter()
            .enumerate()
            .map(|(i, node)| {
                let own = alone(node, &format!("sample {i}"), cost::SHADOWS, bounds_of(node), f, 0, visit);
                let (n, min) = (own.n, own.b.min);
                let layers = (0..n[axis])
                    .map(|l| {
                        let mut slice = Slice {
                            across: Columns::new([n[a], n[b]]),
                            along: [[1, n[b]], [1, n[a]]].map(Columns::new),
                            kinds: Vec::new(),
                        };
                        for i in 0..n[a] {
                            for j in 0..n[b] {
                                let mut p = [0; 3];
                                (p[axis], p[a], p[b]) = (l + min[axis], i + min[a], j + min[b]);
                                if let Some((kind, _)) = own.at(p[0], p[1], p[2]) {
                                    match slice.kinds.iter_mut().find(|k| k.0 == kind) {
                                        Some(k) => k.1 += 1,
                                        None => slice.kinds.push((kind, 1)),
                                    }
                                    slice.across.set([i, j]);
                                    slice.along[0].set([0, j]);
                                    slice.along[1].set([0, i]);
                                }
                            }
                        }
                        slice
                    })
                    .collect();
                (min, layers)
            })
            .collect();
        Shadows { f, axis, things }
    }

    /** The share of the things' sites in the slab from `from` to `to` (exclusive) along the axis whose kinds `of` holds. */
    pub fn share(&self, from: i64, to: i64, of: impl Fn(Kind) -> bool) -> f64 {
        let (mut sum, mut all) = (0u64, 0u64);
        for (min, layers) in &self.things {
            let ceil = |v: i64| (-(-v).div_euclid(self.f) - min[self.axis]).clamp(0, layers.len() as i64) as usize;
            for slice in &layers[ceil(from)..ceil(to)] {
                for &(kind, n) in &slice.kinds {
                    all += n as u64;
                    sum += if of(kind) { n as u64 } else { 0 };
                }
            }
        }
        if all == 0 { 0.0 } else { sum as f64 / all as f64 }
    }

    /**
     * The mean shadow (lattice units²) over directions of what the slab
     * from `from` to `to` (exclusive) along the axis adds to the things':
     * across it, what it adds seen from either side, as slabs of a thing
     * overlap (from above, 2 to 3 times the tree's own in 2 m slabs);
     * along the other axes, its own, as slabs side by side don't.
     */
    pub fn slab(&self, from: i64, to: i64) -> f64 {
        self.mean(from, to, |adds, along| ACROSS * adds + ALONG * along / 2.0)
    }

    /**
     * What the slab from `from` to `to` (exclusive) adds to the things'
     * shadow seen across it, from either side (lattice units²): what the
     * slabs add sums to the shadow.
     */
    pub fn across(&self, from: i64, to: i64) -> f64 {
        self.mean(from, to, |adds, _| adds)
    }

    /** The mean over the things of `of`(what their slab adds to their shadow across the axis, its own shadow along the other two). */
    fn mean(&self, from: i64, to: i64, of: impl Fn(f64, f64) -> f64) -> f64 {
        let (f, mut sum) = (self.f, 0.0);
        for (min, layers) in &self.things {
            let ceil = |v: i64| -(-v).div_euclid(f) - min[self.axis];
            let (lo, hi) = (ceil(from).clamp(0, layers.len() as i64) as usize, ceil(to).clamp(0, layers.len() as i64) as usize);
            if lo >= hi {
                continue;
            }
            let across = |range: std::ops::Range<usize>| Columns::union(layers[range].iter().map(|s| &s.across)).shadow();
            let n = layers.len();
            let adds = (across(lo..n) - across(hi..n) + across(0..hi) - across(0..lo)) / 2.0;
            // Along each of the other two, the slab's layers stacked: a plane of the axis and the third.
            let along: f64 = (0..2).map(|k| Columns::stacked(layers[lo..hi].iter().map(|s| &s.along[k])).shadow()).sum();
            sum += of(adds, along) * (f * f) as f64;
        }
        sum / self.things.len() as f64
    }

    /**
     * Where the things' shadow across the axis falls, for things anywhere
     * in a cell `size` wide (of `Points`, cornered at multiples of it): its
     * mean area (lattice units²) over the column of sites `size` apart at
     * (0, 0), by the cell's offset from it in cells (zero along the axis).
     * A column reaches half its width round its site, so a cell meets four,
     * and a part of the shadow covers each column as often as the thing's
     * place in the cell puts it there.
     */
    pub fn spread(&self, size: i64) -> Vec<([i64; 3], f64)> {
        let (f, axis) = (self.f, self.axis);
        let (a, b) = ((axis + 1) % 3, (axis + 2) % 3);
        // The cells a part p from the thing's foot (given as 2p) covers the column from, with how often: the overlap
        // of the column with the part's places, the cell's corner plus [0, size).
        let cells = |p2: i64| {
            let (r2, size2) = (p2 + size, 2 * size);
            let first = -(r2 + size2 - 1).div_euclid(size2);
            (first..=first + 1).filter_map(move |o| {
                let w = 1.0 - (o * size2 + r2).abs() as f64 / size2 as f64;
                (w > 0.0).then_some((o, w))
            })
        };
        let mut areas = std::collections::BTreeMap::<[i64; 3], f64>::new();
        for (min, layers) in &self.things {
            // Half a column's area at the middle of each pair of neighbouring columns holding a site (`pairs`), its
            // place in halves of the sites' spacing.
            let plane = Columns::union(layers.iter().map(|s| &s.across));
            pairs(plane.n, |p, q| {
                if plane.holds(p) || plane.holds(q) {
                    for (oa, wa) in cells((p[0] + q[0] + 2 * min[a]) * f) {
                        for (ob, wb) in cells((p[1] + q[1] + 2 * min[b]) * f) {
                            let mut offset = [0; 3];
                            (offset[a], offset[b]) = (oa, ob);
                            *areas.entry(offset).or_insert(0.0) += wa * wb * (f * f) as f64 / 2.0;
                        }
                    }
                }
            });
        }
        let things = self.things.len() as f64;
        areas.into_iter().map(|(o, area)| (o, area / things)).collect()
    }
}

/**
 * How many pairs of neighbouring columns along each axis (`pairs`) of the
 * sites `r` times finer than a cell hold one of them in the cell, either
 * of the two: by halves of a column's area, about the 2 r² columns' area
 * of its shadow (a cell's are 2 r² wide), and unevenly by axis, as
 * nearest_site breaks ties one way. Cells of a level are congruent, ties
 * and all (they differ by an even number of cells along the axes), so the
 * count depends on r only.
 */
pub(super) fn pairs_of_cell(r: i64) -> [u32; 3] {
    static PAIRS: [OnceLock<[u32; 3]>; 64] = [const { OnceLock::new() }; 64];
    *PAIRS[r.trailing_zeros() as usize].get_or_init(|| {
        std::array::from_fn(|axis| {
            let (a, b) = ((axis + 1) % 3, (axis + 2) % 3);
            // The columns from (−r, −r), as the cell reaches r along the axes.
            let mut cell = Columns::new([2 * r + 1; 2]);
            for [i, j] in (-r..=r).flat_map(|i| (-r..=r).map(move |j| [i, j])) {
                // The cell is symmetric about its centre's plane across the column, and where a column runs
                // in one of its faces, ties go one way all along it: if the column has a site in the cell,
                // it has one within 2 of that plane (checked by a_cells_pairs_of_columns_are_counted).
                let holds = (-2..=2i64).filter(|k| (i + j + k) & 1 == 0).any(|k| {
                    let mut p = [0; 3];
                    (p[axis], p[a], p[b]) = (k, i, j);
                    nearest_site(p.map(|v| v * ONE), r) == [0; 3]
                });
                if holds {
                    cell.set([i + r, j + r]);
                }
            }
            cell.pairs_held()
        })
    })
}

/**
 * A node's far form at sites `s` apart, measured from its near form:
 * drawn whole two levels finer (spacing f), or at the coarsest level it
 * is drawn at (at most `detail`) if that is coarser, then coarsened.
 * A coarse site stands for the fine sites in its cell (those nearest
 * it). A ray crossing the cell along an axis meets the voxels of two
 * neighbouring columns (`pairs`): it is stopped as often as the pairs that
 * hold a fine site of the cell hold one of the thing, on average over the
 * axes: its share, with which it is drawn.
 * Each of its faces is of a kind of what rays through it see: along the
 * face's normal, a line of nearest neighbours, each such column's first
 * fine site from the face's end is credited to that face of every cell
 * the column crosses, in proportion (a ray that gets to the cell sees
 * that). The site is of a kind of all its faces see, for whatever does
 * not look at faces.
 */
pub(super) fn measure(node: &Node, key: &str, layer: &'static str, bounds: Box3, detail: Fixed, s: i64, seed: u64) -> Sites {
    // Two levels finer, its cells hold enough fine sites to measure, and a tile's things cost less a level coarser.
    let f = (1i64 << ((detail / ONE).max(1).ilog2())).max(s / 4);
    let own = alone(node, key, layer, bounds, f, seed, near);
    cost::add(layer, Work { measured: own.cells.len() as u64, ..Work::default() });
    // Kinds counted, each with how it meets the world.
    type Counts = Vec<(Kind, Op, u32)>;
    // By coarse site: per axis, how many pairs of neighbouring columns hold a fine site of it; per face, what rays
    // through it see first of all, counted by kind.
    #[derive(Default)]
    struct Stats {
        pairs: [u32; 3],
        seen: [Counts; FACE_KINDS],
    }
    let count = |kinds: &mut Counts, kind: Kind, op: Op| match kinds.iter_mut().find(|e| e.0 == kind) {
        Some(e) => e.2 += 1,
        None => kinds.push((kind, op, 1)),
    };
    let n = own.n;
    // A fine point's coordinate along an axis in units of the coarse sites' spacing, then its cell.
    let scaled = |c: usize, v: i64| ((v + own.b.min[c]) * f * ONE).div_euclid(s);
    let cell_of = |p: [i64; 3]| nearest_unit_site([0, 1, 2].map(|c| scaled(c, p[c])));
    // The cells of the coarse sites from those of its box's corners and two more on each side, in an array:
    // nearest_site rounds each coordinate, then may move one by one, the corners' included.
    let lo = cell_of([0; 3]).map(|v| v - 2);
    let m = cell_of(n.map(|v| v - 1)).map(|v| v + 3);
    let m = [0, 1, 2].map(|i| m[i] - lo[i]);
    let index = |c: [i64; 3]| ((c[0] - lo[0]) + m[0] * ((c[1] - lo[1]) + m[1] * (c[2] - lo[2]))) as usize;
    let site_of = |i: usize| {
        let i = i as i64;
        [i % m[0] + lo[0], i / m[0] % m[1] + lo[1], i / (m[0] * m[1]) + lo[2]]
    };
    let mut cells: Vec<Stats> = (0..m[0] * m[1] * m[2]).map(|_| Stats::default()).collect();
    // Fine points by their index in `own` (x first): steps along each axis, and the cell of each fine site
    // (x + y + z even), found once for all the passes.
    let stride = [1, n[0], n[0] * n[1]];
    let parity = own.b.min.iter().sum::<i64>();
    let units: [Vec<Fixed>; 3] = std::array::from_fn(|c| (0..n[c]).map(|v| scaled(c, v)).collect());
    let mut cell_at = vec![u32::MAX; own.cells.len()];
    for z in 0..n[2] {
        for y in 0..n[1] {
            let row = (y * stride[1] + z * stride[2]) as usize;
            for x in ((y + z + parity) & 1..n[0]).step_by(2) {
                let v = [units[0][x as usize], units[1][y as usize], units[2][z as usize]];
                cell_at[row + x as usize] = index(nearest_unit_site(v)) as u32;
            }
        }
    }
    // Along each axis, the cells each column holds a fine site in (a column crosses each cell once: they are
    // convex), then for each pair of neighbouring columns, the cells either holds one in.
    for axis in 0..3 {
        let (a, b) = ((axis + 1) % 3, (axis + 2) % 3);
        let mut held: Vec<Vec<u32>> = vec![Vec::new(); (n[a] * n[b]) as usize];
        for i in 0..n[a] {
            for j in 0..n[b] {
                let column = &mut held[(i * n[b] + j) as usize];
                for k in 0..n[axis] {
                    let p = (i * stride[a] + j * stride[b] + k * stride[axis]) as usize;
                    if own.of(p).is_some() && column.last() != Some(&cell_at[p]) {
                        column.push(cell_at[p]);
                    }
                }
            }
        }
        let none = Vec::new();
        let at = |[i, j]: [i64; 2]| if (0..n[a]).contains(&i) && (0..n[b]).contains(&j) { &held[(i * n[b] + j) as usize] } else { &none };
        pairs([n[a], n[b]], |p, q| {
            let (p, q) = (at(p), at(q));
            for &c in p.iter().chain(q.iter().filter(|c| !p.contains(c))) {
                cells[c as usize].pairs[axis] += 1;
            }
        });
    }
    // Each pair counted against those holding one of the cell's fine sites at all.
    let all = pairs_of_cell(s / f);
    let picks = hash_text(&format!("{key}#measured"), seed);
    let shares: Vec<f64> = cells.iter().map(|stats| (0..3).map(|i| stats.pairs[i] as f64 / all[i] as f64).sum::<f64>() / 3.0).collect();
    // Columns along each line of nearest neighbours d, from its start on the side of −d to its end on the side of
    // d: rays coming from either end see the faces towards it.
    for (from_end, d) in VOXEL_FACES.iter().enumerate().filter(|(_, d)| d.iter().find(|v| **v != 0) == Some(&-1)) {
        let d = d.map(i64::from);
        let from_start = VOXEL_FACES.iter().position(|e| e.map(i64::from) == d.map(|v| -v)).expect("−d is a face");
        let step: i64 = (0..3).map(|c| d[c] * stride[c]).sum();
        // Columns of sites start where a step back leaves the box: on its side of −d along an axis d moves on.
        let edge = |c: usize, v: i64| d[c] != 0 && v == if d[c] < 0 { n[c] - 1 } else { 0 };
        // In order of x, y, z, as counts keep the first way a kind meets the world they are given.
        let starts = (0..n[0]).flat_map(|x| (0..n[1]).map(move |y| [x, y]));
        for [x, y] in starts {
            let zs = if edge(0, x) || edge(1, y) {
                0..n[2]
            } else if d[2] != 0 && n[2] > 0 {
                let z = if d[2] < 0 { n[2] - 1 } else { 0 };
                z..z + 1
            } else {
                0..0
            };
            for z in zs.filter(|z| (x + y + z + parity) & 1 == 0) {
                let start = [x, y, z];
                let origin = x + y * stride[1] + z * stride[2];
                let length =
                    (0..3).filter(|c| d[*c] != 0).map(|c| if d[c] > 0 { n[c] - start[c] } else { start[c] + 1 }).min().expect("d moves");
                // What it holds first and last, and the cells it crosses that the thing is in.
                let (mut first, mut last) = (None, None);
                let mut crossed: Vec<usize> = Vec::new();
                for p in (0..length).map(|t| (origin + t * step) as usize) {
                    let cell = cell_at[p] as usize;
                    if shares[cell] > 0.0 && crossed.last() != Some(&cell) {
                        crossed.push(cell);
                    }
                    if let Some(at) = own.of(p) {
                        first.get_or_insert(at);
                        last = Some(at);
                    }
                }
                let (Some(first), Some(last)) = (first, last) else { continue };
                // What rays along it see first from either end, credited to every cell it crosses: a ray that gets
                // to one, through those before it, sees that.
                for &i in &crossed {
                    count(&mut cells[i].seen[from_start], first.0, first.1);
                    count(&mut cells[i].seen[from_end], last.0, last.1);
                }
            }
        }
    }
    let mut sites = Vec::new();
    for (i, stats) in cells.into_iter().enumerate().filter(|(i, _)| shares[*i] > 0.0) {
        let site = site_of(i);
        let [x, y, z] = site;
        let faces = stats.seen;
        // One pick for all of its faces and itself: faces that see alike are of one kind.
        let u = hash(picks, x * s, y * s, z * s) as u64;
        let pick = |kinds: &Counts| -> (Kind, Op) {
            // In an order of their own: the columns' isn't one.
            let mut kinds = kinds.clone();
            kinds.sort_by_key(|e| e.0);
            let total: u32 = kinds.iter().map(|e| e.2).sum();
            let mut pick = ((u * total as u64) >> 32) as u32;
            let &(kind, op, _) = kinds
                .iter()
                .find(|e| {
                    pick < e.2 || {
                        pick -= e.2;
                        false
                    }
                })
                .expect("a kind");
            (kind, op)
        };
        let mut all: Counts = Vec::new();
        for (kind, op, n) in faces.iter().flatten() {
            match all.iter_mut().find(|e| e.0 == *kind) {
                Some(e) => e.2 += n,
                None => all.push((*kind, *op, *n)),
            }
        }
        // Its share, of each kind as much as its faces see of it, each kind's of what the ones before leave, so that
        // together they stop as much as the cell's share; each with the faces seen.
        let faces = faces.map(|counts| pick(&counts).0);
        let total = all.iter().map(|e| e.2).sum::<u32>().max(1) as f64;
        let mut before = 0.0f64;
        for &(kind, op, n) in &all {
            let part = shares[i] * n as f64 / total;
            let look = Look { kind, faces: faces.iter().any(|k| *k != kind).then_some(faces) };
            let picked = to_pick(hash(picks.wrapping_add(kind as u64 + 1), x * s, y * s, z * s));
            sites.push((site, look, op, share(part / (1.0 - before).max(f64::EPSILON)), picked));
            before += part;
        }
    }
    // In an order of their own, as sites are put in it.
    sites.sort_by_key(|e| e.0);
    sites
}
