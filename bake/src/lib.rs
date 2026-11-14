//! Chunk baking, compiled to WebAssembly (in the web's module, web/wasm/)
//! for the bake workers: the voxels of a chunk cell that can show, as splats
//! (hexagons of one colour) on its slice's centre plane, seen along the
//! face's axis, and a mask of what they cover.
//! A world is a grid per level of detail, or tiles of grids;
//! each slice reads the one its atoms are the voxels of (in_grid).
//! chunk-baker.ts drives it through web/wasm's baker.rs.
//! See docs/architecture.md, section 3.
//! It also finds where a grid's voxels are along the faces' layers
//! (layer_index.rs), and walks a body on the ground (walk.rs).

mod cells;
pub mod fill;
mod layer_index;
pub mod lod;
pub mod slicing;
mod splats;
mod surface;
mod tables;
pub mod walk;

use std::sync::Arc;

use cells::Cells;
use fcc::light::Lighting;
use fcc::{ALPHA, FACE_KINDS, Grid, Lattice, Reader, Tiles, VIEW_FACES, lattices};
use fill::{fill_mask, solid_blocks};
use lod::CellChunk;
use slicing::{LAYER, chunk_cell, slice_centre, spacing, thickness};
pub use splats::Medium;
use splats::{Splat, Splats, cell_medium, cell_relief, cell_splats, chunk_splats, lit_voxel_colour, voxel_colour};
use surface::{cell_surface, surface};
use tables::Geometry;

// For native frontends, which link the baker instead of loading its WebAssembly.
pub use layer_index::LayerIndex;
pub use surface::Slice;

/**
 * Mask pixels per unit of its grid for a slice of `level` as the grid
 * sees it (in_grid): 40 / t, t its thickness (see thickness), between 1
 * (2 pixels across a voxel) and 10. Such a slice lies at distance
 * ≥ t · detail · LAYER, where a unit is at most focal / (t · detail · LAYER)
 * screen pixels: with the default view and detail more than 40 / t, so a
 * mask is coarser than the screen (docs/todo.md), but each chunk's is about
 * as many pixels.
 */
fn mask_scale(level: i32) -> f64 {
    (40.0 / thickness(level, 0) as f64).floor().clamp(1.0, 10.0)
}

/**
 * A slice of the world as the grid it reads sees it: grid ℓ = its atoms'
 * level has the sites of N·FCC, N = 2^ℓ, as its own units, so the slice is
 * slab `layer` of 2^(level − ℓ) of its layers, cut off at cutoff / N: a
 * slice of single voxels. Returns ℓ and that slice.
 */
fn in_grid(slice: &Slice) -> (usize, Slice) {
    let l = slice.atoms;
    let n = spacing(l);
    (l as usize, Slice { level: slice.level - l, atoms: 0, layer: slice.layer, cutoff: slice.cutoff / n as f64 })
}

/**
 * The grid a cell chunk reads, and the sites of it it reads, a box of
 * (a, b, c) in the face's lattice coordinates, in that grid's units,
 * inclusive: a chunk is the same whatever else the grid holds. A cell deep
 * past its cell, where its relief's solid may run on, and half a cell
 * before it and a column or two around it, where its medium's slabs and
 * their neighbours are (Baker::bake_cell).
 */
