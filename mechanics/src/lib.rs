//! Mechanics on the voxels (docs/mechanics.md): matter that moves, bends
//! and breaks, and never leaves the lattice. Statics is its rest.
//!
//! The world is one lattice of sites at a level; mechanics knows nothing
//! but it. Its sites awake that hold matter are cells (`Matter`), each a
//! finite element (element.rs); the rest are asleep, and what bears among
//! them holds still (`Lattice`). Sites sharing a face are welded by as much
//! of it as holds on; cells that meet unwelded push apart (`Contact`). A
//! step is implicit, all nodes at once (solve.rs); matter hops between
//! sites (`hops`); what rests falls asleep, and a site asleep borne on
//! hard is asked to wake (`waking`).

// Small matrices and vectors by index read as the algebra does.
#![allow(clippy::needless_range_loop)]

mod element;
mod solve;

use element::{CORNERS, Element, matrix, turn};
use lattice::{FastMap, FastSet, Holds, opposite};
use solve::{M3, Solver, System, V3, add, cross, dot, find, scale, sets, sub, times, union};

const GRAVITY: f64 = 9.81;

/** A material: density in kg/m³; stiffness, strength in tension, compression and shear, in pascals. */
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Material {
    pub rho: f64,
    pub e: f64,
    pub tension: f64,
    pub compression: f64,
    pub shear: f64,
}

pub const WOOD: Material = Material { rho: 600.0, e: 10e9, tension: 40e6, compression: 30e6, shear: 6e6 };

impl Material {
    /** Where two meet, the weaker of each. */
    pub fn meet(self, o: Material) -> Material {
        let min = |a: f64, b: f64| a.min(b);
        Material {
            rho: min(self.rho, o.rho),
            e: min(self.e, o.e),
            tension: min(self.tension, o.tension),
            compression: min(self.compression, o.compression),
            shear: min(self.shear, o.shear),
        }
    }
}

/** Of a contact: how much it holds against sliding, of what pushes it. */
const FRICTION: f64 = 0.6;
/** Rounds of contacts let go or let slide a step. */
const CONTACT_ROUNDS: usize = 3;
/** The longest step, s; the most a step lets a node move, of a cell. */
const LONGEST_STEP: f64 = 1.0 / 60.0;
const MOVE: f64 = 0.3;
/**
 * Of what bends, how long its motion takes to die away, s (Rayleigh's β:
 * a force −β K v). It slows nothing moving as a whole, which bends
 * nothing; a tree swaying stills within a second or so.
 */
const DAMPING: f64 = 0.3;
/** How far off its site, of a cell, matter moves to the next: past half, so that it does not move back for a little. */
const HOP: f64 = 0.6;
/** How small a step's residual is enough, of its right-hand side's. */
const TOLERANCE: f64 = 1e-3;
/** How fast cells that overlap are pushed apart, at most, m/s: what overlap gives no speed past it. */
const PUSH_APART: f64 = 0.5;
/**
 * A cell whose corners are all slower than SLOW m/s for REST seconds, as
 * are all cells about it, is at rest: slow enough that what starts to
 * topple from near its balance is not.
 */
const SLOW: f64 = 0.005;
const REST: f64 = 0.5;
/** A site asleep wakes where what bears on it, welded or touching, uses more than WAKE of what it bears. */
const WAKE: f64 = 0.5;
/** No unknown, no slot; a slot whose block is turned. */
const NONE: u32 = u32::MAX;
const TURNED: u32 = 1 << 31;
/**
 * Of its strength over a cell's face, the most a contact pushes: in
 * compression, and in shear as friction bears on it, so that what it
 * pushes and holds against sliding never breaks what it is welded to.
 */
const CRUSH: f64 = 0.5;
/** Cells side by side touch; they are taken to meet a little before, so that resting on something is a contact. */
const TOUCH: f64 = 1.01;

/**
 * The world as mechanics asks it: a lattice of sites at a level, of which
 * a Matter's cells are awake and the rest asleep. Neither is asked of a
 * site awake.
 */
pub trait Lattice {
    /** The material of a site asleep, if it bears: it holds still. */
    fn asleep(&mut self, site: [i64; 3]) -> Option<Material>;
    /** How much of the face across face `f` (of VOXEL_FACES) of `site` holds on, of its area: 0 none of it, 1 all. */
    fn joined(&mut self, site: [i64; 3], f: usize) -> f64;
}

/**
 * A cell holding matter: its site, its mass, its material, the share of
 * its cube that bears (its stiffness that share of its material's, more
 * than none), across which faces it is welded (Matter keeps it), what it
 * is.
 */
#[derive(Clone, Debug)]
pub struct Cell<T> {
    pub site: [i64; 3],
    pub mass: f64,
    pub material: Material,
    pub fill: f64,
    pub holds: Holds,
    pub what: T,
}

/** The other cell of a weld or contact: one awake, or a site asleep. */
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Other {
    Cell(u32),
    Still([i64; 3]),
}

