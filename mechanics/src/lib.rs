//! Mechanics on the voxels (docs/mechanics.md): matter that moves, bends
//! and breaks, and never leaves the lattice. Statics is its rest.
//!
//! Each cell of a level (2^level sites a side) that holds matter is a
//! finite element: a cube whose corners are nodes (element.rs). Cells that
//! share a face, each holding on across it (`Holds`), are welded there,
//! and share its corners (`Weld`); cells touching only at an edge or a
//! corner hold on by nothing. What the world holds still holds the corners
//! it is welded at. A weld breaks where the stress across it passes its
//! strength, and its cells no longer share its corners. Cells that meet
//! unwelded push apart, and hold against sliding as far as friction lets
//! them (`Contact`); they are never welded. Across which faces a cell
//! holds on is read once, to weld, and kept as its welds are after
//! (`hold_on`): so what broke lets go, what turned holds on to whichever
//! neighbours its welds face.
//!
//! A step is implicit (backward Euler), all nodes at once (solve.rs), so
//! matter as stiff as wood takes steps as long as its motion allows: what
//! moves together moves rigidly, the solve finding it level by level of
//! the octree. A cell whose corners' middle is more than HOP of a cell off
//! its site moves to the site next to it, if that holds nothing (`hops`):
//! the lattice never moves, matter moves between its sites.

// Small matrices and vectors by index read as the algebra does.
#![allow(clippy::needless_range_loop)]

mod element;
mod solve;

