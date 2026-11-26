//! Chunk baking, for native's flat engine: the whole voxels of a cell
//! chunk (lod.rs) that can show, as pillars of its columns over the plane
//! through its middle, seen along a face's axis (face.rs), and its medium.
//! A world is tiles of grids, one set per level of detail.
//! See docs/architecture.md, section 3.

pub mod face;
pub mod lod;
mod pillars;

use std::sync::Arc;

use face::{FACES, Face, faces};
use lattice::light::Lighting;
use lattice::{ALPHA, BRICK, Look, Tiles, VOXEL_FACES, alpha_of, facing, whole_or_none};
use lod::CellChunk;
pub use pillars::{Half, Medium};
use pillars::{Voxel, cell_medium, cell_relief, lit_voxel_colour, seen, voxel_colour};

/**
 * The grid a cell chunk along `face` reads, and the sites of it it reads,
 * a box, inclusive, in that grid's units: a chunk is the same whatever
 * else the grid holds. Its sites and their neighbours; a cell deeper
 * along the axis, where its pillars' solid may run on, and half a cell
 * nearer, where its medium's slabs are (Baker::bake_cell).
 */
pub fn chunk_reads(face: &Face, chunk: &CellChunk) -> (usize, [i32; 3], [i32; 3]) {
    let (mut lo, mut hi) = chunk.sites();
    let m = chunk.spacing();
    for d in 0..3 {
        let (before, after) = (m / 2 + 1, m);
        match face.f[d].signum() {
            1 => (lo[d], hi[d]) = (lo[d] - before, hi[d] + after),
            -1 => (lo[d], hi[d]) = (lo[d] - after, hi[d] + before),
            _ => (),
        }
        (lo[d], hi[d]) = (lo[d] - 1, hi[d] + 1);
    }
    (chunk.atoms as usize, lo, hi)
}

/**
 * Layers of a slab of a cell's medium (Medium) of cells m sites across: m,
 * as deep as a cell's spacing, but 2 at least, so that a slab of each
 * column holds a site. From layer 1, so that its halves end where a
 * level's and the next finer one's claims of the medium do (Lod::layers).
 */
pub fn slab_layers(m: i32) -> i32 {
    m.max(2)
}

/** One world's grids, and what a bake makes. */
pub struct Baker {
    /**
     * By level: grid ℓ has the sites of level ℓ, in its own units, once set.
     * Shared: native threads bake from one copy.
     */
    grids: Vec<Arc<Tiles>>,
    faces: [Face; FACES],
    /** If the host lights the world, its light; else voxels are shaded by the way their faces face (voxel_colour). */
    pub light: Option<Lighting>,
    /**
     * A cell chunk's columns (bake_cell), each its place among the cell's
     * columns from its column (0, 0), its front's depth past the plane and
     * its pillar's run in half layers, as i16s, low first, and its colour.
     */
    pub columns: Vec<[u32; 3]>,
    /** A cell chunk's medium (bake_cell), by slab and row. */
    pub medium: Vec<Medium>,
}

impl Default for Baker {
    fn default() -> Baker {
        Baker::new()
    }
}

/** A box of sites' looks, as a host shows them (whole_or_none), read once. */
struct Read {
    lo: [i32; 3],
    n: [i32; 3],
    looks: Vec<Look>,
}

impl Read {
    fn new(tiles: &Tiles, lo: [i32; 3], hi: [i32; 3]) -> Read {
        let n = std::array::from_fn(|d| hi[d] - lo[d] + 1);
        let mut looks = Vec::with_capacity((n[0] * n[1] * n[2]) as usize);
        for z in lo[2]..=hi[2] {
            for y in lo[1]..=hi[1] {
                // A tile's grid for each brick's run of the row: tiles are whole bricks.
                let mut grid = None;
                for x in lo[0]..=hi[0] {
                    if x == lo[0] || x & (BRICK - 1) == 0 {
                        grid = tiles.grid_at(x, y, z);
                    }
                    looks.push(grid.map_or(0, |g| whole_or_none(g.get(x, y, z), x, y, z)));
                }
            }
        }
        Read { lo, n, looks }
    }

    /** The look at site p; 0 outside the box. */
    fn at(&self, p: [i32; 3]) -> Look {
        let q: [i32; 3] = std::array::from_fn(|d| p[d] - self.lo[d]);
        if (0..3).any(|d| q[d] < 0 || q[d] >= self.n[d]) {
            return 0;
        }
        self.looks[((q[2] * self.n[1] + q[1]) * self.n[0] + q[0]) as usize]
    }
}

impl Baker {
    pub fn new() -> Baker {
        Baker { grids: Vec::new(), faces: faces(), light: None, columns: Vec::new(), medium: Vec::new() }
    }

    /**
     * Sets the tiles of `level` to ones other bakers may share. Tiles may
     * come and go but not change, so a chunk baked stays right as long as
     * it had all the tiles it reads (chunk_reads).
     */
    pub fn share_tiles(&mut self, level: usize, tiles: Arc<Tiles>) {
        while self.grids.len() <= level {
            self.grids.push(Arc::new(Tiles::new(0, [])));
        }
        self.grids[level] = tiles;
    }

