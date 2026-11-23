//! The mine: galleries dug straight through lumps of aged cheese, along
//! x and z on levels a storey apart, stairs down beside some of them,
//! chambers where they meet, propped with timber and lit by lanterns.
//! A part of the cheese (lib.rs): it holds the space where its lumps
//! outbid the holed cheese, and digs from the points it owns, so its
//! galleries run on into the caverns at its edges.

use std::sync::Arc;

use worldgen::generator::{Field, Layer, Meets, Op, Shape};
use worldgen::grid::Box3;
use worldgen::land::owns;
use worldgen::noise::{Fx, Rng, Scope, fx, hash, hash64, smoothstep};
use worldgen::scalar::Scalar;
use worldgen::sites::spacing;
use worldgen::units::m;

use crate::{K, cells};

/** Its points a storey apart: a gallery's length between two along x or z, a storey between levels. */
const G: i64 = m(12.0);
const STOREY: i64 = m(6.0);
/** Galleries: wide and high enough to walk. */
const WIDE: i64 = m(2.5);
const HIGH: i64 = m(3.0);
/** Chambers where galleries meet. */
const CHAMBER: i64 = m(7.0);
const CHAMBER_HIGH: i64 = m(4.0);
/** Stairs: a step down RISE each TREAD (fcc's walk.rs steps up 0.55 m), a storey over a gallery's length. */
const RISE: i64 = m(0.5);
const STEPS: i64 = STOREY / RISE;
const TREAD: i64 = G / STEPS;
const _: () = assert!(STEPS * RISE == STOREY && STEPS * TREAD == G, "stairs climb a storey over a gallery's length in whole steps");
/** Timber props every PROPS along a gallery, PROP thick; drawn while sites are PROP apart at most. */
const PROPS: i64 = m(3.0);
const PROP: i64 = m(0.25);
/** A lantern's size, hung under a chamber's or a gallery's ceiling at a point. */
const LANTERN: i64 = m(0.3);
/** How far what a point digs reaches from it: a gallery and the stairs beside it, a chamber. */
const REACH: i64 = G + 2 * WIDE;

/** The chances a point digs each thing: galleries along x and z, stairs beside each, a chamber. */
const GALLERY: Fx = fx(0.6);
const STAIRS: Fx = fx(0.3);
const CHAMBER_CHANCE: Fx = fx(0.25);

/**
 * Lumps: lattice noise in [0, 1) through space, random values every
 * LUMP, eased between. In a cell of the lattice it is linear along each
 * axis in the eased offset, which rises with the offset: so over a box
 * in a cell no lower nor higher than at the box's corners, and over any
 * box than at the corners of the cells it meets.
 */
const LUMP: i64 = m(96.0);
/** The mine holds where its lumps are over THRESHOLD; none within CLEAR of the origin, where the camera starts in a cavern. */
const THRESHOLD: Fx = fx(0.62);
const CLEAR: i64 = m(48.0);
/** The most cells of lumps along an axis a box's range is read from its corners in each; past that, from theirs. */
const RANGED: i64 = 2;

/** The mine of a world's seed. */
pub struct Mine {
    lumps: u64,
    digs: u64,
}

impl Mine {
    pub fn new(scope: &Scope) -> Mine {
        Mine { lumps: scope.seed("lumps"), digs: scope.seed("digs") }
    }

    /** The lumps' value at corner (i, j, k) of their lattice. */
    fn corner(&self, c: [i64; 3]) -> Fx {
        Fx::fraction(hash(self.lumps, c[0], c[1], c[2]))
    }