use element::{CORNERS, Element};
use lattice::{FastMap, FastSet, Holds, opposite};
use solve::{M3, Solver, System, V3, add, dot, scale, sub, times};

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
    fn meet(self, o: Material) -> Material {
        let min = |a: f64, b: f64| a.min(b);
        Material {
            rho: min(self.rho, o.rho),
            e: min(self.e, o.e),
            tension: min(self.tension, o.tension),
            compression: min(self.compression, o.compression),
            shear: min(self.shear, o.shear),
        }
    }

    /** The share of its strength a stress `s` uses across a plane of normal `n`: pulled, pushed, slid; 1 breaks it. */
    fn used(&self, s: &M3, n: V3) -> f64 {
        let t = times(s, n);
        let normal = dot(t, n);
        let slide = dot_len(sub(t, scale(n, normal)));
        (normal / self.tension).max(-normal / self.compression).max(slide / self.shear)
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
 * Slower than SLOW m/s, by mass, for REST seconds, matter is at rest:
 * slow enough that what starts to topple from near its balance is not.
 */
const SLOW: f64 = 0.005;
const REST: f64 = 0.5;
/** Cells side by side touch; they are taken to meet a little before, so that resting on something is a contact. */
const TOUCH: f64 = 1.01;

/**
 * A cell holding matter: its site, its mass, its material, across which
 * faces it holds on (as given; in Matter, those it is welded across),
 * what it is.
 */
#[derive(Clone, Debug)]
pub struct Cell<T> {
    pub site: [i64; 3],
    pub mass: f64,
    pub material: Material,
    pub holds: Holds,
    pub what: T,
}

/** The other cell of a weld or contact: one that moves, or a site of the world that does not. */
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Other {
    Cell(u32),
    Still([i64; 3]),
}

/**
 * Two cells that share a face, and so its corners: each as a corner of
 * `i` and of `j`; the way from i to j, as they lay; the material where
 * they meet.
 */
#[derive(Clone, Debug)]
struct Weld {
    i: u32,
    j: Other,
    corners: Vec<(u8, u8)>,
    n: V3,
    material: Material,
}

/** A node: a corner, or corners welded, of cells: where it is, how fast it moves, its mass (an eighth of each cell's), whether the world holds it. */
#[derive(Clone, Copy, Debug)]
struct Node {
    x: V3,
    v: V3,
    mass: f64,
    held: bool,
}

/** A contact a step: its cells, the way from i to j, how much the two overlap, its stiffness along that way and across it. */
struct Contact {
    i: u32,
    j: Other,
    n: V3,
    overlap: f64,
    k: f64,
    t: f64,
}

/** The sites about a site, across its faces, edges and corners: what it may meet. */
fn about(s: [i64; 3]) -> impl Iterator<Item = [i64; 3]> {
    (0..27).filter(|&d| d != 13).map(move |d| [s[0] + d % 3 - 1, s[1] + d / 3 % 3 - 1, s[2] + d / 9 - 1])
}

/** The sites across a site's faces: what it is welded to. */
pub fn faces(s: [i64; 3]) -> impl Iterator<Item = [i64; 3]> {
    lattice::VOXEL_FACES.into_iter().map(move |f| [s[0] + i64::from(f[0]), s[1] + i64::from(f[1]), s[2] + i64::from(f[2])])
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

/** What moves of a world, at one level, and how it is welded. */
pub struct Matter<T> {
    pub level: u32,
    /** A cell's size, metres. */
    h: f64,
    pub cells: Vec<Cell<T>>,
    at: FastMap<[i64; 3], u32>,
    welds: Vec<Weld>,
    welded: FastSet<(u32, Other)>,
    /** Each cell's corners, as nodes; and each cell as an element. */
    corners: Vec<[u32; 8]>,
    nodes: Vec<Node>,
    elements: Vec<Element>,
    /** Seconds simulated; of them, the last it has been slower than SLOW. */
    pub time: f64,
    still: f64,
    /** Rounds of the solve, all steps. */
    pub rounds: usize,
}

impl<T> Matter<T> {
    /**
     * Matter of `cells` at `level`, sites `unit` metres across at level 0,
     * each welded to those across its faces, and to what the world holds
     * still there (`still`: the material of a site that does not move, and
     * across which faces it holds on), where both hold on across it.
     */
    pub fn new(level: u32, unit: f64, cells: Vec<Cell<T>>, still: &mut dyn FnMut([i64; 3]) -> Option<(Material, Holds)>) -> Matter<T> {
        let h = unit * f64::from(1u32 << level);
        let at: FastMap<[i64; 3], u32> = cells.iter().enumerate().map(|(i, c)| (c.site, i as u32)).collect();
        let mut welds = Vec::new();
        for (i, c) in cells.iter().enumerate() {
            for (f, s) in faces(c.site).enumerate() {
                let (j, material, holds) = match at.get(&s) {
                    Some(&j) if j as usize > i => (Other::Cell(j), cells[j as usize].material, cells[j as usize].holds),
                    Some(_) => continue,
                    None => match still(s) {
                        Some((m, holds)) => (Other::Still(s), m, holds),
                        None => continue,
                    },
                };
                if c.holds & (1 << f) == 0 || holds & (1 << opposite(f)) == 0 {
                    continue;
                }
                let corners =
                    (0..8).filter_map(|a| (0..8).find(|&b| corner(c.site, a) == corner(s, b)).map(|b| (a as u8, b as u8))).collect();
                let d = sub(s.map(|v| v as f64), c.site.map(|v| v as f64));
                welds.push(Weld { i: i as u32, j, corners, n: scale(d, 1.0 / dot_len(d)), material: c.material.meet(material) });
            }
        }
        let elements = cells.iter().map(|_| Element::default()).collect();
        let mut matter = Matter {
            level,
            h,
            cells,
            at,
            welds,
            welded: FastSet::default(),
            corners: Vec::new(),
            nodes: Vec::new(),
            elements,
            time: 0.0,
            still: 0.0,
            rounds: 0,
        };
        matter.join(&[], &[]);
        matter.strain();
        matter
    }

    /**
     * The nodes: each cell's corners, joined where welds share them, held
     * where the world welds them; each where `before` had it (each cell's
     * corners there, `old` its nodes), or at its lattice point.
     */
    fn join(&mut self, before: &[[u32; 8]], old: &[Node]) {
        let n = self.cells.len();
        let mut up: Vec<usize> = (0..8 * n).collect();
        fn find(up: &mut [usize], mut i: usize) -> usize {
            while up[i] != i {
                up[i] = up[up[i]];
                i = up[i];
            }
            i
        }
        let mut held = vec![false; 8 * n];
        for w in &self.welds {
            for &(a, b) in &w.corners {
                let p = 8 * w.i as usize + a as usize;
                match w.j {
                    Other::Cell(j) => {
                        let (r, s) = (find(&mut up, p), find(&mut up, 8 * j as usize + b as usize));
                        up[r.max(s)] = r.min(s);
                    }
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
                let (i, c) = (p / 8, p % 8);
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
        self.welded = self.welds.iter().flat_map(|w| [(w.i, w.j)].into_iter().chain(cell_of(w.j).map(|j| (j, Other::Cell(w.i))))).collect();
        self.hold_on();
    }

    /** Each cell holding on across the faces it is welded across to what is there: as its welds and sites are now. */
    fn hold_on(&mut self) {
        for i in 0..self.cells.len() {
            let welded = |s: [i64; 3]| {
                let other = self.at.get(&s).map_or(Other::Still(s), |&j| Other::Cell(j));
                self.welded.contains(&(i as u32, other))
            };
            let holds = faces(self.cells[i].site).enumerate().filter(|&(_, s)| welded(s)).fold(0, |h, (f, _)| h | 1 << f);
            self.cells[i].holds = holds;
        }
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

    /** Each cell's turn and strain, as its corners are now: what a step's system and a weld's stress are of. */
    fn strain(&mut self) {
        for i in 0..self.cells.len() {
            let x = self.corners[i].map(|k| self.nodes[k as usize].x);
            self.elements[i].follow(&x, self.h);
        }
    }

    /** The share of its strength each weld uses: the most across it at any of its corners, as either of its cells bears it. */
    fn used(&self) -> Vec<f64> {
        let e = |i: usize| self.cells[i].material.e;
        let at = |i: usize, c: u8, w: &Weld| w.material.used(&scaled(&self.elements[i].stress(c as usize, self.h), e(i)), w.n);
        let weld = |w: &Weld| {
            let mine = w.corners.iter().map(|&(a, _)| at(w.i as usize, a, w)).fold(0.0, f64::max);
            match w.j {
                Other::Cell(j) => w.corners.iter().map(|&(_, b)| at(j as usize, b, w)).fold(mine, f64::max),
                Other::Still(_) => mine,
            }
        };
        self.welds.iter().map(weld).collect()
    }

    /** Takes out the cells at `sites`, and their welds. */
    pub fn take_out(&mut self, sites: &FastSet<[i64; 3]>) {
        let mut index = vec![u32::MAX; self.cells.len()];
        let mut kept = 0;
        for (i, c) in self.cells.iter().enumerate() {
            if !sites.contains(&c.site) {
                index[i] = kept;
                kept += 1;
            }
        }
        let map = |j: Other| match j {
            Other::Cell(j) => (index[j as usize] != u32::MAX).then(|| Other::Cell(index[j as usize])),
            s => Some(s),
        };
        self.welds = std::mem::take(&mut self.welds)
            .into_iter()
            .filter_map(|w| {
                let i = index[w.i as usize];
                (i != u32::MAX).then_some(())?;
                Some(Weld { i, j: map(w.j)?, ..w })
            })
            .collect();
        let kept = |i: &usize| index[*i] != u32::MAX;
        let before: Vec<[u32; 8]> = (0..self.cells.len()).filter(kept).map(|i| self.corners[i]).collect();
        let mut k = (0..self.cells.len()).map(|i| kept(&i));
        self.cells.retain(|_| k.next().expect("one each"));
        let mut k = (0..self.elements.len()).map(|i| kept(&i));
        self.elements.retain(|_| k.next().expect("one each"));
        self.at = self.cells.iter().enumerate().map(|(i, c)| (c.site, i as u32)).collect();
        let old = std::mem::take(&mut self.nodes);
        self.join(&before, &old);
    }

    /** The cells not welded that overlap (balls a cell across), with `still` the world's sites that stop them. */
    fn contacts(&self, still: &mut dyn FnMut([i64; 3]) -> bool) -> Vec<Contact> {
        let mut contacts = Vec::new();
        for (i, c) in self.cells.iter().enumerate() {
            let p = self.middle(i);
            for s in about(c.site) {
                let j = match self.at.get(&s) {
                    Some(&j) if j as usize > i => Other::Cell(j),
                    Some(_) => continue,
                    None if still(s) => Other::Still(s),
                    None => continue,
                };
                if self.welded.contains(&(i as u32, j)) {
                    continue;
                }
                let between = sub(self.other(j), p);
                let length = dot_len(between);
                if length < self.h * TOUCH && length > 1e-9 {
                    let other = match j {
                        Other::Cell(j) => self.cells[j as usize].material,
                        Other::Still(_) => c.material,
                    };
                    let k = c.material.meet(other).e * self.h;
                    contacts.push(Contact { i: i as u32, j, n: scale(between, 1.0 / length), overlap: self.h - length, k, t: k });
                }
            }
        }
        contacts
    }

    /**
     * The system of a step `dt` long over the nodes the world does not
     * hold (`free`, each node's unknown): (M + (dt² + β dt) K) v′ = M v +
     * dt F, with K the cells' stiffness, each turned with its cell; and
     * the contacts', each shared out among the corners of its cells, as
     * their mass is.
     */
    fn system(&self, dt: f64, contacts: &[Contact], free: &[u32]) -> (System, Vec<V3>) {
        let unknowns: Vec<usize> = (0..self.nodes.len()).filter(|&k| free[k] != u32::MAX).collect();
        let mut s = System {
            sites: unknowns.iter().map(|&k| self.nodes[k].x.map(|v| (v / self.h).round() as i64)).collect(),
            at: unknowns.iter().map(|&k| self.nodes[k].x).collect(),
            diag: unknowns.iter().map(|&k| block([1.0, 0.0, 0.0], self.nodes[k].mass, self.nodes[k].mass)).collect(),
            pairs: Vec::new(),
        };
        let mut b: Vec<V3> = unknowns.iter().map(|&k| scale(add(self.nodes[k].v, [0.0, -GRAVITY * dt, 0.0]), self.nodes[k].mass)).collect();
        let add_block = |s: &mut System, p: u32, q: u32, m: &M3| {
            let (p, q) = (free[p as usize], free[q as usize]);
            if p == u32::MAX || q == u32::MAX {
                return;
            }
            if p == q {
                (0..3).for_each(|r| (0..3).for_each(|c| s.diag[p as usize][r][c] += m[r][c]));
            } else {
                s.pairs.push((p, q, *m));
            }
        };
        let stiff = dt * dt + DAMPING * dt;
        for (i, e) in self.elements.iter().enumerate() {
            let k = self.cells[i].material.e * self.h;
            let nodes = self.corners[i];
            for (a, f) in e.forces().iter().enumerate() {
                if let Some(b) = b.get_mut(free[nodes[a] as usize] as usize) {
                    *b = add(*b, scale(*f, -k * dt));
                }
            }
            for a in 0..8 {
                for c in a..8 {
                    add_block(&mut s, nodes[a], nodes[c], &scaled(&e.stiffness(a, c), k * stiff));
                }
            }
        }
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
        s.merge();
        (s, b)
    }

    /**
     * A step of `dt`: the nodes' new speeds, all at once, and where they
     * move by them; contacts that would pull let go, those that would hold
     * past friction let slide.
     */
    fn solve_step(&mut self, dt: f64, still: &mut dyn FnMut([i64; 3]) -> bool) {
        let mut contacts = self.contacts(still);
        let mut count = 0;
        let free: Vec<u32> = self
            .nodes
            .iter()
            .map(|n| {
                if n.held {
                    return u32::MAX;
                }
                count += 1;
                count - 1
            })
            .collect();
        let mut v: Vec<V3> = self.nodes.iter().filter(|n| !n.held).map(|n| n.v).collect();
        let mut solver: Option<Solver> = None;
        let speeds = |v: &[V3]| -> Vec<V3> { free.iter().map(|&u| if u == u32::MAX { [0.0; 3] } else { v[u as usize] }).collect() };
        for _ in 0..CONTACT_ROUNDS {
            let (system, b) = self.system(dt, &contacts, &free);
            // The levels made once a step serve its rounds: they solve as well as they fit, never wrongly.
            let solver = solver.get_or_insert_with(|| Solver::new(&system, self.h));
            self.rounds += solver.solve(&system, &b, &mut v, TOLERANCE);
            let all = speeds(&v);
            let mut changed = false;
            contacts.retain_mut(|c| {
                let rel = sub(self.velocity(c.j, &all), self.velocity(Other::Cell(c.i), &all));
                let along = dot(rel, c.n);
                let push = c.k * (c.overlap.min(PUSH_APART * dt) - dt * along);
                if push < 0.0 {
                    changed = true;
                    return false;
                }
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
        }
        for (n, v) in self.nodes.iter_mut().zip(speeds(&v)) {
            n.v = v;
            n.x = add(n.x, scale(v, dt));
        }
    }

    /**
     * A step of time, as long as lets no node move more than MOVE of a
     * cell, at most LONGEST_STEP: the nodes move, welds past their
     * strength break, matter moves between sites (`hops`). `still` says
     * which of the world's sites stop matter.
     */
    pub fn step(&mut self, still: &mut dyn FnMut([i64; 3]) -> bool) {
        let fastest = self.nodes.iter().map(|n| dot_len(n.v)).fold(0.0, f64::max);
        let dt = LONGEST_STEP.min(MOVE * self.h / fastest.max(1e-9));
        self.solve_step(dt, still);
        self.time += dt;
        self.still = if self.speed() < SLOW { self.still + dt } else { 0.0 };
        self.strain();
        let used = self.used();
        if used.iter().any(|&u| u > 1.0) {
            let mut used = used.into_iter();
            self.welds.retain(|_| used.next().expect("one each") <= 1.0);
            let (before, old) = (self.corners.clone(), std::mem::take(&mut self.nodes));
            self.join(&before, &old);
        }
        self.hops(still)
    }

    /**
     * A cell whose corners' middle is more than HOP of a cell off its site
     * moves across the face it is past, to a site that holds nothing. Only
     * which site shows it changes; where it is does not.
     */
    fn hops(&mut self, still: &mut dyn FnMut([i64; 3]) -> bool) {
        for i in 0..self.cells.len() {
            let middle = self.middle(i);
            for a in 0..3 {
                loop {
                    let from = self.cells[i].site;
                    let off = middle[a] - self.site_middle(from)[a];
                    if off.abs() <= self.h * HOP {
                        break;
                    }
                    let mut to = from;
                    to[a] += off.signum() as i64;
                    if self.at.contains_key(&to) || still(to) {
                        break;
                    }
                    self.at.remove(&from);
                    self.at.insert(to, i as u32);
                    self.cells[i].site = to;
                }
            }
        }
        self.hold_on();
    }

    /** Whether it has come to rest. */
    pub fn at_rest(&self) -> bool {
        self.still >= REST
    }

    /** How fast the matter moves, m/s, by mass: the root of the nodes' mean square. */
    fn speed(&self) -> f64 {
        let mass: f64 = self.nodes.iter().map(|n| n.mass).sum();
        (self.nodes.iter().map(|n| n.mass * dot(n.v, n.v)).sum::<f64>() / mass.max(1e-12)).sqrt()
    }

    /** How many welds hold. */
    pub fn welds(&self) -> usize {
        self.welds.len()
    }
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
