//! Chunk baking, compiled to WebAssembly for the bake workers: the voxel
//! faces of a chunk cell, projected from an eye onto its slice's centre
//! plane and filled into pixels. A world is a grid per level of detail;
//! each slice reads the one its atoms are the voxels of (in_grid).
//! chunk-baker.ts drives it through the
//! exports at the bottom. See docs/architecture.md, section 3.
//! The same module finds what nearer layers cover on the main thread
//! (coverage.rs).

mod cells;
mod coverage;
mod faces;
mod fill;
mod slicing;
mod surface;
mod world;

use std::cell::RefCell;
use std::collections::HashMap;

use cells::Cells;
use faces::{Faces, chunk_faces};
use fill::{Scratch, fill_faces, solid_blocks};
use slicing::{LAYER, atom_level, chunk_cell, spacing, thickness};
use surface::{Slice, Voxel, cell_surface};
use world::{FACE_KINDS, GEOMETRY, Geometry, Grid, LATTICE, Lattice, Reader, VIEW_FACES};

/**
 * Pixels per unit used when rasterising the nearest slices: 20 pixels
 * across a voxel, 2 units wide, so that rhombi magnified near the camera
 * keep their slanted edges. Chunks are rasterised camera-independently so
 * they can be cached and reused as the camera moves; perspective scaling
 * is applied when compositing.
 */
const RENDER_SCALE: f64 = 10.0;

/**
 * Chunk pixels per unit of its grid for a slice of `level` as the grid
 * sees it (in_grid): 40 / t, t its thickness
 * (see thickness), between 1 (2 pixels across a voxel) and RENDER_SCALE.
 * Such a slice lies at distance ≥ t · detail · LAYER, where a unit is at
 * most focal / (t · detail · LAYER) screen pixels: with the default view
 * and leak more than 40 / t, so chunks are magnified (docs/todo.md).
 * Other settings just scale the chunks on screen.
 */
