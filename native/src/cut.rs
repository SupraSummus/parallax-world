//! Cutting: what a stroke where one looks takes out of the world. The ray
//! from the eye along the view meets a voxel of level 0 within REACH; the
//! stroke takes out what is within RADIUS of it, a ball: so strokes one
//! after another, each meeting what the last left, cut deeper, and a
//! notch is cut as an axe cuts one. What it takes out, the world does
//! (World::take_out).

use lattice::Tiles;
use lattice::ray::first_hit;
use worldgen::units::m;

use crate::view::Vec3;

/** How far from the eye a stroke reaches, lattice units: an arm and a tool. */
pub const REACH: f64 = m(3.0) as f64;

/** How far from the voxel met a stroke takes out, lattice units. */
pub const RADIUS: i64 = m(0.25);

/**
 * The level-0 sites a stroke from `eye` along `forward` (lattice units)
 * takes out of `tiles`, level 0's: none if it meets no voxel within REACH
 * of tiles made.
 */
pub fn stroke(tiles: &Tiles, eye: Vec3, forward: Vec3) -> Vec<[i64; 3]> {
    let look = |[x, y, z]: [i64; 3]| tiles.at(x as i32, y as i32, z as i32);
    let Some((_, hit, _)) = first_hit(eye, forward, REACH, |site| look(site).is_some_and(|l| l != 0)) else { return Vec::new() };
    let r = -RADIUS..=RADIUS;
    let ball = r.clone().flat_map(|dx| r.clone().flat_map(move |dy| (-RADIUS..=RADIUS).map(move |dz| [dx, dy, dz])));
    ball.filter(|d| d.iter().map(|v| v * v).sum::<i64>() <= RADIUS * RADIUS)
        .map(|d| [hit[0] + d[0], hit[1] + d[1], hit[2] + d[2]])
        .filter(|&site| look(site).is_some_and(|l| l != 0))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use lattice::{BRICK_SITES, Grid, look};
    use std::sync::Arc;

    /** A post 1 m square, x and z in [0, 8), y in [0, 64), as level 0's tiles. */
    fn post() -> Tiles {
        let mut grid = Grid::new([0; 3], [8, 64, 8], 8);
        for (b, brick) in grid.bricks.iter_mut().enumerate() {
            *brick = b as i32;
        }
        grid.cells = vec![look([120, 80, 40], 255); 8 * BRICK_SITES];
        // One tile 64 sites a side holds it all.
        Tiles::new(6, [([0, 0, 0], Arc::new(grid))])
    }

    #[test]
    fn a_stroke_takes_out_a_ball_about_what_it_meets() {
        let tiles = post();
        // From the side, level, at a height of 2.5 m (20 units).
        let sites = stroke(&tiles, [-8.0, 20.5, 4.5], [1.0, 0.0, 0.0]);
        let near = |s: &[i64; 3]| (s[0] * s[0] + (s[1] - 20).pow(2) + (s[2] - 4).pow(2)) <= RADIUS * RADIUS;
        assert!(!sites.is_empty() && sites.iter().all(near), "{sites:?}");
        assert!(
            sites.contains(&[0, 20, 4]) && sites.contains(&[RADIUS, 20, 4]) && sites.contains(&[0, 20 + RADIUS, 4]),
            "into the post, from its face"
        );
        assert!(stroke(&tiles, [-8.0 - REACH, 20.5, 4.5], [1.0, 0.0, 0.0]).is_empty(), "out of reach");
        assert!(stroke(&tiles, [-8.0, 20.5, 4.5], [-1.0, 0.0, 0.0]).is_empty(), "away from it");
    }
}
