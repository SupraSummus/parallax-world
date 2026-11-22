//! Chunk baking, for native's chunks engine: the whole voxels of a cell
//! chunk (lod.rs) that can show, as a relief of its columns over the plane
//! through its site, seen along the face's axis, and its medium.
//! A world is tiles of grids, one set per level of detail.
//! See docs/architecture.md, section 3.

mod cells;
pub mod face;
pub mod lod;
mod pillars;
mod surface;
mod tables;

use std::sync::Arc;

use cells::Cells;
use face::{LAYER, spacing};
use fcc::light::Lighting;
use fcc::{ALPHA, FACE_KINDS, Lattice, Reader, Tiles, VIEW_FACES, alpha_of, lattices};
use lod::CellChunk;
pub use pillars::Medium;
use pillars::{Pillars, cell_medium, cell_relief, lit_voxel_colour, voxel_colour};
use surface::surface;
use tables::Geometry;

/**
 * The grid a cell chunk reads, and the sites of it it reads, a box of
 * (a, b, c) in the face's lattice coordinates, in that grid's units,
 * inclusive: a chunk is the same whatever else the grid holds. A cell deep
 * past its cell, where its relief's solid may run on, and half a cell
 * before it, where its medium's slabs are (Baker::bake_cell); a column or
 * two around it, which nothing reads any more (docs/todo.md).
 */
pub fn chunk_reads(chunk: &CellChunk) -> (usize, [i32; 3], [i32; 3]) {
    let (min, max) = chunk.bounds();
    let m = chunk.spacing();
    (chunk.atoms as usize, [min[0] - 4, min[1] - 2, min[2] - m - 2], [max[0] + 4, max[1] + 2, max[2] + 2 * m])
}

/**
 * The sites a cell chunk's medium reads, a box as chunk_reads has it: the
 * slabs of its columns whose middle sites it holds (cell_medium), half a
 * cell deeper each way.
 */
pub fn medium_reads(chunk: &CellChunk) -> ([i32; 3], [i32; 3]) {
    let ((from, to), m) = (chunk.bounds(), chunk.spacing());
    ([from[0], from[1], from[2] - m], [to[0], to[1], to[2] + m])
}

/**
 * Layers of a slab of a cell's medium (Medium) of cells m apart: m, as
 * deep as a cell's spacing, but 2 at least, so that a slab of each column
 * holds a site. From layer 1, so that its halves end where a level's and
 * the next finer one's claims of the medium do (Lod::layers), on layers
 * of grid 0 that are whole numbers of half spacings.
 */
pub fn slab_layers(m: i32) -> i32 {
    m.max(2)
}

/** One world's grids, shades and lattices, and the buffers reused across bakes. */
pub struct Baker {
    /**
     * By level: grid ℓ has the sites of 2^ℓ·FCC, in its own units, once set.
     * Shared: native threads bake from one copy.
     */
    grids: Vec<Arc<Tiles>>,
    geometry: Geometry,
    lattices: [Lattice; VIEW_FACES],
    /** Per view face, its face kinds' shades (tables::shades). */
    shades: [[f32; FACE_KINDS]; VIEW_FACES],
    /** Per view face, its face kinds' in the world (tables::world_kinds), as `light` has them. */
    world_kinds: [[usize; FACE_KINDS]; VIEW_FACES],
    /** If the host lights the world, its light; else voxels are shaded by the way their faces face (voxel_colour). */
    pub light: Option<Lighting>,
    /** Which cell of N·FCC owns each site, by N, for cell chunks (bake_cell). */
    owners: Vec<(i32, Cells)>,
    pillars: Pillars,
    /**
     * A cell chunk's relief's columns (bake_cell), each its place among the
     * cell's columns from the least (a, b) of its bounds, its front's depth
     * past the plane and its pillar's run in half layers, as i16s, low
     * first, and its colour.
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

impl Baker {
    pub fn new() -> Baker {
        let lattices = lattices();
        Baker {
            grids: Vec::new(),
            geometry: tables::geometry(&lattices[0]),
            lattices,
            shades: tables::shades(&lattices),
            world_kinds: tables::world_kinds(&lattices),
            light: None,
            owners: Vec::new(),
            pillars: Pillars::default(),
            columns: Vec::new(),
            medium: Vec::new(),
        }
    }

    /**
     * Sets the tiles of `level` to ones other bakers may share. Tiles may
     * come and go but not change, so a chunk baked stays right as long as
     * it had all the tiles it reads (chunk_reads).
     */
    pub fn share_tiles(&mut self, level: usize, tiles: Arc<Tiles>) {
        self.reach(level);
        self.grids[level] = tiles;
    }

