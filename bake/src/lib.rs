//! Chunk baking, compiled to WebAssembly (in the worlds' module) for the
//! bake workers: the voxels of a chunk cell that can show, as splats
//! (hexagons of one colour) on its slice's centre plane, seen along the
//! face's axis, and a mask of what they cover.
//! A world is a grid per level of detail, or tiles of grids;
//! each slice reads the one its atoms are the voxels of (in_grid).
//! chunk-baker.ts drives it through the
//! exports at the bottom. See docs/architecture.md, section 3.
//! The same module finds what nearer layers cover on the main thread
//! (coverage.rs), where a grid's voxels are along the faces' layers
//! (layer_index.rs), and walks a body on the ground (walk.rs).

mod cells;
mod coverage;
mod fill;
mod layer_index;
pub mod light;
pub mod lod;
pub mod slicing;
mod splats;
mod surface;
mod tables;
pub mod walk;
mod world;

use std::cell::RefCell;
use std::sync::Arc;

use cells::Cells;
use fill::{fill_mask, solid_blocks};
use light::Lighting;
use lod::CellChunk;
use slicing::{LAYER, chunk_cell, slice_centre, spacing, thickness};
use splats::{Splat, Splats, cell_relief, cell_splats, chunk_splats, lit_voxel_colour, voxel_colour};
use surface::{cell_surface, surface};
use world::{Geometry, Reader};

// For native frontends, which link the baker instead of loading its WebAssembly.
pub use coverage::Coverage;
pub use layer_index::LayerIndex;
pub use surface::Slice;
pub use tables::VOXEL_FACES;
pub use world::{ALPHA, FACE_KINDS, FACE_WORDS, Faces, Grid, Lattice, Tiles, VIEW_FACES, face_words, lattices};

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
 * past its cell, where its relief's solid may run on (Baker::bake_cell).
 */
