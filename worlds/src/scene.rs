//! Scenes made elsewhere, as worlds: voxels on the FCC lattice read
//! from a file (scripts/voxelize.py makes one from a glTF scene), each
//! coarser level made from the one below. Nothing is outside the scene.
//!
//! The file, little-endian: b"FCC0"; the palette's size (u32, at most
//! 255) and its colours (r, g, b); the box's min and size (i32 × 3 each,
//! lattice units, min and width even); then its sites (x + y + z even)
//! in order z, y, x, as runs: the number of empty sites, of full ones
//! (varints), and the full ones' ids (bytes, 1 the first colour); a run
//! with no full ones ends it.

use std::rc::Rc;

use worldgen::World;
use worldgen::generator::{Field, Generator, KindDef, Layer, Op};
use worldgen::units::m;

/** One level's sites, of any lattice point of its box: 0 none, else 1 + the palette's index. */
struct Level {
    min: [i64; 3],
    n: [i64; 3],
    ids: Vec<u8>,
}

impl Level {
    fn new(min: [i64; 3], max: [i64; 3]) -> Level {
        let n = [0, 1, 2].map(|i| max[i] - min[i]);
        Level { min, n, ids: vec![0; (n[0] * n[1] * n[2]) as usize] }
    }

    fn index(&self, p: [i64; 3]) -> Option<usize> {
        let q = [0, 1, 2].map(|i| p[i] - self.min[i]);
        (0..3).all(|i| 0 <= q[i] && q[i] < self.n[i]).then(|| ((q[2] * self.n[1] + q[1]) * self.n[0] + q[0]) as usize)
    }

    fn get(&self, p: [i64; 3]) -> u8 {
        self.index(p).map_or(0, |i| self.ids[i])
    }

    /**
     * The level above: a site is full where FULL of its voxel is, and
     * holds what most of that does. Its voxel holds the site under it
     * whole, half of each of the 12 nearest, and a sixth of each of the 6
     * next (on its 4-fold corners).
     */
    fn coarser(&self) -> Level {
        // A site of margin: a finer site's voxel reaches into the coarser ones beside its own.
        let min = self.min.map(|v| v.div_euclid(2) - 1);
        let max = [0, 1, 2].map(|i| (self.min[i] + self.n[i]).div_euclid(2) + 2);
        let mut level = Level::new(min, max);
        let mut weights = [0u32; 256];
        for z in min[2]..max[2] {
            for y in min[1]..max[1] {
                let mut x = min[0] + (min[0] + y + z).rem_euclid(2);
                while x < max[0] {
                    let c = [2 * x, 2 * y, 2 * z];
                    let near = NEAR.map(|(d, w)| (self.get([c[0] + d[0], c[1] + d[1], c[2] + d[2]]), w));
                    let (mut total, mut best) = (0, (0, 0u8));
                    for (id, w) in near.into_iter().filter(|(id, _)| *id != 0) {
                        total += w;
                        weights[id as usize] += w;
                        best = best.max((weights[id as usize], id));
                    }
                    if total >= FULL {
                        let i = level.index([x, y, z]).expect("in its box");
                        level.ids[i] = best.1;
                    }
                    for (id, _) in near {
                        weights[id as usize] = 0;
                    }
                    x += 2;
                }
            }
        }
        level
    }

    /** The lowest and highest of its full sites' y, lattice units; none if it has none. */
    fn heights(&self, spacing: i64) -> Option<(i64, i64)> {
        let layer = |y: i64| (0..self.n[2]).any(|z| (0..self.n[0]).any(|x| self.ids[((z * self.n[1] + y) * self.n[0] + x) as usize] != 0));
        let lo = (0..self.n[1]).find(|&y| layer(y))?;
        let hi = (0..self.n[1]).rev().find(|&y| layer(y))?;
        Some(((self.min[1] + lo) * spacing, (self.min[1] + hi) * spacing))
    }
}

/**
 * How much of a coarser site's voxel the finer sites in it must fill
 * (of 48, NEAR's shares) for it to be full: a sheet a site thick through
 * its middle fills 22, one between two coarser sites 12 of each.
 */
