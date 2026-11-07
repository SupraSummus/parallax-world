//! The data: the valley seen from above, each column's topmost voxel as a
//! class of kinds; and its towns, rebuilt from the public API as
//! worlds/src/valley.rs builds them.

use std::rc::Rc;

use worldgen::generator::{Kind, generate};
use worldgen::grid::Box3;
use worlds::fields;
use worlds::forest::{self, K as F, Relief};
use worlds::landmarks::{self, K as L};
use worlds::moor::{self, K as M};
use worlds::mountains::{self, K as A};
use worlds::railway::{self, K as R};
use worlds::town::{self, K as T, Towns};

use crate::image::Image;

/**
 * Each of the valley's kinds' class: what a column shows from above,
 * coarser than its kinds. 21 classes: the forest's below 10, the town's
 * below 16, then the fields'.
 */
pub fn class_of_kind(kinds: usize) -> Vec<u8> {
    use worlds::fields::K as P;
    let forest: [(&[F], u8); 10] = [
        (&[F::moss, F::grass, F::tallGrass], 0),
        (&[F::litter, F::needles, F::soil, F::fallenLeaf], 1),
        (&[F::path], 2),
        (&[F::stone, F::mossyStone], 3),
        (&[F::bark, F::spruceBark, F::birchBark, F::birchMark, F::deadwood, F::barkFurrow, F::spruceScale, F::twig, F::cone], 4),
        (&[F::spruce], 5),
        (&[F::oakGreen, F::birchGreen], 6),
        (&[F::oakOrange, F::oakGold, F::birchGold], 7),
        (&[F::fern, F::bracken, F::bush], 8),
        (
            &[
                F::flowerYellow,
                F::flowerViolet,
                F::flowerWhite,
                F::flowerRed,
                F::amanita,
                F::amanitaSpot,
                F::mushroomStem,
                F::bolete,
                F::berry,
            ],
            9,
        ),
    ];
    let town: [(&[T], u8); 6] = [
        (&[T::roofRed, T::roofTile, T::roofBrown, T::slate, T::thatch, T::copper], 10),
        (
            &[
                T::plasterOchre,
                T::plasterWhite,
                T::plasterPink,
                T::plasterBlue,
                T::plasterSage,
                T::plasterGrey,
                T::faluRed,
                T::brick,
                T::timber,
                T::trim,
                T::shutterGreen,
                T::shutterBlue,
                T::glass,
                T::door,
                T::churchStone,
            ],
            11,
        ),
        (&[T::cobble], 12),
        (&[T::paving, T::pavingDark, T::slab], 13),
        (&[T::planks, T::counter, T::goods, T::iron, T::lamp], 14),
        (&[T::water], 15),
    ];
    let farm: [(&[P], u8); 5] =
        [(&[P::wheat], 16), (&[P::stubble], 17), (&[P::furrow, P::tilth], 18), (&[P::crop], 19), (&[P::meadow, P::meadowGrass], 20)];
    let mut class = vec![u8::MAX; kinds];
    let mut set = |kind: Kind, c: u8| class[kind as usize] = c;
    forest.iter().for_each(|(ks, c)| ks.iter().for_each(|k| set(forest::k(*k), *c)));
    town.iter().for_each(|(ks, c)| ks.iter().for_each(|k| set(town::k(*k), *c)));
    farm.iter().for_each(|(ks, c)| ks.iter().for_each(|k| set(fields::k(*k), *c)));
    set(forest::k(F::hay), 17);
    // The domains after the fields', which the experiments don't tell apart: as the forest's ground, rock or undergrowth,
    // the towns' walls or fittings.
    for (kind, c) in [
        (mountains::k(A::alpine), 0),
        (mountains::k(A::scree), 3),
        (mountains::k(A::rock), 3),
        (mountains::k(A::snow), 3),
        (landmarks::k(L::sand), 0),
        (landmarks::k(L::reed), 8),
        (landmarks::k(L::lily), 8),
        (landmarks::k(L::sail), 11),
        (landmarks::k(L::bronze), 14),
        (moor::k(M::heather), 8),
        (moor::k(M::heatherSprigs), 8),
        (moor::k(M::bog), 0),
        (moor::k(M::grassBlades), 0),
        (moor::k(M::lichen), 3),
        (moor::k(M::drystone), 3),
        (railway::k(R::ballast), 3),
    ] {
        set(kind, c);
    }
    assert!(class.iter().all(|c| *c != u8::MAX), "a kind without a class");
    class
}

/** The valley's towns, built as worlds/src/valley.rs builds them (it keeps them private). */
pub fn towns(seed: u64) -> Rc<Towns> {
    Rc::new(Towns::new(seed, Rc::new(Relief::new(seed))))
}

/** An area of columns at a level: its first column (level units) and size. */
#[derive(Clone, Copy, Debug)]
pub struct Area {
    pub level: u32,
    pub x0: i64,
    pub z0: i64,
    pub nx: usize,
    pub nz: usize,
}

/** The classes of the valley of `seed` over `area` (x fastest, z rows), or read from `cache` if there. */
pub fn valley(seed: u64, area: Area, cache: &str) -> Image {
    if let Ok(bytes) = std::fs::read(cache)
        && bytes.len() == area.nx * area.nz
    {
        return Image::new(area.nx, area.nz, bytes.iter().map(|b| *b as u16).collect());
    }
    const TILE: usize = 64;
    let world = worlds::valley::valley(seed);
    let generator = &world.generator;
    let class = class_of_kind(generator.kinds.len());
    let mut material_class = vec![0u8];
    for (kind, def) in generator.kinds.iter().enumerate() {
        material_class.extend(def.colors.iter().map(|_| class[kind]));
    }
    let (min_y, max_y) = world.heights_at(area.level);
    let mut out = vec![0u8; area.nx * area.nz];
    for tz in (0..area.nz).step_by(TILE) {
        for tx in (0..area.nx).step_by(TILE) {
            let (x0, z0) = (area.x0 + tx as i64, area.z0 + tz as i64);
            let b = Box3 { min: [x0, min_y, z0], max: [x0 + TILE as i64, max_y, z0 + TILE as i64] };
            let grid = generate(generator, &b, area.level);
            for j in tz..(tz + TILE).min(area.nz) {
                for i in tx..(tx + TILE).min(area.nx) {
                    let (x, z) = (area.x0 + i as i64, area.z0 + j as i64);
                    let id = (min_y..max_y).rev().filter(|y| (x + y + z) & 1 == 0).map(|y| grid.get(x, y, z)).find(|id| *id != 0);
                    out[j * area.nx + i] = material_class[id.unwrap_or(0) as usize];
                }
            }
        }
    }
    if let Some(dir) = std::path::Path::new(cache).parent() {
        std::fs::create_dir_all(dir).expect("cache directory");
    }
    std::fs::write(cache, &out).expect("cache written");
    Image::new(area.nx, area.nz, out.into_iter().map(|b| b as u16).collect())
}