/**
 * Two cells that share a face, and so its corners: each as a corner of
 * `i` and of `j`; the way from i to j, as they lay; the material where
 * they meet; how much of the face holds on.
 */
#[derive(Clone, Debug)]
struct Weld {
    i: u32,
    j: Other,
    corners: Vec<(u8, u8)>,
    n: V3,
    material: Material,
    area: f64,
}

/** A node: a corner, or corners welded, of cells: where it is, how fast it moves, its mass (an eighth of each cell's), whether a site asleep holds it. */
#[derive(Clone, Copy, Debug)]
struct Node {
    x: V3,
    v: V3,
    mass: f64,
    held: bool,
}

/**
 * A contact a step: its cells, the way from i to j, how much the two
 * overlap, its stiffness along that way and across it, the most it
 * pushes, and the share of that it was asked to.
 */
struct Contact {
    i: u32,
    j: Other,
    n: V3,
    overlap: f64,
    k: f64,
    t: f64,
    most: f64,
    used: f64,
}

/**
 * The solver's levels, kept from step to step: made for the nodes as they
 * are, while the contacts a step finds are those it found (`touching`)
 * and none lets go, and while a step's first solve (`last`) takes no
 * more than a round past the `first` they served. Made anew, they cost
 * about as much as several rounds.
 */
struct Kept {
    solver: Solver,
    touching: Vec<(u32, Other)>,
    first: usize,
    last: usize,
}

/** The sites about a site, across its faces, edges and corners: what it may meet. */
pub fn about(s: [i64; 3]) -> impl Iterator<Item = [i64; 3]> {
    (0..27).filter(|&d| d != 13).map(move |d| [s[0] + d % 3 - 1, s[1] + d / 3 % 3 - 1, s[2] + d / 9 - 1])
}

/** The sites across a site's faces: what it is welded to. */
pub fn faces(s: [i64; 3]) -> impl Iterator<Item = [i64; 3]> {
    lattice::VOXEL_FACES.into_iter().map(move |f| [s[0] + i64::from(f[0]), s[1] + i64::from(f[1]), s[2] + i64::from(f[2])])
}

/** The face of `a` that `b` is across, if it is. */
fn face_to(a: [i64; 3], b: [i64; 3]) -> Option<usize> {
    faces(a).position(|s| s == b)
}

/** A corner of a site: the lattice point it is at. */
fn corner(site: [i64; 3], c: usize) -> [i64; 3] {
    let b = CORNERS[c];
    [site[0] + b[0], site[1] + b[1], site[2] + b[2]]
}

/** k n nᵀ + t (I − n nᵀ). */
fn block(n: V3, k: f64, t: f64) -> M3 {
    std::array::from_fn(|r| std::array::from_fn(|c| (k - t) * n[r] * n[c] + if r == c { t } else { 0.0 }))
}

fn scaled(m: &M3, s: f64) -> M3 {
    m.map(|row| row.map(|x| x * s))
}

/** The part of a world's lattice awake, at one level, and how it is welded. */
pub struct Matter<T> {
    pub level: u32,
    /** A cell's size, metres. */
    h: f64,
    pub cells: Vec<Cell<T>>,
    at: FastMap<[i64; 3], u32>,
    welds: Vec<Weld>,
    welded: FastSet<(u32, Other)>,
    /** Each cell's corners, as nodes; each cell as an element; how long each has been slow, s. */
    corners: Vec<[u32; 8]>,
    nodes: Vec<Node>,
    elements: Vec<Element>,
    slow: Vec<f64>,
    /**
     * Each node's unknown, NONE if a site asleep holds it; the pairs of
     * unknowns the elements join; each element's pair of corners a < c
     * there (at 8 a + c): its index, TURNED if c's unknown is the first.
     */
    free: Vec<u32>,
    pattern: Vec<(u32, u32)>,
    slots: Vec<[u32; 64]>,
    /** Sites asleep asked to wake. */
    waking: FastSet<[i64; 3]>,
    /** The solver's levels, kept from step to step while they serve (`Kept`). */
    kept: Option<Kept>,
    /** Seconds simulated. */
    pub time: f64,
    /** Rounds of the solve, all steps; welds broken. */
    pub rounds: usize,
    pub broke: usize,
}

impl<T> Matter<T> {
    /** None of the lattice of `level` awake, its sites `unit` metres across at level 0. */
    pub fn new(level: u32, unit: f64) -> Matter<T> {
        Matter {
            level,
            h: unit * f64::from(1u32 << level),
            cells: Vec::new(),
            at: FastMap::default(),
            welds: Vec::new(),
            welded: FastSet::default(),
            corners: Vec::new(),
            nodes: Vec::new(),
            elements: Vec::new(),
            slow: Vec::new(),
            free: Vec::new(),
            pattern: Vec::new(),
            slots: Vec::new(),
            waking: FastSet::default(),
            kept: None,
            time: 0.0,
            rounds: 0,
            broke: 0,
        }
    }