fn render_scale(level: i32) -> f64 {
    (40.0 / thickness(level) as f64).floor().clamp(1.0, RENDER_SCALE)
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

/** Cell surfaces kept for baking the same cell from other eyes. */
const MAX_SURFACES: usize = 1024;

/** A baked chunk: see ChunkImage. Pixels and blocks stay in the baker until the next bake. */
pub struct Image {
    pub width: usize,
    pub height: usize,
    pub origin_x: f64,
    pub origin_y: f64,
    pub scale: f64,
    pub faces_drawn: usize,
}

/** Face, grid level, then the slice as the grid sees it: level, layer, cutoff (as bits), i, j. */
type SurfaceKey = (usize, usize, i32, i32, u64, i32, i32);

/** One world's grids, palettes and lattices, and the buffers reused across bakes. */
pub struct Baker {
    /** By level: grid ℓ has the sites of 2^ℓ·FCC, in its own units. They share materials. */
    grids: Vec<Grid>,
    geometry: Geometry,
    lattices: [Lattice; VIEW_FACES],
    /** Per view face: material · FACE_KINDS + kind → RGBA. */
    palettes: Vec<u32>,
    /** Which cell of 2·FCC owns each site, for cutoffs. */
    pairs: Cells,
    /** Cell surfaces, with the bake count they were last used at. */
    surfaces: HashMap<SurfaceKey, (Vec<Voxel>, u64)>,
    uses: u64,
    faces: Faces,
    scratch: Scratch,
    pub pixels: Vec<u32>,
    pub blocks: Vec<u8>,
    /** The faces as numbers, for faces(). */
    numbers: Vec<f64>,
}

impl Baker {
    pub fn new(grids: Vec<Grid>, geometry: Geometry, lattices: [Lattice; VIEW_FACES], palettes: Vec<u32>) -> Baker {
        Baker {
            grids,
            geometry,
            lattices,
            palettes,
            pairs: Cells::new(2),
            surfaces: HashMap::new(),
            uses: 0,
            faces: Faces::new(),
            scratch: Scratch::default(),
            pixels: Vec::new(),
            blocks: Vec::new(),
            numbers: Vec::new(),
        }
    }

    /**
     * Fills self.faces with the faces of a chunk (see chunk_faces) of a
     * slice as grid `level` sees it (in_grid), from its cell's surface,
     * cached.
     */
    #[allow(clippy::too_many_arguments)]
    fn chunk_faces(&mut self, face: usize, level: usize, slice: &Slice, i: i32, j: i32, eye: [i32; 3], fill: bool) {
        self.uses += 1;
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
        chunk_faces(&mut self.faces, &self.geometry, &surface.0, slice, eye, fill);
    }

    /**
     * Bakes chunk (i, j) of a slice of view face `face` from `eye` (a site,
     * in the face's lattice coordinates), its faces grown to fill gaps if
     * `fill`, clipped to the cell and cropped to what is drawn. See
     * ChunkBaker.bake.
     */
    pub fn bake(&mut self, face: usize, slice: &Slice, i: i32, j: i32, eye: [i32; 3], fill: bool) -> Image {
        let (level, slice, eye) = in_grid(slice, eye);
        self.chunk_faces(face, level, &slice, i, j, eye, fill);

        // Drawn extent, clipped to the cell, in the grid's units.
        let cell = chunk_cell(slice.level, i, j);
        let (left, bottom) = (cell.left as f64 * LAYER, cell.bottom as f64);
        let min_x = self.faces.min_x.max(left);
        let max_x = self.faces.max_x.min(left + cell.width);
        let min_y = self.faces.min_y.max(bottom);
        let max_y = self.faces.max_y.min(bottom + cell.height);

        let scale = render_scale(slice.level);
        let empty = max_x <= min_x || max_y <= min_y;
        let width = if empty { 0 } else { ((max_x - min_x) * scale).ceil() as usize };
        let height = if empty { 0 } else { ((max_y - min_y) * scale).ceil() as usize };
        self.pixels.clear();
        self.pixels.resize(width * height, 0);
        let palettes = self.palettes.len() / VIEW_FACES;
        let palette = &self.palettes[face * palettes..(face + 1) * palettes];
        let faces_drawn = fill_faces(&mut self.scratch, &self.faces, palette, &mut self.pixels, width, height, min_x, max_y, scale);
        solid_blocks(&mut self.blocks, &self.pixels, width, height);
        // In world units.
        let n = spacing(level as i32) as f64;
        Image { width, height, origin_x: min_x * n, origin_y: max_y * n, scale: scale / n, faces_drawn }
    }
}

// The WebAssembly interface: chunk-baker.ts fills a world's tables through
// the pointers these return, then bakes chunk after chunk, reading each
// result from RESULT and the pixel and block buffers it points to.

/** A world being handed over, then baking it. */
enum State {
    None,
    Loading { grids: Vec<Grid>, geometry: Vec<f64>, lattices: Vec<i32>, palettes: Vec<u32> },
    Ready(Box<Baker>),
}

thread_local! {
    static STATE: RefCell<State> = const { RefCell::new(State::None) };
    static RESULT: RefCell<[f64; 9]> = const { RefCell::new([0.0; 9]) };
}

/** Starts a world of `materials` materials. */
#[unsafe(no_mangle)]
pub extern "C" fn world_start(materials: usize) {
    STATE.with_borrow_mut(|state| {
        *state = State::Loading {
            grids: Vec::new(),
            geometry: vec![0.0; FACE_KINDS * GEOMETRY],
            lattices: vec![0; VIEW_FACES * LATTICE],
            palettes: vec![0; VIEW_FACES * materials * FACE_KINDS],
        };
    })
}

/** Adds the loading world's grid of the next level, of the box given; returns where to write its sites. */
#[unsafe(no_mangle)]
pub extern "C" fn world_grid(min_x: i32, min_y: i32, min_z: i32, nx: i32, ny: i32, nz: i32) -> *mut u8 {
    STATE.with_borrow_mut(|state| match state {
        State::Loading { grids, .. } => {
            let cells = (nx / 2 * ny * nz).max(0) as usize;
            grids.push(Grid { min: [min_x, min_y, min_z], n: [nx, ny, nz], cells: vec![0; cells] });
            grids.last_mut().expect("just pushed").cells.as_mut_ptr()
        }
        _ => std::ptr::null_mut(),
    })
}

/** Where to write the loading world's tables: 0 geometry (f64), 1 lattices (i32), 2 palettes (u32). */
#[unsafe(no_mangle)]
pub extern "C" fn world_table(table: u32) -> *mut u8 {
    STATE.with_borrow_mut(|state| match state {
        State::Loading { geometry, lattices, palettes, .. } => match table {
            0 => geometry.as_mut_ptr() as *mut u8,
            1 => lattices.as_mut_ptr() as *mut u8,
            _ => palettes.as_mut_ptr() as *mut u8,
        },
        _ => std::ptr::null_mut(),
    })
}

/** Finishes loading. */
#[unsafe(no_mangle)]
pub extern "C" fn world_ready() {
    STATE.with_borrow_mut(|state| {
        if let State::Loading { grids, geometry, lattices, palettes } = std::mem::replace(state, State::None) {
            let lattices = std::array::from_fn(|f| {
                let l = &lattices[f * LATTICE..(f + 1) * LATTICE];
                Lattice { r: [l[0], l[1], l[2]], u: [l[3], l[4], l[5]], f: [l[6], l[7], l[8]] }
            });
            *state = State::Ready(Box::new(Baker::new(grids, Geometry::from_table(&geometry), lattices, palettes)));
        }
    })
}

/**
 * Bakes a chunk (see Baker::bake); returns RESULT: width, height, origin x,
 * origin y, scale, faces drawn, pixels pointer, blocks pointer, blocks
 * length.
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
        let image = baker.bake(face as usize, &Slice { level, layer, cutoff }, i, j, [eye_a, eye_b, eye_c], fill != 0);
        RESULT.with_borrow_mut(|result| {
            *result = [
                image.width as f64,
                image.height as f64,
                image.origin_x,
                image.origin_y,
                image.scale,
                image.faces_drawn as f64,
                baker.pixels.as_ptr() as usize as f64,
                baker.blocks.as_ptr() as usize as f64,
                baker.blocks.len() as f64,
            ];
            result.as_ptr()
        })
    })
}

/**
 * The faces of a chunk before filling, for tests: RESULT holds their count
 * and where they are, 17 f64 each: corners (8), colour, 1 if reached,
 * growth (4) and site (3) (see Face), in world units.
 */
#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub extern "C" fn faces(
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
        let (level, slice, eye) = in_grid(&Slice { level, layer, cutoff }, [eye_a, eye_b, eye_c]);
        baker.chunk_faces(face as usize, level, &slice, i, j, eye, fill != 0);
        let n = spacing(level as i32);
        baker.numbers.clear();
        for f in &baker.faces.list {
            baker.numbers.extend(f.corners.map(|v| v * n as f64));
            baker.numbers.extend([f.colour as f64, if f.reached { 1.0 } else { 0.0 }]);
            baker.numbers.extend(f.growth.map(|v| v * n as f64));
            baker.numbers.extend(f.site.map(|v| f64::from(v * n)));
        }
        RESULT.with_borrow_mut(|result| {
            result[0] = baker.faces.list.len() as f64;
            result[1] = baker.numbers.as_ptr() as usize as f64;
            result.as_ptr()
        })
    })
}
