//! The forest's trees: spruces, birches and oaks, each a trunk, branches
//! and a crown of leaf clusters kept to its room. `random` picks their shapes.

use std::sync::Arc;

use worldgen::curve::Curve;
use worldgen::generator::{ANY_SIZE, Child, Far, Fill, Node, Op, Point, Shape, ball, bounds_of_shapes, capsule, lathe};
use worldgen::grid::Box3;
use worldgen::js::{self, round};
use worldgen::noise::{Rng, fx, hash, unit};
use worldgen::points::Room;
use worldgen::units::{UNIT_SIZE, m};

use super::{K, PI, k, vox};

const BRANCH_RADIUS: f64 = 1.2;
/** How branches bend: once, off straight by up to a third of a metre at their middle, by where their ends are. */
fn branch() -> Curve<3> {
    Curve::wandering(0xb2a4c4, vec![fx(3.0)])
}
const FINE: f64 = 1.0;
/** How much of a site's rays a spray's leaves stop. */
const LEAVES: f64 = 0.45;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Species {
    Spruce,
    Birch,
    Oak,
}
pub(super) const SPECIES: [Species; 3] = [Species::Spruce, Species::Birch, Species::Oak];

pub type GroundAt<'a> = &'a dyn Fn(i64, i64) -> i64;

/** A tree standing alone, its height picked by `random` too. */
pub fn make_tree(species: Species, x: f64, ground: f64, z: f64, random: &mut Rng, ground_at: GroundAt) -> Node {
    let height = height(species, random.next_u32()) as f64;
    grow(species, x, ground, z, height, Arc::new(Room::everywhere()), random, ground_at)
}

/** A tree `height` tall (units) whose crown keeps to its `room` (only across: its walls are upright). */
#[allow(clippy::too_many_arguments)]
pub(super) fn grow(
    species: Species,
    x: f64,
    ground: f64,
    z: f64,
    height: f64,
    room: Arc<Room>,
    random: &mut Rng,
    ground_at: GroundAt,
) -> Node {
    match species {
        Species::Spruce => spruce_tree(x, ground, z, height, room, random, ground_at),
        Species::Birch => birch_tree(x, ground, z, height, room, random),
        Species::Oak => oak_tree(x, ground, z, height, room, random, ground_at),
    }
}

/** How tall a tree of `species` grows (units), by a hash `pick`: from its least to its most, rounded. */
pub(super) fn height(species: Species, pick: u32) -> i64 {
    let (least, more) = match species {
        Species::Spruce => (m(14.0), m(10.0)),
        Species::Birch => (m(12.0), m(7.0)),
        Species::Oak => (m(12.0), m(8.0)),
    };
    least + ((pick as i64 * more + (1 << 31)) >> 32)
}

/** Each species' bark. */
pub(super) fn bark_of(species: Species) -> K {
    match species {
        Species::Spruce => K::spruceBark,
        Species::Birch => K::birchBark,
        Species::Oak => K::bark,
    }
}

/** A tree's foliage, by a fraction `pick`: each of its species' colours as often as its trees have it. */
pub(super) fn foliage(species: Species, pick: f64) -> K {
    match species {
        Species::Spruce => K::spruce,
        Species::Birch if pick < 0.65 => K::birchGold,
        Species::Birch => K::birchGreen,
        Species::Oak if pick < 0.45 => K::oakGreen,
        Species::Oak if pick < 0.75 => K::oakOrange,
        Species::Oak => K::oakGold,
    }
}

/** A tree of `shapes`, with the parts its crown hides as a child, and its crown's leaf clusters. */
fn tree(shapes: Vec<Shape>, hidden: Vec<Shape>, fine: Vec<Child>, clusters: Vec<Child>) -> Node {
    let mut children = vec![Child::new("hidden", m(0.5), Node::of(hidden))];
    children.extend(fine);
    children.extend(clusters);
    Node::of(shapes).with_children(children)
}

