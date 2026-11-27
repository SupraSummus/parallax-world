//! How what a cut brings down falls (fall.rs): as a rigid body, under its
//! weight, pushed back by the world where it reaches into it. The lattice
//! never moves: each pose is sampled onto it, matter moving between its
//! sites.
//!
//! Each cell weighs as much as it fills its site (its pieces' alpha). The
//! body moves six ways; each cell that bears (fall.rs's `kind`), on its
//! surface, is a ball half a site across, which the world's bearing sites
//! push back across their faces, with friction (sequential impulses). So
//! a body on a support topples off it if its weight lies past its edge,
//! stays if not, and one on nothing falls.
//!
//! A pose sampled: a site holds the body's cell its middle comes from, if
//! that bears more than what the world holds there (`Kind`'s order).

use lattice::{FastMap, FastSet};
use worldgen::grid::Held;
use worldgen::units::UNIT_SIZE;

use crate::fall::{Kind, kind};
use crate::view::Vec3;

/** Seconds a step takes; between poses shown; a fall is followed for, at most. */
const DT: f64 = 1.0 / 240.0;
const EVERY: f64 = 1.0 / 20.0;
const LONGEST: f64 = 8.0;
/** Rounds of impulses a step. */
const ROUNDS: usize = 8;
/** Of what pushes a contact back, how much it holds against sliding, at most. */
const FRICTION: f64 = 0.6;
/**
 * Of how deep a contact reaches, past LET_BE, the share pushed back a
 * step: by a motion of its own, which moves the body but is not kept
 * (split impulses), so that pushing back gives it no speed.
 */
const BACK: f64 = 0.2;
const LET_BE: f64 = 0.02;
/** Slower than SLOW sites a second (and a tenth of it, radians) for REST seconds, it is at rest. */
const SLOW: f64 = 0.5;
const REST: f64 = 0.3;

/** What falls: each cell's site, and what it holds. */
pub struct Body {
    pub cells: Vec<([i64; 3], Held)>,
}

/** The sites a falling body holds, and which of its cells each holds. */
pub type Holding = FastMap<[i64; 3], u32>;

/** A pose of a falling body, `t` seconds in. */
pub struct Pose {
    pub t: f64,
    pub sites: Holding,
}

type M3 = [[f64; 3]; 3];

