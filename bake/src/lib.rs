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
use slicing::{LAYER, chunk_cell, spacing, thickness};
use splats::{Splats, chunk_splats, lit_voxel_colour, voxel_colour};
use surface::{cell_reads, cell_surface};
use world::{Geometry, Reader};

// For native frontends, which link the baker instead of loading its WebAssembly.
pub use coverage::Coverage;
pub use layer_index::LayerIndex;
pub use surface::Slice;
pub use tables::VOXEL_FACES;
pub use world::{FACE_KINDS, FACE_WORDS, Faces, Grid, Lattice, Tiles, VIEW_FACES, face_words, lattices};

/**
 * Mask pixels per unit of its grid for a slice of `level` as the grid
 * sees it (in_grid): 40 / t, t its thickness (see thickness), between 1
 * (2 pixels across a voxel) and 10. Such a slice lies at distance
 * ≥ t · detail · LAYER, where a unit is at most focal / (t · detail · LAYER)
 * screen pixels: with the default view and leak more than 40 / t, so a
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
 * The grid chunk (i, j) of a slice reads, and the sites of it it reads, a
 * box of (a, b, c) in the face's lattice coordinates, in that grid's units,
 * inclusive: a chunk is the same whatever else the grid holds.
 */
pub fn chunk_reads(slice: &Slice, i: i32, j: i32) -> (usize, [i32; 3], [i32; 3]) {
    let (level, slice) = in_grid(slice);
    let (min, max) = cell_reads(&slice, i, j);
    (level, min, max)
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
    /** Per view face, its face kinds' in the world (tables::world_kinds), for lit grids. */
    world_kinds: [[usize; FACE_KINDS]; VIEW_FACES],
    /** Which cell of 2·FCC owns each site, for cutoffs. */
    pairs: Cells,
    splats: Splats,
    mask: Vec<u8>,
    pub blocks: Vec<u8>,
    /** The splats as numbers (see ChunkSplats). */
    pub numbers: Vec<f32>,
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
            pairs: Cells::new(2),
            splats: Splats::new(),
            mask: Vec::new(),
            blocks: Vec::new(),
            numbers: Vec::new(),
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
        let (tiles, lattice) = (&self.grids[level], self.lattices[face]);
        let reader = Reader::new(tiles, lattice);
        let surface = cell_surface(&reader, &self.geometry, &self.pairs, &slice, i, j);
        let palettes = self.palettes.len() / VIEW_FACES;
        let palette = &self.palettes[face * palettes..(face + 1) * palettes];
        // Lit by the grid's light if it has any, else shaded by the way its faces face (voxel_colour).
        let colour = |voxel: &surface::Voxel| {
            let [x, y, z] = lattice.world(voxel.site);
            match tiles.light(x, y, z) {
                Some(light) => lit_voxel_colour(voxel, &light, &self.world_kinds[face], lattice.f.map(|v| -v)),
                None => voxel_colour(voxel, palette),
            }
        };
        chunk_splats(&mut self.splats, &self.geometry, &colour, &surface, &slice);

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

        // In world units, relative to the top-left corner.
        self.spacing = spacing(level as i32);
        let n = self.spacing as f64;
        self.numbers.clear();
        for s in &splats.splats {
            self.numbers.extend([
                ((s.x - min_x) * n) as f32,
                ((s.y - max_y) * n) as f32,
                // Its height is √2 times that.
                (s.width * n) as f32,
                s.run as f32,
                // Its depth in the alpha byte, as splats are opaque: within ±2^ATOM_SHIFT half layers.
                f32::from_bits(s.colour & 0x00ff_ffff | ((128 + s.depth) as u32) << 24),
            ]);
        }
        Baked {
            width,
            height,
            origin_x: min_x * n,
            origin_y: max_y * n,
            extent_width: if empty { 0.0 } else { (max_x - min_x) * n },
            extent_height: if empty { 0.0 } else { (max_y - min_y) * n },
            scale: scale / n,
            count: splats.splats.len(),
        }
    }
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
