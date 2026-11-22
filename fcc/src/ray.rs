//! Rays across the lattice's voxels, exactly. A voxel is where six slabs
//! cross, one per pair of opposite faces, so a ray goes through it as
//! through a box (as voxels.wgsl's `crossings`), on into the neighbour
//! across the face it leaves by. Sites in a level's units, i64.

use crate::{FACE_KINDS, VOXEL_FACES, opposite};

/** The voxel's faces (VOXEL_FACES), as offsets of sites: (p − site) · face ≤ 1 inside. */
pub const FACES: [[i64; 3]; FACE_KINDS] = {
    let mut faces = [[0; 3]; FACE_KINDS];
    let mut k = 0;
    while k < FACE_KINDS {
        faces[k] = [VOXEL_FACES[k][0] as i64, VOXEL_FACES[k][1] as i64, VOXEL_FACES[k][2] as i64];
        k += 1;
    }
    faces
};

/** The site whose voxel holds `p`. */
pub fn nearest_site(p: [f64; 3]) -> [i64; 3] {
    let mut r = p.map(f64::round);
    if (r[0] + r[1] + r[2]) as i64 & 1 != 0 {
        // Off by one in the coordinate rounded the furthest.
        let e = [0, 1, 2].map(|i| (p[i] - r[i]).abs());
        let k = if e[0] >= e[1] && e[0] >= e[2] {
            0
        } else if e[1] >= e[2] {
            1
        } else {
            2
        };
        r[k] += if p[k] > r[k] { 1.0 } else { -1.0 };
    }
    r.map(|v| v as i64)
}

/**
 * How many crossings of a site one along a row of a lattice stands for.
 * A row runs through its sites' middles, along their longest chord, √2
 * spacings; a ray at any offset weaves through the teeth of the rows
 * beside, crossing 3/2 as many sites, each by a shorter chord (a voxel's
 * mean chord along a face's direction is its volume, 2, over its shadow,
 * a hexagon of 3/√2). A whole voxel's shadow falls on the rows beside
 * too: a row's ray does not see that.
 */
pub const CROSSING: f64 = 1.5;

/**
 * How many times what a site of `alpha` stops and glows in one crossing
 * it does in `crossings` (part of one, or CROSSING): what it stops,
 * 1 − (1 − alpha)^crossings, over alpha. A whole voxel, once.
 */
pub fn over(alpha: f64, crossings: f64) -> f64 {
    if alpha > 0.0 && crossings != 1.0 { (1.0 - (1.0 - alpha).powf(crossings)) / alpha } else { crossings }
}

/** One face of each pair of opposite ones (`opposite`). */
const PAIRS: [usize; FACE_KINDS / 2] = [0, 1, 4, 5, 8, 9];

/** A ray: how far along its line it is, and the site of the voxel it is in. */
#[derive(Clone, Copy, Debug)]
pub struct Ray {
    pub origin: [f64; 3],
    pub direction: [f64; 3],
    pub t: f64,
    pub site: [i64; 3],
}

/** Where a ray's line enters and leaves a voxel, and the faces it crosses there. */
struct Chord {
    enters: (f64, usize),
    leaves: (f64, usize),
}

impl Ray {
    /** From `origin` along `direction`, `t` along it. */
    pub fn new(origin: [f64; 3], direction: [f64; 3], t: f64) -> Ray {
        let site = nearest_site(std::array::from_fn(|i| origin[i] + t * direction[i]));
        Ray { origin, direction, t, site }
    }

    /** Where it is. */
    pub fn at(&self) -> [f64; 3] {
        std::array::from_fn(|i| self.origin[i] + self.t * self.direction[i])
    }

    /** Its line through its voxel, where the slabs −1 ≤ (p − site) · face ≤ 1 cross: in at the last it enters, out at the first it leaves. */
    fn chord(&self) -> Chord {
        let r: [f64; 3] = std::array::from_fn(|i| self.origin[i] - self.site[i] as f64);
        let mut chord = Chord { enters: (f64::NEG_INFINITY, FACE_KINDS), leaves: (f64::INFINITY, FACE_KINDS) };
        for k in PAIRS {
            let along = dot(FACES[k].map(|v| v as f64), self.direction);
            if along == 0.0 {
                continue;
            }
            // Out through the face it goes towards, in through the one opposite.
            let (out, back) = if along > 0.0 { (k, opposite(k)) } else { (opposite(k), k) };
            let (inverse, across) = (1.0 / along.abs(), dot(FACES[out].map(|v| v as f64), r));
            let (enters, leaves) = ((-1.0 - across) * inverse, (1.0 - across) * inverse);
            if enters > chord.enters.0 {
                chord.enters = (enters, back);
            }
            if leaves < chord.leaves.0 {
                chord.leaves = (leaves, out);
            }
        }
        chord
    }