/** A vertical trunk, `width` units square, from just below `ground` up to `top`. */
fn trunk(x: f64, ground: f64, z: f64, top: f64, width: f64, kind: K) -> Shape {
    Shape::cuboid([x, ground - 1.0, z], [x + width, top + 1.0, z + width], kind).paint()
}

/** A round trunk from `foot` up to `top`, narrowing from `radius[0]` to `radius[1]`. */
#[derive(Clone, Copy)]
struct Bole {
    foot: Point,
    top: Point,
    radius: [f64; 2],
}

impl Bole {
    /** From just below `ground` at (x, z) up to `top`, leaning by up to 0.4 m each way. */
    fn standing(x: f64, ground: f64, z: f64, top: f64, radius: [f64; 2], random: &mut Rng) -> Bole {
        let mut lean = || (random.next() - 0.5) * vox(0.8);
        Bole { foot: [x, ground - 1.0, z], top: [x + lean(), top + 1.0, z + lean()], radius }
    }

    /** Its axis at height y, within its ends. */
    fn axis(&self, y: f64) -> Point {
        let t = ((y - self.foot[1]) / (self.top[1] - self.foot[1])).clamp(0.0, 1.0);
        [0, 1, 2].map(|i| self.foot[i] + t * (self.top[i] - self.foot[i]))
    }

    /** Its radius along it, narrowing evenly. */
    fn radius(&self) -> impl Fn(f64) -> f64 + Send + Sync + 'static {
        let [r0, r1] = self.radius;
        move |t| r0 + t * (r1 - r0)
    }

    fn shape(&self, kind: K) -> Shape {
        Shape::capsule(self.foot, self.top, self.radius(), kind).paint()
    }

    /**
     * Furrows painted down it, by `seed`: one every `apart` units around
     * its foot, in runs about a metre long, each a little to the side of
     * the last, with gaps between.
     */
    fn furrows(self, apart: f64, kind: K, seed: u64) -> Shape {
        let count = 3f64.max(round(2.0 * PI * self.radius[0] / apart));
        let run = vox(1.0);
        let furrow = move |x: f64, y: f64, z: f64| {
            let [ax, _, az] = self.axis(y);
            let u = (js::atan2(z - az, x - ax) / (2.0 * PI) + 0.5) * count;
            let k = u.floor() as i64;
            let along = y + unit(hash(seed, k, 0, 0)) * run;
            let j = (along / run).floor() as i64;
            let (shift, gap) = (unit(hash(seed, k, j, 1)) - 0.5, unit(hash(seed, k, j, 2)) * 0.3);
            let within = (u - k as f64 - 0.5 - 0.4 * shift).abs() < 0.25 && js::wrap(along, run) < run * (1.0 - gap);
            if within { 1.0 } else { 0.0 }
        };
        capsule(self.foot, self.top, self.radius(), k(kind), Fill::At(Arc::new(furrow)), Op::Paint)
    }
}

/**
 * A tree's branches and their tips, where its leaves grow, in its room.
 * Between its leaves they show, from above too, so they are drawn with
 * the tree.
 */
struct Crown {
    kind: K,
    forks: usize,
    room: Arc<Room>,
    tubes: Vec<Shape>,
    tips: Vec<Point>,
}

impl Crown {
    fn new(kind: K, forks: usize, room: Arc<Room>) -> Crown {
        Crown { kind, forks, room, tubes: Vec::new(), tips: Vec::new() }
    }