    /** Whether any of it is awake. */
    pub fn awake(&self) -> bool {
        !self.cells.is_empty()
    }

    /** The cell at `site`, if it is awake. */
    pub fn at(&self, site: [i64; 3]) -> Option<usize> {
        self.at.get(&site).map(|&i| i as usize)
    }

    /**
     * Wakes `cells`, sites asleep till now, at rest where they are: welded
     * where the lattice says their faces hold on, to each other and to
     * sites asleep, and to cells awake as those were welded to them asleep.
     */
    pub fn wake(&mut self, cells: Vec<Cell<T>>, lattice: &mut dyn Lattice) {
        let first = self.cells.len();
        for c in cells {
            assert!(self.at.insert(c.site, self.cells.len() as u32).is_none(), "{:?} awake already", c.site);
            self.cells.push(c);
            self.elements.push(Element::default());
            self.slow.push(0.0);
        }
        let new = |at: &FastMap<[i64; 3], u32>, s: [i64; 3]| at.get(&s).filter(|&&j| j as usize >= first).copied();
        for w in &mut self.welds {
            if let Other::Still(s) = w.j
                && let Some(j) = new(&self.at, s)
            {
                w.j = Other::Cell(j);
            }
        }
        for i in first..self.cells.len() {
            let c = &self.cells[i];
            for (f, s) in faces(c.site).enumerate() {
                let (j, material) = match self.at.get(&s) {
                    Some(&j) if j as usize > i => (Other::Cell(j), self.cells[j as usize].material),
                    Some(_) => continue,
                    None => match lattice.asleep(s) {
                        Some(m) => (Other::Still(s), m),
                        None => continue,
                    },
                };
                let area = lattice.joined(c.site, f);
                if area > 0.0 {
                    self.welds.push(weld(i, c.site, j, s, c.material.meet(material), area));
                }
            }
        }
        let at = &self.at;
        self.waking.retain(|s| !at.contains_key(s));
        let before = self.corners.clone();
        let old = std::mem::take(&mut self.nodes);
        self.join(&before, &old);
        self.strain();
    }

    /**
     * The lattice changed at `sites` (cut), where cells awake may be, and
     * the host has had those cells hold what they do now: a cell bearing
     * nothing (`fill` 0) goes, as asleep; each weld of a cell at one of
     * them, or to one, holds by as much as the lattice says now, if any.
     * The cells gone.
     */
    pub fn changed(&mut self, sites: &FastSet<[i64; 3]>, lattice: &mut dyn Lattice) -> Vec<Cell<T>> {
        let gone: Vec<bool> = self.cells.iter().map(|c| c.fill == 0.0).collect();
        let gone = if gone.contains(&true) { self.remove(&gone) } else { Vec::new() };
        let cells = &self.cells;
        let site = |j: Other| match j {
            Other::Cell(j) => cells[j as usize].site,
            Other::Still(s) => s,
        };
        self.welds.retain_mut(|w| {
            let (a, b) = (cells[w.i as usize].site, site(w.j));
            if !sites.contains(&a) && !sites.contains(&b) {
                return true;
            }
            // A weld not across a face of its cells' sites, as they have moved, holds as it did.
            if let Some(f) = face_to(a, b) {
                w.area = lattice.joined(a, f);
            }
            w.area > 0.0
        });
        let before = self.corners.clone();
        let old = std::mem::take(&mut self.nodes);
        self.join(&before, &old);
        gone
    }

