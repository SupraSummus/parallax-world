//! Mechanics by local rules, on the lattice, with `mechanics/`'s interface
//! (docs/mechanics.md, "By local rules"). Every site holding matter is a
//! cell, a rigid cube; between two cells is a spring, a bond if they hold on
//! to each other, a contact if not, either acting half way between them. A
//! step is explicit: each spring works out what it does from its cells as
//! they are, then each cell moves by its springs.

// Small matrices and vectors by index read as the algebra does.
#![allow(clippy::needless_range_loop)]

use crew::Shared;
use lattice::{FastMap, FastSet};
pub use mechanics::{Cell, Lattice, Material, WOOD, about, faces};

pub type V3 = [f64; 3];
type Q = [f64; 4];

/** Up is +y, as the hosts keep it. */
const GRAVITY: V3 = [0.0, -9.81, 0.0];
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
/** Springs a thread works, at least: fewer, and it waits at the barriers longer than it works. */
const SPRINGS_A_THREAD: usize = 256;

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

/** A cell's matter: where it is, how it is turned, as a quaternion and as a matrix, and how it moves. */
#[derive(Clone, Copy, Debug)]
struct Body {
    x: V3,
    q: Q,
    turn: [V3; 3],
    v: V3,
    w: V3,
}

impl Body {
    fn at(x: V3, q: Q) -> Body {
        Body { x, q, turn: matrix(q), v: [0.0; 3], w: [0.0; 3] }
    }
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

/** A bond's stiffness and damping, along, across, in bending and in torsion; its section's side, a square of its area. */
#[derive(Clone, Copy)]
struct Stiff {
    k: [f64; 4],
    c: [f64; 4],
    side: f64,
}

/** A contact's stiffness and damping, and its strength. */
#[derive(Clone, Copy)]
struct Touch {
    k: f64,
    c: f64,
    strength: f64,
}

/** What a spring is, for a frame: bond `b` as stiff as it is, a contact, or a bond broken. */
#[derive(Clone, Copy)]
enum Law {
    Bond(u32, Stiff),
    Touch(Touch),
    Broken,
}

/**
 * A spring between the cells at two slots, by its law, and what it does in
 * a round: the force on its first cell, its opposite on its second, and the
 * moment on each, about its middle.
 */
#[derive(Clone, Copy)]
struct Spring {
    at: [u32; 2],
    law: Law,
    f: V3,
    m: [V3; 2],
}

/**
 * What a frame's rounds read and move. The cells, by slot: those awake, in
 * order along a Z-order curve, so a thread's part of them is near each
 * other; then those asleep that a spring reaches, as heavy as the world.
 * The springs, in the order of the lower slot of their two, so a thread's are of its
 * cells mostly; and each cell awake's springs, by which end it is of each.
 */
struct Frame {
    body: Vec<Body>,
    mass: Vec<f64>,
    /** Of each slot awake, its cell; of each slot asleep, from the first after them, its site. */
    order: Vec<u32>,
    asleep: Vec<[i64; 3]>,
    springs: Vec<Spring>,
    start: Vec<usize>,
    ends: Vec<(u32, usize)>,
    dt: f64,
}

fn face_to(a: [i64; 3], b: [i64; 3]) -> Option<usize> {
    faces(a).position(|s| s == b)
}

pub struct Matter<T> {
    pub level: u32,
    h: f64,
    pub cells: Vec<Cell<T>>,
    body: Vec<Body>,
    at: FastMap<[i64; 3], u32>,
    bonds: Vec<Bond>,
    slow: Vec<f64>,
    /** Where the matter of cells that fell asleep lies, while any of it is awake: elsewhere a cell asleep lies at its site's middle. */
    still: FastMap<[i64; 3], Body>,
    waking: FastSet<[i64; 3]>,
    pub time: f64,
    /** Steps taken, each a round of every cell; bonds broken. */
    pub rounds: usize,
    pub broke: usize,
    /** Threads a step's rounds are worked on, at most. */
    pub threads: usize,
}

impl<T> Matter<T> {
    pub fn new(level: u32, unit: f64) -> Matter<T> {
        Matter {
            level,
            h: unit * f64::from(1u32 << level),
            cells: Vec::new(),
            body: Vec::new(),
            at: FastMap::default(),
            bonds: Vec::new(),
            slow: Vec::new(),
            still: FastMap::default(),
            waking: FastSet::default(),
            time: 0.0,
            rounds: 0,
            broke: 0,
            threads: crew::cores(),
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
        self.body[i].x
    }

    /** Where the matter of the cell asleep at `site` lies, still. */
    fn still(&self, site: [i64; 3]) -> Body {
        self.still.get(&site).copied().unwrap_or_else(|| Body::at(self.site_middle(site), ID))
    }

    fn body_of(&self, s: Site) -> Body {
        match s {
            Site::Awake(i) => self.body[i as usize],
            Site::Asleep(s) => self.still(s),
        }
    }

    fn site_of(&self, s: Site) -> [i64; 3] {
        match s {
            Site::Awake(i) => self.cells[i as usize].site,
            Site::Asleep(s) => s,
        }
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
            self.body.push(self.still(c.site));
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
        let p = at.map(|s| self.body_of(s));
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
        let all = std::mem::take(&mut self.cells).into_iter().zip(std::mem::take(&mut self.body)).zip(std::mem::take(&mut self.slow));
        for (i, ((c, b), s)) in all.enumerate() {
            if gone[i] {
                self.still.insert(c.site, Body::at(b.x, b.q));
                taken.push(c);
            } else {
                self.cells.push(c);
                self.body.push(b);
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
            let x = b.at.map(|s| self.body_of(s).x);
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

    /** A bond's stiffness, as a beam of its material a cell long, and its damping, of `m`, the mass its cells move with. */
    fn stiffness(&self, b: &Bond, m: f64) -> Stiff {
        let e = b.material.e;
        let g = e / (2.0 * (1.0 + NU));
        let side = b.area.sqrt() * self.h;
        let a = side * side;
        // A square's second moment, and its torsion constant, 0.141 of its side to the fourth.
        let (i, j) = (a * a / 12.0, 0.141 * a * a);
        let k = [e * a, g * a, e * i, g * j].map(|v| v / self.h);
        let inertia = m * self.h * self.h / 6.0;
        let c = std::array::from_fn(|n| 2.0 * DAMPING * (k[n] * if n < 2 { m } else { inertia }).sqrt());
        Stiff { k, c, side }
    }

    /** A contact of `material`: as stiff as a bond along, damped as `m` asks; as strong as a face in compression or, held against sliding, in shear. */
    fn touch(&self, material: Material, m: f64) -> Touch {
        let k = material.e * self.h;
        let strength = material.compression.min(material.shear / FRICTION) * self.h * self.h;
        Touch { k, c: 2.0 * DAMPING * (k * m).sqrt(), strength }
    }

    /**
     * Bond `b`, its cells as `body` has them at `at`: what it does to them,
     * and the share of its strength it uses. The gap between its anchors
     * and the turn between their frames, each in the frame half way
     * between, are as small as matter bears, so taken to first order.
     */
    fn strained(b: &Bond, s: &Stiff, at: [u32; 2], body: &[Body]) -> ((V3, [V3; 2]), f64) {
        let p = at.map(|k| &body[k as usize]);
        let arm: [V3; 2] = std::array::from_fn(|k| times(&p[k].turn, b.anchors[k].a));
        let f: [Q; 2] = std::array::from_fn(|k| qmul(p[k].q, b.anchors[k].c));
        let f1 = aligned(f[1], f[0]);
        let turn = qmul(conj(f[0]), f1);
        let to = matrix(normal(add4(f[0], f1)));
        let theta = scale([turn[1], turn[2], turn[3]], 2.0);
        let velocity = |k: usize| add(p[k].v, cross(p[k].w, arm[k]));
        let gap = times_t(&to, sub(add(p[1].x, arm[1]), add(p[0].x, arm[0])));
        let speed = times_t(&to, sub(velocity(1), velocity(0)));
        let spin = times_t(&to, sub(p[1].w, p[0].w));
        let n = b.n;
        let split = |v: V3| {
            let along = dot(v, n);
            (along, sub(v, scale(n, along)))
        };
        let ((along, slide), (along_v, slide_v)) = (split(gap), split(speed));
        let ((twist, bend), (twist_w, bend_w)) = (split(theta), split(spin));
        let (k, c) = (&s.k, &s.c);
        let spring = |x: V3, rate: V3, k: f64, c: f64| add(scale(x, k), scale(rate, c));
        let force = times(&to, add(spring(scale(n, along), scale(n, along_v), k[0], c[0]), spring(slide, slide_v, k[1], c[1])));
        let moment = times(&to, add(spring(scale(n, twist), scale(n, twist_w), k[3], c[3]), spring(bend, bend_w, k[2], c[2])));
        // Its stresses, as beam theory has them at the face: pulled or pushed and bent, at its edge; slid and twisted, at the middle of a side.
        let mat = &b.material;
        let (side, a) = (s.side, s.side * s.side);
        let bent = k[2] * len(bend) * (side / 2.0) / (a * a / 12.0);
        let pull = k[0] * along / a;
        let shear = 1.5 * k[1] * len(slide) / a + k[3] * twist.abs() / (0.208 * side * a);
        let used = ((pull + bent) / mat.tension).max((bent - pull) / mat.compression).max(shear / mat.shear);
        ((force, [add(cross(arm[0], force), moment), scale(add(cross(arm[1], force), moment), -1.0)]), used)
    }

    /**
     * Two cells touching as they are, as balls a cell across, at the point
     * half way between them: the force on the first there, as much as
     * their overlap and how fast they close push it apart, held against
     * sliding as far as friction lets it, and the moment that is on each;
     * and the share of its strength that push uses. A bond past its
     * strength breaks; a contact gives.
     */
    fn touched(h: f64, t: &Touch, at: [u32; 2], body: &[Body]) -> ((V3, [V3; 2]), f64) {
        let [p, o] = at.map(|k| &body[k as usize]);
        let d = sub(p.x, o.x);
        let l2 = dot(d, d);
        if l2 >= h * h || l2 < 1e-24 {
            return (([0.0; 3], [[0.0; 3]; 2]), 0.0);
        }
        let l = l2.sqrt();
        let normal = scale(d, 1.0 / l);
        let arm = scale(d, -0.5);
        let rel = sub(add(p.v, cross(p.w, arm)), add(o.v, cross(o.w, scale(arm, -1.0))));
        // Pushed apart by the overlap as a bond's spring is, but no faster than PUSH_APART: what lands stops.
        // Held against sliding as it is damped, but no harder than friction lets it.
        let closing = -dot(rel, normal);
        let deep = if closing < -PUSH_APART { 0.0 } else { h - l };
        let push = (t.k * deep + t.c * closing).clamp(0.0, t.strength);
        let force = sub(scale(normal, push), drag(sub(rel, scale(normal, -closing)), t.c, FRICTION * push));
        // The arms are opposite and the force on the second is too: the moments on the two are one.
        let moment = cross(arm, force);
        ((force, [moment, moment]), push / t.strength)
    }

    /**
     * What a frame's rounds read, as the cells are now: the cells by slot;
     * the springs, each bond's and those of the cells not held on to each
     * other near enough to touch while nothing moves more than `reach`; and
     * how long a round is stable.
     */
    fn frame_of(&self, lattice: &mut dyn Lattice, reach: f64) -> Frame {
        let (n, h) = (self.cells.len(), self.h);
        let mut order: Vec<u32> = (0..n as u32).collect();
        order.sort_by_key(|&i| z_order(self.cells[i as usize].site));
        let mut rank = vec![0; n];
        for (s, &i) in order.iter().enumerate() {
            rank[i as usize] = s as u32;
        }
        let mut body: Vec<Body> = order.iter().map(|&i| self.body[i as usize]).collect();
        let mut mass: Vec<f64> = order.iter().map(|&i| self.cells[i as usize].mass).collect();
        let (mut asleep, mut slot_of) = (Vec::new(), FastMap::default());
        let mut slot = |s: Site| match s {
            Site::Awake(i) => rank[i as usize],
            Site::Asleep(site) => *slot_of.entry(site).or_insert_with(|| {
                body.push(self.still(site));
                mass.push(f64::INFINITY);
                asleep.push(site);
                (body.len() - 1) as u32
            }),
        };
        // The springs' cells and their laws: each bond's, then each pair of cells not held on to each other that may touch.
        let mut pairs: Vec<([u32; 2], Option<Material>)> = Vec::new();
        for b in &self.bonds {
            pairs.push((b.at.map(&mut slot), None));
        }
        let bonded: FastSet<[u32; 2]> = pairs.iter().map(|&([a, b], _)| [a.min(b), a.max(b)]).collect();
        // Cells awake by the box two cells across their middle is in: two that may touch are in boxes side by side.
        let in_box = |x: V3| x.map(|v| (v / (2.0 * h)).floor() as i64);
        let mut boxes: FastMap<[i64; 3], Vec<u32>> = FastMap::default();
        for (i, b) in self.body.iter().enumerate() {
            boxes.entry(in_box(b.x)).or_default().push(i as u32);
        }
        for (i, b) in self.body.iter().enumerate() {
            let near = |o: &Body| len(sub(o.x, b.x)) < h + 2.0 * reach;
            let mut others: Vec<(Site, Material)> = Vec::new();
            let in_boxes = std::iter::once(in_box(b.x)).chain(about(in_box(b.x))).flat_map(|b| boxes.get(&b)).flatten();
            others.extend(
                in_boxes
                    .filter(|&&j| j > i as u32 && near(&self.body[j as usize]))
                    .map(|&j| (Site::Awake(j), self.cells[j as usize].material)),
            );
            // Cells asleep lie at their sites, so those that may touch are a site away.
            let here = b.x.map(|v| (v / h).floor() as i64);
            for site in std::iter::once(here).chain(about(here)) {
                if !self.at.contains_key(&site)
                    && let Some(m) = lattice.asleep(site)
                    && near(&self.still(site))
                {
                    others.push((Site::Asleep(site), m));
                }
            }
            for (other, material) in others {
                let at = [rank[i], slot(other)];
                if !bonded.contains(&[at[0].min(at[1]), at[0].max(at[1])]) {
                    pairs.push((at, Some(self.cells[i].material.meet(material))));
                }
            }
        }
        // Each spring damped as the mass its cells move with asks: one asleep, as heavy as the world, moves with none of its own.
        let reduced = |[a, b]: [u32; 2]| 1.0 / (1.0 / mass[a as usize] + 1.0 / mass[b as usize]);
        let mut springs: Vec<Spring> = pairs
            .into_iter()
            .enumerate()
            .map(|(k, (at, touching))| {
                let law = match touching {
                    None => Law::Bond(k as u32, self.stiffness(&self.bonds[k], reduced(at))),
                    Some(m) => Law::Touch(self.touch(m, reduced(at))),
                };
                Spring { at, law, f: [0.0; 3], m: [[0.0; 3]; 2] }
            })
            .collect();
        springs.sort_by_key(|s| s.at[0].min(s.at[1]));
        // Each cell awake's springs, and which end of each it is.
        let mut start = vec![0; n + 1];
        for &i in springs.iter().flat_map(|s| &s.at).filter(|&&i| (i as usize) < n) {
            start[i as usize + 1] += 1;
        }
        for i in 0..n {
            start[i + 1] += start[i];
        }
        let mut ends = vec![(0, 0); start[n]];
        let mut next = start.clone();
        for (k, s) in springs.iter().enumerate() {
            for (end, &i) in s.at.iter().enumerate().filter(|&(_, &i)| (i as usize) < n) {
                ends[next[i as usize]] = (k as u32, end);
                next[i as usize] += 1;
            }
        }
        // The longest round stable: each cell's fastest motion by the springs that hold it, as Gershgorin bounds it.
        let (mut along, mut turning) = (vec![0.0; n], vec![0.0; n]);
        for s in &springs {
            let (k, kr) = match s.law {
                Law::Bond(_, st) => (st.k[0] + 2.0 * st.k[1], 2.0 * st.k[2] + st.k[3] + st.k[1] * h * h / 2.0),
                Law::Touch(t) => (t.k, 0.0),
                Law::Broken => (0.0, 0.0),
            };
            for i in s.at.map(|i| i as usize).into_iter().filter(|&i| i < n) {
                along[i] += k;
                turning[i] += kr;
            }
        }
        let most = (0..n).map(|s| (along[s] / mass[s]).max(turning[s] / (mass[s] * h * h / 6.0))).fold(0.0, f64::max);
        // Damped, a swing ω fast is stable in steps shorter than 2 (√(1 + ζ²) − ζ) / ω.
        let damped = 2.0 * ((1.0 + DAMPING * DAMPING).sqrt() - DAMPING);
        Frame { body, mass, order, asleep, springs, start, ends, dt: SAFETY * damped / (2.0 * most).sqrt() }
    }

    /**
     * A step of time, FRAME at most and as long as moves nothing more than
     * REACH of a cell, in rounds as long as stay stable. Each round is two
     * phases, worked on threads: each spring, from its cells as they are,
     * works out what it does to them; then each cell awake, pushed by its
     * springs and its weight, moves. Bonds past their strength break. Then
     * matter moves to the next site, and what is at rest falls asleep, and
     * is given back.
     */
    pub fn step(&mut self, lattice: &mut dyn Lattice) -> Vec<Cell<T>> {
        if self.cells.is_empty() {
            return Vec::new();
        }
        let fastest = self.body.iter().map(|b| len(b.v) + len(b.w) * self.h).fold(0.0, f64::max);
        let length = FRAME.min(REACH * self.h / fastest.max(1e-9));
        let Frame { mut body, mass, order, asleep, mut springs, start, ends, dt } = self.frame_of(lattice, REACH * self.h);
        let rounds = (length / dt).ceil().max(1.0) as usize;
        let dt = length / rounds as f64;
        let (n, h, bonds) = (self.cells.len(), self.h, &self.bonds);
        let count = springs.len();
        let threads = self.threads.min(count / SPRINGS_A_THREAD).max(1);
        let (cells, pulls) = (Shared::new(&mut body), Shared::new(&mut springs));
        let found = crew::together(threads, |t, barrier| {
            let (my_springs, my_cells) = (crew::part(count, t, threads), crew::part(n, t, threads));
            let (mut broke, mut woken) = (Vec::new(), Vec::new());
            for _ in 0..rounds {
                {
                    // SAFETY: in this phase the cells are only read, and each thread writes its own springs.
                    let (body, springs) = unsafe { (cells.all(), pulls.part(my_springs.clone())) };
                    for s in springs {
                        let ((f, m), used) = match s.law {
                            Law::Bond(b, stiff) => {
                                let strained = Self::strained(&bonds[b as usize], &stiff, s.at, body);
                                if strained.1 > 1.0 {
                                    // A bond broken holds nothing from the next round on.
                                    s.law = Law::Broken;
                                    broke.push(b);
                                }
                                strained
                            }
                            Law::Touch(touch) => Self::touched(h, &touch, s.at, body),
                            Law::Broken => (([0.0; 3], [[0.0; 3]; 2]), 0.0),
                        };
                        (s.f, s.m) = (f, m);
                        woken.extend(s.at.into_iter().filter(|&i| i as usize >= n && used > WAKE));
                    }
                }
                barrier.wait();
                {
                    // SAFETY: in this phase the springs are only read, and each thread writes its own cells.
                    let (springs, body) = unsafe { (pulls.all(), cells.part(my_cells.clone())) };
                    for (i, b) in my_cells.clone().zip(body) {
                        let (mut force, mut torque) = ([0.0; 3], [0.0; 3]);
                        for &(s, end) in &ends[start[i]..start[i + 1]] {
                            let s = &springs[s as usize];
                            force = add(force, if end == 0 { s.f } else { scale(s.f, -1.0) });
                            torque = add(torque, s.m[end]);
                        }
                        let m = mass[i];
                        b.v = add(b.v, add(scale(force, dt / m), scale(GRAVITY, dt)));
                        b.w = add(b.w, scale(torque, dt * 6.0 / (m * h * h)));
                        b.x = add(b.x, scale(b.v, dt));
                        b.q = normal(qmul(exp(scale(b.w, dt)), b.q));
                        b.turn = matrix(b.q);
                    }
                }
                barrier.wait();
            }
            (broke, woken)
        });
        for (broke, woken) in found {
            self.broke += broke.len();
            for b in broke {
                self.bonds[b as usize].area = 0.0;
            }
            self.waking.extend(woken.into_iter().map(|i| asleep[i as usize - n]));
        }
        self.bonds.retain(|b| b.area > 0.0);
        for (&i, b) in order.iter().zip(&body) {
            self.body[i as usize] = *b;
        }
        self.rounds += rounds;
        self.time += length;
        for i in 0..n {
            let b = &self.body[i];
            let speed = len(b.v) + len(b.w) * self.h / 2.0;
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
                    let off = self.body[i].x[a] - self.site_middle(from)[a];
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

/** Where site `s` is along a Z-order curve: sites near each other on it are near in space. */
fn z_order(s: [i64; 3]) -> u64 {
    let mut z = 0;
    for b in 0..21 {
        for (a, &v) in s.iter().enumerate() {
            z |= ((v.wrapping_add(1 << 20) as u64 >> b) & 1) << (3 * b + a);
        }
    }
    z
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

mod crew;
#[cfg(test)]
mod tests;