    /**
     * Bakes a cell chunk (lod.rs) along face `face` over the plane through
     * its middle: the pillars of its whole voxels (cell_relief), as
     * `columns`; or, its medium's chunk, its see-through ones as `medium`
     * (cell_medium); and where its column (0, 0)'s middle is in the face's
     * axes, in world units: where its columns are drawn from. They stay in
     * the baker until the next bake. Unclipped: no other chunk of its level
     * draws its voxels, and depth decides between them.
     * Its medium is by slabs of its columns (slab_layers), whose halves'
     * ends are where a level's claims of the medium end (lod.rs), so that
     * the compositor draws a half or not, where a level's medium ends.
     */
    pub fn bake_cell(&mut self, face: usize, chunk: &CellChunk) -> [f64; 2] {
        let level = chunk.atoms as usize;
        if self.grids.len() <= level {
            self.share_tiles(level, Arc::new(Tiles::new(0, [])));
        }
        let face = self.faces[face];
        let (k, m) = (face.k(), chunk.spacing());
        let tiles = Arc::clone(&self.grids[level]);
        let (_, lo, hi) = chunk_reads(&face, chunk);
        let read = Read::new(&tiles, lo, hi);
        let seen = seen(&face);
        let facing: Vec<usize> = facing(face.f.map(f64::from)).collect();
        // The voxels of `sites` that show, each with its open and cut faces: whole ones cut where the cell is.
        let voxels = |from: [i32; 3], to: [i32; 3], whole: bool| {
            let mut voxels = Vec::new();
            for z in from[2]..=to[2] {
                for y in from[1]..=to[1] {
                    for x in from[0]..=to[0] {
                        let p = [x, y, z];
                        let look = read.at(p);
                        if look == 0 || (alpha_of(look) == ALPHA) != whole {
                            continue;
                        }
                        let (mut open, mut cut) = (0u8, 0u8);
                        for (kind, n) in VOXEL_FACES.iter().enumerate() {
                            let q = std::array::from_fn(|d| p[d] + n[d]);
                            if alpha_of(read.at(q)) < ALPHA {
                                open |= 1 << kind;
                            } else if whole && !chunk.holds(q) {
                                cut |= 1 << kind;
                            }
                        }
                        if open == 0 && cut == 0 {
                            continue;
                        }
                        // Its look along the face: through the faces turned towards the camera.
                        let grid = tiles.grid_at(x, y, z).expect("read");
                        let sum = facing.iter().fold([0u32; 3], |s, &kind| {
                            let f = grid.face(x, y, z, kind).to_le_bytes();
                            [s[0] + u32::from(f[0]), s[1] + u32::from(f[1]), s[2] + u32::from(f[2])]
                        });
                        let n = facing.len() as u32;
                        let [r, g, b] = sum.map(|c| ((c + n / 2) / n) as u8);
                        let [sr, sg, sb, _] = grid.stops(x, y, z).to_le_bytes();
                        let stops = if whole { [ALPHA; 3] } else { [sr, sg, sb] };
                        let look = u32::from_le_bytes([r, g, b, alpha_of(look)]);
                        voxels.push(Voxel { site: face.of(p), look, stops, open, cut });
                    }
                }
            }
            voxels
        };
        let centre = chunk.centre(&face);
        let least = chunk.columns(&face);
        let mut light = self.light.take();
        let mut colour = |voxel: &Voxel| {
            let rgb = match &mut light {
                Some(light) => lit_voxel_colour(voxel, &light(level, face.world(voxel.site)), face.f.map(|v| -v)),
                None => voxel_colour(voxel),
            };
            rgb & 0x00ff_ffff | voxel.look & 0xff00_0000
        };
        self.columns.clear();
        self.medium.clear();
        if chunk.medium {
            // A cell deeper and nearer than the cell, but its neighbours: what slabs whose middle it holds hold.
            let around = voxels(lo.map(|v| v + 1), hi.map(|v| v - 1), false);
            let owns = |a, b, c| chunk.holds(face.world([a, b, c]));
            self.medium = cell_medium(seen, k, &mut colour, &around, centre, slab_layers(m), least, &owns);
        } else {
            let (from, to) = chunk.sites();
            let whole = voxels(from, to, true);
            // Read as far as a cell past the cell along the axis.
            let limit = chunk.layer_range(&face)[1] + k * m;
            let is_whole = |abc: [i32; 3]| alpha_of(read.at(face.world(abc))) == ALPHA;
            let pillars = cell_relief(&face, seen, &mut colour, &whole, centre, limit, &is_whole);
            let i16s = |low: i32, high: i32| low as u16 as u32 | (high as u16 as u32) << 16;
            let [a0, b0] = least;
            self.columns.extend(pillars.iter().map(|p| [i16s(p.site[0] - a0, p.site[1] - b0), i16s(p.depth, p.run), p.colour]));
        }
        self.light = light;
        let unit = f64::from(1 << chunk.atoms);
        let middle = face.middle();
        [(f64::from(least[0]) + middle[0]) * face.layer() * unit, (f64::from(least[1]) + middle[1]) * unit]
    }
}

/** The distance along `face`'s axis of the plane `chunk` is drawn over, in world units. */
pub fn plane(face: &Face, chunk: &CellChunk) -> f64 {
    (f64::from(chunk.centre(face)) / 2.0 + face.middle()[2]) * face.layer() * f64::from(1 << chunk.atoms)
}

/** A half layer of grid `atoms` along `face`, in world units: the unit of pillars' depths. */
pub fn half_layer(face: &Face, atoms: i32) -> f64 {
    face.layer() * f64::from(1 << atoms) / 2.0
}