    /**
     * The nodes: each cell's corners, joined where welds share them, held
     * where a site asleep welds them; each where `before` had it (each
     * cell's corners there, `old` its nodes), or at its lattice point.
     */
    fn join(&mut self, before: &[[u32; 8]], old: &[Node]) {
        self.kept = None;
        let n = self.cells.len();
        let mut up: Vec<usize> = (0..8 * n).collect();
        let mut held = vec![false; 8 * n];
        for w in &self.welds {
            for &(a, b) in &w.corners {
                let p = 8 * w.i as usize + a as usize;
                match w.j {
                    Other::Cell(j) => union(&mut up, p, 8 * j as usize + b as usize),
                    Other::Still(_) => held[p] = true,
                }
            }
        }
        let mut node = vec![u32::MAX; 8 * n];
        self.nodes.clear();
        self.corners = vec![[0; 8]; n];
        for p in 0..8 * n {
            let root = find(&mut up, p);
            if node[root] == u32::MAX {
                node[root] = self.nodes.len() as u32;
                let (i, c) = (root / 8, root % 8);
                let x = match before.get(i) {
                    Some(b) => old[b[c] as usize],
                    None => Node { x: corner(self.cells[i].site, c).map(|v| v as f64 * self.h), v: [0.0; 3], mass: 0.0, held: false },
                };
                self.nodes.push(Node { mass: 0.0, held: false, ..x });
            }
            let k = node[root] as usize;
            self.corners[p / 8][p % 8] = k as u32;
            self.nodes[k].mass += self.cells[p / 8].mass / 8.0;
            self.nodes[k].held |= held[p];
        }
        let mut count = 0;
        self.free = Vec::with_capacity(self.nodes.len());
        for n in &self.nodes {
            self.free.push(if n.held { NONE } else { count });
            count += u32::from(!n.held);
        }
        let mut index: FastMap<(u32, u32), u32> = FastMap::default();
        self.pattern.clear();
        self.slots = self
            .corners
            .iter()
            .map(|nodes| {
                let mut slots = [NONE; 64];
                for a in 0..8 {
                    for c in a + 1..8 {
                        let (p, q) = (self.free[nodes[a] as usize], self.free[nodes[c] as usize]);
                        if p == NONE || q == NONE {
                            continue;
                        }
                        let key = (p.min(q), p.max(q));
                        let pattern = &mut self.pattern;
                        let k = *index.entry(key).or_insert_with(|| {
                            pattern.push(key);
                            pattern.len() as u32 - 1
                        });
                        slots[8 * a + c] = k | if p > q { TURNED } else { 0 };
                    }
                }
                slots
            })
            .collect();
        self.welded = self.welds.iter().flat_map(|w| [(w.i, w.j)].into_iter().chain(cell_of(w.j).map(|j| (j, Other::Cell(w.i))))).collect();
        self.hold_on();
    }

    /**
     * Each cell holding on across the faces its welds lie across now: the
     * way from it to what it is welded to, as they lie, nearest a face's.
     * So what turned holds on to whichever neighbours its welds now face,
     * and a weld holds on while one of its cells has hopped and the other
     * not yet.
     */
    fn hold_on(&mut self) {
        let mut holds = vec![0; self.cells.len()];
        for w in &self.welds {
            let d = sub(self.other(w.j), self.middle(w.i as usize));
            let axis = (0..3).max_by(|&a, &b| d[a].abs().total_cmp(&d[b].abs())).expect("an axis");
            let f = 2 * axis + usize::from(d[axis] > 0.0);
            holds[w.i as usize] |= 1 << f;
            if let Other::Cell(j) = w.j {
                holds[j as usize] |= 1 << opposite(f);
            }
        }
        for (c, h) in self.cells.iter_mut().zip(holds) {
            c.holds = h;
        }
    }

    /**
     * Takes the cells `gone` says out, asleep: cells welded to them are
     * welded to their sites instead, which hold still. The cells taken out.
     */
    fn remove(&mut self, gone: &[bool]) -> Vec<Cell<T>> {
        let mut index = vec![u32::MAX; self.cells.len()];
        let mut kept = 0;
        for i in 0..self.cells.len() {
            if !gone[i] {
                index[i] = kept;
                kept += 1;
            }
        }
        let cells = &self.cells;
        self.welds = std::mem::take(&mut self.welds)
            .into_iter()
            .filter_map(|w| {
                let i = w.i as usize;
                match (gone[i], w.j) {
                    (false, Other::Cell(j)) if gone[j as usize] => Some(Weld { i: index[i], j: Other::Still(cells[j as usize].site), ..w }),
                    (false, Other::Cell(j)) => Some(Weld { i: index[i], j: Other::Cell(index[j as usize]), ..w }),
                    (false, Other::Still(_)) => Some(Weld { i: index[i], ..w }),
                    (true, Other::Cell(j)) if !gone[j as usize] => Some(Weld {
                        i: index[j as usize],
                        j: Other::Still(cells[i].site),
                        corners: w.corners.iter().map(|&(a, b)| (b, a)).collect(),
                        n: scale(w.n, -1.0),
                        ..w
                    }),
                    (true, _) => None,
                }
            })
            .collect();
        let before: Vec<[u32; 8]> = (0..self.cells.len()).filter(|&i| !gone[i]).map(|i| self.corners[i]).collect();
        let mut taken = Vec::new();
        let (mut cells, mut elements, mut slow) = (Vec::new(), Vec::new(), Vec::new());
        let all = std::mem::take(&mut self.cells).into_iter().zip(std::mem::take(&mut self.elements)).zip(std::mem::take(&mut self.slow));
        for (i, ((c, e), s)) in all.enumerate() {
            if gone[i] {
                taken.push(c);
            } else {
                cells.push(c);
                elements.push(e);
                slow.push(s);
            }
        }
        (self.cells, self.elements, self.slow) = (cells, elements, slow);
        self.at = self.cells.iter().enumerate().map(|(i, c)| (c.site, i as u32)).collect();
        let old = std::mem::take(&mut self.nodes);
        self.join(&before, &old);
        taken
    }

    /** Puts all of it to sleep where it is: the cells, holding on as they are welded. */
    pub fn sleep(&mut self) -> Vec<Cell<T>> {
        let gone = vec![true; self.cells.len()];
        self.remove(&gone)
    }