    /** The lumps at `p`. */
    fn lumps(&self, p: [i64; 3]) -> Fx {
        let (cell, t) =
            (p.map(|v| v.div_euclid(LUMP)), p.map(|v| smoothstep(Fx::ZERO, Fx::ONE, Fx::from_bits((v.rem_euclid(LUMP) << 16) / LUMP))));
        let lerp = |a: Fx, b: Fx, t: Fx| a + (b - a) * t;
        let at = |di, dj, dk| self.corner([cell[0] + di, cell[1] + dj, cell[2] + dk]);
        let row = |dj, dk| lerp(at(0, dj, dk), at(1, dj, dk), t[0]);
        lerp(lerp(row(0, 0), row(1, 0), t[1]), lerp(row(0, 1), row(1, 1), t[1]), t[2])
    }

    /**
     * The least and the most of the lumps over box `b`: at the corners of
     * its part in each cell it meets, or of the cells, if more than
     * RANGED along an axis.
     */
    fn lumps_over(&self, b: &Box3) -> (Fx, Fx) {
        let [lo, hi] = [b.min, b.max.map(|v| v - 1)].map(|p| p.map(|v| v.div_euclid(LUMP)));
        let values: Vec<Fx> = if (0..3).any(|a| hi[a] - lo[a] >= RANGED) {
            cells(lo, hi.map(|v| v + 1)).map(|c| self.corner(c)).collect()
        } else {
            let part = |c: [i64; 3], a: usize| [b.min[a].max(c[a] * LUMP), (b.max[a] - 1).min((c[a] + 1) * LUMP - 1)];
            let corners = |c: [i64; 3]| cells([0; 3], [1; 3]).map(move |k| [0, 1, 2].map(|a| part(c, a)[k[a] as usize]));
            cells(lo, hi).flat_map(corners).map(|p| self.lumps(p)).collect()
        };
        let [l, h] = values.into_iter().fold([Fx::ONE, Fx::ZERO], |[l, h], v| [l.min(v), h.max(v)]);
        // Less and more by the bits products' flooring may lose.
        let lost = Fx::from_bits(8);
        (l - lost, h + lost)
    }
}

/** How far a distance from the origin is past CLEAR, a fraction of it: under 0 within it. */
fn clear(d: i64) -> Fx {
    Fx::from_bits(((d.min(4 * CLEAR) - CLEAR) << 16) / CLEAR)
}

impl Mine {
    /** Its bid for space: by its lumps, none within CLEAR of the origin. */
    pub fn bid(self: &Arc<Self>) -> Scalar {
        let (at, over) = (self.clone(), self.clone());
        Scalar::new(move |p| at.bid_at(p), move |b| over.bids_over(b))
    }

    fn bid_at(&self, p: [i64; 3]) -> Fx {
        (self.lumps(p) - THRESHOLD).min(clear(p.map(i64::abs).into_iter().max().unwrap_or(0)))
    }

    fn bids_over(&self, b: &Box3) -> (Fx, Fx) {
        let (lo, hi) = self.lumps_over(b);
        // How far from the origin along the farthest axis: the nearest and the farthest of the box.
        let near = (0..3).map(|a| (b.min[a]).max(1 - b.max[a]).max(0)).max().unwrap_or(0);
        let far = (0..3).map(|a| b.min[a].abs().max((b.max[a] - 1).abs())).max().unwrap_or(0);
        ((lo - THRESHOLD).min(clear(near)), (hi - THRESHOLD).min(clear(far)))
    }

    /** Its layers, by `hold`, its share of space. */
    pub fn layers(self: Arc<Self>, hold: Scalar) -> Vec<Layer> {
        let (galleries, dug, lit) = (self.clone(), hold.clone(), hold.clone());
        vec![
            Layer::Field(rock(hold.clone())),
            Layer::Field(
                Field::shapes("galleries", 1.0, Op::Carve, move |b| galleries.digs(&dug, b, spacing())).within(hold.clone(), REACH),
            ),
            Layer::Field(Field::shapes("lanterns", 2.0, Op::Fill, move |b| self.lights(&lit, b)).within(hold, REACH)),
        ]
    }
}

/**
 * Its lumps' rock: the cheese they are of painted aged where it owns it;
 * by its share's bounds, nothing of a box or all of it, unasked.
 */
