//! The valley's sky: clouds the wind drives, about its high summits. They
//! are what moves in it (worldgen's time.rs, a prototype): a medium, flat
//! underneath and heaped where the cover is thick, read where the wind
//! has carried it to, so only the tiles they are in are made anew.

use worldgen::domain::Scope;
use worldgen::generator::{Field, Layer, Meets, Op, Span};
use worldgen::kinds;
use worldgen::noise::{Fbm, fx, smoothstep};
use worldgen::time;
use worldgen::units::{Units, m};

use super::railway;

kinds! {
    base: railway::BASE + railway::KINDS.len() as u8;
    cloud: 8, ["#f2f4f6", "#eceff2", "#f7f8fa"], walk_through, medium;
}

/** The clouds' flat base, far over the towns, and how deep they heap over it at most (lattice units). */
const BASE_HEIGHT: i64 = m(700.0);
const DEPTH: Units = Units::m(60.0);
/** Some 25 m of cloud is seen through. */
const EXTINCTION: f64 = 3.9 / m(25.0) as f64;
/** The wind, lattice units a second: level, so the clouds keep their height. */
const WIND: [i64; 3] = [m(6.0), 0, m(2.5)];

/** The clouds of the valley of `seed`, where the cover, carried by the wind, is thick: deeper the thicker it is. */
pub fn clouds(seed: u64) -> Layer {
    let cover = Fbm::<{ m(400.0) }, 4>::new(Scope::new(seed).of("sky").seed("clouds"));
    let clouds = Field::columns("clouds", 6.0, Op::Fill, move |x, z| {
        let [dx, _, dz] = time::drift(WIND);
        let depth = (smoothstep(fx(0.5), fx(0.72), cover.at(x - dx, z - dz)) * DEPTH).round_units().0;
        if depth <= 0 {
            return Vec::new();
        }
        vec![Span::medium(BASE_HEIGHT, depth, K::cloud, EXTINCTION)]
    });
    Layer::Field(clouds.meets(|b, _| if b.max[1] < BASE_HEIGHT || b.min[1] > BASE_HEIGHT + DEPTH.0 { Meets::Nothing } else { Meets::Any }))
}

#[cfg(test)]
mod tests {
    use std::ops::Range;

    use worldgen::World;
    use worldgen::tiles::{TILE, make_tile};

    use super::*;
    use crate::valley::valley;

    /** The tile of level 5 the clouds are in over the origin, and the one under it, at `now`: each as made, and while when it stays so. */
    fn tiles(world: &World, now: i64) -> [(Vec<u32>, Range<i64>); 2] {
        let (level, y) = (5, (BASE_HEIGHT >> 5).div_euclid(TILE));
        [y, y - 1].map(|y| time::at(now, || make_tile(world, level, [0, y, 0]).cells))
    }

    #[test]
    fn clouds_move_and_what_is_under_them_stays() {
        let sky = || valley(1).alone("clouds");
        let world = sky();
        let [(clouds, stay), (under, under_stays)] = tiles(&world, 0);
        assert!(clouds.iter().any(|&look| look != 0), "clouds in their tile");
        assert_eq!(under_stays, time::ALWAYS, "under them, no time is read");
        // As made while they stay; moved before and after, as a world made then has them.
        for now in [stay.start, stay.end - 1] {
            assert_eq!(tiles(&world, now)[0].0, clouds);
        }
        for now in [stay.start - 1, stay.end] {
            let [(moved, _), (still, _)] = tiles(&world, now);
            assert!(moved != clouds && still == under, "the clouds moved, alone");
            assert_eq!(tiles(&sky(), now)[0].0, moved);
        }
        assert_eq!(tiles(&world, 0)[0].0, clouds, "and back, where they were");
    }
}