    /** The sites asleep asked to wake since this was asked last. */
    pub fn waking(&mut self) -> Vec<[i64; 3]> {
        self.waking.drain().collect()
    }

    /** Where a cell's matter is, metres: its corners' middle. */
    fn middle(&self, i: usize) -> V3 {
        self.corners[i].iter().fold([0.0; 3], |m, &k| add(m, scale(self.nodes[k as usize].x, 0.125)))
    }

    /** How fast a cell moves, with `v` its nodes' speeds. */
    fn velocity(&self, j: Other, v: &[V3]) -> V3 {
        match j {
            Other::Cell(j) => self.corners[j as usize].iter().fold([0.0; 3], |m, &k| add(m, scale(v[k as usize], 0.125))),
            Other::Still(_) => [0.0; 3],
        }
    }

    fn site_middle(&self, site: [i64; 3]) -> V3 {
        site.map(|v| (v as f64 + 0.5) * self.h)
    }

    fn other(&self, j: Other) -> V3 {
        match j {
            Other::Cell(j) => self.middle(j as usize),
            Other::Still(s) => self.site_middle(s),
        }
    }

    /** A cell's stiffness, Pa: its material's over its share of its cube that bears. */
    fn e(&self, i: usize) -> f64 {
        self.cells[i].material.e * self.cells[i].fill
    }

    /** Each cell's turn and strain, as its corners are now: what a step's system and a weld's stress are of. */
    fn strain(&mut self) {
        for i in 0..self.cells.len() {
            let x = self.corners[i].map(|k| self.nodes[k as usize].x);
            self.elements[i].follow(&x, self.h);
        }
    }

    /**
     * The share of its strength each weld uses, as either of its cells
     * bears it, over as much of it as holds on: pulled or pushed across it
     * at its most at a corner of the face, as bending does; slid along it
     * as the face's corners do on the mean, as a trilinear cube bent bears
     * a shear at its corners that is its own (it locks), not the matter's,
     * and that is gone over the face.
     */
    fn used(&self) -> Vec<f64> {
        // Each cell's stress at each of its corners, once.
        let stress: Vec<[M3; 8]> =
            (0..self.cells.len()).map(|i| std::array::from_fn(|c| scaled(&self.elements[i].stress(c, self.h), self.e(i)))).collect();
        // As cell `i` bears it, at its corners of the face: the first of each pair, or the second.
        let across = |i: usize, second: bool, w: &Weld| {
            let m = &w.material;
            let (mut normal, mut slide) = (0.0f64, [0.0; 3]);
            for &(a, b) in &w.corners {
                let c = if second { b } else { a };
                let t = times(&stress[i][c as usize], w.n);
                let along = dot(t, w.n);
                normal = normal.max(along / m.tension).max(-along / m.compression);
                slide = add(slide, sub(t, scale(w.n, along)));
            }
            normal.max(dot_len(slide) / w.corners.len() as f64 / m.shear)
        };
        let weld = |w: &Weld| {
            let most = match w.j {
                Other::Cell(j) => across(w.i as usize, false, w).max(across(j as usize, true, w)),
                Other::Still(_) => across(w.i as usize, false, w),
            };
            most / w.area
        };
        self.welds.iter().map(weld).collect()
    }

    /**
     * The cells not welded that overlap (balls a cell across), the sites
     * asleep that bear among them: about where each cell is, which is its
     * site's but while a hop waits.
     */
    fn contacts(&self, lattice: &mut dyn Lattice) -> Vec<Contact> {
        let middles: Vec<V3> = (0..self.cells.len()).map(|i| self.middle(i)).collect();
        let mut contacts = Vec::new();
        let mut met: FastSet<(u32, u32)> = FastSet::default();
        for (i, c) in self.cells.iter().enumerate() {
            let p = middles[i];
            let here = p.map(|v| (v / self.h).floor() as i64);
            // Not the site it is in, if not its own: so deep in it, which way it would be pushed out is anyone's guess.
            for s in about(here) {
                // How far it is first, the cheapest to ask: a pair too far is too far from either side.
                let (j, between) = match self.at.get(&s) {
                    Some(&j) if j as usize != i => (Other::Cell(j), sub(middles[j as usize], p)),
                    Some(_) => continue,
                    None => (Other::Still(s), sub(self.site_middle(s), p)),
                };
                let length = dot_len(between);
                if !(length < self.h * TOUCH && length > 1e-9) {
                    continue;
                }
                let other = match j {
                    Other::Cell(j) if met.insert((j.min(i as u32), j.max(i as u32))) => self.cells[j as usize].material,
                    Other::Cell(_) => continue,
                    Other::Still(s) => match lattice.asleep(s) {
                        Some(m) => m,
                        None => continue,
                    },
                };
                if self.welded.contains(&(i as u32, j)) {
                    continue;
                }
                let meet = c.material.meet(other);
                let most = CRUSH * meet.compression.min(meet.shear / FRICTION) * self.h * self.h;
                let k = meet.e * self.h;
                let n = scale(between, 1.0 / length);
                contacts.push(Contact { i: i as u32, j, n, overlap: self.h - length, k, t: k, most, used: 0.0 });
            }
        }
        contacts
    }