fn rock(hold: Scalar) -> Field {
    let over = hold.clone();
    Field::at("rock", 0.5, Op::Paint, move |x, y, z| owns(hold.at([x, y, z])).then_some((K::aged.into(), None))).meets(move |b, _| {
        match over.over(b) {
            (_, most) if !owns(most) => Meets::Nothing,
            (least, _) if owns(least) => Meets::All(K::aged.into()),
            _ => Meets::Any,
        }
    })
}

/** The point of the mine at (i, j, k): its floor's centre. */
fn point([i, j, k]: [i64; 3]) -> [i64; 3] {
    [i * G, j * STOREY, k * G]
}

/** What a point digs, as `Mine::dig` rolls it. */
struct Dig {
    /** A gallery along x, along z; stairs down beside each. */
    gallery: [bool; 2],
    stairs: [bool; 2],
    chamber: bool,
}

impl Mine {
    /**
     * A place in it: in the first gallery along x, by its middle, of the
     * points it owns nearest the origin, cube by cube about it.
     */
    pub fn gallery(&self, hold: &Scalar) -> [i64; 3] {
        let digs = |c: [i64; 3]| self.dig(hold, c).is_some_and(|dig| dig.gallery[0]);
        let r = (1..).find(|&r| cells([-r; 3], [r; 3]).any(digs)).expect("a gallery somewhere");
        let c = cells([-r; 3], [r; 3]).find(|&c| digs(c)).expect("found");
        let [x, y, z] = point(c);
        let (x, y) = (x + G / 2, y + m(1.5));
        [x, y - (x + y + z).rem_euclid(2), z]
    }

    /** What point (i, j, k) digs, if the mine owns it. */
    fn dig(&self, hold: &Scalar, c: [i64; 3]) -> Option<Dig> {
        if !owns(hold.at(point(c))) {
            return None;
        }
        let mut rng = Rng::new(hash64(self.digs, c[0], c[1], c[2]));
        let gallery = [rng.chance(GALLERY), rng.chance(GALLERY)];
        let stairs = [0, 1].map(|a| rng.chance(STAIRS) && gallery[a]);
        Some(Dig { gallery, stairs, chamber: rng.chance(CHAMBER_CHANCE) })
    }

    /** The points whose digging may reach box `b`. */
    fn near(&self, hold: &Scalar, b: &Box3) -> impl Iterator<Item = ([i64; 3], Dig)> {
        let [lo, hi] = [b.min.map(|v| v - REACH), b.max.map(|v| v + REACH)];
        let cell = [G, STOREY, G];
        let [lo, hi] = [lo, hi].map(|p| [0, 1, 2].map(|a| p[a].div_euclid(cell[a])));
        cells(lo, hi).filter_map(|c| Some((c, self.dig(hold, c)?))).collect::<Vec<_>>().into_iter()
    }

    /**
     * The holes dug near box `b`, carving, those wider than `spacing`
     * (as the cheese's holes, lib.rs's `carved`): none past the chambers'.
     */
    fn digs(&self, hold: &Scalar, b: &Box3, spacing: i64) -> Vec<Shape> {
        let mut shapes = Vec::new();
        if spacing >= CHAMBER {
            return shapes;
        }
        for (c, dig) in self.near(hold, b) {
            let [x, y, z] = point(c).map(|v| v as f64);
            let (w, h) = (WIDE as f64 / 2.0, HIGH as f64);
            if dig.chamber {
                let r = CHAMBER as f64 / 2.0;
                shapes.push(Shape::cuboid([x - r, y, z - r], [x + r, y + CHAMBER_HIGH as f64, z + r], K::aged).carve());
            }
            if spacing >= WIDE {
                continue;
            }
            for a in [0, 2].into_iter().filter(|&a| dig.gallery[a / 2]) {
                // u along the gallery, v across it.
                let at = |u: f64, y: f64, v: f64| if a == 0 { [x + u, y, z + v] } else { [x + v, y, z + u] };
                shapes.push(Shape::cuboid(at(-w, y, -w), at(G as f64 + w, y + h, w), K::aged).carve());
                if dig.stairs[a / 2] {
                    // Beside it, a step down each tread, on down to the floor a storey under its far end.
                    for n in 0..STEPS {
                        let (u0, u1) = ((n * TREAD) as f64, ((n + 1) * TREAD) as f64 + if n == STEPS - 1 { w } else { 0.0 });
                        let floor = y - ((n + 1) * RISE) as f64;
                        shapes.push(Shape::cuboid(at(u0, floor, w), at(u1, floor + h, 3.0 * w), K::aged).carve());
                    }
                }
            }
        }
        shapes
    }

