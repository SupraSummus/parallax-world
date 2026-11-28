//! Mechanics by local rules, on the lattice: an experiment with
//! `mechanics/`'s interface (docs/mechanics.md, "By local rules"). Every
//! site holding matter is a cell, a rigid cube; between two cells is a
//! spring, a bond if they hold on to each other, a contact if not; a step
//! is explicit, each cell moved by its springs as they were.

// Small matrices and vectors by index read as the algebra does.
#![allow(clippy::needless_range_loop)]

use lattice::{FastMap, FastSet};
pub use mechanics::{Cell, Lattice, Material, WOOD, about, faces};

pub type V3 = [f64; 3];
type Q = [f64; 4];

const GRAVITY: f64 = 9.81;
/** Of every material: its Poisson's ratio. */
const NU: f64 = 0.3;
const FRICTION: f64 = 0.6;
/** Seconds a step is, at most: what is told of, cells asleep and hops, are as of a step. */
const FRAME: f64 = 1.0 / 120.0;
/** Of the longest step stable, how long a step is. */
const SAFETY: f64 = 0.8;
/** Of a cell, the most anything moves a step: what a cell may meet is looked for as far, each way, and no farther. */
const REACH: f64 = 0.1;
/** Every spring's damping, of critical for the two cells it holds: what bends rings out in a few swings. */
const DAMPING: f64 = 0.5;
/** How fast cells that touch are pushed apart by their overlap, at most, m/s: contacts are not elastic. */
const PUSH_APART: f64 = 0.5;
const SLOW: f64 = 0.005;
const REST: f64 = 0.5;
/** A cell asleep wakes where what bears on it uses more than WAKE of its strength. */
const WAKE: f64 = 0.5;
const HOP: f64 = 0.6;
const ID: Q = [1.0, 0.0, 0.0, 0.0];