    /**
     * The system of a step `dt` long over the nodes no site asleep holds
     * (`free`): (M + (dt² + β dt) K) v′ = M v + dt F, with K the cells'
     * stiffness, each turned with its cell, in the pairs `pattern` has.
     */
    fn system(&self, dt: f64) -> (System, Vec<V3>) {
        let unknowns: Vec<usize> = (0..self.nodes.len()).filter(|&k| self.free[k] != NONE).collect();
        let mut s = System {
            sites: unknowns.iter().map(|&k| self.nodes[k].x.map(|v| (v / self.h).round() as i64)).collect(),
            at: unknowns.iter().map(|&k| self.nodes[k].x).collect(),
            diag: unknowns.iter().map(|&k| block([1.0, 0.0, 0.0], self.nodes[k].mass, self.nodes[k].mass)).collect(),
            pairs: self.pattern.iter().map(|&(p, q)| (p, q, [[0.0; 3]; 3])).collect(),
        };
        let mut b: Vec<V3> = unknowns.iter().map(|&k| scale(add(self.nodes[k].v, [0.0, -GRAVITY * dt, 0.0]), self.nodes[k].mass)).collect();
        let stiff = dt * dt + DAMPING * dt;
        for (i, e) in self.elements.iter().enumerate() {
            let k = self.e(i) * self.h;
            let nodes = self.corners[i];
            for (a, f) in e.forces().iter().enumerate() {
                if let Some(b) = b.get_mut(self.free[nodes[a] as usize] as usize) {
                    *b = add(*b, scale(*f, -k * dt));
                }
            }
            for a in 0..8 {
                let p = self.free[nodes[a] as usize];
                if p != NONE {
                    let m = e.stiffness(a, a);
                    (0..3).for_each(|r| (0..3).for_each(|c| s.diag[p as usize][r][c] += m[r][c] * k * stiff));
                }
                for c in a + 1..8 {
                    let slot = self.slots[i][8 * a + c];
                    if slot == NONE {
                        continue;
                    }
                    let m = e.stiffness(a, c);
                    let into = &mut s.pairs[(slot & !TURNED) as usize].2;
                    let turned = slot & TURNED != 0;
                    (0..3).for_each(|r| (0..3).for_each(|c| into[r][c] += if turned { m[c][r] } else { m[r][c] } * k * stiff));
                }
            }
        }
        (s, b)
    }

    /**
     * `system` and `b` with the contacts' stiffness and forces, each shared
     * out among the corners of its cells, as their mass is.
     */
    fn with_contacts(&self, dt: f64, system: &System, b: &[V3], contacts: &[Contact]) -> (System, Vec<V3>) {
        let (mut s, mut b) = (system.clone(), b.to_vec());
        let free = &self.free;
        let add_block = |s: &mut System, p: u32, q: u32, m: &M3| {
            let (p, q) = (free[p as usize], free[q as usize]);
            if p == NONE || q == NONE {
                return;
            }
            if p == q {
                (0..3).for_each(|r| (0..3).for_each(|c| s.diag[p as usize][r][c] += m[r][c]));
            } else {
                // The same pair twice is two blocks, which a product adds: so is the matrix.
                s.pairs.push((p, q, *m));
            }
        };
        for c in contacts {
            // On the cells' middles, each its corners' mean: an eighth of the force on each corner, C / 64 between any two.
            // Not damped: a contact damped would hold what parts from it, as glue.
            let m = scaled(&block(c.n, c.k, c.t), dt * dt / 64.0);
            let force = scale(c.n, -c.k * c.overlap.min(PUSH_APART * dt) * dt / 8.0);
            let cells = [Some(c.i), cell_of(c.j)].map(|i| i.map(|i| self.corners[i as usize]));
            for (side, corners) in cells.iter().enumerate() {
                let Some(corners) = corners else { continue };
                for a in 0..8 {
                    if let Some(b) = b.get_mut(free[corners[a] as usize] as usize) {
                        *b = add(*b, scale(force, if side == 0 { 1.0 } else { -1.0 }));
                    }
                    for c in a..8 {
                        add_block(&mut s, corners[a], corners[c], &m);
                    }
                }
            }
            if let [Some(mine), Some(theirs)] = cells {
                for a in 0..8 {
                    for c in 0..8 {
                        add_block(&mut s, mine[a], theirs[c], &scaled(&m, -1.0));
                    }
                }
            }
        }
        (s, b)
    }