    /**
     * The lanterns near box `b`, under the ceiling at each point that digs,
     * while its galleries are dug; and the timber propping each gallery,
     * while sites are PROP apart at most.
     */
    fn lights(&self, hold: &Scalar, b: &Box3) -> Vec<Shape> {
        let s = spacing();
        if s >= WIDE {
            return Vec::new();
        }
        let mut shapes = Vec::new();
        for (c, dig) in self.near(hold, b) {
            let [x, y, z] = point(c).map(|v| v as f64);
            let (w, h, l) = (WIDE as f64 / 2.0, HIGH as f64, LANTERN as f64 / 2.0);
            if dig.chamber || dig.gallery.contains(&true) {
                let top = y + if dig.chamber { CHAMBER_HIGH } else { HIGH } as f64;
                shapes.push(Shape::cuboid([x - l, top - 4.0 * l, z - l], [x + l, top - 2.0 * l, z + l], K::lantern));
            }
            if s > PROP {
                continue;
            }
            for a in [0, 2].into_iter().filter(|&a| dig.gallery[a / 2]) {
                let at = |u: f64, y: f64, v: f64| if a == 0 { [x + u, y, z + v] } else { [x + v, y, z + u] };
                let p = PROP as f64;
                for n in 1..G / PROPS {
                    let u = (n * PROPS) as f64;
                    for side in [-w, w - p] {
                        shapes.push(Shape::cuboid(at(u - p / 2.0, y, side), at(u + p / 2.0, y + h, side + p), K::timber));
                    }
                    shapes.push(Shape::cuboid(at(u - p / 2.0, y + h - p, -w), at(u + p / 2.0, y + h, w), K::timber));
                }
            }
        }
        shapes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lumps_lie_within_their_range_over_a_box() {
        let mine = Mine::new(&Scope::new(3).of("mine"));
        let mut rng = Rng::new(9);
        for n in 0..2000 {
            // Boxes of every size, up to many cells.
            let min = [0; 3].map(|_| rng.below(4 * LUMP as u32) as i64 - 2 * LUMP);
            let b = Box3 { min, max: min.map(|v| v + 1 + rng.below((LUMP << (n % 4)) as u32 / 4) as i64) };
            let (lo, hi) = mine.bids_over(&b);
            for _ in 0..50 {
                let p = [0, 1, 2].map(|a| b.min[a] + rng.below((b.max[a] - b.min[a]) as u32) as i64);
                let bid = mine.bid_at(p);
                assert!(lo <= bid && bid <= hi, "{p:?} bids {bid:?}, not within {lo:?} to {hi:?} over {b:?}");
            }
        }
    }

    #[test]
    fn holds_about_a_quarter_of_the_space_in_lumps() {
        // Lumps apart in the holed cheese, whose caverns then reach through all of it.
        let mine = Mine::new(&Scope::new(0).of("mine"));
        let mut rng = Rng::new(1);
        let n = 100_000;
        let held = (0..n).filter(|_| mine.lumps([0; 3].map(|_| rng.below(1 << 20) as i64 - (1 << 19))) > THRESHOLD).count();
        assert!((0.2..0.35).contains(&(held as f64 / n as f64)), "{held} of {n}");
    }
}