    /**
     * A branch of `fork` from `start` to `end`, `radius` thick at its foot,
     * forking at its tip into two, shorter and thinner, spreading apart and
     * a little up, until the last fork. It stops short of its room's walls
     * by a leaf cluster's room, or as far as it starts if nearer: a branch
     * doesn't grow into its neighbours' crowns, but forks on.
     */
    fn grow(&mut self, start: Point, end: Point, radius: f64, fork: usize, random: &mut Rng) {
        let t = self.room.reach(start, end, cluster_room().min(self.room.margin(start)));
        let end = [0, 1, 2].map(|i| start[i] + t * (end[i] - start[i]));
        let (tip, length) = (radius * 0.7, js::hypot(&[0, 1, 2].map(|i| end[i] - start[i])));
        // Cut back to nothing, its leaves grow where it would have started, once.
        if length < 1.0 {
            if !self.tips.contains(&start) {
                self.tips.push(start);
            }
            return;
        }
        self.tubes.push(Shape::tube(&branch(), start, end, move |t| radius - t * (radius - tip), self.kind).paint());
        // Leaves at the forks' ends but the limbs': on the tips alone, the limbs showed between them from above.
        if fork > 0 {
            self.tips.push(end);
        }
        if fork == self.forks {
            return;
        }
        let d = [0, 1, 2].map(|i| (end[i] - start[i]) / length);
        // Across it, at random.
        let w = direction(random);
        let wd: f64 = (0..3).map(|i| w[i] * d[i]).sum();
        let u = [0, 1, 2].map(|i| w[i] - wd * d[i]);
        let ul = js::hypot(&u).max(1e-9);
        for side in [1.0, -1.0] {
            let a = 0.4 + random.next() * 0.3;
            let mut v = [0, 1, 2].map(|i| d[i] * js::cos(a) + side * u[i] / ul * js::sin(a));
            v[1] += 0.2;
            let reach = length * (0.5 + random.next() * 0.1) / js::hypot(&v);
            self.grow(end, [0, 1, 2].map(|i| end[i] + v[i] * reach), tip, fork + 1, random);
        }
    }
}

/** A random direction, evenly over the sphere. */
pub(super) fn direction(random: &mut Rng) -> Point {
    let y = 2.0 * random.next() - 1.0;
    let a = 2.0 * PI * random.next();
    let r = (1.0 - y * y).sqrt();
    [r * js::cos(a), y, r * js::sin(a)]
}

/** The least room around a leaf cluster's centre: with less, it doesn't grow there. */
fn cluster_room() -> f64 {
    vox(0.6)
}

/** The smallest leaf cluster: with less room, none grows. */
fn least_cluster() -> f64 {
    vox(0.3)
}

/** The share of a leaf cluster's sprays that are turning. */
const TURNING: f64 = 0.25;

/**
 * A cluster of leaves around `centre`, reaching `radius`: sprays of
 * leaves on twigs around a core as dense as `fill` at level 0, further a
 * ball as dense as they are on average, see-through where they are.
 */
#[allow(clippy::too_many_arguments)]
fn leaf_cluster(name: String, centre: Point, radius: f64, kind: K, fill: f64, random: &mut Rng, droop: f64, turning: K) -> Child {
    let seed = random.next_u64();
    // Sprays from `inner` out, around a core `core` wide.
    let spray = vox(0.45).min(radius * 0.4);
    let count = 4f64.max(round(0.95 * js::pow(radius / spray, 2.0)));
    let (inner, core) = (radius * 0.3, radius * 0.55);
    let sprays = move |random: &mut Rng| {
        let (mut twigs, mut leaves) = (Vec::new(), Vec::new());
        let at = |d: Point, r: f64| [centre[0] + d[0] * r, centre[1] + d[1] * r, centre[2] + d[2] * r];
        for _ in 0..count as usize {
            let [dx, dy, dz] = direction(random);
            let length = js::hypot(&[dx, dy - droop, dz]);
            let d = [dx / length, (dy - droop) / length, dz / length];
            let tip = radius - spray;
            twigs.push(Shape::capsule(at(d, radius * 0.1), at(d, tip - spray / 2.0), FINE, K::twig));
            let leaf = if random.chance(fx(TURNING)) { turning } else { kind };
            leaves.push(capsule(at(d, inner), at(d, tip), spray, k(leaf), Fill::Const(LEAVES), Op::Fill));
        }
        twigs.push(ball(centre, core, k(kind), Fill::Const(fill), Op::Fill));
        twigs.extend(leaves);
        twigs
    };
    let bounds = ball(centre, radius, k(kind), Fill::Const(fill), Op::Fill).bounds;
    // As dense as the core and the sprays at each distance r from the centre. The sprays, radial, cover
    // count · π spray² of the sphere of radius r; clumped, they stop as many rays as half that spread evenly
    // (all of it, the ball's shadow was a quarter to a third more).
    let density = move |x: f64, y: f64, z: f64| {
        let r = js::hypot(&[x - centre[0], y - centre[1], z - centre[2]]);
        let cored = if r < core { fill } else { 0.0 };
        let sprays = if r >= inner { LEAVES * (0.5 * count * spray * spray / (4.0 * r * r)).min(1.0) } else { 0.0 };
        1.0 - (1.0 - cored) * (1.0 - sprays)
    };
    // The ball is of one kind, turning as often as a spray is: right on average over a tree's clusters, not in each.
    let far_kind = if Rng::new(seed ^ 0x7475_726e).chance(fx(TURNING)) { turning } else { kind };
    let far = ball(centre, radius, k(far_kind), Fill::At(Arc::new(density)), Op::Fill);
    Child::new(name, ANY_SIZE, Node::lazy(bounds, seed, sprays).with_far(Far::of(fx(FINE), vec![far])))
}

