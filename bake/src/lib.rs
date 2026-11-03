//! Chunk baking, compiled to WebAssembly (in worldgen's module) for the
//! bake workers: the voxels of a chunk cell that can show, as splats
//! (hexagons of one colour) projected from an eye onto its slice's centre
//! plane, and a mask of what they cover.
//! A world is a grid per level of detail, or tiles of grids;
//! each slice reads the one its atoms are the voxels of (in_grid).
//! chunk-baker.ts drives it through the
//! exports at the bottom. See docs/architecture.md, section 3.
//! The same module finds what nearer layers cover on the main thread
//! (coverage.rs).

mod cells;
mod coverage;
mod fill;
mod slicing;
mod splats;
mod surface;
mod tables;
mod world;

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;

use cells::Cells;
use fill::{fill_mask, solid_blocks};
use slicing::{LAYER, atom_level, chunk_cell, spacing, thickness};
use splats::{Splats, chunk_splats};
use surface::{Voxel, cell_reads, cell_surface};
use world::{Geometry, LATTICE, Reader};

// For native frontends, which link the baker instead of loading its WebAssembly.
pub use coverage::Coverage;
pub use surface::Slice;
pub use world::{Grid, Lattice, Tiles, VIEW_FACES};

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
    (40.0 / thickness(level) as f64).floor().clamp(1.0, 10.0)
}

/**
 * A slice (level, layer, cutoff) of the world and its eye as the grid the
 * slice reads sees them: grid ℓ = atom_level(level) has the sites of N·FCC,
 * N = 2^ℓ, as its own units, so the slice is slab `layer` of
 * 2^(level − ℓ) of its layers, cut off at cutoff / N: a slice of single
 * voxels. The eye, a site of 2^level·FCC, is one of the grid's.
 * Returns ℓ, that slice and that eye.
 */
fn in_grid(slice: &Slice, eye: [i32; 3]) -> (usize, Slice, [i32; 3]) {
    let l = atom_level(slice.level);
    let n = spacing(l);
    (l as usize, Slice { level: slice.level - l, layer: slice.layer, cutoff: slice.cutoff / n as f64 }, eye.map(|v| v / n))
}

/**
 * The grid chunk (i, j) of a slice reads, and the sites of it it reads, a
 * box of (a, b, c) in the face's lattice coordinates, in that grid's units,
 * inclusive: a chunk is the same whatever else the grid holds.
 */
pub fn chunk_reads(slice: &Slice, i: i32, j: i32) -> (usize, [i32; 3], [i32; 3]) {
    let (level, slice, _) = in_grid(slice, [0; 3]);
    let (min, max) = cell_reads(&slice, i, j);
    (level, min, max)
}

/** Cell surfaces kept for baking the same cell from other eyes. */
const MAX_SURFACES: usize = 1024;

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
    pub painted: usize,
    pub grown: usize,
}

/** Face, grid level, then the slice as the grid sees it: level, layer, cutoff (as bits), i, j. */
type SurfaceKey = (usize, usize, i32, i32, u64, i32, i32);

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
    /** Which cell of 2·FCC owns each site, for cutoffs. */
    pairs: Cells,
    /** Cell surfaces, with the bake count they were last used at. */
    surfaces: HashMap<SurfaceKey, (Vec<Voxel>, u64)>,
    uses: u64,
    splats: Splats,
    mask: Vec<bool>,
    pub blocks: Vec<u8>,
    /** The splats as numbers (see ChunkSplats), painted then grown. */
    pub numbers: Vec<f32>,
    /** Their sites, filled by sites(), and the spacing of the grid they were read from. */
    sites: Vec<i32>,
    spacing: i32,
}