    /**
     * A step of `dt`: the nodes' new speeds, all at once, and where they
     * move by them; contacts that would pull let go, those that would hold
     * past friction let slide. The contacts, each with the share of the
     * most it pushes it was asked to.
     */
    fn solve_step(&mut self, dt: f64, lattice: &mut dyn Lattice) -> Vec<Contact> {
        let mut contacts = self.contacts(lattice);
        let mut v: Vec<V3> = self.nodes.iter().filter(|n| !n.held).map(|n| n.v).collect();
        let touching: Vec<(u32, Other)> = contacts.iter().map(|c| (c.i, c.j)).collect();
        let mut kept = self.kept.take().filter(|k| k.touching == touching && k.last <= k.first + 1);
        let free = &self.free;
        let speeds = |v: &[V3]| -> Vec<V3> { free.iter().map(|&u| if u == NONE { [0.0; 3] } else { v[u as usize] }).collect() };
        let (elements, b) = self.system(dt);
        for round in 0..CONTACT_ROUNDS {
            let (system, b) = self.with_contacts(dt, &elements, &b, &contacts);
            // Levels kept serve as well as they fit, never wrongly; made anew, they serve the rounds after.
            let solved = kept.as_ref().and_then(|k| k.solver.solve(&system, &b, &mut v, TOLERANCE));
            let rounds = solved.unwrap_or_else(|| {
                let solver = Solver::new(&system, self.h);
                let rounds = solver.solve(&system, &b, &mut v, TOLERANCE).unwrap_or(solve::ROUNDS);
                kept = Some(Kept { solver, touching: touching.clone(), first: rounds, last: rounds });
                rounds
            });
            if round == 0
                && let Some(k) = &mut kept
            {
                k.last = rounds;
            }
            self.rounds += rounds;
            let all = speeds(&v);
            let mut changed = false;
            let before = contacts.len();
            contacts.retain_mut(|c| {
                let rel = sub(self.velocity(c.j, &all), self.velocity(Other::Cell(c.i), &all));
                let along = dot(rel, c.n);
                let push = c.k * (c.overlap.min(PUSH_APART * dt) - dt * along);
                if push < 0.0 {
                    changed = true;
                    return false;
                }
                // Pushed harder, it crushes, a little each step, rather than break what it is welded to: as stiff as pushes `most`.
                c.used = c.used.max(push / c.most);
                let push = if push > c.most * 1.01 {
                    (c.k, c.t) = (c.k * c.most / push, c.t * c.most / push);
                    changed = true;
                    c.most
                } else {
                    push
                };
                let held = c.t * dt * dot_len(sub(rel, scale(c.n, along)));
                // A hundredth past friction, so that what was let slide is not let slide again.
                if held > FRICTION * push * 1.01 {
                    c.t *= FRICTION * push / held;
                    changed = true;
                }
                true
            });
            if !changed {
                break;
            }
            // Levels made with a contact let go fit what is left badly, as if it still held what it parts.
            if contacts.len() < before {
                kept = None;
            }
        }
        self.kept = kept;
        self.advance(&speeds(&v), dt);
        contacts
    }

    /**
     * Moves the nodes at speeds `v` for `dt`: each piece's turn as a whole
     * exactly, and only what bends it along a line. Moved along a line,
     * what turns would be stretched each step by its turn in the step
     * squared, which damping lets go of only slowly: a post toppling would
     * break. A piece turns about its middle, or, held, about its held
     * nodes, which hold still.
     */
    fn advance(&mut self, v: &[V3], dt: f64) {
        let (piece, count) = sets(self.nodes.len(), self.corners.iter().flat_map(|c| c[1..].iter().map(|&k| (c[0] as usize, k as usize))));
        // Of each piece, its free nodes by mass and its held nodes by count: how much, and where and how fast, summed.
        let mut of = vec![[(0.0, [0.0; 3], [0.0; 3]); 2]; count];
        for (k, n) in self.nodes.iter().enumerate() {
            let p = &mut of[piece[k] as usize][usize::from(n.held)];
            let w = if n.held { 1.0 } else { n.mass };
            *p = (p.0 + w, add(p.1, scale(n.x, w)), add(p.2, scale(v[k], w)));
        }
        let (middle, speed): (Vec<V3>, Vec<V3>) = of
            .iter()
            .map(|&[free, held]| {
                let (w, x, v) = if held.0 > 0.0 { held } else { free };
                (scale(x, 1.0 / w), scale(v, 1.0 / w))
            })
            .unzip();
        let mut spin = vec![[0.0; 3]; count];
        let mut inertia = vec![[[0.0; 3]; 3]; count];
        for (k, n) in self.nodes.iter().enumerate() {
            let p = piece[k] as usize;
            let r = sub(n.x, middle[p]);
            spin[p] = add(spin[p], scale(cross(r, sub(v[k], speed[p])), n.mass));
            for i in 0..3 {
                for j in 0..3 {
                    inertia[p][i][j] += n.mass * (if i == j { dot(r, r) } else { 0.0 } - r[i] * r[j]);
                }
            }
        }
        let omega: Vec<V3> = (0..count).map(|p| times(&solve::inverse(&inertia[p]), spin[p])).collect();
        let turned: Vec<M3> = omega.iter().map(|&w| matrix(turn(scale(w, dt)))).collect();
        for (k, n) in self.nodes.iter_mut().enumerate() {
            let p = piece[k] as usize;
            let relative = sub(v[k], speed[p]);
            n.v = add(speed[p], times(&turned[p], relative));
            if !n.held {
                let r = sub(n.x, middle[p]);
                let bends = sub(relative, cross(omega[p], r));
                n.x = add(add(middle[p], scale(speed[p], dt)), times(&turned[p], add(r, scale(bends, dt))));
            }
        }
    }