const FULL: u32 = 12;

/** The finer sites in a coarser one's voxel, and their shares of it, in sixths. */
const NEAR: [([i64; 3], u32); 19] = [
    ([0, 0, 0], 6),
    ([1, 1, 0], 3),
    ([1, -1, 0], 3),
    ([-1, 1, 0], 3),
    ([-1, -1, 0], 3),
    ([1, 0, 1], 3),
    ([1, 0, -1], 3),
    ([-1, 0, 1], 3),
    ([-1, 0, -1], 3),
    ([0, 1, 1], 3),
    ([0, 1, -1], 3),
    ([0, -1, 1], 3),
    ([0, -1, -1], 3),
    ([2, 0, 0], 1),
    ([-2, 0, 0], 1),
    ([0, 2, 0], 1),
    ([0, -2, 0], 1),
    ([0, 0, 2], 1),
    ([0, 0, -2], 1),
];

/** A file's bytes, read in order. */
struct Reader<'a>(std::slice::Iter<'a, u8>);

impl Reader<'_> {
    fn byte(&mut self) -> u8 {
        *self.0.next().expect("the file goes on")
    }

    fn i32(&mut self) -> i32 {
        i32::from_le_bytes(std::array::from_fn(|_| self.byte()))
    }

    /** 7 bits a byte, low first. */
    fn varint(&mut self) -> i64 {
        let (mut v, mut shift) = (0, 0);
        loop {
            let b = self.byte();
            v |= i64::from(b & 0x7f) << shift;
            shift += 7;
            if b < 0x80 {
                return v;
            }
        }
    }
}

/** A scene: its levels, from 0 up to one whose sites are as far apart as the scene is wide. */
pub struct Scene {
    colours: Vec<u32>,
    levels: Vec<Level>,
}

impl Scene {
    /** Reads a file (see the module's comment); panics on a malformed one. */
    pub fn read(bytes: &[u8]) -> Scene {
        let mut r = Reader(bytes.iter());
        assert_eq!([r.byte(), r.byte(), r.byte(), r.byte()], *b"FCC0", "not a scene");
        let count = r.i32();
        assert!((0..=255).contains(&count), "{count} colours: ids are bytes");
        let colours = (0..count).map(|_| u32::from_be_bytes([0, r.byte(), r.byte(), r.byte()])).collect();
        let min: [i64; 3] = std::array::from_fn(|_| r.i32().into());
        let n: [i64; 3] = std::array::from_fn(|_| r.i32().into());
        assert!(min.iter().all(|v| v % 2 == 0) && n[0] % 2 == 0, "min and width even");
        let mut level = Level::new(min, [0, 1, 2].map(|i| min[i] + n[i]));
        // A site's rank in the file's order: each row along x has n[0] / 2.
        let mut rank = 0;
        loop {
            rank += r.varint();
            let full = r.varint();
            if full == 0 {
                break;
            }
            for _ in 0..full {
                let (row, x) = (rank / (n[0] / 2), rank % (n[0] / 2));
                let (y, z) = (row % n[1], row / n[1]);
                let id = r.byte();
                assert!(id != 0 && i32::from(id) <= count, "id {id} of {count} colours");
                level.ids[((z * n[1] + y) * n[0] + 2 * x + (y + z) % 2) as usize] = id;
                rank += 1;
            }
        }
        let wide = *n.iter().max().expect("three");
        let mut levels = vec![level];
        while 1 << (levels.len() - 1) < wide {
            levels.push(levels.last().expect("a level").coarser());
        }
        Scene { colours, levels }
    }

    /** The site's id at (x, y, z), lattice units, at the level of `spacing`: 0 if none or above its levels. */
    fn at(&self, x: i64, y: i64, z: i64, spacing: i64) -> u8 {
        let k = spacing.trailing_zeros();
        self.levels.get(k as usize).map_or(0, |level| level.get([x, y, z].map(|v| v >> k)))
    }

