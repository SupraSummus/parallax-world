//! A world's tiles, as hosts and light take them: each level's sites in
//! cubes fixed in the world (`tile_box`), a tile made with those it shares
//! most of its cost with (`made_with`), only what may show of them
//! (generate_shown), and parted into the grids hosts draw (`make_tiles`),
//! each saying while when it stays as made (time.rs).

use std::ops::Range;

use crate::World;
use crate::generator::{Drew, generate_shown, only};
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

/**
 * A tile as hosts draw it: its key, its grid, and while when it stays as
 * made, of the world's time (time.rs): always, unless what may be drawn
 * in it moves.
 */
pub type Made = ([i64; 3], Grid, Range<i64>);

/** The tiles `sites` are of (`make_sites`), by key, as hosts draw them, see-through ones too: each its part, staying as its bricks do. */
pub fn parted(sites: &VoxelGrid, world: &World, level: u32, key: [i64; 3]) -> Vec<Made> {
    let (grid, heights) = (sites.to_grid(), world.heights_at(level));
    // What was made may end short of a tile, or before it.
    let (low, high) = (grid.min[UP] as i64, (grid.min[UP] + grid.n[UP]) as i64);
    let part = |key: [i64; 3]| {
        let b = tile_box(heights, key);
        let (from, to) = (b.min[UP].clamp(low, high), b.max[UP].clamp(low, high));
        let (mut min, mut n) = (grid.min, grid.n);
        (min[UP], n[UP]) = (from as i32, (to - from) as i32);
        (key, grid.part(min, n), sites.stays(&b))
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
pub fn make_tiles(world: &World, level: u32, key: [i64; 3]) -> Vec<Made> {
    parted(&make_sites(world, level, key), world, level, key)
}

/**
 * Tile `key` of `level` of `world` alone, as hosts draw it: what may show
 * of its box (generate_shown), of only the layers `drew` names if given;
 * while when it stays so; and the layers that drew in it or read the time,
 * if known (`Drew`). A host makes a tile so anew when what moves in it
 * moved (time.rs), the others made with it staying as made, and then of
 * only those layers.
 */
pub fn make_tile(world: &World, level: u32, key: [i64; 3], drew: Option<&Drew>) -> (Grid, Range<i64>, Option<Drew>) {
    let b = tile_box(world.heights_at(level), key);
    let sites = only(drew, || generate_shown(&world.generator, &b, level, UP));
    (sites.to_grid(), sites.stays(&b), sites.drew)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::generator::Generator;
    use crate::time;

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

    /**
     * Of tiles made together, only those a field that reads the time may
     * draw in (Field::meets) stay as made for less than always: a slab of
     * mist drifting high over a ground. With no word of where it draws, it
     * may draw anywhere: all of them.
     */
    #[test]
    fn only_the_tiles_what_moves_may_be_in_stay_for_less_than_always() {
        use crate::generator::{Field, KindDef, Layer, Meets, Op, Span};
        let (low, high) = (200, 210);
        let world = |bounded: bool| {
            let kind = |name, medium| KindDef { name, kind_type: 10, colors: &["#808080"], walk_through: medium, glows: false, medium };
            let ground = Field::spans("ground", 0.0, Op::Fill, UP, |_, _| vec![Span::solid(-1000, 0, 0)]);
            let mist = Field::spans("mist", 1.0, Op::Fill, UP, move |x, _| {
                let [dx, _, _] = time::drift([40, 0, 0]);
                if (x - dx).rem_euclid(64) < 32 { vec![Span::medium(low, high - low, 1, 0.05)] } else { Vec::new() }
            });
            let mist = if bounded {
                mist.meets(move |b, _| if b.max[UP] <= low || b.min[UP] >= high { Meets::Nothing } else { Meets::Any })
            } else {
                mist
            };
            let layers = vec![Layer::Field(ground), Layer::Field(mist)];
            let generator = Generator { kinds: vec![kind("ground", false), kind("mist", true)], seed: 1, layers };
            World { generator, spawn: [0; 3], heights: Some((-64, 320)), points: Vec::new() }
        };
        for bounded in [true, false] {
            let world = world(bounded);
            let (tiles, _) = time::at(1000, || make_tiles(&world, 0, [0, 0, 0]));
            assert!(tiles.len() > 4, "{} tiles made together", tiles.len());
            for (key, grid, stays) in tiles {
                let b = tile_box(world.heights_at(0), key);
                let mist = b.min[UP] < high && b.max[UP] > low;
                assert!(stays.contains(&1000), "tile {key:?}: {stays:?}");
                assert_eq!(stays == time::ALWAYS, bounded && !mist, "tile {key:?}, bounded {bounded}: {stays:?}");
                // The mist in it, made alone, the same while it stays, and moved after.
                if mist {
                    let alone = |now: i64| {
                        let ((grid, stays, _), _) = time::at(now, || make_tile(&world, 0, key, None));
                        (grid, stays)
                    };
                    let (then, while_then) = alone(1000);
                    assert!(then.cells == grid.cells && while_then == stays, "tile {key:?} alone");
                    assert!(alone(stays.end - 1).0.cells == then.cells && alone(stays.end).0.cells != then.cells, "tile {key:?}");
                }
            }
        }
    }

    /**
     * A tile made alone says the layers that drew in it or read the time:
     * a slab of mist drifting high over a ground, a ledge in it over one
     * tile. Made anew of only them, it is as made of all.
     */
    #[test]
    fn a_tile_made_anew_of_the_layers_that_drew_in_it_is_as_made_of_all() {
        use crate::generator::{Field, KindDef, Layer, Op, Span};
        let kind = |name, medium| KindDef { name, kind_type: 10, colors: &["#808080"], walk_through: medium, glows: false, medium };
        let ground = Field::spans("ground", 0.0, Op::Fill, UP, |_, _| vec![Span::solid(-1000, 0, 0)]);
        let ledge = Field::spans("ledge", 0.5, Op::Fill, UP, |x, _| if x < 16 { vec![Span::solid(198, 202, 0)] } else { Vec::new() });
        let mist = Field::spans("mist", 1.0, Op::Fill, UP, |x, _| {
            let [dx, _, _] = time::drift([40, 0, 0]);
            if (x - dx).rem_euclid(64) < 32 { vec![Span::medium(200, 10, 1, 0.05)] } else { Vec::new() }
        });
        let layers = vec![Layer::Field(ground), Layer::Field(ledge), Layer::Field(mist)];
        let generator = Generator { kinds: vec![kind("ground", false), kind("mist", true)], seed: 1, layers };
        let world = World { generator, spawn: [0; 3], heights: Some((-64, 320)), points: Vec::new() };
        let made = |key: [i64; 3], now: i64, drew: Option<&Drew>| time::at(now, || make_tile(&world, 0, key, drew)).0;
        for (key, names) in [([0, 6, 0], vec!["ledge", "mist"]), ([1, 6, 0], vec!["mist"])] {
            let (_, stays, drew) = made(key, 1000, None);
            let drew = drew.expect("no rock under it");
            assert_eq!(drew, Drew(names.into()), "tile {key:?}");
            for now in [stays.end, stays.start - 1, 9000] {
                let ((all, stays, _), (only, only_stays, again)) = (made(key, now, None), made(key, now, Some(&drew)));
                let b = tile_box(world.heights_at(0), key);
                let sites =
                    (b.min[0]..b.max[0]).flat_map(|x| (b.min[1]..b.max[1]).flat_map(move |y| (b.min[2]..b.max[2]).map(move |z| [x, y, z])));
                assert!(sites.map(|p| p.map(|v| v as i32)).all(|[x, y, z]| all.get(x, y, z) == only.get(x, y, z)), "tile {key:?} at {now}");
                assert!(
                    all.cells.iter().any(|&look| look != 0) && stays == only_stays && again == Some(drew.clone()),
                    "tile {key:?} at {now}"
                );
            }
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