    /**
     * A step of time, as long as lets no node move more than MOVE of a
     * cell, at most LONGEST_STEP: the nodes move, welds past their
     * strength break, matter moves between sites (`hops`), what is at rest
     * falls asleep. The cells that fell asleep: the lattice holds them
     * still from now on, holding on as they say.
     */
    pub fn step(&mut self, lattice: &mut dyn Lattice) -> Vec<Cell<T>> {
        if self.cells.is_empty() {
            return Vec::new();
        }
        let fastest = self.nodes.iter().map(|n| dot_len(n.v)).fold(0.0, f64::max);
        let dt = LONGEST_STEP.min(MOVE * self.h / fastest.max(1e-9));
        let contacts = self.solve_step(dt, lattice);
        self.time += dt;
        for i in 0..self.cells.len() {
            let fastest = self.corners[i].iter().map(|&k| dot_len(self.nodes[k as usize].v)).fold(0.0, f64::max);
            self.slow[i] = if fastest < SLOW { self.slow[i] + dt } else { 0.0 };
        }
        self.strain();
        let used = self.used();
        let borne = self.welds.iter().zip(&used).map(|(w, &u)| (w.j, u)).chain(contacts.iter().map(|c| (c.j, c.used)));
        for (j, used) in borne {
            if let Other::Still(s) = j
                && used > WAKE
            {
                self.waking.insert(s);
            }
        }
        if used.iter().any(|&u| u > 1.0) {
            let mut used = used.into_iter();
            let before = self.welds.len();
            self.welds.retain(|_| used.next().expect("one each") <= 1.0);
            self.broke += before - self.welds.len();
            let (before, old) = (self.corners.clone(), std::mem::take(&mut self.nodes));
            self.join(&before, &old);
        }
        self.hops(lattice);
        let rested = |i: usize| self.slow[i] >= REST;
        let gone: Vec<bool> = (0..self.cells.len())
            .map(|i| rested(i) && about(self.cells[i].site).all(|s| self.at.get(&s).is_none_or(|&j| rested(j as usize))))
            .collect();
        if gone.contains(&true) { self.remove(&gone) } else { Vec::new() }
    }

    /**
     * A cell whose corners' middle is more than HOP of a cell off its site
     * moves across the face it is past, to a site that holds nothing. Only
     * which site shows it changes; where it is does not.
     */
    fn hops(&mut self, lattice: &mut dyn Lattice) {
        let middles: Vec<V3> = (0..self.cells.len()).map(|i| self.middle(i)).collect();
        // Again while any hops: a cell moves into a site another left this step, whichever went first.
        let mut hopped = true;
        while hopped {
            hopped = false;
            for (i, middle) in middles.iter().enumerate() {
                for a in 0..3 {
                    let from = self.cells[i].site;
                    let off = middle[a] - self.site_middle(from)[a];
                    if off.abs() <= self.h * HOP {
                        continue;
                    }
                    let mut to = from;
                    to[a] += off.signum() as i64;
                    if self.at.contains_key(&to) || lattice.asleep(to).is_some() {
                        continue;
                    }
                    self.at.remove(&from);
                    self.at.insert(to, i as u32);
                    self.cells[i].site = to;
                    hopped = true;
                }
            }
        }
        self.hold_on();
    }

    /** How many welds hold. */
    pub fn welds(&self) -> usize {
        self.welds.len()
    }
}

/** A weld of cell `i` at `a` to `j` at `b`, across a face of them. */
fn weld(i: usize, a: [i64; 3], j: Other, b: [i64; 3], material: Material, area: f64) -> Weld {
    let corners = (0..8).filter_map(|p| (0..8).find(|&q| corner(a, p) == corner(b, q)).map(|q| (p as u8, q as u8))).collect();
    let d = sub(b.map(|v| v as f64), a.map(|v| v as f64));
    Weld { i: i as u32, j, corners, n: scale(d, 1.0 / dot_len(d)), material, area }
}

fn cell_of(j: Other) -> Option<u32> {
    match j {
        Other::Cell(j) => Some(j),
        Other::Still(_) => None,
    }
}

fn dot_len(a: V3) -> f64 {
    dot(a, a).sqrt()
}

#[cfg(test)]
mod tests;