    /** Its box at level 0, lattice units: min and max (exclusive). */
    fn bounds(&self) -> ([i64; 3], [i64; 3]) {
        let l = &self.levels[0];
        (l.min, [0, 1, 2].map(|i| l.min[i] + l.n[i]))
    }

    /** The scene as a world: a kind per colour, drawn as a field; the camera at its middle, `eye` metres over its lowest point. */
    pub fn world(self, eye: f64) -> World {
        let kinds = self
            .colours
            .iter()
            .map(|rgb| {
                // Kinds' colours are static: a world lives as long as its host.
                let colour: &'static str = Box::leak(format!("#{rgb:06x}").into_boxed_str());
                KindDef { name: "scene", kind_type: 1, colors: Box::leak(Box::new([colour])), walk_through: false, glows: false }
            })
            .collect();
        let (min, max) = self.bounds();
        let spawn = [(min[0] + max[0]) / 2, min[1] + m(eye), (min[2] + max[2]) / 2];
        // Coarser levels may reach a site past level 0.
        let heights = (0..self.levels.len()).filter_map(|k| self.levels[k].heights(1 << k));
        let heights = heights.reduce(|a, b| (a.0.min(b.0), a.1.max(b.1))).unwrap_or((min[1], min[1]));
        let scene = Rc::new(self);
        let field = Field::at("scene", 0.0, Op::Fill, move |x, y, z, s| match scene.at(x, y, z, s) {
            0 => None,
            id => Some((id - 1, None)),
        });
        World { generator: Generator { kinds, seed: 0, layers: vec![Layer::Field(field)] }, spawn, heights, points: Vec::new() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /** A scene of two colours, its box at `min` of size `n`, the sites `full` (in the box's coordinates) with their ids. */
    fn scene(min: [i32; 3], n: [i32; 3], full: &[([i32; 3], u8)]) -> Scene {
        let mut file = b"FCC0".to_vec();
        file.extend(2i32.to_le_bytes());
        file.extend([0; 6]);
        file.extend(min.iter().chain(&n).flat_map(|v| v.to_le_bytes()));
        let mut ranked: Vec<_> = full.iter().map(|&([x, y, z], id)| (((z * n[1] + y) * n[0] + x) / 2, id)).collect();
        ranked.sort();
        let mut at = 0;
        // A run of one site each; small boxes' gaps fit a byte.
        for (rank, id) in ranked {
            file.extend([(rank - at) as u8, 1, id]);
            at = rank + 1;
        }
        file.extend([0, 0]);
        Scene::read(&file)
    }

    /** The sites of a floor at y of the box's coordinates, across a box 8 wide. */
    fn floor(y: i32) -> Vec<([i32; 3], u8)> {
        (0..8).flat_map(|z| (0..8).map(move |x| [x, y, z])).filter(|p| (p[0] + p[1] + p[2]) % 2 == 0).map(|p| (p, 1)).collect()
    }

    #[test]
    fn a_coarser_site_holds_what_most_of_its_voxel_does() {
        // The site at the middle of 1, three of its nearest of 2: 2 fills more of the coarse voxel.
        let scene = scene([-2, -2, -2], [6, 5, 5], &[([2, 2, 2], 1), ([3, 3, 2], 2), ([1, 3, 2], 2), ([2, 3, 3], 2)]);
        assert_eq!(scene.at(0, 0, 0, 2), 2);
    }

    #[test]
    fn coarser_levels_keep_a_floor_a_site_thick_and_opaque() {
        // Through the coarse sites at y = 0: they hold it, those over and under it don't.
        let through = scene([0, -4, 0], [8, 8, 8], &floor(4));
        assert_eq!([through.at(2, -2, 0, 2), through.at(2, 0, 2, 2), through.at(2, 2, 0, 2)], [0, 1, 0]);
        // Between them: both hold it, so no ray goes through.
        let between = scene([0, -4, 0], [8, 8, 8], &floor(5));
        assert_eq!([between.at(2, 0, 2, 2), between.at(4, 2, 2, 2), between.at(2, 4, 2, 2)], [1, 1, 0]);
    }
}