fn add(a: Vec3, b: Vec3) -> Vec3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
fn sub(a: Vec3, b: Vec3) -> Vec3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn scale(a: Vec3, s: f64) -> Vec3 {
    a.map(|v| v * s)
}
fn dot(a: Vec3, b: Vec3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn cross(a: Vec3, b: Vec3) -> Vec3 {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}
fn times(m: &M3, v: Vec3) -> Vec3 {
    [dot(m[0], v), dot(m[1], v), dot(m[2], v)]
}
fn transposed(m: &M3) -> M3 {
    std::array::from_fn(|r| std::array::from_fn(|c| m[c][r]))
}
fn product(a: &M3, b: &M3) -> M3 {
    std::array::from_fn(|r| std::array::from_fn(|c| (0..3).map(|k| a[r][k] * b[k][c]).sum()))
}

/** The inverse of a matrix that has one, by its cofactors. */
fn inverse(m: &M3) -> M3 {
    let c = |r: usize, k: usize| {
        m[(r + 1) % 3][(k + 1) % 3] * m[(r + 2) % 3][(k + 2) % 3] - m[(r + 1) % 3][(k + 2) % 3] * m[(r + 2) % 3][(k + 1) % 3]
    };
    let det = (0..3).map(|k| m[0][k] * c(0, k)).sum::<f64>();
    std::array::from_fn(|r| std::array::from_fn(|k| c(k, r) / det))
}

/** A unit quaternion (w, x, y, z) as a matrix. */
fn matrix([w, x, y, z]: [f64; 4]) -> M3 {
    [
        [1.0 - 2.0 * (y * y + z * z), 2.0 * (x * y - w * z), 2.0 * (x * z + w * y)],
        [2.0 * (x * y + w * z), 1.0 - 2.0 * (x * x + z * z), 2.0 * (y * z - w * x)],
        [2.0 * (x * z - w * y), 2.0 * (y * z + w * x), 1.0 - 2.0 * (x * x + y * y)],
    ]
}

/** Unit quaternion `q` turned at `w` radians a second for DT. */
fn turned([qw, qx, qy, qz]: [f64; 4], w: Vec3) -> [f64; 4] {
    let dq = [
        -w[0] * qx - w[1] * qy - w[2] * qz,
        w[0] * qw + w[1] * qz - w[2] * qy,
        w[1] * qw + w[2] * qx - w[0] * qz,
        w[2] * qw + w[0] * qy - w[1] * qx,
    ];
    let q: [f64; 4] = std::array::from_fn(|k| [qw, qx, qy, qz][k] + 0.5 * DT * dq[k]);
    let n = q.iter().map(|x| x * x).sum::<f64>().sqrt();
    q.map(|x| x / n)
}

fn middle(site: [i64; 3]) -> Vec3 {
    site.map(|v| v as f64 + 0.5)
}

fn site_of(p: Vec3) -> [i64; 3] {
    p.map(|v| v.floor() as i64)
}

/** The site across face `face` (lattice's FACES) of `site`, and the axis the face is across. */
fn across(site: [i64; 3], face: [i64; 3]) -> ([i64; 3], usize) {
    ([site[0] + face[0], site[1] + face[1], site[2] + face[2]], (0..3).find(|&a| face[a] != 0).expect("a face"))
}

/** How far `p`, in `site`, is from its face towards `face`. */
fn to_face(p: Vec3, site: [i64; 3], face: [i64; 3], axis: usize) -> f64 {
    if face[axis] > 0 { site[axis] as f64 + 1.0 - p[axis] } else { p[axis] - site[axis] as f64 }
}

/**
 * Where a ball half a site across at `p` reaches into bearing sites of
 * `world`, each the way it is pushed back and how deep: out of the one it
 * is in by its nearest face not against another; out of one across a face.
 */
fn contacts_at(p: Vec3, world: &mut dyn FnMut([i64; 3]) -> Kind, out: &mut Vec<(Vec3, f64)>) {
    let site = site_of(p);
    let inside = world(site) == Kind::Bearing;
    let mut nearest: Option<(Vec3, f64)> = None;
    for face in lattice::ray::FACES {
        let (next, axis) = across(site, face);
        let to = to_face(p, site, face, axis);
        let normal = face.map(|v| v as f64);
        if inside && world(next) != Kind::Bearing && nearest.is_none_or(|n| to < n.1) {
            nearest = Some((normal, to));
        } else if !inside && to < 0.5 && world(next) == Kind::Bearing {
            out.push((scale(normal, -1.0), 0.5 - to));
        }
    }
    out.extend(nearest.map(|(normal, to)| (normal, to + 0.5)));
}

/** A body as rigid: its mass, its middle of mass, the inverse of its inertia there, its surface's bearing cells from that middle. */
struct Rigid {
    mass: f64,
    centre: Vec3,
    inverse_inertia: M3,
    surface: Vec<Vec3>,
}

impl Rigid {
    fn of(body: &Body, index: &Holding) -> Rigid {
        let weighs = |held: &Held| f64::from(held.0.0) / f64::from(lattice::ALPHA);
        let mass: f64 = body.cells.iter().map(|c| weighs(&c.1)).sum();
        let centre = scale(body.cells.iter().fold([0.0; 3], |a, c| add(a, scale(middle(c.0), weighs(&c.1)))), 1.0 / mass);
        // Each cell a cube a site across: a sixth of its mass on each axis of its own.
        let mut inertia = [[0.0; 3]; 3];
        for (site, held) in &body.cells {
            let (r, m) = (sub(middle(*site), centre), weighs(held));
            for a in 0..3 {
                for b in 0..3 {
                    inertia[a][b] += m * (if a == b { dot(r, r) + 1.0 / 6.0 } else { 0.0 } - r[a] * r[b]);
                }
            }
        }
        let bears = |s: [i64; 3]| index.get(&s).is_some_and(|&i| kind(&body.cells[i as usize].1) == Kind::Bearing);
        let on_surface = |s: [i64; 3]| bears(s) && lattice::ray::FACES.iter().any(|&f| !bears(across(s, f).0));
        let surface = body.cells.iter().filter(|c| on_surface(c.0)).map(|c| sub(middle(c.0), centre)).collect();
        Rigid { mass, centre, inverse_inertia: inverse(&inertia), surface }
    }
}

/** Where a body is and how it moves: its middle of mass, turn, speed and spin. */
struct State {
    at: Vec3,
    q: [f64; 4],
    v: Vec3,
    w: Vec3,
}

/** A contact a step: where it is from the middle of mass, the way it pushes, how deep it reaches, how much it pushed and pushed back. */
struct Contact {
    r: Vec3,
    normal: Vec3,
    depth: f64,
    pushed: f64,
    pushed_back: f64,
}

/** How a body of `mass` and turned inverse inertia `i` gives way at `r` to an impulse along `n`: speed there per impulse. */
fn giving(mass: f64, i: &M3, r: Vec3, n: Vec3) -> f64 {
    1.0 / mass + dot(n, cross(times(i, cross(r, n)), r))
}

/** The impulse along `n` at `r` that brings `(v, w)` there to `speed` along `n`. */
fn stopping(mass: f64, i: &M3, r: Vec3, n: Vec3, (v, w): (Vec3, Vec3), speed: f64) -> f64 {
    (speed - dot(add(v, cross(w, r)), n)) / giving(mass, i, r, n)
}

/** `(v, w)` after `impulse` at `r`. */
fn pushed(mass: f64, i: &M3, r: Vec3, impulse: Vec3, (v, w): (Vec3, Vec3)) -> (Vec3, Vec3) {
    (add(v, scale(impulse, 1.0 / mass)), add(w, times(i, cross(r, impulse))))
}

impl State {
    /** A step of DT: weight, then the world's contacts with `rigid`'s surface, then the motion. */
    fn step(&mut self, rigid: &Rigid, world: &mut dyn FnMut([i64; 3]) -> Kind) {
        let (m, turn) = (rigid.mass, matrix(self.q));
        self.v = add(self.v, [0.0, -statics::model::GRAVITY / UNIT_SIZE * DT, 0.0]);
        let i = product(&product(&turn, &rigid.inverse_inertia), &transposed(&turn));
        let mut contacts: Vec<Contact> = Vec::new();
        let mut found = Vec::new();
        for &r0 in &rigid.surface {
            let r = times(&turn, r0);
            found.clear();
            contacts_at(add(self.at, r), world, &mut found);
            contacts.extend(found.iter().map(|&(normal, depth)| Contact { r, normal, depth, pushed: 0.0, pushed_back: 0.0 }));
        }
        // First all at once, each its share of what it asks, so that none takes a blow first; then each in turn,
        // every round the other way through them.
        let share = 1.0 / contacts.len().max(1) as f64;
        let (mut vw, mut back) = ((self.v, self.w), ([0.0; 3], [0.0; 3]));
        let start = vw;
        for c in &mut contacts {
            c.pushed = stopping(m, &i, c.r, c.normal, start, 0.0).max(0.0) * share;
            vw = pushed(m, &i, c.r, scale(c.normal, c.pushed), vw);
        }
        for round in 0..ROUNDS {
            let order: Box<dyn Iterator<Item = &mut Contact>> =
                if round % 2 == 0 { Box::new(contacts.iter_mut()) } else { Box::new(contacts.iter_mut().rev()) };
            for c in order {
                let more = stopping(m, &i, c.r, c.normal, vw, 0.0).max(-c.pushed);
                c.pushed += more;
                vw = pushed(m, &i, c.r, scale(c.normal, more), vw);
                // Pushed back as deep as it reaches, apart.
                let more = stopping(m, &i, c.r, c.normal, back, BACK / DT * (c.depth - LET_BE).max(0.0)).max(-c.pushed_back);
                c.pushed_back += more;
                back = pushed(m, &i, c.r, scale(c.normal, more), back);
                // Friction, against the sliding, as much as the push holds.
                let u = add(vw.0, cross(vw.1, c.r));
                let slide = sub(u, scale(c.normal, dot(u, c.normal)));
                let fast = dot(slide, slide).sqrt();
                if fast > 1e-9 {
                    let along = scale(slide, 1.0 / fast);
                    let held = stopping(m, &i, c.r, along, vw, 0.0).max(-FRICTION * c.pushed);
                    vw = pushed(m, &i, c.r, scale(along, held), vw);
                }
            }
        }
        (self.v, self.w) = vw;
        self.at = add(self.at, scale(add(self.v, back.0), DT));
        self.q = turned(self.q, add(self.w, back.1));
    }

    fn still(&self) -> bool {
        dot(self.v, self.v).sqrt() < SLOW && dot(self.w, self.w).sqrt() < SLOW / 10.0
    }

    fn apply(&self, rigid: &Rigid, p: Vec3) -> Vec3 {
        add(self.at, times(&matrix(self.q), sub(p, rigid.centre)))
    }

    fn inverse(&self, rigid: &Rigid, q: Vec3) -> Vec3 {
        add(rigid.centre, times(&transposed(&matrix(self.q)), sub(q, self.at)))
    }

    /** The body as it is now, sampled onto the lattice in `world`: each site about where a cell's middle goes holds the cell its own middle comes from, where that bears more than what is there. */
    fn sampled(&self, rigid: &Rigid, body: &Body, index: &Holding, world: &mut dyn FnMut([i64; 3]) -> Kind) -> Holding {
        let mut seen: FastSet<[i64; 3]> = FastSet::default();
        let mut sites = Holding::default();
        for (site, _) in &body.cells {
            let to = site_of(self.apply(rigid, middle(*site)));
            for near in std::iter::once(to).chain(lattice::ray::FACES.map(|f| across(to, f).0)) {
                if seen.insert(near)
                    && let Some(&i) = index.get(&site_of(self.inverse(rigid, middle(near))))
                    && kind(&body.cells[i as usize].1) > world(near)
                {
                    sites.insert(near, i);
                }
            }
        }
        sites
    }
}

/**
 * How `body` falls in `world` (what each site holds; where the body is,
 * it is not looked at): its poses, from where it is till it comes to rest.
 */
pub fn fall(body: &Body, world: &mut dyn FnMut([i64; 3]) -> Kind) -> Vec<Pose> {
    let index: Holding = body.cells.iter().enumerate().map(|(i, c)| (c.0, i as u32)).collect();
    let world = &mut |s: [i64; 3]| if index.contains_key(&s) { Kind::Empty } else { world(s) };
    let rigid = Rigid::of(body, &index);
    let mut state = State { at: rigid.centre, q: [1.0, 0.0, 0.0, 0.0], v: [0.0; 3], w: [0.0; 3] };
    let mut poses = vec![Pose { t: 0.0, sites: state.sampled(&rigid, body, &index, world) }];
    let (mut t, mut still) = (0.0, 0.0);
    while t < LONGEST && still < REST {
        state.step(&rigid, world);
        t += DT;
        still = if state.still() { still + DT } else { 0.0 };
        if t - poses[poses.len() - 1].t >= EVERY || still >= REST {
            poses.push(Pose { t, sites: state.sampled(&rigid, body, &index, world) });
        }
    }
    poses
}

/** What changes from a body holding `before` to holding `after`: the sites it leaves hold nothing, those it comes to its cell's matter. */
pub fn changes(before: &Holding, after: &Holding, body: &Body) -> Vec<([i64; 3], Held)> {
    let left = before.keys().filter(|s| !after.contains_key(*s)).map(|&s| (s, Held::default()));
    let come = after.iter().filter(|(s, i)| before.get(*s) != Some(i)).map(|(&s, &i)| (s, body.cells[i as usize].1));
    left.chain(come).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /** A post 4 sites square and 40 high, from y = 1 up. */
    fn post() -> Body {
        let held: Held = ((255, [100; 3]), ([0; 3], [0; 3]), 255, [0; 3], None, None);
        Body { cells: (0..4).flat_map(|x| (1..41).flat_map(move |y| (0..4).map(move |z| ([x, y, z], held)))).collect() }
    }

    /** How the post falls on ground under y = −1 with, at y = −1, a stump of x in `stump` (y = 0 cut away): its last pose's sites. */
    fn on_stump(stump: std::ops::Range<i64>) -> Vec<[i64; 3]> {
        let mut world = |s: [i64; 3]| match s[1] {
            ..-1 => Kind::Bearing,
            -1 if stump.contains(&s[0]) && (0..4).contains(&s[2]) => Kind::Bearing,
            _ => Kind::Empty,
        };
        fall(&post(), &mut world).pop().expect("a pose").sites.into_keys().collect()
    }

    fn heights(sites: &[[i64; 3]]) -> (i64, i64) {
        (sites.iter().map(|s| s[1]).min().expect("sites"), sites.iter().map(|s| s[1]).max().expect("sites"))
    }

    #[test]
    fn a_post_on_a_stump_under_all_of_it_settles_on_it_and_stands() {
        let sites = on_stump(0..4);
        assert_eq!(sites.len(), 640, "all of it");
        assert_eq!(heights(&sites), (0, 39), "down the kerf, upright");
    }

    #[test]
    fn a_post_on_a_stump_under_half_of_it_topples_off_it_and_lies() {
        let sites = on_stump(0..2);
        assert!((550..760).contains(&sites.len()), "as much matter: {}", sites.len());
        assert!(heights(&sites).1 < 12, "lying: {:?}", heights(&sites));
        assert!(sites.iter().any(|s| s[0] > 25), "off the side it is not borne on, +x");
    }

    #[test]
    fn a_post_on_nothing_falls_till_it_meets_the_ground_and_stands_on_it() {
        let mut world = |s: [i64; 3]| if s[1] < -20 { Kind::Bearing } else { Kind::Empty };
        let sites: Vec<[i64; 3]> = fall(&post(), &mut world).pop().expect("a pose").sites.into_keys().collect();
        assert!((610..670).contains(&sites.len()), "as much matter: {}", sites.len());
        let (low, high) = heights(&sites);
        assert!((-20..=-19).contains(&low) && high >= low + 38, "on the ground, upright: {low} to {high}");
    }
}