    /** Empty grids up to `level`, if it has none. */
    fn reach(&mut self, level: usize) {
        while self.grids.len() <= level {
            self.grids.push(Arc::new(Tiles::new(0, [])));
        }
    }

    /**
     * Bakes a cell chunk (lod.rs) of view face `face` over the plane through
     * its site: the relief of its whole voxels (cell_relief), as `columns`;
     * or, its medium's chunk, its see-through ones as `medium`
     * (cell_medium); and the top-left corner of where they may be drawn,
     * in world units. They stay in the baker until the next bake.
     * Unclipped: no other chunk of its level draws its voxels, and depth
     * decides between them. What is covered is left to the GPU.
     * Its medium is by slabs of its columns (slab_layers), whose halves'
     * ends are where a level's claims of the medium end (lod.rs), so that
     * the compositor draws a half or not, where a level's medium ends.
     */
    pub fn bake_cell(&mut self, face: usize, chunk: &CellChunk) -> [f64; 2] {
        let level = chunk.atoms as usize;
        self.reach(level);
        let m = chunk.spacing();
        let mut owners = std::mem::take(&mut self.owners);
        if !owners.iter().any(|(k, _)| *k == m) {
            owners.push((m, Cells::new(m)));
        }
        let cells = &owners.iter().find(|(k, _)| *k == m).expect("made").1;
        let holds = |a: i32, b: i32, c: i32| cells.owner(a, b, c) == chunk.site;
        let tiles = Arc::clone(&self.grids[level]);
        let reader = Reader::new(&tiles, self.lattices[face]);
        let (from, to) = chunk.bounds();
        let whole: Vec<_> = if chunk.medium {
            Vec::new()
        } else {
            surface(&reader, &self.geometry, from, to, holds).into_iter().filter(|v| v.alpha() == ALPHA).collect()
        };
        // From a site on along the axis, the last whole voxel before any that is not, a cell deep past the cell at most.
        let runs_on = |[a, b, mut c]: [i32; 3]| {
            let whole = |c| {
                let mut whole = false;
                reader.row(a, b, c, 1, |_, look| whole = alpha_of(look) == ALPHA);
                whole
            };
            while c + 2 <= to[2] + 2 * m && whole(c + 2) {
                c += 2;
            }
            c
        };
        // Over the plane through its site.
        let centre = 2 * chunk.site[2];
        self.paint(face, level, |pillars, geometry, colour| cell_relief(pillars, geometry, colour, &whole, centre, &runs_on));
        let s = &self.pillars;
        let [a, b, _] = from;
        let i16s = |low: i32, high: i32| low as u16 as u32 | (high as u16 as u32) << 16;
        self.columns.clear();
        self.columns.extend(s.pillars.iter().map(|p| [i16s(p.site[0] - a, p.site[1] - b), i16s(p.depth, p.run), p.colour]));
        // None, but maybe medium: from its least column.
        let corner = s.corner().unwrap_or([a as f64 * LAYER, b as f64]);
        let origin = corner.map(|v| v * spacing(level as i32) as f64);
        let mut parts = std::mem::take(&mut self.medium);
        parts.clear();
        if chunk.medium {
            let (lo, hi) = medium_reads(chunk);
            let around = surface(&reader, &self.geometry, lo, hi, |_, _, _| true);
            let slab = slab_layers(m);
            self.paint(face, level, |_, geometry, colour| cell_medium(&mut parts, geometry, colour, &around, centre, slab, &holds));
            parts.iter_mut().for_each(|p| p.column = [p.column[0] - a, p.column[1] - b]);
        }
        self.medium = parts;
        self.owners = owners;
        origin
    }

    /** What `paint` makes of voxels read from grid `level` along view face `face`, given their colours. */
    fn paint(&mut self, face: usize, level: usize, paint: impl FnOnce(&mut Pillars, &Geometry, &mut dyn FnMut(&surface::Voxel) -> u32)) {
        let lattice = self.lattices[face];
        let shades = &self.shades[face];
        let (light, kinds) = (&mut self.light, &self.world_kinds[face]);
        let mut colour = |voxel: &surface::Voxel| {
            let rgb = match light {
                Some(light) => lit_voxel_colour(voxel, &light(level, lattice.world(voxel.site)), kinds, lattice.f.map(|v| -v)),
                None => voxel_colour(voxel, shades),
            };
            rgb & 0x00ff_ffff | voxel.look & 0xff00_0000
        };
        paint(&mut self.pillars, &self.geometry, &mut colour);
    }
}
