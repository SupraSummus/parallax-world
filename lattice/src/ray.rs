//! Rays across the lattice's voxels, exactly: cubes, site p the cube
//! [p, p + 1), so a ray goes through one as through a box (as voxels.wgsl
//! does), on into the neighbour across the face it leaves by. Sites in a
//! level's units, i64.

use crate::{FACE_KINDS, VOXEL_FACES};

/** The voxel's faces (VOXEL_FACES), as offsets of sites. */
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
    p.map(|v| v.floor() as i64)
}

/**
 * How many times what a site of `alpha` stops and glows in one crossing
 * it does in `crossings`: what it stops, 1 − (1 − alpha)^crossings, over
 * alpha. A site's alpha is what it stops of a ray crossing it along an
 * axis, a chord of 1 (as rows of sites run, and light goes): a ray at a
 * slant crosses more sites, each by a shorter chord, its path through
 * each its crossings of it (a medium's; pieces held in part stop a ray
 * whole or not at all, whatever its chord: `whole_or_none`).
 */
pub fn over(alpha: f64, crossings: f64) -> f64 {
    if alpha > 0.0 && crossings != 1.0 { (1.0 - (1.0 - alpha).powf(crossings)) / alpha } else { crossings }
}

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

    /** Its line through its voxel, where the slabs site ≤ p < site + 1 cross: in at the last it enters, out at the first it leaves. */
    fn chord(&self) -> Chord {
        let mut chord = Chord { enters: (f64::NEG_INFINITY, FACE_KINDS), leaves: (f64::INFINITY, FACE_KINDS) };
        for axis in 0..3 {
            let d = self.direction[axis];
            if d == 0.0 {
                continue;
            }
            let (low, high) = ((self.site[axis] as f64 - self.origin[axis]) / d, ((self.site[axis] + 1) as f64 - self.origin[axis]) / d);
            // Out through the face it goes towards, in through the one opposite (−axis is 2·axis, +axis 2·axis + 1).
            let (enters, leaves, out) = if d > 0.0 { (low, high, 2 * axis + 1) } else { (high, low, 2 * axis) };
            if enters > chord.enters.0 {
                chord.enters = (enters, out ^ 1);
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

    /** How far it goes in its voxel, from where it is to where it leaves it or `end`s: its crossings of it (`over`). */
    pub fn path(&self, end: f64) -> f64 {
        (self.chord().leaves.0.min(end) - self.t).max(0.0)
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

#[cfg(test)]
mod tests {
    use super::*;

    fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
        a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
    }

    #[test]
    fn a_ray_crosses_voxel_after_voxel_each_holding_where_it_is() {
        let directions = [[0.8, 0.5, 0.3], [-0.3, 0.1, -0.9], [0.6, -0.61, 0.2], [0.0, -1.0, 0.0], [1.0, 1.0, 0.1]];
        for d in directions {
            let length = dot(d, d).sqrt();
            let direction = d.map(|v| v / length);
            let mut ray = Ray::new([-3.13, 2.37, 0.71], direction, 0.0);
            for _ in 0..200 {
                let (from, site) = (ray.t, ray.site);
                ray.cross();
                assert!(ray.t >= from, "{d:?}: forward");
                let middle: [f64; 3] = std::array::from_fn(|i| ray.origin[i] + (from + ray.t) / 2.0 * direction[i]);
                assert_eq!(nearest_site(middle), site, "{d:?}: the ray between {from} and {} is in the voxel it crossed", ray.t);
                let step: [i64; 3] = std::array::from_fn(|i| ray.site[i] - site[i]);
                assert!(FACES.contains(&step), "{d:?}: to a neighbour");
                // It enters on the plane of the face it says.
                let (k, at) = (ray.entered(), ray.at());
                let plane = (ray.site[k / 2] + (k as i64 & 1)) as f64;
                assert!((at[k / 2] - plane).abs() < 1e-9, "{d:?}: enters by its face");
                let chord = ray.chord();
                assert!(
                    (ray.path(f64::INFINITY) - (chord.leaves.0 - chord.enters.0)).abs() < 1e-9,
                    "{d:?}: entering, the whole chord is ahead"
                );
            }
        }
    }
}