impl Baker {
    /** A baker for the view's faces' lattices (FACES in view.ts) and the world's materials' colours, 0xrrggbb by id. */
    pub fn new(lattices: [Lattice; VIEW_FACES], colours: &[u32]) -> Baker {
        Baker {
            grids: Vec::new(),
            last: 0,
            geometry: tables::geometry(&lattices[0]),
            lattices,
            palettes: tables::palettes(&lattices, colours),
            pairs: Cells::new(2),
            surfaces: HashMap::new(),
            uses: 0,
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
        self.surfaces.retain(|key, _| key.1 != level);
        self.last = level;
        self.grids[level] = Arc::new(Tiles::one(grid));
        Arc::get_mut(&mut self.grids[level]).and_then(Tiles::only_mut).expect("just set")
    }

    /**
     * Sets the tiles of `level` to ones other bakers may share. Tiles may
     * come and go but not change, so the surfaces kept stay right as long
     * as every chunk baked has all the tiles it reads (chunk_reads).
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
     * Fills self.splats with the splats of a chunk (see chunk_splats) of a
     * slice as grid `level` sees it (in_grid), from its cell's surface,
     * cached.
     */
    #[allow(clippy::too_many_arguments)]
    fn chunk_splats(&mut self, face: usize, level: usize, slice: &Slice, i: i32, j: i32, eye: [i32; 3], fill: bool) {
        self.uses += 1;
        self.reach(level);
        let key = (face, level, slice.level, slice.layer, slice.cutoff.to_bits(), i, j);
        if !self.surfaces.contains_key(&key) {
            if self.surfaces.len() >= MAX_SURFACES {
                let oldest = self.surfaces.iter().min_by_key(|(_, (_, used))| *used).map(|(k, _)| *k);
                if let Some(oldest) = oldest {
                    self.surfaces.remove(&oldest);
                }
            }
            let reader = Reader::new(&self.grids[level], self.lattices[face]);
            self.surfaces.insert(key, (cell_surface(&reader, &self.geometry, &self.pairs, slice, i, j), 0));
        }
        let surface = self.surfaces.get_mut(&key).expect("just inserted");
        surface.1 = self.uses;
        let palettes = self.palettes.len() / VIEW_FACES;
        let palette = &self.palettes[face * palettes..(face + 1) * palettes];
        chunk_splats(&mut self.splats, &self.geometry, palette, &surface.0, slice, eye, fill);
    }

    /**
     * Bakes chunk (i, j) of a slice of view face `face` from `eye` (a site,
     * in the face's lattice coordinates), grown to fill gaps if `fill`:
     * its splats that reach into the cell, the extent of what they draw
     * clipped to the cell, and a mask of it. See ChunkBaker.bake.
     */
    pub fn bake(&mut self, face: usize, slice: &Slice, i: i32, j: i32, eye: [i32; 3], fill: bool) -> Baked {
        let (level, slice, eye) = in_grid(slice, eye);
        self.chunk_splats(face, level, &slice, i, j, eye, fill);

        // Drawn extent, clipped to the cell, in the grid's units.
        let cell = chunk_cell(slice.level, i, j);
        let (left, bottom) = (cell.left as f64 * LAYER, cell.bottom as f64);
        let splats = &mut self.splats;
        let min_x = splats.min_x.max(left);
        let max_x = splats.max_x.min(left + cell.width);
        let min_y = splats.min_y.max(bottom);
        let max_y = splats.max_y.min(bottom + cell.height);
        let empty = max_x <= min_x || max_y <= min_y;
        // Splats that miss the cell draw nothing.
        let reaches = |s: &splats::Splat| {
            !empty && s.x + s.width > min_x && s.x - s.width < max_x && s.y + s.height > min_y && s.y - s.height < max_y
        };
        splats.painted.retain(reaches);
        splats.grown.retain(reaches);

        let scale = mask_scale(slice.level);
        let width = if empty { 0 } else { ((max_x - min_x) * scale).ceil() as usize };
        let height = if empty { 0 } else { ((max_y - min_y) * scale).ceil() as usize };
        self.mask.clear();
        self.mask.resize(width * height, false);
        fill_mask(splats, &mut self.mask, width, height, min_x, max_y, scale);
        solid_blocks(&mut self.blocks, &self.mask, width, height);

        // In world units, relative to the top-left corner.
        self.spacing = spacing(level as i32);
        let n = self.spacing as f64;
        self.numbers.clear();
        for s in splats.painted.iter().chain(&splats.grown) {
            self.numbers.extend([
                ((s.x - min_x) * n) as f32,
                ((s.y - max_y) * n) as f32,
                (s.width * n) as f32,
                (s.height * n) as f32,
                f32::from_bits(s.colour),
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
            painted: splats.painted.len(),
            grown: splats.grown.len(),
        }
    }
}

// The WebAssembly interface: chunk-baker.ts fills a world's tables through
// the pointers these return, then bakes chunk after chunk, reading each
// result from RESULT and the splat and block buffers it points to.

/** A world being handed over, then baking it. */
enum State {
    None,
    Loading { lattices: Vec<i32>, colours: Vec<u32> },
    Ready(Box<Baker>),
}

thread_local! {
    static STATE: RefCell<State> = const { RefCell::new(State::None) };
    static RESULT: RefCell<[f64; 12]> = const { RefCell::new([0.0; 12]) };
}

/** Starts a world of `materials` materials. */
#[unsafe(no_mangle)]
pub extern "C" fn world_start(materials: usize) {
    STATE.with_borrow_mut(|state| {
        *state = State::Loading { lattices: vec![0; VIEW_FACES * LATTICE], colours: vec![0; materials] };
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

/** Where to write the loading world's tables: 0 the faces' lattices (i32, see Baker::new), 1 the materials' colours (u32). */
#[unsafe(no_mangle)]
pub extern "C" fn world_table(table: u32) -> *mut u8 {
    STATE.with_borrow_mut(|state| match state {
        State::Loading { lattices, colours } => match table {
            0 => lattices.as_mut_ptr() as *mut u8,
            _ => colours.as_mut_ptr() as *mut u8,
        },
        _ => std::ptr::null_mut(),
    })
}

/** Finishes loading. */
#[unsafe(no_mangle)]
pub extern "C" fn world_ready() {
    STATE.with_borrow_mut(|state| {
        if let State::Loading { lattices, colours } = std::mem::replace(state, State::None) {
            let lattices = std::array::from_fn(|f| {
                let l = &lattices[f * LATTICE..(f + 1) * LATTICE];
                Lattice { r: [l[0], l[1], l[2]], u: [l[3], l[4], l[5]], f: [l[6], l[7], l[8]] }
            });
            *state = State::Ready(Box::new(Baker::new(lattices, &colours)));
        }
    })
}

/**
 * Bakes a chunk (see Baker::bake); returns RESULT: mask width, height,
 * origin x, origin y, extent width, height, mask scale, painted and grown
 * splats, splats pointer (f32, 5 each), blocks pointer, blocks length.
 */
#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub extern "C" fn bake(
    face: u32,
    level: i32,
    layer: i32,
    cutoff: f64,
    i: i32,
    j: i32,
    eye_a: i32,
    eye_b: i32,
    eye_c: i32,
    fill: u32,
) -> *const f64 {
    STATE.with_borrow_mut(|state| {
        let State::Ready(baker) = state else { return std::ptr::null() };
        let baked = baker.bake(face as usize, &Slice { level, layer, cutoff }, i, j, [eye_a, eye_b, eye_c], fill != 0);
        RESULT.with_borrow_mut(|result| {
            *result = [
                baked.width as f64,
                baked.height as f64,
                baked.origin_x,
                baked.origin_y,
                baked.extent_width,
                baked.extent_height,
                baked.scale,
                baked.painted as f64,
                baked.grown as f64,
                baker.numbers.as_ptr() as usize as f64,
                baker.blocks.as_ptr() as usize as f64,
                baker.blocks.len() as f64,
            ];
            result.as_ptr()
        })
    })
}

/**
 * The sites of the last baked chunk's splats, painted then grown, for
 * tests: a, b, c (i32) each, in world units.
 */
#[unsafe(no_mangle)]
pub extern "C" fn sites() -> *const i32 {
    STATE.with_borrow_mut(|state| match state {
        State::Ready(baker) => {
            let Baker { splats, sites, spacing, .. } = &mut **baker;
            sites.clear();
            sites.extend(splats.painted.iter().chain(&splats.grown).flat_map(|s| s.site.map(|v| v * *spacing)));
            sites.as_ptr()
        }
        _ => std::ptr::null(),
    })
}