pub fn chunk_reads(chunk: &CellChunk) -> (usize, [i32; 3], [i32; 3]) {
    let (min, max) = chunk.bounds();
    (chunk.atoms as usize, [min[0] - 2, min[1] - 1, min[2] - 2], [max[0] + 2, max[1] + 1, max[2] + 2 * chunk.spacing()])
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
    /** Their sites, filled by sites(), and the spacing of the grid they were read from. */
    sites: Vec<i32>,
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
            sites: Vec::new(),
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

    fn last_grid(&mut self) -> Option<&mut Grid> {
        self.grids.get_mut(self.last).and_then(Arc::get_mut).and_then(Tiles::only_mut)
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
     * its site: the relief of its whole voxels (cell_relief), as `columns`,
     * and its see-through voxels a ray can enter (cell_splats), as splats,
     * and where they may be drawn. Unclipped: no other chunk of its level
     * draws its voxels, and depth decides between them. No mask: what is
     * covered is left to the GPU.
     * Its numbers are its see-through splats': depth past the plane in half
     * layers, colour (its alpha its voxel's), and the layer of the cell of
     * the next finer level holding each voxel, in half layers past the
     * plane: where that level draws it too, the compositor leaves it out,
     * so that none counts twice.
     */
    pub fn bake_cell(&mut self, face: usize, chunk: &CellChunk) -> Baked {
        let level = chunk.atoms as usize;
        self.reach(level);
        let m = chunk.spacing();
        // At level 0, none finer: its own.
        let finer = if chunk.level > 0 { m / 2 } else { m };
        let mut owners = std::mem::take(&mut self.owners);
        for n in [m, finer] {
            if !owners.iter().any(|(k, _)| *k == n) {
                owners.push((n, Cells::new(n)));
            }
        }
        let own = |n: i32| &owners.iter().find(|(k, _)| *k == n).expect("made").1;
        let holds = |a: i32, b: i32, c: i32| own(m).owner(a, b, c) == chunk.site;
        let tiles = Arc::clone(&self.grids[level]);
        let reader = Reader::new(&tiles, self.lattices[face]);
        let (from, to) = chunk.bounds();
        let (whole, see_through): (Vec<_>, Vec<_>) =
            surface(&reader, &self.geometry, from, to, holds).into_iter().partition(|v| v.alpha == ALPHA);
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
        self.paint(face, level, |splats, geometry, colour| cell_splats(splats, geometry, colour, &see_through, centre));
        let s = &self.splats;
        let (min_x, max_x, min_y, max_y) = (s.min_x.min(relief[0]), s.max_x.max(relief[1]), s.min_y.min(relief[2]), s.max_y.max(relief[3]));
        let extent = if min_x < max_x { [max_x - min_x, max_y - min_y] } else { [0.0; 2] };
        self.blocks.clear();
        let finer = own(finer);
        let numbers = |s: &Splat| {
            let [a, b, c] = s.site;
            [s.depth as f32, f32::from_bits(s.colour), (2 * finer.owner(a, b, c)[2] - centre) as f32]
        };
        let baked = self.numbers(level, min_x, max_y, extent, [0, 0], 1.0, numbers);
        self.owners = owners;
        baked
    }

    /**
     * A slab of grid `level` along view face `face` from layer c0: its
     * layers `layers` of a box of columns (a, b) a, b of one parity, `columns` across from
     * a0 (2 apart, the odd rows' one further on) and `rows` up from b0 (a0
     * and b0 even). Per column, row by row: the colour of its first voxel
     * in the slab (its alpha the voxel's) and where its run of voxels
     * from there ends, both layers past c0 (first | last << 16);
     * [0, u32::MAX] where the column is empty.
     */
    pub fn bake_slab(
        &mut self,
        face: usize,
        level: usize,
        [a0, b0, c0]: [i32; 3],
        [columns, rows]: [usize; 2],
        layers: std::ops::Range<i32>,
    ) -> Vec<[u32; 2]> {
        debug_assert!(a0 & 1 == 0 && b0 & 1 == 0);
        self.reach(level);
        let mut texels = vec![[0, u32::MAX]; columns * rows];
        let reader = Reader::new(&self.grids[level], self.lattices[face]);
        let to = [a0 + 2 * columns as i32, b0 + rows as i32 - 1, layers.end - 1];
        let voxels = surface(&reader, &self.geometry, [a0, b0, layers.start], to, |_, _, c| layers.contains(&c));
        let behind = self.geometry.offsets.iter().position(|&o| o == [0, 0, 2]).expect("+F");
        let lattice = self.lattices[face];
        let palettes = self.palettes.len() / VIEW_FACES;
        let palette = &self.palettes[face * palettes..(face + 1) * palettes];
        let (light, kinds) = (&mut self.light, &self.world_kinds[face]);
        // Near to far within the slab: surface reads them far to near.
        for voxel in voxels.iter().rev() {
            let [a, b, c] = voxel.site;
            let (i, j) = (((a - a0 - ((b - b0) & 1)) / 2) as usize, (b - b0) as usize);
            if i >= columns || j >= rows {
                continue;
            }
            let texel = &mut texels[j * columns + i];
            let at = (c - c0) as u32;
            if texel[1] == u32::MAX {
                let rgb = match light {
                    Some(light) => lit_voxel_colour(voxel, &light(level, lattice.world(voxel.site)), kinds, lattice.f.map(|v| -v)),
                    None => voxel_colour(voxel, palette),
                };
                // Its run ends at it until a voxel behind it says otherwise.
                *texel = [rgb & 0x00ff_ffff | u32::from(voxel.alpha) << 24, at | 0xffff << 16];
            }
            if (voxel.open | voxel.cut) & (1 << behind) != 0 && texel[1] >> 16 == 0xffff {
                texel[1] = texel[1] & 0xffff | at << 16;
            }
        }
        texels
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

// The WebAssembly interface: chunk-baker.ts writes a world's colours and
// grids through the pointers these return, then bakes chunk after chunk,
// reading each result from RESULT and the splat and block buffers it
// points to.

/** A world being handed over, then baking it. */
enum State {
    None,
    Loading { colours: Vec<u32> },
    Ready(Box<Baker>),
}

thread_local! {
    static STATE: RefCell<State> = const { RefCell::new(State::None) };
    static RESULT: RefCell<[f64; 11]> = const { RefCell::new([0.0; 11]) };
}

/** Starts a world of `materials` materials; returns where to write their colours (u32, 0xrrggbb by id), then call world_ready. */
#[unsafe(no_mangle)]
pub extern "C" fn world_start(materials: usize) -> *mut u32 {
    STATE.with_borrow_mut(|state| {
        let mut colours = vec![0; materials];
        let at = colours.as_mut_ptr();
        *state = State::Loading { colours };
        at
    })
}

/**
 * Sets the ready world's grid of `level`, of the box given and `used`
 * bricks of sites; returns where to write its bricks (i32), then its sites
 * at world_cells(). Chunks of a level are baked only once it has a grid.
 */
#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub extern "C" fn world_grid(level: usize, min_x: i32, min_y: i32, min_z: i32, nx: i32, ny: i32, nz: i32, used: usize) -> *mut u8 {
    STATE.with_borrow_mut(|state| match state {
        State::Ready(baker) => baker.set_grid(level, Grid::new([min_x, min_y, min_z], [nx, ny, nz], used)).bricks.as_mut_ptr() as *mut u8,
        _ => std::ptr::null_mut(),
    })
}

/** Where to write the sites of the grid world_grid set last. */
#[unsafe(no_mangle)]
pub extern "C" fn world_cells() -> *mut u8 {
    STATE.with_borrow_mut(|state| match state {
        State::Ready(baker) => baker.last_grid().map_or(std::ptr::null_mut(), |g| g.cells.as_mut_ptr()),
        _ => std::ptr::null_mut(),
    })
}

/**
 * Where to write the faces (Grid::faces) of the grid world_grid set
 * last: FACE_WORDS words (u32) for each of `sites` sites, by index.
 */
#[unsafe(no_mangle)]
pub extern "C" fn world_faces(sites: usize) -> *mut u32 {
    STATE.with_borrow_mut(|state| match state {
        State::Ready(baker) => baker.last_grid().map_or(std::ptr::null_mut(), |g| {
            g.faces = vec![0; sites * FACE_WORDS];
            g.faces.as_mut_ptr()
        }),
        _ => std::ptr::null_mut(),
    })
}

/** Finishes loading. */
#[unsafe(no_mangle)]
pub extern "C" fn world_ready() {
    STATE.with_borrow_mut(|state| {
        if let State::Loading { colours } = std::mem::replace(state, State::None) {
            *state = State::Ready(Box::new(Baker::new(&colours)));
        }
    })
}

/**
 * Bakes a chunk (see Baker::bake); returns RESULT: mask width, height,
 * origin x, origin y, extent width, height, mask scale, splats, splats
 * pointer (f32, 5 each), blocks pointer, blocks length.
 */
#[unsafe(no_mangle)]
pub extern "C" fn bake(face: u32, level: i32, atoms: i32, layer: i32, cutoff: f64, i: i32, j: i32) -> *const f64 {
    STATE.with_borrow_mut(|state| {
        let State::Ready(baker) = state else { return std::ptr::null() };
        let baked = baker.bake(face as usize, &Slice { level, atoms, layer, cutoff }, i, j);
        RESULT.with_borrow_mut(|result| {
            *result = [
                baked.width as f64,
                baked.height as f64,
                baked.origin_x,
                baked.origin_y,
                baked.extent_width,
                baked.extent_height,
                baked.scale,
                baked.count as f64,
                baker.numbers.as_ptr() as usize as f64,
                baker.blocks.as_ptr() as usize as f64,
                baker.blocks.len() as f64,
            ];
            result.as_ptr()
        })
    })
}

/**
 * The sites of the last baked chunk's splats, for
 * tests: a, b, c (i32) each, in world units.
 */
#[unsafe(no_mangle)]
pub extern "C" fn sites() -> *const i32 {
    STATE.with_borrow_mut(|state| match state {
        State::Ready(baker) => {
            let Baker { splats, sites, spacing, .. } = &mut **baker;
            sites.clear();
            sites.extend(splats.splats.iter().flat_map(|s| s.site.map(|v| v * *spacing)));
            sites.as_ptr()
        }
        _ => std::ptr::null(),
    })
}