    /** Takes it into the next voxel, across the face it leaves its own by. */
    pub fn cross(&mut self) {
        let (leaves, out) = self.chord().leaves;
        self.t = self.t.max(leaves);
        self.site = std::array::from_fn(|i| self.site[i] + FACES[out][i]);
    }

    /** The face it enters its voxel by. */
    pub fn entered(&self) -> usize {
        self.chord().enters.1
    }

    /** The share of its voxel's chord it crosses, from where it is to where it leaves the voxel or `end`s. */
    pub fn part(&self, end: f64) -> f64 {
        let Chord { enters: (enters, _), leaves: (leaves, _) } = self.chord();
        let chord = leaves - enters;
        if chord > 0.0 { ((leaves.min(end) - self.t) / chord).clamp(0.0, 1.0) } else { 0.0 }
    }
}

/**
 * Where a ray from `origin` along `direction` first enters a voxel whose
 * site `holds`, if before `end`: how far, the site and the face it enters by.
 */
pub fn first_hit(
    origin: [f64; 3],
    direction: [f64; 3],
    end: f64,
    mut holds: impl FnMut([i64; 3]) -> bool,
) -> Option<(f64, [i64; 3], usize)> {
    let mut ray = Ray::new(origin, direction, 0.0);
    while ray.t <= end {
        if holds(ray.site) {
            return Some((ray.t, ray.site, ray.entered()));
        }
        ray.cross();
    }
    None
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_ray_crosses_voxel_after_voxel_each_holding_where_it_is() {
        // Not along a cube's diagonal: from here, that runs along faces, between voxels.
        let directions = [[0.8, 0.5, 0.3], [-0.3, 0.1, -0.9], [0.6, -0.61, 0.2], [0.0, -1.0, 0.0], [1.0, 1.0, 0.0]];
        for d in directions {
            let length = dot(d, d).sqrt();
            let direction = d.map(|v| v / length);
            let mut ray = Ray::new([-3.1, 2.3, 0.7], direction, 0.0);
            for _ in 0..200 {
                let (from, site) = (ray.t, ray.site);
                ray.cross();
                assert!(ray.t >= from, "{d:?}: forward");
                let middle: [f64; 3] = std::array::from_fn(|i| ray.origin[i] + (from + ray.t) / 2.0 * direction[i]);
                assert_eq!(nearest_site(middle), site, "{d:?}: the ray between {from} and {} is in the voxel it crossed", ray.t);
                let step: [i64; 3] = std::array::from_fn(|i| ray.site[i] - site[i]);
                assert!(FACES.contains(&step), "{d:?}: to a neighbour");
                // It enters on the plane of the face it says (at an edge, of either face there).
                let r: [f64; 3] = std::array::from_fn(|i| ray.at()[i] - ray.site[i] as f64);
                assert!((dot(r, FACES[ray.entered()].map(|v| v as f64)) - 1.0).abs() < 1e-9, "{d:?}: enters by its face");
                assert!((ray.part(f64::INFINITY) - 1.0).abs() < 1e-9, "{d:?}: entering, the whole chord is ahead");
            }
        }
    }

    #[test]
    fn the_nearest_site_is_the_one_whose_voxel_holds_the_point() {
        // Inside a voxel, (p − site) · face ≤ 1 for every face.
        for n in 0..1000 {
            let p = [n as f64 * 0.731 % 9.0 - 4.5, n as f64 * 0.377 % 7.0 - 3.5, n as f64 * 0.913 % 5.0 - 2.5];
            let s = nearest_site(p);
            assert_eq!((s[0] + s[1] + s[2]) & 1, 0, "{p:?}: a site");
            assert!(
                FACES.iter().all(|f| (0..3).map(|i| (p[i] - s[i] as f64) * f[i] as f64).sum::<f64>() <= 1.0 + 1e-9),
                "{p:?} in {s:?}'s voxel"
            );
        }
    }
}