fn add(a: V3, b: V3) -> V3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn scale(a: V3, s: f64) -> V3 {
    a.map(|v| v * s)
}
fn dot(a: V3, b: V3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn cross(a: V3, b: V3) -> V3 {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}
fn len(a: V3) -> f64 {
    dot(a, a).sqrt()
}

fn qmul(a: Q, b: Q) -> Q {
    [
        a[0] * b[0] - a[1] * b[1] - a[2] * b[2] - a[3] * b[3],
        a[0] * b[1] + a[1] * b[0] + a[2] * b[3] - a[3] * b[2],
        a[0] * b[2] - a[1] * b[3] + a[2] * b[0] + a[3] * b[1],
        a[0] * b[3] + a[1] * b[2] - a[2] * b[1] + a[3] * b[0],
    ]
}
fn conj(q: Q) -> Q {
    [q[0], -q[1], -q[2], -q[3]]
}
fn normal(q: Q) -> Q {
    let n = q.iter().map(|v| v * v).sum::<f64>().sqrt();
    q.map(|v| v / n)
}
/** The turn by `w`, about it by as many radians as it is long. */
fn exp(w: V3) -> Q {
    let a = len(w);
    let s = if a > 1e-12 { (a / 2.0).sin() / a } else { 0.5 };
    [(a / 2.0).cos(), w[0] * s, w[1] * s, w[2] * s]
}
/** A unit quaternion as a matrix. */
fn matrix([w, x, y, z]: Q) -> [V3; 3] {
    [
        [1.0 - 2.0 * (y * y + z * z), 2.0 * (x * y - w * z), 2.0 * (x * z + w * y)],
        [2.0 * (x * y + w * z), 1.0 - 2.0 * (x * x + z * z), 2.0 * (y * z - w * x)],
        [2.0 * (x * z - w * y), 2.0 * (y * z + w * x), 1.0 - 2.0 * (x * x + y * y)],
    ]
}
fn times(m: &[V3; 3], v: V3) -> V3 {
    [dot(m[0], v), dot(m[1], v), dot(m[2], v)]
}
/** `v` turned by `q`. */
fn rot(q: Q, v: V3) -> V3 {
    times(&matrix(q), v)
}

fn times_t(m: &[V3; 3], v: V3) -> V3 {
    std::array::from_fn(|c| m[0][c] * v[0] + m[1][c] * v[1] + m[2][c] * v[2])
}

/** Where a cell's matter is and how it is turned. */
#[derive(Clone, Copy, Debug)]
struct Pose {
    x: V3,
    q: Q,
}

#[derive(Clone, Copy, Debug, Default)]
struct Motion {
    v: V3,
    w: V3,
}

/** A cell: awake, by its index, or asleep, by its site. */
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Site {
    Awake(u32),
    Asleep([i64; 3]),
}

/** Where a spring holds a cell and the spring's frame at rest, both in the cell's own frame. */
#[derive(Clone, Copy, Debug)]
struct Anchor {
    a: V3,
    c: Q,
}

/**
 * Two cells held on to each other across a face, by a beam through it:
 * its anchors, which at rest are at one point with one frame; the way from
 * the first to the second in that frame; the material where they meet; how
 * much of the face holds on.
 */
#[derive(Clone, Debug)]
struct Bond {
    at: [Site; 2],
    anchors: [Anchor; 2],
    n: V3,
    material: Material,
    area: f64,
}

/** Two cells, the first awake, that may meet this step, and the material where they would. */
#[derive(Clone, Copy)]
struct Touch {
    at: [Site; 2],
    material: Material,
}

/** A bond's stiffness and damping, along, across, in bending and in torsion; its section's side, a square of its area. */
#[derive(Clone, Copy)]
struct Stiff {
    k: [f64; 4],
    c: [f64; 4],
    side: f64,
}

/** What a step reads (`Matter::frame_of`): each bond's stiffness, what may touch, how long a step is stable. */
struct Frame {
    stiff: Vec<Stiff>,
    touch: Vec<Touch>,
    dt: f64,
}

/** What a step finds: a bond broken, a cell asleep to wake. */
enum Event {
    Broke(usize),
    Wake([i64; 3]),
}

fn face_to(a: [i64; 3], b: [i64; 3]) -> Option<usize> {
    faces(a).position(|s| s == b)
}

pub struct Matter<T> {
    pub level: u32,
    h: f64,
    pub cells: Vec<Cell<T>>,
    pose: Vec<Pose>,
    motion: Vec<Motion>,
    at: FastMap<[i64; 3], u32>,
    bonds: Vec<Bond>,
    slow: Vec<f64>,
    /** Where the matter of cells that fell asleep lies, while any of it is awake: elsewhere a cell asleep lies at its site's middle. */
    still: FastMap<[i64; 3], Pose>,
    waking: FastSet<[i64; 3]>,
    pub time: f64,
    /** Steps taken, each a round of every cell; bonds broken. */
    pub rounds: usize,
    pub broke: usize,
}

impl<T> Matter<T> {
    pub fn new(level: u32, unit: f64) -> Matter<T> {
        Matter {
            level,
            h: unit * f64::from(1u32 << level),
            cells: Vec::new(),
            pose: Vec::new(),
            motion: Vec::new(),
            at: FastMap::default(),
            bonds: Vec::new(),
            slow: Vec::new(),
            still: FastMap::default(),
            waking: FastSet::default(),
            time: 0.0,
            rounds: 0,
            broke: 0,
        }
    }

    pub fn awake(&self) -> bool {
        !self.cells.is_empty()
    }

    pub fn at(&self, site: [i64; 3]) -> Option<usize> {
        self.at.get(&site).map(|&i| i as usize)
    }

    pub fn welds(&self) -> usize {
        self.bonds.len()
    }

    fn site_middle(&self, s: [i64; 3]) -> V3 {
        s.map(|v| (v as f64 + 0.5) * self.h)
    }

    /** Where cell `i`'s matter is, metres. */
    pub fn middle(&self, i: usize) -> V3 {
        self.pose[i].x
    }

    fn pose_of(&self, s: Site) -> Pose {
        match s {
            Site::Awake(i) => self.pose[i as usize],
            Site::Asleep(s) => self.still.get(&s).copied().unwrap_or(Pose { x: self.site_middle(s), q: ID }),
        }
    }

    fn motion_of(&self, s: Site) -> Motion {
        match s {
            Site::Awake(i) => self.motion[i as usize],
            Site::Asleep(_) => Motion::default(),
        }
    }

    /** How heavy a cell is: one asleep is as heavy as the world. */
    fn mass_of(&self, s: Site) -> f64 {
        match s {
            Site::Awake(i) => self.cells[i as usize].mass,
            Site::Asleep(_) => f64::INFINITY,
        }
    }

    fn site_of(&self, s: Site) -> [i64; 3] {
        match s {
            Site::Awake(i) => self.cells[i as usize].site,
            Site::Asleep(s) => s,
        }
    }

    /** Of two cells, the mass that moves as their spring stretches. */
    fn reduced(&self, at: [Site; 2]) -> f64 {
        1.0 / (1.0 / self.mass_of(at[0]) + 1.0 / self.mass_of(at[1]))
    }

    /**
     * Wakes `cells`, asleep till now, at rest where their matter lies: held
     * on where the lattice says, to each other and to cells asleep; what
     * held on to them asleep holds on to them awake.
     */
    pub fn wake(&mut self, cells: Vec<Cell<T>>, lattice: &mut dyn Lattice) {
        let first = self.cells.len();
        for c in cells {
            assert!(self.at.insert(c.site, self.cells.len() as u32).is_none(), "{:?} awake already", c.site);
            self.pose.push(self.pose_of(Site::Asleep(c.site)));
            self.motion.push(Motion::default());
            self.slow.push(0.0);
            self.cells.push(c);
        }
        for b in &mut self.bonds {
            for s in &mut b.at {
                if let Site::Asleep(site) = *s
                    && let Some(&i) = self.at.get(&site)
                {
                    *s = Site::Awake(i);
                }
            }
        }
        for i in first..self.cells.len() {
            let site = self.cells[i].site;
            for (f, s) in faces(site).enumerate() {
                let (other, material) = match self.at.get(&s) {
                    Some(&j) if j as usize > i => (Site::Awake(j), self.cells[j as usize].material),
                    Some(_) => continue,
                    None => match lattice.asleep(s) {
                        Some(m) => (Site::Asleep(s), m),
                        None => continue,
                    },
                };
                let area = lattice.joined(site, f);
                if area > 0.0 {
                    let bond = self.bond([Site::Awake(i as u32), other], self.cells[i].material.meet(material), area);
                    self.bonds.push(bond);
                }
            }
        }
        let at = &self.at;
        self.waking.retain(|s| !at.contains_key(s));
        self.hold_on();
    }

    /** A bond between two cells at rest as they lie: anchored half way between them, in the frame half way between theirs. */
    fn bond(&self, at: [Site; 2], material: Material, area: f64) -> Bond {
        let p = at.map(|s| self.pose_of(s));
        let point = scale(add(p[0].x, p[1].x), 0.5);
        let frame = normal(add4(p[0].q, aligned(p[1].q, p[0].q)));
        let anchors = p.map(|p| Anchor { a: rot(conj(p.q), sub(point, p.x)), c: qmul(conj(p.q), frame) });
        let d = rot(conj(frame), sub(p[1].x, p[0].x));
        Bond { at, anchors, n: scale(d, 1.0 / len(d)), material, area }
    }

    /** As `mechanics::Matter::changed`: cells holding nothing that bears fall asleep; bonds at `sites` hold as the lattice says now. */
    pub fn changed(&mut self, sites: &FastSet<[i64; 3]>, lattice: &mut dyn Lattice) -> Vec<Cell<T>> {
        let gone: Vec<bool> = self.cells.iter().map(|c| c.fill == 0.0).collect();
        let gone = if gone.contains(&true) { self.remove(&gone) } else { Vec::new() };
        for k in 0..self.bonds.len() {
            let [a, c] = self.bonds[k].at.map(|s| self.site_of(s));
            // A bond not across a face of its cells' sites, as they have moved, holds as it did.
            if (sites.contains(&a) || sites.contains(&c))
                && let Some(f) = face_to(a, c)
            {
                self.bonds[k].area = lattice.joined(a, f);
            }
        }
        self.bonds.retain(|b| b.area > 0.0);
        self.hold_on();
        gone
    }

    /** Puts the cells `gone` says to sleep where their matter lies: what held on to them holds on to them asleep. */
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
        let asleep = |s: Site| match s {
            Site::Awake(i) if gone[i as usize] => Site::Asleep(cells[i as usize].site),
            Site::Awake(i) => Site::Awake(index[i as usize]),
            s => s,
        };
        for b in &mut self.bonds {
            b.at = b.at.map(asleep);
        }
        self.bonds.retain(|b| b.at.iter().any(|s| matches!(s, Site::Awake(_))));
        let mut taken = Vec::new();
        let all = std::mem::take(&mut self.cells)
            .into_iter()
            .zip(std::mem::take(&mut self.pose))
            .zip(std::mem::take(&mut self.motion))
            .zip(std::mem::take(&mut self.slow));
        for (i, (((c, p), m), s)) in all.enumerate() {
            if gone[i] {
                self.still.insert(c.site, p);
                taken.push(c);
            } else {
                self.cells.push(c);
                self.pose.push(p);
                self.motion.push(m);
                self.slow.push(s);
            }
        }
        if self.cells.is_empty() {
            self.still.clear();
        }
        self.at = self.cells.iter().enumerate().map(|(i, c)| (c.site, i as u32)).collect();
        self.hold_on();
        taken
    }

    pub fn sleep(&mut self) -> Vec<Cell<T>> {
        let gone = vec![true; self.cells.len()];
        self.remove(&gone)
    }

    pub fn waking(&mut self) -> Vec<[i64; 3]> {
        self.waking.drain().collect()
    }

    /** Each cell awake holding on across the faces its bonds lie across now. */
    fn hold_on(&mut self) {
        let mut holds = vec![0; self.cells.len()];
        for b in &self.bonds {
            let x = b.at.map(|s| self.pose_of(s).x);
            for (k, s) in b.at.iter().enumerate() {
                if let Site::Awake(i) = *s {
                    let d = sub(x[1 - k], x[k]);
                    let axis = (0..3).max_by(|&a, &c| d[a].abs().total_cmp(&d[c].abs())).expect("an axis");
                    holds[i as usize] |= 1 << (2 * axis + usize::from(d[axis] > 0.0));
                }
            }
        }
        for (c, h) in self.cells.iter_mut().zip(holds) {
            c.holds = h;
        }
    }

    /** A bond's stiffness, as a beam of its material a cell long, and its damping, of the mass its cells move with. */
    fn stiffness(&self, b: &Bond) -> Stiff {
        let e = b.material.e;
        let g = e / (2.0 * (1.0 + NU));
        let side = b.area.sqrt() * self.h;
        let a = side * side;
        // A square's second moment, and its torsion constant, 0.141 of its side to the fourth.
        let (i, j) = (a * a / 12.0, 0.141 * a * a);
        let k = [e * a, g * a, e * i, g * j].map(|v| v / self.h);
        let m = self.reduced(b.at);
        let inertia = m * self.h * self.h / 6.0;
        let c = std::array::from_fn(|n| 2.0 * DAMPING * (k[n] * if n < 2 { m } else { inertia }).sqrt());
        Stiff { k, c, side }
    }

    /**
     * Bond `b` as it is: the force and moment on its first cell, where its
     * anchors are, and the share of its strength it uses. The gap between
     * its anchors and the turn between their frames, each in the frame half
     * way between, are as small as matter bears, so taken to first order.
     */
    fn strained(&self, b: &Bond, s: &Stiff) -> (V3, V3, [V3; 2], f64) {
        let p = b.at.map(|s| self.pose_of(s));
        let m = b.at.map(|s| self.motion_of(s));
        let at: [V3; 2] = std::array::from_fn(|k| add(p[k].x, rot(p[k].q, b.anchors[k].a)));
        let f: [Q; 2] = std::array::from_fn(|k| qmul(p[k].q, b.anchors[k].c));
        let f1 = aligned(f[1], f[0]);
        let turn = qmul(conj(f[0]), f1);
        let to = matrix(normal(add4(f[0], f1)));
        let theta = scale([turn[1], turn[2], turn[3]], 2.0);
        let velocity = |k: usize| add(m[k].v, cross(m[k].w, sub(at[k], p[k].x)));
        let gap = times_t(&to, sub(at[1], at[0]));
        let speed = times_t(&to, sub(velocity(1), velocity(0)));
        let spin = times_t(&to, sub(m[1].w, m[0].w));
        let n = b.n;
        let split = |v: V3| {
            let along = dot(v, n);
            (along, sub(v, scale(n, along)))
        };
        let ((along, slide), (along_v, slide_v)) = (split(gap), split(speed));
        let ((twist, bend), (twist_w, bend_w)) = (split(theta), split(spin));
        let (k, c) = (&s.k, &s.c);
        let spring = |x: V3, rate: V3, k: f64, c: f64| add(scale(x, k), scale(rate, c));
        let force = add(spring(scale(n, along), scale(n, along_v), k[0], c[0]), spring(slide, slide_v, k[1], c[1]));
        let moment = add(spring(scale(n, twist), scale(n, twist_w), k[3], c[3]), spring(bend, bend_w, k[2], c[2]));
        // Its stresses, as beam theory has them at the face: pulled or pushed and bent, at its edge; slid and twisted, at the middle of a side.
        let mat = &b.material;
        let (side, a) = (s.side, s.side * s.side);
        let bent = k[2] * len(bend) * (side / 2.0) / (a * a / 12.0);
        let pull = k[0] * along / a;
        let shear = 1.5 * k[1] * len(slide) / a + k[3] * twist.abs() / (0.208 * side * a);
        let used = ((pull + bent) / mat.tension).max((bent - pull) / mat.compression).max(shear / mat.shear);
        (times(&to, force), times(&to, moment), at, used)
    }

    /**
     * Two cells touching as they are, as balls a cell across: the force on
     * the first, as much as their overlap and how fast they close push it
     * apart, held against sliding as far as friction lets it; and the
     * share of its strength that push uses. A bond past its strength
     * breaks; a contact gives.
     */
    fn touched(&self, t: &Touch) -> Option<(V3, f64)> {
        let p = t.at.map(|s| self.pose_of(s).x);
        let d = sub(p[0], p[1]);
        let l2 = dot(d, d);
        if l2 >= self.h * self.h || l2 < 1e-24 {
            return None;
        }
        let l = l2.sqrt();
        let normal = scale(d, 1.0 / l);
        let [m0, m1] = t.at.map(|s| self.motion_of(s).v);
        let rel = sub(m0, m1);
        let k = t.material.e * self.h;
        let c = 2.0 * DAMPING * (k * self.reduced(t.at)).sqrt();
        // Pushed apart by the overlap as a bond's spring is, but no faster than PUSH_APART: what lands stops.
        // Held against sliding as it is damped, but no harder than friction lets it.
        let closing = -dot(rel, normal);
        let deep = if closing < -PUSH_APART { 0.0 } else { self.h - l };
        // Its strength, a face's in compression or, held against sliding, in shear: pushed past it, it gives, as what crushes does.
        let m = &t.material;
        let strength = m.compression.min(m.shear / FRICTION) * self.h * self.h;
        let push = (k * deep + c * closing).clamp(0.0, strength);
        let held = drag(sub(rel, scale(normal, -closing)), c, FRICTION * push);
        Some((sub(scale(normal, push), held), push / strength))
    }

    /**
     * What a step reads, as the cells are now: each bond's stiffness, the
     * pairs of cells not held on to each other near enough to touch while
     * nothing moves more than `reach`, and how long a step is stable.
     */
    fn frame_of(&self, lattice: &mut dyn Lattice, reach: f64) -> Frame {
        let n = self.cells.len();
        let stiff: Vec<Stiff> = self.bonds.iter().map(|b| self.stiffness(b)).collect();
        let bonded: FastSet<[Site; 2]> = self.bonds.iter().flat_map(|b| [b.at, [b.at[1], b.at[0]]]).collect();
        let mut touch = Vec::new();
        let mut met: FastSet<(u32, u32)> = FastSet::default();
        for i in 0..n {
            let me = Site::Awake(i as u32);
            let x = self.pose[i].x;
            let here = x.map(|v| (v / self.h).floor() as i64);
            for s in std::iter::once(here).chain(about(here)) {
                let (other, material) = match self.at.get(&s) {
                    Some(&j) if j as usize != i && met.insert((j.min(i as u32), j.max(i as u32))) => {
                        (Site::Awake(j), self.cells[j as usize].material)
                    }
                    Some(_) => continue,
                    None => match lattice.asleep(s) {
                        Some(m) => (Site::Asleep(s), m),
                        None => continue,
                    },
                };
                if len(sub(self.pose_of(other).x, x)) < self.h + 2.0 * reach && !bonded.contains(&[me, other]) {
                    touch.push(Touch { at: [me, other], material: self.cells[i].material.meet(material) });
                }
            }
        }
        // The longest step stable: each cell's fastest motion by the springs that hold it, as Gershgorin bounds it.
        let (mut k, mut kr) = (vec![0.0; n], vec![0.0; n]);
        let mut add_to = |at: [Site; 2], along: f64, turning: f64| {
            for s in at {
                if let Site::Awake(i) = s {
                    k[i as usize] += along;
                    kr[i as usize] += turning;
                }
            }
        };
        for (b, s) in self.bonds.iter().zip(&stiff) {
            add_to(b.at, s.k[0] + 2.0 * s.k[1], 2.0 * s.k[2] + s.k[3] + s.k[1] * self.h * self.h / 2.0);
        }
        for t in &touch {
            add_to(t.at, t.material.e * self.h, 0.0);
        }
        let most =
            (0..n).map(|i| (k[i] / self.cells[i].mass).max(kr[i] / (self.cells[i].mass * self.h * self.h / 6.0))).fold(0.0, f64::max);
        // Damped, a swing ω fast is stable in steps shorter than 2 (√(1 + ζ²) − ζ) / ω.
        let damped = 2.0 * ((1.0 + DAMPING * DAMPING).sqrt() - DAMPING);
        Frame { stiff, touch, dt: SAFETY * damped / (2.0 * most).sqrt() }
    }

    /**
     * A step of time, FRAME at most and as long as moves nothing more than
     * REACH of a cell, in steps as long as stay stable: in each, every
     * cell awake is pushed by its springs as they were, and by its weight,
     * and moves; bonds past their strength break. Then matter moves to the
     * next site, and what is at rest falls asleep, and is given back.
     */
    pub fn step(&mut self, lattice: &mut dyn Lattice) -> Vec<Cell<T>> {
        if self.cells.is_empty() {
            return Vec::new();
        }
        let fastest = self.motion.iter().map(|m| len(m.v) + len(m.w) * self.h).fold(0.0, f64::max);
        let length = FRAME.min(REACH * self.h / fastest.max(1e-9));
        let frame = self.frame_of(lattice, REACH * self.h);
        let steps = (length / frame.dt).ceil().max(1.0) as usize;
        let dt = length / steps as f64;
        let n = self.cells.len();
        let mut pushed = vec![([0.0; 3], [0.0; 3]); n];
        let mut events = Vec::new();
        for _ in 0..steps {
            pushed.fill(([0.0; 3], [0.0; 3]));
            // Each spring: a force on its first cell, its opposite on its second, where it holds them.
            let mut push = |at: [Site; 2], f: V3, m: V3, on: [V3; 2], used: f64, events: &mut Vec<Event>| {
                for (k, s) in at.into_iter().enumerate() {
                    let sign = if k == 0 { 1.0 } else { -1.0 };
                    match s {
                        Site::Awake(i) => {
                            let (i, x) = (i as usize, self.pose[i as usize].x);
                            let p = &mut pushed[i];
                            *p = (add(p.0, scale(f, sign)), add(p.1, scale(add(cross(sub(on[k], x), f), m), sign)));
                        }
                        Site::Asleep(s) if used > WAKE => events.push(Event::Wake(s)),
                        Site::Asleep(_) => {}
                    }
                }
            };
            for (k, (b, s)) in self.bonds.iter().zip(&frame.stiff).enumerate() {
                if b.area == 0.0 {
                    continue;
                }
                let (f, m, on, used) = self.strained(b, s);
                push(b.at, f, m, on, used, &mut events);
                if used > 1.0 {
                    events.push(Event::Broke(k));
                }
            }
            for t in &frame.touch {
                if let Some((f, used)) = self.touched(t) {
                    let on = t.at.map(|s| self.pose_of(s).x);
                    push(t.at, f, [0.0; 3], on, used, &mut events);
                }
            }
            for (i, &(force, torque)) in pushed.iter().enumerate() {
                let m = self.cells[i].mass;
                let mo = &mut self.motion[i];
                mo.v = add(add(mo.v, scale(force, dt / m)), [0.0, -GRAVITY * dt, 0.0]);
                mo.w = add(mo.w, scale(torque, dt * 6.0 / (m * self.h * self.h)));
                let p = &mut self.pose[i];
                p.x = add(p.x, scale(mo.v, dt));
                p.q = normal(qmul(exp(scale(mo.w, dt)), p.q));
            }
            for e in events.drain(..) {
                match e {
                    // A bond broken holds nothing from now on, and goes after the steps.
                    Event::Broke(k) => {
                        self.bonds[k].area = 0.0;
                        self.broke += 1;
                    }
                    Event::Wake(s) => _ = self.waking.insert(s),
                }
            }
            self.rounds += 1;
        }
        self.bonds.retain(|b| b.area > 0.0);
        self.time += length;
        for i in 0..n {
            let m = &self.motion[i];
            let speed = len(m.v) + len(m.w) * self.h / 2.0;
            self.slow[i] = if speed < SLOW { self.slow[i] + length } else { 0.0 };
        }
        self.hops(lattice);
        self.hold_on();
        let rested = |i: usize| self.slow[i] >= REST;
        let gone: Vec<bool> =
            (0..n).map(|i| rested(i) && about(self.cells[i].site).all(|s| self.at.get(&s).is_none_or(|&j| rested(j as usize)))).collect();
        if gone.contains(&true) { self.remove(&gone) } else { Vec::new() }
    }

    /** A cell more than HOP of a cell off its site moves across the face it is past, to a site that holds nothing. */
    fn hops(&mut self, lattice: &mut dyn Lattice) {
        let mut hopped = true;
        while hopped {
            hopped = false;
            for i in 0..self.cells.len() {
                for a in 0..3 {
                    let from = self.cells[i].site;
                    let off = self.pose[i].x[a] - self.site_middle(from)[a];
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
    }
}

/** What holds back a contact sliding at `rate`: as much as damping `c` asks, but no more than friction's `most`. */
fn drag(rate: V3, c: f64, most: f64) -> V3 {
    let r = len(rate);
    if r * c <= most { scale(rate, c) } else { scale(rate, most / r) }
}

fn add4(a: Q, b: Q) -> Q {
    std::array::from_fn(|k| a[k] + b[k])
}

/** `q`, or its negative, the same turn: whichever is nearer `to`. */
fn aligned(q: Q, to: Q) -> Q {
    if (0..4).map(|k| q[k] * to[k]).sum::<f64>() < 0.0 { q.map(|v| -v) } else { q }
}

#[cfg(test)]
mod tests;