pub fn chunk_reads(chunk: &CellChunk) -> (usize, [i32; 3], [i32; 3]) {
    let (min, max) = chunk.bounds();
    let m = chunk.spacing();
    (chunk.atoms as usize, [min[0] - 4, min[1] - 2, min[2] - m - 2], [max[0] + 4, max[1] + 2, max[2] + 2 * m])
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

/** A baked chunk: see ChunkSplats. Its numbers and blocks stay in the baker until the next bake. */
pub struct Baked {
    /** The mask's size in pixels. */
    pub width: usize,
    pub height: usize,
    /** The top-left corner of what is drawn, its size, and the mask's pixels per unit. */
    pub origin_x: f64,
    pub origin_y: f64,
    pub extent_width: f64,
    pub extent_height: f64,
    pub scale: f64,
    /** How many splats. */
    pub count: usize,
}

/** One world's grids, palettes and lattices, and the buffers reused across bakes. */
pub struct Baker {
    /**
     * By level: grid ℓ has the sites of 2^ℓ·FCC, in its own units, once set. They share materials.
     * Shared: native threads bake from one copy.
     */
    grids: Vec<Arc<Tiles>>,
    /** The level set last. */
    last: usize,
    geometry: Geometry,
    lattices: [Lattice; VIEW_FACES],
    /** Per view face: material · FACE_KINDS + kind → RGBA. */
    palettes: Vec<u32>,
    /** Per view face, its face kinds' in the world (tables::world_kinds), as `light` has them. */
    world_kinds: [[usize; FACE_KINDS]; VIEW_FACES],
    /** If the host lights the world, its light; else voxels are shaded by the way their faces face (voxel_colour). */
    pub light: Option<Lighting>,
    /** Which cell of 2·FCC owns each site, for cutoffs. */
    pairs: Cells,
    /** Which cell of N·FCC owns each site, by N, for cell chunks (bake_cell). */
    owners: Vec<(i32, Cells)>,
    splats: Splats,
    mask: Vec<u8>,
    pub blocks: Vec<u8>,
    /** The splats as numbers (see ChunkSplats). */
    pub numbers: Vec<f32>,
    /**
     * A cell chunk's relief's columns (bake_cell), each its place among the
     * cell's columns from the least (a, b) of its bounds, its front's depth
     * past the plane and its pillar's run in half layers, as i16s, low
     * first, and its colour.
     */
    pub columns: Vec<[u32; 3]>,
    /** A cell chunk's medium (bake_cell), by slab and row. */
    pub medium: Vec<Medium>,
    /** The spacing of the grid the last chunk's splats were read from. */
    spacing: i32,
}

impl Baker {
    /** A baker for the world's materials' colours, 0xrrggbb by id. */
    pub fn new(colours: &[u32]) -> Baker {
        let lattices = lattices();
        Baker {
            grids: Vec::new(),
            last: 0,
            geometry: tables::geometry(&lattices[0]),
            lattices,
            palettes: tables::palettes(&lattices, colours),
            world_kinds: tables::world_kinds(&lattices),
            light: None,
            pairs: Cells::new(2),
            owners: Vec::new(),
            splats: Splats::new(),
            mask: Vec::new(),
            blocks: Vec::new(),
            numbers: Vec::new(),
            columns: Vec::new(),
            medium: Vec::new(),
            spacing: 1,
        }
    }

    /** Sets the grid of `level`, replacing what it had; levels without one so far read as empty. */
    pub fn set_grid(&mut self, level: usize, grid: Grid) -> &mut Grid {
        self.reach(level);
        self.last = level;
        self.grids[level] = Arc::new(Tiles::one(grid));
        Arc::get_mut(&mut self.grids[level]).and_then(Tiles::only_mut).expect("just set")
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

    /** The grid set_grid set last, to write its sites, while no other baker shares it. */
    pub fn last_grid(&mut self) -> Option<&mut Grid> {
        self.grids.get_mut(self.last).and_then(Arc::get_mut).and_then(Tiles::only_mut)
    }

    /** The sites of the last baked chunk's splats, in world units (for tests). */
    pub fn splat_sites(&self) -> impl Iterator<Item = [i32; 3]> + '_ {
        self.splats.splats.iter().map(|s| s.site.map(|v| v * self.spacing))
    }

    /**
     * Bakes chunk (i, j) of a slice of view face `face`: its splats that
     * may be drawn in the cell (chunk_splats), the extent of where they
     * may be drawn clipped to the cell, and a mask of it.
     * See ChunkBaker.bake.
     */
    pub fn bake(&mut self, face: usize, slice: &Slice, i: i32, j: i32) -> Baked {
        let (level, slice) = in_grid(slice);
        self.reach(level);
        let reader = Reader::new(&self.grids[level], self.lattices[face]);
        let surface = cell_surface(&reader, &self.geometry, &self.pairs, &slice, i, j);
        let centre = (2.0 * slice_centre(&slice) / LAYER).round() as i32;
        self.paint(face, level, |splats, geometry, colour| chunk_splats(splats, geometry, colour, &surface, centre));

        // Where they may be drawn, clipped to the cell, in the grid's units.
        let cell = chunk_cell(slice.level, i, j);
        let (left, bottom) = (cell.left as f64 * LAYER, cell.bottom as f64);
        let splats = &mut self.splats;
        let min_x = splats.min_x.max(left);
        let max_x = splats.max_x.min(left + cell.width);
        let min_y = splats.min_y.max(bottom);
        let max_y = splats.max_y.min(bottom + cell.height);
        let empty = max_x <= min_x || max_y <= min_y;
        // Splats that miss the cell draw nothing.
        splats.splats.retain(|s| {
            let (w, h) = s.reach();
            !empty && s.x + w > min_x && s.x - w < max_x && s.y + h > min_y && s.y - h < max_y
        });

        let scale = mask_scale(slice.level);
        let width = if empty { 0 } else { ((max_x - min_x) * scale).ceil() as usize };
        let height = if empty { 0 } else { ((max_y - min_y) * scale).ceil() as usize };
        self.mask.clear();
        self.mask.resize(width * height, 0);
        fill_mask(splats, &mut self.mask, width, height, min_x, max_y, scale);
        solid_blocks(&mut self.blocks, &self.mask, width, height);
        let extent = if empty { [0.0; 2] } else { [max_x - min_x, max_y - min_y] };
        self.numbers(level, min_x, max_y, extent, [width, height], scale, slice_numbers)
    }

    /**
     * Bakes a cell chunk (lod.rs) of view face `face` over the plane through
     * its site: of its whole voxels, if `relief`, their relief
     * (cell_relief), as `columns`, else those a ray can enter as splats
     * (cell_splats), no columns; or, its medium's chunk, its see-through
     * ones as `medium` (cell_medium); and where they may be drawn.
     * Unclipped: no other chunk of its level draws its voxels, and depth
     * decides between them. No mask: what is covered is left to the GPU.
     * Its numbers are its splats': depth past the plane in half layers, and
     * colour.
     * Its medium is by slabs of its columns (slab_layers), whose halves'
     * ends are where a level's claims of the medium end (lod.rs), so that
     * the compositor draws a half or not, where a level's medium ends.
     */
    pub fn bake_cell(&mut self, face: usize, chunk: &CellChunk, relief: bool) -> Baked {
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
            surface(&reader, &self.geometry, from, to, holds).into_iter().filter(|v| v.alpha == ALPHA).collect()
        };
        let (whole, hexagons) = if relief { (whole, Vec::new()) } else { (Vec::new(), whole) };
        // From a site on along the axis, the last whole voxel before any that is not, a cell deep past the cell at most.
        let runs_on = |[a, b, mut c]: [i32; 3]| {
            let whole = |c| {
                let mut whole = false;
                reader.row(a, b, c, 1, |_, material, alpha| whole = material != 0 && alpha == ALPHA);
                whole
            };
            while c + 2 <= to[2] + 2 * m && whole(c + 2) {
                c += 2;
            }
            c
        };
        // Over the plane through its site.
        let centre = 2 * chunk.site[2];
        self.paint(face, level, |splats, geometry, colour| cell_relief(splats, geometry, colour, &whole, centre, &runs_on));
        let s = &self.splats;
        let [a, b, _] = from;
        let i16s = |low: i32, high: i32| low as u16 as u32 | (high as u16 as u32) << 16;
        self.columns.clear();
        self.columns.extend(s.splats.iter().map(|s| [i16s(s.site[0] - a, s.site[1] - b), i16s(s.depth, s.run), s.colour]));
        let relief = [s.min_x, s.max_x, s.min_y, s.max_y];
        self.paint(face, level, |splats, geometry, colour| cell_splats(splats, geometry, colour, &hexagons, centre));
        let s = &self.splats;
        let (min_x, max_x, min_y, max_y) = (s.min_x.min(relief[0]), s.max_x.max(relief[1]), s.min_y.min(relief[2]), s.max_y.max(relief[3]));
        // None, but maybe medium: from its least column.
        let (extent, min_x, max_y) =
            if min_x < max_x { ([max_x - min_x, max_y - min_y], min_x, max_y) } else { ([0.0; 2], a as f64 * LAYER, b as f64) };
        self.blocks.clear();
        let numbers = |s: &Splat| [s.depth as f32, f32::from_bits(s.colour)];
        let baked = self.numbers(level, min_x, max_y, extent, [0, 0], 1.0, numbers);
        let mut parts = std::mem::take(&mut self.medium);
        parts.clear();
        if chunk.medium {
            // Its slabs, those of whose middle sites it holds, and their neighbours.
            let (lo, hi) = ([from[0] - 2, from[1] - 1, from[2] - m], [to[0] + 2, to[1] + 1, to[2] + m]);
            let around = surface(&reader, &self.geometry, lo, hi, |_, _, _| true);
            let slab = slab_layers(m);
            self.paint(face, level, |_, geometry, colour| cell_medium(&mut parts, geometry, colour, &around, centre, slab, &holds));
            parts.iter_mut().for_each(|p| p.column = [p.column[0] - a, p.column[1] - b]);
        }
        self.medium = parts;
        self.owners = owners;
        baked
    }

    /** The splats `splats` makes of voxels read from grid `level` along view face `face`, given their colours. */
    fn paint(&mut self, face: usize, level: usize, splats: impl FnOnce(&mut Splats, &Geometry, &mut dyn FnMut(&surface::Voxel) -> u32)) {
        let lattice = self.lattices[face];
        let palettes = self.palettes.len() / VIEW_FACES;
        let palette = &self.palettes[face * palettes..(face + 1) * palettes];
        let (light, kinds) = (&mut self.light, &self.world_kinds[face]);
        let mut colour = |voxel: &surface::Voxel| {
            let rgb = match light {
                Some(light) => lit_voxel_colour(voxel, &light(level, lattice.world(voxel.site)), kinds, lattice.f.map(|v| -v)),
                None => voxel_colour(voxel, palette),
            };
            rgb & 0x00ff_ffff | u32::from(voxel.alpha) << 24
        };
        splats(&mut self.splats, &self.geometry, &mut colour);
    }

    /**
     * The splats as numbers, in world units relative to the top-left corner
     * (min_x, max_y) of what they draw, `extent` wide and high in the grid's
     * units, and the mask's size and pixels per unit; `last` gives each
     * splat's last two.
     */
    #[allow(clippy::too_many_arguments)]
    fn numbers<const N: usize>(
        &mut self,
        level: usize,
        min_x: f64,
        max_y: f64,
        extent: [f64; 2],
        [width, height]: [usize; 2],
        scale: f64,
        last: impl Fn(&Splat) -> [f32; N],
    ) -> Baked {
        self.spacing = spacing(level as i32);
        let n = self.spacing as f64;
        self.numbers.clear();
        for s in &self.splats.splats {
            self.numbers.extend([
                ((s.x - min_x) * n) as f32,
                ((s.y - max_y) * n) as f32,
                // Its height is √2 times that.
                (s.width * n) as f32,
            ]);
            self.numbers.extend(last(s));
        }
        Baked {
            width,
            height,
            origin_x: min_x * n,
            origin_y: max_y * n,
            extent_width: extent[0] * n,
            extent_height: extent[1] * n,
            scale: scale / n,
            count: self.splats.splats.len(),
        }
    }
}

/**
 * A slice's splat's last two numbers (see ChunkSplats): its run, and its
 * colour with its depth in the alpha byte, as slices draw voxels whole:
 * within ±2^ATOM_SHIFT half layers.
 */
fn slice_numbers(s: &Splat) -> [f32; 2] {
    [s.run as f32, f32::from_bits(s.colour & 0x00ff_ffff | ((128 + s.depth) as u32) << 24)]
}
