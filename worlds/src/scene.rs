//! Scenes made elsewhere, as worlds: cubes of level 0 read from a file
//! (scripts/voxelize.py makes one from a glTF scene), each coarser level
//! made from the one below. Nothing is outside the scene.
//!
//! The file, little-endian: b"CUB0"; the palette's size (u32, at most
//! 255) and its colours (r, g, b); the box's min and size (i32 × 3 each,
//! lattice units); then its sites in order z, y, x, as runs: the number
//! of empty sites, of full ones (varints), and the full ones' ids (bytes,
//! 1 the first colour); a run with no full ones ends it.

use std::sync::Arc;

use worldgen::World;
use worldgen::generator::{Field, Generator, KindDef, Layer, Meets, Op};
use worldgen::grid::Box3;
use worldgen::sites;
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

    /** The level above: a site is full where FULL of the eight in its cube are, and holds what most of them do. */
    fn coarser(&self) -> Level {
        let min = self.min.map(|v| v.div_euclid(2));
        let max = [0, 1, 2].map(|i| (self.min[i] + self.n[i] + 1).div_euclid(2));
        let mut level = Level::new(min, max);
        let mut weights = [0u32; 256];
        for z in min[2]..max[2] {
            for y in min[1]..max[1] {
                for x in min[0]..max[0] {
                    let ids = (0..8).map(|k| self.get([2 * x + (k & 1), 2 * y + (k >> 1 & 1), 2 * z + (k >> 2)]));
                    let (mut total, mut best) = (0, (0, 0u8));
                    for id in ids.clone().filter(|&id| id != 0) {
                        total += 1;
                        weights[id as usize] += 1;
                        best = best.max((weights[id as usize], id));
                    }
                    if total >= FULL {
                        let i = level.index([x, y, z]).expect("in its box");
                        level.ids[i] = best.1;
                    }
                    for id in ids {
                        weights[id as usize] = 0;
                    }
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

/** Of the eight sites in a coarser one's cube, how many must be full for it to be: half, so a sheet a site thick stays one. */
const FULL: u32 = 4;

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
        assert_eq!([r.byte(), r.byte(), r.byte(), r.byte()], *b"CUB0", "not a scene");
        let count = r.i32();
        assert!((0..=255).contains(&count), "{count} colours: ids are bytes");
        let colours = (0..count).map(|_| u32::from_be_bytes([0, r.byte(), r.byte(), r.byte()])).collect();
        let min: [i64; 3] = std::array::from_fn(|_| r.i32().into());
        let n: [i64; 3] = std::array::from_fn(|_| r.i32().into());
        let mut level = Level::new(min, [0, 1, 2].map(|i| min[i] + n[i]));
        // A site's rank in the file's order is its index.
        let mut rank = 0;
        loop {
            rank += r.varint();
            let full = r.varint();
            if full == 0 {
                break;
            }
            for _ in 0..full {
                let id = r.byte();
                assert!(id != 0 && i32::from(id) <= count, "id {id} of {count} colours");
                level.ids[rank as usize] = id;
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
                KindDef {
                    name: "scene",
                    kind_type: 1,
                    colors: Box::leak(Box::new([colour])),
                    walk_through: false,
                    glows: false,
                    medium: false,
                }
            })
            .collect();
        let (min, max) = self.bounds();
        let spawn = [(min[0] + max[0]) / 2, min[1] + m(eye), (min[2] + max[2]) / 2];
        // Coarser levels may reach a site past level 0.
        let heights = (0..self.levels.len()).filter_map(|k| self.levels[k].heights(1 << k));
        let heights = heights.reduce(|a, b| (a.0.min(b.0), a.1.max(b.1))).unwrap_or((min[1], min[1]));
        let scene = Arc::new(self);
        let field = Field::at("scene", 0.0, Op::Fill, move |x, y, z| match scene.at(x, y, z, sites::spacing()) {
            0 => None,
            id => Some((id - 1, None)),
        })
        // Nothing past its box.
        .meets(move |b, _| if b.overlaps(&Box3 { min, max }) { Meets::Any } else { Meets::Nothing });
        World {
            generator: Generator { kinds, seed: 0, layers: vec![Layer::Field(field)] },
            spawn,
            heights: Some(heights),
            points: Vec::new(),
            edits: Default::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /** A scene of two colours, its box at `min` of size `n`, the sites `full` (in the box's coordinates) with their ids. */
    fn scene(min: [i32; 3], n: [i32; 3], full: &[([i32; 3], u8)]) -> Scene {
        let mut file = b"CUB0".to_vec();
        file.extend(2i32.to_le_bytes());
        file.extend([0; 6]);
        file.extend(min.iter().chain(&n).flat_map(|v| v.to_le_bytes()));
        let mut ranked: Vec<_> = full.iter().map(|&([x, y, z], id)| ((z * n[1] + y) * n[0] + x, id)).collect();
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
        (0..8).flat_map(|z| (0..8).map(move |x| ([x, y, z], 1))).collect()
    }

    #[test]
    fn a_coarser_site_holds_what_most_of_its_cube_does() {
        // Of the eight in the cube of coarse site 0, one of 1 and three of 2.
        let most = scene([0; 3], [4; 3], &[([0, 0, 0], 1), ([1, 0, 0], 2), ([0, 1, 0], 2), ([1, 1, 1], 2)]);
        assert_eq!(most.at(0, 0, 0, 2), 2);
        // Three are not enough.
        let sparse = scene([0; 3], [4; 3], &[([0, 0, 0], 1), ([1, 0, 0], 2), ([0, 1, 0], 2)]);
        assert_eq!(sparse.at(0, 0, 0, 2), 0);
    }

    #[test]
    fn coarser_levels_keep_a_floor_a_site_thick() {
        // In the coarse sites it is in, at every level, whichever half of them it fills.
        for y in [4, 5] {
            let floor = scene([0, 0, 0], [8, 8, 8], &floor(y));
            assert_eq!([floor.at(2, 2, 2, 2), floor.at(2, 4, 2, 2), floor.at(2, 6, 2, 2)], [0, 1, 0], "at {y}");
            assert_eq!([floor.at(0, 0, 0, 4), floor.at(0, 4, 0, 4)], [0, 1], "at {y}");
        }
    }
}
