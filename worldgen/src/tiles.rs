//! A world's tiles, as hosts and light take them: each level's sites in
//! cubes fixed in the world (`tile_box`), a tile made with those it shares
//! most of its cost with (`made_with`), only what may show of them
//! (generate_shown), and parted into the grids hosts draw (`make_tiles`).

use crate::World;
use crate::generator::generate_shown;
use crate::grid::{BRICK, Box3, Grid, VoxelGrid};
use crate::world::UP;

/**
 * Sites a tile is a side, at each level: small, as rays ask for little of
 * a larger one, and light reads a few sites around what it lights.
 */
pub const TILE: i64 = 32;

/**
 * Tile `key`'s box: a cube TILE sites a side, holding x in
 * [key[0] · TILE, (key[0] + 1) · TILE), y and z likewise, cut along UP to
 * `heights` rounded out to whole bricks, so that every tile's bricks are
 * on one grid; empty (no sites along UP) if it is all above or below them.
 */
pub fn tile_box((low, high): (i64, i64), key: [i64; 3]) -> Box3 {
    let cube = Box3 { min: key.map(|k| k * TILE), max: key.map(|k| (k + 1) * TILE) };
    let low = low.div_euclid(BRICK) * BRICK;
    let high = (high + BRICK - 1).div_euclid(BRICK) * BRICK;
    let (from, to) = (low.clamp(cube.min[UP], cube.max[UP]), high.clamp(cube.min[UP], cube.max[UP]));
    cube.slab(UP, from, to.max(from))
}

/**
 * The tiles of `level` of `world` made with tile `key`, itself among
 * them, bottom up: those above and below it within the world's heights,
 * as a world with heights lies across UP, and what they share (their
 * fields' lines, the things reaching several) costs most of each; alone,
 * if past its heights, empty, or in an endless one, where tiles cost as
 * much together as apart.
 */
pub fn made_with(world: &World, level: u32, key: [i64; 3]) -> impl Iterator<Item = [i64; 3]> + use<> {
    let (low, high) = world.heights_at(level);
    let within = low.div_euclid(TILE)..=(high - 1).div_euclid(TILE);
    let ups = if world.heights.is_some() && within.contains(&key[UP]) { within } else { key[UP]..=key[UP] };
    ups.map(move |up| {
        let mut key = key;
        key[UP] = up;
        key
    })
}

/**
 * The sites of the tiles made with tile `key` of `level` of `world`
 * (`made_with`), as one box, what may show of it (generate_shown).
 */
pub fn make_sites(world: &World, level: u32, key: [i64; 3]) -> VoxelGrid {
    let heights = world.heights_at(level);
    let boxes = made_with(world, level, key).map(|key| tile_box(heights, key));
    let all = boxes.reduce(|all, b| Box3 { min: all.min, max: b.max }).expect("its own");
    generate_shown(&world.generator, &all, level, UP)
}

/** The tiles `sites` are of (`make_sites`), by key, as hosts draw them, see-through ones too: each its part. */
pub fn parted(sites: &VoxelGrid, world: &World, level: u32, key: [i64; 3]) -> Vec<([i64; 3], Grid)> {
    let (grid, heights) = (sites.to_grid(), world.heights_at(level));
    // What was made may end short of a tile, or before it.
    let (low, high) = (grid.min[UP] as i64, (grid.min[UP] + grid.n[UP]) as i64);
    let part = |key: [i64; 3]| {
        let b = tile_box(heights, key);
        let (from, to) = (b.min[UP].clamp(low, high), b.max[UP].clamp(low, high));
        let (mut min, mut n) = (grid.min, grid.n);
        (min[UP], n[UP]) = (from as i32, (to - from) as i32);
        (key, grid.part(min, n))
    };
    made_with(world, level, key).map(part).collect()
}

/**
 * Tile `key` of `level` of `world` and those made with it (`made_with`),
 * by key, as hosts draw them: generated as one box and parted. Each shows
 * as made alone; what is buried may differ in shade, rock being of one
 * from a box's bottom up. Their sites are not kept: a host that lights
 * them makes them into the light (light's Shared::make_tiles).
 */
pub fn make_tiles(world: &World, level: u32, key: [i64; 3]) -> Vec<([i64; 3], Grid)> {
    parted(&make_sites(world, level, key), world, level, key)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::generator::Generator;

    #[test]
    fn tiles_are_cubes_cut_to_the_heights_at_bricks() {
        let heights = (-70, 140);
        let cube = |key: [i64; 3]| Box3 { min: key.map(|k| k * TILE), max: key.map(|k| (k + 1) * TILE) };
        assert_eq!(tile_box(heights, [2, 4, -3]), Box3 { max: [3 * TILE, 144, -2 * TILE], ..cube([2, 4, -3]) });
        assert_eq!(tile_box(heights, [0, -3, 0]), Box3 { min: [0, -72, 0], ..cube([0, -3, 0]) });
        assert_eq!(tile_box(heights, [1, 2, 3]), cube([1, 2, 3]), "whole within them");
        for key in [[0, 5, 0], [0, -4, 0]] {
            let b = tile_box(heights, key);
            assert!(b.min[1] == b.max[1], "{key:?}: none of it in the heights");
        }
    }

    /** What takes tiles made together (a host's queue, the light's store) counts on each of them naming the same ones. */
    #[test]
    fn tiles_made_together_each_name_the_same() {
        let world = |heights| World {
            generator: Generator { kinds: Vec::new(), seed: 1, layers: Vec::new() },
            spawn: [0; 3],
            heights,
            points: Vec::new(),
        };
        for (heights, level) in [(Some((-70, 140)), 0), (Some((-70, 140)), 2), (None, 0)] {
            let world = world(heights);
            for y in -6..=6 {
                let with: Vec<[i64; 3]> = made_with(&world, level, [3, y, -2]).collect();
                assert!(with.contains(&[3, y, -2]), "{heights:?}, level {level}: tile {y} not with itself in {with:?}");
                for &other in &with {
                    assert_eq!(
                        made_with(&world, level, other).collect::<Vec<_>>(),
                        with,
                        "{heights:?}, level {level}: tiles {y} and {}",
                        other[1]
                    );
                }
            }
        }
    }
}