/** A child of a thing within `bounds`, drawn only at level 0, its shapes made only then. */
fn fine_child(name: &str, bounds: Box3, random: &mut Rng, make: impl Fn(&mut Rng) -> Vec<Shape> + Send + Sync + 'static) -> Child {
    Child::new(name, FINE as i64, Node::lazy(bounds, random.next_u64(), make))
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum BarkStyle {
    Furrows,
    Dashes,
}

/** Where `bark` draws on a trunk [x, x + width) × [from, to) × [z, z + width): a mark reaches a unit past it. */
fn bark_bounds(x: f64, z: f64, width: f64, from: f64, to: f64) -> Box3 {
    let (min, max) = ([x, from, z], [x + width, to, z + width]);
    Box3 { min: min.map(|v| v.floor() as i64 - 1), max: max.map(|v| v.ceil() as i64 + 1) }
}

/** Bark on the four faces of a trunk [x, x + width) × [from, to) × [z, z + width). */
#[allow(clippy::too_many_arguments)]
fn bark(x: f64, z: f64, width: f64, from: f64, to: f64, style: BarkStyle, kind: K, random: &mut Rng) -> Vec<Shape> {
    let mut shapes = Vec::new();
    let mark = |f: usize, u: f64, length: f64, y0: f64, y1: f64| {
        let layer = if f.is_multiple_of(2) {
            if f < 2 { z } else { x }
        } else if f < 2 {
            z + width - 1.0
        } else {
            x + width - 1.0
        };
        if f < 2 {
            Shape::cuboid([u, y0, layer], [u + length, y1, layer + 1.0], kind).paint()
        } else {
            Shape::cuboid([layer, y0, u], [layer + 1.0, y1, u + length], kind).paint()
        }
    };
    let start = |f: usize| if f < 2 { x } else { z };
    for f in 0..4 {
        if style == BarkStyle::Furrows {
            let mut n = 1f64.max(round(width / 3.0));
            while n > 0.0 {
                let mut u = start(f) + random.below(width as u32 - 1) as f64;
                let mut y = from;
                while y < to {
                    let length = vox(0.4 + random.next() * 1.2);
                    shapes.push(mark(f, u, 2.0, y, (y + length).min(to)));
                    y += length + (random.next() * vox(0.3)).floor();
                    u = start(f).max((start(f) + width - 2.0).min(u + random.below(3) as f64 - 1.0));
                }
                n -= 1.0;
            }
        } else {
            let mut y = from;
            while y < to {
                if random.chance(fx(0.5)) {
                    let u = start(f) + random.below(width as u32 - 1) as f64;
                    let length = 2.0 + random.below(2) as f64;
                    shapes.push(mark(f, u, length, y, y + 1.0));
                }
                y += vox(0.25);
            }
        }
    }
    shapes
}

/** Roots spreading from a trunk's foot around (x, z) into the ground, `half` its half-width. */
fn roots(x: f64, ground: f64, z: f64, half: f64, kind: K, random: &mut Rng, ground_at: GroundAt) -> Vec<Shape> {
    let count = 4.0 + random.below(3) as f64;
    (0..count as usize)
        .map(|i| {
            let angle = (i as f64 / count) * PI * 2.0 + random.next() * 0.6;
            let (c, s) = (js::cos(angle), js::sin(angle));
            let out = half + vox(0.5 + random.next() * 0.7);
            let (ex, ez) = (x + c * out, z + s * out);
            let p = [x + c * half * 0.6, ground + vox(0.35), z + s * half * 0.6];
            // Into the ground at its end, but down no further than it reaches out: past a cliff's or a
            // cutting's edge, it ran down through the rock and out of its face, to the foot below.
            let q = [ex, ((ground_at(round(ex) as i64, round(ez) as i64) - 1) as f64).max(ground - out), ez];
            let r = vox(0.15 + random.next() * 0.08);
            // Thicker at its foot.
            Shape::capsule(p, q, move |t| r * (1.3 - 0.7 * t), kind)
        })
        .collect()
}

/** Conical spruce with drooping whorls of branches, its crown cut where its room ends. */
fn spruce_tree(x: f64, ground: f64, z: f64, height: f64, room: Arc<Room>, random: &mut Rng, ground_at: GroundAt) -> Node {
    let width = if height >= vox(20.0) { vox(1.0) } else { vox(0.5) };
    let top = ground + height;
    let base = ground + round(height * (0.15 + random.next() * 0.15));
    let bole = trunk(x, ground, z, base, width, K::spruceBark);
    let radius = height * (0.17 + random.next() * 0.05);
    let whorl = vox(1.5 + random.next() * 0.6);
    let (cx, cz) = (x + (width - 1.0) / 2.0, z + (width - 1.0) / 2.0);
    let crown_top = top + vox(0.5);
    let tip = vox(0.35);
    let r = move |y: f64| {
        // Each whorl is widest at its bottom, giving the layered silhouette; at its top, half as wide.
        let phase = js::wrap(top - y, whorl) / whorl;
        radius * (1.0 - (y - base) / (crown_top - base)) * (0.5 + 0.5 * phase) + tip
    };
    // Dense within a trunk's width of its axis, so the fans' gaps show no bark. The trunk goes up as far as the
    // crown, even at a whorl's top, holds that core: above, it would stick out, a brown spike for a tip.
    let core = width;
    let hidden_top = base + (crown_top - base) * (1.0 - 2.0 * (core - tip) / radius);
    let hidden = vec![Shape::cuboid([x, base + 1.0, z], [x + width, hidden_top, z + width], K::spruceBark).paint()];
    let (from, to) = (base - 0.5, crown_top + 0.5);
    let density = move |distance: f64, py: f64| 0.75 - 0.3 * js::pow(distance / r(py), 3.0);
    let roomy = move |px: f64, pz: f64| room.margin([px, 0.0, pz]) >= 0.0;
    let spokes = 6.0 + random.below(3) as f64;
    let sector = (2.0 * PI) / spokes;
    let twist = random.next() * sector;
    let half_width = |distance: f64| vox(0.15) + 0.07 * distance;
    let covered = move |distance: f64| if distance < core { 1.0 } else { 1f64.min((2.0 * half_width(distance)) / (distance * sector)) };
    let near_room = roomy.clone();
    let fans = move |px: f64, py: f64, pz: f64| {
        let distance = js::hypot(&[px - cx, pz - cz]);
        if !near_room(px, pz) {
            return 0.0;
        }
        if distance < core {
            return density(distance, py);
        }
        let a = js::atan2(pz - cz, px - cx) - twist - ((top - py) / whorl).floor() * 2.39996;
        let off = js::wrap(a, sector);
        if off.min(sector - off) * distance < half_width(distance) { density(distance, py) } else { 0.0 }
    };
    let crown = |fill: Fill| lathe([cx, from, cz], [cx, to, cz], move |t| r(round(from + t * (to - from))), k(K::spruce), fill, Op::Fill);
    let foot = roots(cx, ground, cz, width / 2.0, K::spruceBark, random, ground_at);
    let fine = vec![fine_child("bark", bounds_of_shapes([&bole.bounds, &bark_bounds(x, z, width, ground, base)]), random, move |random| {
        bark(x, z, width, ground, base, BarkStyle::Furrows, K::spruceScale, random)
    })];
    let mut shapes = vec![bole.clone()];
    shapes.extend(foot.iter().cloned());
    shapes.push(crown(Fill::At(Arc::new(fans))));
    let mut near = tree(shapes, hidden, fine, Vec::new());
    // Its hidden parts are not drawn beyond level 0: inside a crown this dense they don't show.
    let mut far = vec![bole];
    far.extend(foot);
    far.push(crown(Fill::At(Arc::new(move |px, py, pz| {
        let distance = js::hypot(&[px - cx, pz - cz]);
        if roomy(px, pz) { density(distance, py) * covered(distance) } else { 0.0 }
    }))));
    near.far = Some(Far::of(fx(FINE), far));
    near
}

/** Slender white birch with a wavy trunk and a loose, clumpy crown of drooping twigs, in its room. */
fn birch_tree(x: f64, ground: f64, z: f64, height: f64, room: Arc<Room>, random: &mut Rng) -> Node {
    let width = vox(0.375);
    let (mut shapes, mut marks) = (Vec::new(), Vec::new());
    let mut segments: Vec<[f64; 4]> = Vec::new();
    // The trunk shifts by a unit now and then.
    let (mut tx, mut tz, mut from) = (x, z, ground - 1.0);
    let mut y = ground - 1.0;
    while y <= ground + height + 1.0 {
        let end = y == ground + height + 1.0;
        if end || ((y - ground) % vox(3.0) == 0.0 && y - ground > vox(4.0) && random.chance(fx(0.35))) {
            shapes.push(Shape::cuboid([tx, from, tz], [tx + width, y, tz + width], K::birchBark).paint());
            segments.push([tx, tz, from.max(ground), y]);
            if end {
                break;
            }
            if random.chance(fx(0.5)) {
                tx += if random.chance(fx(0.5)) { -1.0 } else { 1.0 };
            } else {
                tz += if random.chance(fx(0.5)) { -1.0 } else { 1.0 };
            }
            from = y;
        }
        // Black marks in bands half a metre tall.
        if (y - ground) % vox(0.5) == 0.0 && random.chance(fx(0.18)) {
            marks.push(Shape::cuboid([tx, y, tz], [tx + width, y + vox(0.5), tz + width], K::birchMark).paint());
        }
        y += 1.0;
    }
    shapes.extend(marks);
    let foliage = foliage(Species::Birch, random.next());
    let turning = if foliage == K::birchGold { K::birchGreen } else { K::birchGold };
    let crown_bottom = ground + height * 0.45;
    let crown_top = ground + height + vox(1.0);
    let cy = (crown_bottom + crown_top) / 2.0;
    let ry = (crown_top - crown_bottom) / 2.0;
    let rx = vox(2.2 + random.next() * 1.6);
    let count = 16.0 + random.below(8) as f64;
    let mid = (width - 1.0) / 2.0;
    let bend = branch();
    let (mut branches, mut clusters) = (Vec::new(), Vec::new());
    for i in 0..count as usize {
        // Uniform point in the crown ellipsoid.
        let o = loop {
            let o = [0, 1, 2].map(|_| random.next() * 2.0 - 1.0);
            if o.iter().map(|v| v * v).sum::<f64>() <= 1.0 {
                break o;
            }
        };
        // Squeezed into its room: drawn in towards the trunk as far as the crown's edge that way is.
        let (axis, out) = ([tx + mid, cy + o[1] * ry, tz + mid], js::hypot(&[o[0], o[2]]).max(1e-9));
        let edge = [axis[0] + o[0] / out * rx, axis[1], axis[2] + o[2] / out * rx];
        let squeeze = room.reach(axis, edge, cluster_room().min(room.margin(axis)));
        let p = [axis[0] + o[0] * rx * squeeze, axis[1], axis[2] + o[2] * rx * squeeze];
        // None where there is no room: a tree a taller one stands over has none, and stands bare.
        if room.margin(p) < least_cluster() {
            continue;
        }
        branches.push(Shape::tube(&bend, [tx + mid, p[1] - vox(0.5), tz + mid], p, BRANCH_RADIUS, K::birchBark).paint());
        let radius = vox(1.0 + random.next() * 0.6).min(room.margin(p));
        clusters.push(leaf_cluster(format!("leaves {i}"), p, radius, foliage, 0.35, random, 0.6, turning));
    }
    let marked: Vec<Box3> = segments.iter().map(|&[sx, sz, y0, y1]| bark_bounds(sx, sz, width, y0, y1)).collect();
    let dashes = fine_child("bark", bounds_of_shapes(shapes.iter().map(|s| &s.bounds).chain(&marked)), random, move |random| {
        segments.iter().flat_map(|&[sx, sz, y0, y1]| bark(sx, sz, width, y0, y1, BarkStyle::Dashes, K::birchMark, random)).collect()
    });
    tree(shapes, branches, vec![dashes], clusters)
}

/** Broad oak: thick trunk, a few limbs forking twice, a leaf cluster at each tip, as far as its room lets them. */
fn oak_tree(x: f64, ground: f64, z: f64, height: f64, room: Arc<Room>, random: &mut Rng, ground_at: GroundAt) -> Node {
    let trunk_top = ground + round(height * (0.45 + random.next() * 0.1));
    let width = vox(1.0);
    // The middle of `width` sites from x and z.
    let (cx, cz) = (x + (width - 1.0) / 2.0, z + (width - 1.0) / 2.0);
    let bole = Bole::standing(cx, ground, cz, trunk_top, [0.55 / UNIT_SIZE, 0.33 / UNIT_SIZE], random);
    let mut shapes = vec![bole.shape(K::bark)];
    let foliage = foliage(Species::Oak, random.next());
    let turning = if foliage == K::oakGold { K::oakOrange } else { K::oakGold };
    shapes.extend(roots(cx, ground, cz, width / 2.0, K::bark, random, ground_at));
    let mut crown = Crown::new(K::bark, 2, room.clone());
    // A leader up the middle and limbs around it; their forks reach about as far again.
    let crest = bole.axis(ground + height - vox(5.0));
    crown.grow(bole.axis(trunk_top - vox(0.5)), crest, BRANCH_RADIUS * 2.0, 0, random);
    let limbs = 4.0 + random.below(3) as f64;
    for i in 0..limbs as usize {
        let angle = (i as f64 / limbs) * PI * 2.0 + random.next() * 0.8;
        let (reach, start) = (vox(2.0 + random.next() * 1.2), bole.axis(trunk_top - (random.next() * vox(2.0)).floor()));
        let (rise, out) = (vox(1.0 + random.next() * 1.5), [js::cos(angle), 0.0, js::sin(angle)]);
        // Smaller, forks and all, as far as its room is narrower than the crown it would grow that way.
        let edge = [0, 1, 2].map(|i| start[i] + out[i] * (2.0 * reach + vox(1.4)));
        let squeeze = room.reach(start, edge, cluster_room().min(room.margin(start)));
        let end = [start[0] + out[0] * reach * squeeze, start[1] + rise * squeeze, start[2] + out[2] * reach * squeeze];
        crown.grow(start, end, BRANCH_RADIUS * 2.0, 0, random);
    }
    let clusters = crown
        .tips
        .iter()
        .enumerate()
        .filter_map(|(i, tip)| {
            let radius = vox(1.1 + random.next() * 0.6).min(room.margin(*tip));
            (radius >= least_cluster()).then(|| leaf_cluster(format!("leaves {i}"), *tip, radius, foliage, 0.4, random, 0.0, turning))
        })
        .collect();
    let seed = random.next_u64();
    let fine = vec![fine_child("bark", shapes[0].bounds, random, move |_| vec![bole.furrows(3.0, K::barkFurrow, seed)])];
    shapes.extend(crown.tubes);
    tree(shapes, Vec::new(), fine, clusters)
}
