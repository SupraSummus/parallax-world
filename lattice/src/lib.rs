//! The cubic lattice and its voxels, cubes, as every crate sees them:
//! the voxel's faces, the levels of detail a world is held at, an octree
//! (a voxel of level ℓ + 1 is eight of level ℓ), the grids of sites
//! generators make and hosts read, tiles of grids, the light from voxels'
//! faces (light.rs), rays across them (ray.rs), a body walking on them
//! (walk.rs); and maps keyed by sites and tiles.
//! The crate keeps its name from the face-centred cubic lattice the worlds
//! were on (docs/cubes.md). See docs/architecture.md.

pub mod light;
pub mod ray;
pub mod walk;

use std::collections::{HashMap, HashSet};
use std::hash::{BuildHasherDefault, Hasher};
use std::sync::Arc;

/**
 * A hasher for keys of a few integers, sites', cells' and tiles', as
 * FxHash: a rotate, a xor and a multiply per word; SipHash, the standard
 * one, took much of the light's time, and of frames' with many cells.
 */
#[derive(Default)]
pub struct Fast(u64);

impl Hasher for Fast {
    fn finish(&self) -> u64 {
        self.0
    }
    fn write(&mut self, bytes: &[u8]) {
        let (words, rest) = bytes.as_chunks::<8>();
        for word in words {
            self.write_u64(u64::from_le_bytes(*word));
        }
        if !rest.is_empty() {
            let mut word = [0; 8];
            word[..rest.len()].copy_from_slice(rest);
            self.write_u64(u64::from_le_bytes(word));
        }
    }
    // An array's length, hashed before it, and a key's small integers: as `write` would, without its bytes' loop.
    fn write_usize(&mut self, v: usize) {
        self.write_u64(v as u64);
    }
    fn write_u32(&mut self, v: u32) {
        self.write_u64(v.into());
    }
    fn write_u8(&mut self, v: u8) {
        self.write_u64(v.into());
    }
    fn write_u64(&mut self, v: u64) {
        self.0 = (self.0.rotate_left(5) ^ v).wrapping_mul(0x51_7c_c1_b7_27_22_0a_95);
    }
}

pub type FastMap<K, V> = HashMap<K, V, BuildHasherDefault<Fast>>;
pub type FastSet<K> = HashSet<K, BuildHasherDefault<Fast>>;

/**
 * The voxel's 6 faces' normals in the world, the offsets of the
 * neighbours across them; a face's index is its kind: −x, +x, −y, +y, −z,
 * +z, the axis k / 2.
 */
pub const VOXEL_FACES: [[i32; 3]; FACE_KINDS] = [[-1, 0, 0], [1, 0, 0], [0, -1, 0], [0, 1, 0], [0, 0, -1], [0, 0, 1]];

/** Kinds of voxel faces. */
pub const FACE_KINDS: usize = 6;

/** The kind of the face opposite face `k`: VOXEL_FACES are so ordered. */
pub const fn opposite(k: usize) -> usize {
    k ^ 1
}

/**
 * Across which of its faces a site's matter holds on, a bit per kind of
 * VOXEL_FACES: two sites are joined across a face where both hold on
 * across it, so what only touches is not (docs/mechanics.md).
 */
pub type Holds = u8;

/** Holding on across every face. */
pub const HOLDS_ALL: Holds = (1 << FACE_KINDS) - 1;

/**
 * The faces (kinds of VOXEL_FACES) a ray along `direction` sees a voxel
 * by: those turned most against it, alike (a ray straight down sees
 * three). What a site looks like along a direction is the mean of how it
 * looks through them, wherever the ray enters it: one rule for every
 * host and check (voxels.wgsl's `seen`, worldgen's pictures).
 */
pub fn facing(direction: [f64; 3]) -> impl Iterator<Item = usize> {
    let against = move |k: usize| (0..3).map(|i| VOXEL_FACES[k][i] as f64 * direction[i]).sum::<f64>();
    let least = (0..FACE_KINDS).map(against).fold(f64::INFINITY, f64::min);
    (0..FACE_KINDS).filter(move |&k| against(k) <= least + 1e-9)
}

/** Levels of a world's grids: an octree, a site of level ℓ + 1 the cube of eight of level ℓ. */
pub const GRID_LEVELS: usize = 9;

/** Bricks of the sites of a box (see VoxelGrid in worldgen's grid.rs). */
pub const BRICK: i32 = 8;
/** Sites a brick holds: its BRICK³ points. */
pub const BRICK_SITES: usize = (BRICK * BRICK * BRICK) as usize;

/** A whole voxel's alpha: it stops every ray through its site. */
pub const ALPHA: u8 = 255;

/**
 * What a voxel looks like, in a word: its colour, sRGB, a byte a channel
 * from red in the lowest, and in the highest its alpha, how much of the
 * rays through its site it stops. 0 is none: what is there stops some.
 * Under ALPHA, it is of pieces held in part (`in_part`) if red is odd,
 * which stop each ray whole or not at all, as often as the alpha says;
 * else of a medium, which stops some of every ray: of the channel it
 * stops least (Grid's `stops`).
 */
pub type Look = u32;

/** A look of colour `rgb` (sRGB) and `alpha`. */
pub const fn look([r, g, b]: [u8; 3], alpha: u8) -> Look {
    u32::from_le_bytes([r, g, b, alpha])
}

/** The alpha of a look. */
#[inline]
pub const fn alpha_of(look: Look) -> u8 {
    (look >> 24) as u8
}

/** A look of colour `rgb` (sRGB) stopping `alpha` of the rays, under ALPHA: of pieces held in part, or of a medium (Look). */
pub const fn partial([r, g, b]: [u8; 3], alpha: u8, pieces: bool) -> Look {
    let alpha = if alpha == ALPHA { ALPHA - 1 } else { alpha };
    look([if pieces { r | 1 } else { r & !1 }, g, b], alpha)
}

/** Whether a look is of pieces held in part (Look): under ALPHA, its red odd. */
#[inline]
pub const fn in_part(look: Look) -> bool {
    look & 1 == 1 && alpha_of(look) != 0 && alpha_of(look) != ALPHA
}

/** Whether a look is of a medium (Look): under ALPHA, its red even. */
#[inline]
pub const fn medium(look: Look) -> bool {
    look & 1 == 0 && alpha_of(look) != 0 && alpha_of(look) != ALPHA
}

/**
 * Whether pieces held in part stopping `alpha` of rays (of ALPHA) at site
 * (x, y, z) show whole: where their alpha is over a coin of the site, so as
 * often as they stop rays. Every host shows them so (voxels.wgsl's copy too),
 * none by a finer coin: far off, a voxel is a few pixels.
 */
pub fn shows_whole(alpha: u8, x: i32, y: i32, z: i32) -> bool {
    let h = (x as u32).wrapping_mul(0x9e37_79b1) ^ (y as u32).wrapping_mul(0x85eb_ca77) ^ (z as u32).wrapping_mul(0xc2b2_ae3d);
    let h = (h ^ h >> 15).wrapping_mul(0x2c1b_3c6d);
    alpha > ((h ^ h >> 12) >> 24) as u8
}

/** What a host shows of `look` at site (x, y, z): pieces held in part whole or nothing (`shows_whole`). */
pub fn whole_or_none(look: Look, x: i32, y: i32, z: i32) -> Look {
    if !in_part(look) {
        look
    } else if shows_whole(alpha_of(look), x, y, z) {
        look | 0xff00_0000
    } else {
        0
    }
}

/** Words per site in a grid's `views`: its index in `cells`, then its look along each face kind (VOXEL_FACES). */
pub const VIEW_WORDS: usize = 1 + FACE_KINDS;
/** Words per site in a grid's `glow`: its index in `cells`, then what it glows per channel. */
pub const GLOW_WORDS: usize = 4;
/** Words per site in a grid's `stops`: its index in `cells`, then what it stops of each channel, a byte each as a look's colour. */
pub const STOPS_WORDS: usize = 2;

/**
 * The sites of a box in bricks, BRICK units wide (see VoxelGrid).
 * A voxel looks as its site's in `cells` says, unless seen through a face
 * `views` gives a look of its own: what is seen of it looking at that face.
 * Walking is stopped by those `solid` marks.
 */
#[derive(Clone)]
pub struct Grid {
    pub min: [i32; 3],
    pub n: [i32; 3],
    /** Per brick, z-major, then y, then x: where its sites start in `cells`, in BRICK_SITES; −1 if empty. Bricks may share sites (worldgen's VoxelGrid::bury). */
    pub bricks: Vec<i32>,
    /** Each brick's sites, z-major, then y, then x: each voxel's look, 0 if there is none. */
    pub cells: Vec<Look>,
    /** The sites that do not look alike through every face, by index in `cells`: VIEW_WORDS each. */
    pub views: Vec<u32>,
    /** The sites that glow, shown or not, by index in `cells`: GLOW_WORDS each, for the light (light/). */
    pub glow: Vec<u32>,
    /**
     * The see-through sites that do not stop every channel alike, by index
     * in `cells`: STOPS_WORDS each. Of a ray crossing one it stops that
     * much of each channel and shows its colour of it; its look's alpha is
     * of the channel it stops least, to what takes one for all.
     */
    pub stops: Vec<u32>,
    /** Bit i % 32 of word i / 32: whether the voxel at index i of `cells` stops a walking body (lattice's walk.rs). */
    pub solid: Vec<u32>,
}

/** Of the sites in `words`, `per` words a site, sorted by index: where the first of index `i` or more is (in sites). */
fn first_from(words: &[u32], per: usize, i: usize) -> usize {
    let (mut lo, mut hi) = (0, words.len() / per);
    while lo < hi {
        let mid = (lo + hi) / 2;
        if (words[mid * per] as usize) < i {
            lo = mid + 1;
        } else {
            hi = mid;
        }
    }
    lo
}

/** Whether `words`, `per` words a site, sorted by index, has any site at `sites`. */
fn has(words: &[u32], per: usize, sites: std::ops::Range<usize>) -> bool {
    words.get(first_from(words, per, sites.start) * per).is_some_and(|&i| sites.contains(&(i as usize)))
}

/** The words after site index `i` in `words`, `per` words a site, sorted by index; None if it has none. */
fn words_of(words: &[u32], per: usize, i: usize) -> Option<&[u32]> {
    let at = first_from(words, per, i) * per;
    words.get(at..at + per).filter(|w| w[0] as usize == i).map(|w| &w[1..])
}

impl Grid {
    /** A grid of the box with `used` bricks of sites; `bricks` and `cells` to be written. */
    pub fn new(min: [i32; 3], n: [i32; 3], used: usize) -> Grid {
        let count = n.map(|v| (v.max(0) + BRICK - 1) / BRICK);
        Grid {
            min,
            n,
            bricks: vec![-1; (count[0] * count[1] * count[2]) as usize],
            cells: vec![0; used * BRICK_SITES],
            views: Vec::new(),
            glow: Vec::new(),
            stops: Vec::new(),
            solid: vec![0; used * BRICK_SITES / 32],
        }
    }

    /**
     * Its part in the box of `n` sites from `min`, a box of its bricks:
     * their sites, shared as they are here. What a grid made as one box
     * holds of each of the tiles it spans.
     */
    pub fn part(&self, min: [i32; 3], n: [i32; 3]) -> Grid {
        debug_assert!((0..3).all(|a| (min[a] - self.min[a]) % BRICK == 0), "a part is of whole bricks");
        let mut part = Grid::new(min, n, 0);
        let count = n.map(|v| (v.max(0) + BRICK - 1) / BRICK);
        // By brick of sites here, where it is in the part, in bricks.
        let mut moved = vec![None; self.cells.len() / BRICK_SITES];
        for (place, [i, j, k]) in
            (0..count[2]).flat_map(|k| (0..count[1]).flat_map(move |j| (0..count[0]).map(move |i| [i, j, k]))).enumerate()
        {
            let Ok(at) = usize::try_from(self.brick(min[0] + i * BRICK, min[1] + j * BRICK, min[2] + k * BRICK)) else { continue };
            let to = *moved[at].get_or_insert_with(|| {
                part.cells.extend_from_slice(&self.cells[at * BRICK_SITES..][..BRICK_SITES]);
                part.solid.extend_from_slice(&self.solid[at * BRICK_SITES / 32..][..BRICK_SITES / 32]);
                part.cells.len() / BRICK_SITES - 1
            });
            part.bricks[place] = to as i32;
        }
        // Each record's site where it is in the part, and where it starts here, sorted: no record copied till it is placed.
        let kept = |words: &[u32], per: usize| -> Vec<u32> {
            let to = |i: u32| moved[i as usize / BRICK_SITES].map(|to| (to * BRICK_SITES + i as usize % BRICK_SITES) as u32);
            let mut sites: Vec<(u32, u32)> =
                words.chunks_exact(per).enumerate().filter_map(|(r, w)| Some((to(w[0])?, (r * per) as u32))).collect();
            sites.sort_unstable_by_key(|&(to, _)| to);
            let mut kept = Vec::with_capacity(sites.len() * per);
            for (to, from) in sites {
                kept.push(to);
                kept.extend_from_slice(&words[from as usize + 1..][..per - 1]);
            }
            kept
        };
        (part.views, part.glow) = (kept(&self.views, VIEW_WORDS), kept(&self.glow, GLOW_WORDS));
        part.stops = kept(&self.stops, STOPS_WORDS);
        // Grown brick by brick, they hold room for up to as much again; a tile is kept long.
        part.cells.shrink_to_fit();
        part.solid.shrink_to_fit();
        part
    }

    /** The place in `bricks` of the brick holding site (x, y, z); None if it is outside. */
    #[inline]
    pub fn place(&self, x: i32, y: i32, z: i32) -> Option<usize> {
        let (x, y, z) = (x - self.min[0], y - self.min[1], z - self.min[2]);
        if x < 0 || x >= self.n[0] || y < 0 || y >= self.n[1] || z < 0 || z >= self.n[2] {
            return None;
        }
        let bricks = [(self.n[0] + BRICK - 1) / BRICK, (self.n[1] + BRICK - 1) / BRICK];
        Some((((z >> 3) * bricks[1] + (y >> 3)) * bricks[0] + (x >> 3)) as usize)
    }

    /** The brick holding site (x, y, z): where its sites start in `cells`, in bricks; −1 if it is empty or outside. */
    #[inline]
    pub fn brick(&self, x: i32, y: i32, z: i32) -> i32 {
        self.place(x, y, z).map_or(-1, |at| self.bricks[at])
    }

    /** Site (x, y, z)'s index in `cells`; None if its brick is empty or it is outside. */
    #[inline]
    fn index(&self, x: i32, y: i32, z: i32) -> Option<usize> {
        let at = self.brick(x, y, z);
        let (x, y, z) = (x - self.min[0], y - self.min[1], z - self.min[2]);
        (at >= 0).then(|| at as usize * BRICK_SITES + (((z & 7) * BRICK + (y & 7)) * BRICK + (x & 7)) as usize)
    }

    /** The look of the voxel at site (x, y, z); 0 if empty or outside. */
    #[inline]
    pub fn get(&self, x: i32, y: i32, z: i32) -> Look {
        self.index(x, y, z).map_or(0, |i| self.cells[i])
    }

    /** The look of the voxel at site (x, y, z) through face `kind` (of VOXEL_FACES); 0 if empty or outside. */
    #[inline]
    pub fn face(&self, x: i32, y: i32, z: i32, kind: usize) -> Look {
        let Some(i) = self.index(x, y, z) else { return 0 };
        match self.cells[i] {
            look if look == 0 || self.views.is_empty() => look,
            look => self.views_at(i).map_or(look, |views| views[kind]),
        }
    }

    /** Whether the voxel at site (x, y, z) stops a walking body; not if empty or outside. */
    pub fn solid(&self, x: i32, y: i32, z: i32) -> bool {
        self.index(x, y, z).is_some_and(|i| self.solid[i / 32] >> (i % 32) & 1 != 0)
    }

    /** What site (x, y, z) glows per channel (worldgen's GLOW_ONE), shown or not; nothing outside. */
    pub fn glow(&self, x: i32, y: i32, z: i32) -> [u32; 3] {
        match self.index(x, y, z).and_then(|i| words_of(&self.glow, GLOW_WORDS, i)) {
            Some(&[r, g, b]) => [r, g, b],
            _ => [0; 3],
        }
    }

    /** The looks through each face of the site at index `i` of `cells`, if they differ. */
    pub fn views_at(&self, i: usize) -> Option<&[Look]> {
        words_of(&self.views, VIEW_WORDS, i)
    }

    /** The sites at `sites` (indices in `cells`) that look unlike through some faces, in order: each's index and its looks through each face. */
    pub fn views_in(&self, sites: std::ops::Range<usize>) -> impl Iterator<Item = (usize, &[Look])> {
        let from = first_from(&self.views, VIEW_WORDS, sites.start) * VIEW_WORDS;
        self.views[from..].as_chunks::<VIEW_WORDS>().0.iter().map(|w| (w[0] as usize, &w[1..])).take_while(move |(i, _)| *i < sites.end)
    }

    /** What the see-through site at index `i` of `cells` stops of each channel, a byte each as a look's colour: its alpha of all, unless `stops` says. */
    pub fn stops_at(&self, i: usize) -> Look {
        let alpha = alpha_of(self.cells[i]);
        words_of(&self.stops, STOPS_WORDS, i).map_or(look([alpha; 3], 0), |w| w[0])
    }

    /** What the see-through voxel at site (x, y, z) stops of each channel, a byte each as a look's colour (`stops_at`); 0 if empty or outside. */
    pub fn stops(&self, x: i32, y: i32, z: i32) -> Look {
        self.index(x, y, z).map_or(0, |i| self.stops_at(i))
    }

    /** Whether any site at `sites` (indices in `cells`) stops channels unlike. */
    pub fn has_stops(&self, sites: std::ops::Range<usize>) -> bool {
        has(&self.stops, STOPS_WORDS, sites)
    }
}

/** A tile's (i, j, k): it holds x in [i · 2^shift, (i + 1) · 2^shift), y by j and z by k likewise. */
pub type TileId = [i32; 3];

/** Tiles a block of Blocks holds a side, as a shift, and in all. */
const BLOCK_SHIFT: u32 = 2;
const BLOCK_TILES: usize = 1 << (3 * BLOCK_SHIFT);

/**
 * A `T` per tile, held in blocks of tiles, each shared with the copies
 * made of it: a copy takes as long as it has blocks, and a tile set or
 * taken out copies its block alone. So those who each read the tiles as
 * they were when they took them pay for what changed since, not for all
 * there is.
 */
#[derive(Clone)]
pub struct Blocks<T> {
    blocks: FastMap<TileId, Arc<Block<T>>>,
    len: usize,
}

/** A block's tiles, z-major, then y, and how many it holds. */
#[derive(Clone)]
struct Block<T> {
    tiles: [Option<T>; BLOCK_TILES],
    held: usize,
}

/** The block tile `id` is in, and its place there. */
#[inline]
fn block_of(id: TileId) -> (TileId, usize) {
    let within = id.map(|v| (v & ((1 << BLOCK_SHIFT) - 1)) as usize);
    (id.map(|v| v >> BLOCK_SHIFT), (within[2] << BLOCK_SHIFT | within[1]) << BLOCK_SHIFT | within[0])
}

impl<T> Default for Blocks<T> {
    fn default() -> Blocks<T> {
        Blocks { blocks: FastMap::default(), len: 0 }
    }
}

impl<T: Clone> Blocks<T> {
    /** How many tiles it holds. */
    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /** What tile `id` holds, if any. */
    #[inline]
    pub fn get(&self, id: TileId) -> Option<&T> {
        let (block, place) = block_of(id);
        self.blocks.get(&block)?.tiles[place].as_ref()
    }

    /** Sets what tile `id` holds; what it held. */
    pub fn insert(&mut self, id: TileId, value: T) -> Option<T> {
        let (block, place) = block_of(id);
        let block = self.blocks.entry(block).or_insert_with(|| Arc::new(Block { tiles: std::array::from_fn(|_| None), held: 0 }));
        let block = Arc::make_mut(block);
        let held = block.tiles[place].replace(value);
        if held.is_none() {
            block.held += 1;
            self.len += 1;
        }
        held
    }

    /** Takes out what tile `id` holds, if any. */
    pub fn remove(&mut self, id: TileId) -> Option<T> {
        let (at, place) = block_of(id);
        let block = self.blocks.get_mut(&at).filter(|b| b.tiles[place].is_some())?;
        let block = Arc::make_mut(block);
        let held = block.tiles[place].take();
        block.held -= 1;
        self.len -= 1;
        if block.held == 0 {
            self.blocks.remove(&at);
        }
        held
    }

    /** Each tile it holds, in no order. */
    pub fn iter(&self) -> impl Iterator<Item = (TileId, &T)> {
        self.blocks.iter().flat_map(|(at, block)| {
            let id = move |place: usize| {
                std::array::from_fn(|d| at[d] << BLOCK_SHIFT | (place >> (d as u32 * BLOCK_SHIFT) & ((1 << BLOCK_SHIFT) - 1)) as i32)
            };
            block.tiles.iter().enumerate().filter_map(move |(place, tile)| Some((id(place), tile.as_ref()?)))
        })
    }
}

impl<T: Clone> FromIterator<(TileId, T)> for Blocks<T> {
    fn from_iter<I: IntoIterator<Item = (TileId, T)>>(tiles: I) -> Blocks<T> {
        let mut blocks = Blocks::default();
        for (id, value) in tiles {
            blocks.insert(id, value);
        }
        blocks
    }
}

/**
 * A level's sites as grids on a tiling of space by cubes, 2^shift sites a
 * side: a site no tile has a grid for is unknown; one a tile's grid does
 * not hold, empty. The grids are shared, and the table of them in blocks
 * (Blocks), so a copy with tiles added or taken out is cheap to make.
 */
#[derive(Clone)]
pub struct Tiles {
    shift: u32,
    /** Where the tiling starts along each axis: 0, or the least i32 for the one tile of `one`. */
    origin: i64,
    grids: Blocks<Arc<Grid>>,
}

impl Tiles {
    /** The tiles 2^shift sites a side that have a grid: `tiles` holds each one's id and grid, its box within the tile. */
    pub fn new(shift: u32, tiles: impl IntoIterator<Item = (TileId, Arc<Grid>)>) -> Tiles {
        Tiles { shift, origin: 0, grids: tiles.into_iter().collect() }
    }

    /** A grid as the only tile: one holding all space, 2^32 sites a side from the least i32. */
    pub fn one(grid: Grid) -> Tiles {
        Tiles { shift: 32, origin: i32::MIN as i64, grids: [([0; 3], Arc::new(grid))].into_iter().collect() }
    }

    /** Gives tile `id` a grid, its box within the tile. */
    pub fn insert(&mut self, id: TileId, grid: Arc<Grid>) {
        self.grids.insert(id, grid);
    }

    /** Tile `id` has no grid any more. */
    pub fn remove(&mut self, id: TileId) {
        self.grids.remove(id);
    }

    /** The look at site (x, y, z), 0 if empty; None if no tile has a grid there. */
    pub fn at(&self, x: i32, y: i32, z: i32) -> Option<Look> {
        Some(self.grid(self.tile(x, y, z))?.get(x, y, z))
    }

    /** The grid of the tile site (x, y, z) is in, if it has one. */
    pub fn grid_at(&self, x: i32, y: i32, z: i32) -> Option<&Grid> {
        self.grid(self.tile(x, y, z))
    }

    /** Whether the voxel at site (x, y, z) stops a walking body (Grid::solid); None if no tile has a grid there. */
    pub fn solid(&self, x: i32, y: i32, z: i32) -> Option<bool> {
        Some(self.grid(self.tile(x, y, z))?.solid(x, y, z))
    }

    /** Whether tile `id` has a grid. */
    pub fn has(&self, id: TileId) -> bool {
        self.grid(id).is_some()
    }

    /** Whether the tiles from `lo` to `hi`, inclusive, all have grids. */
    pub fn has_all(&self, [lo, hi]: [TileId; 2]) -> bool {
        (lo[2]..=hi[2]).all(|k| (lo[1]..=hi[1]).all(|j| (lo[0]..=hi[0]).all(|i| self.has([i, j, k]))))
    }

    /** The tile site (x, y, z) is in. */
    #[inline]
    fn tile(&self, x: i32, y: i32, z: i32) -> TileId {
        [x, y, z].map(|v| ((v as i64 - self.origin) >> self.shift) as i32)
    }

    /** The grid of tile `id`, if it has one. */
    #[inline]
    fn grid(&self, id: TileId) -> Option<&Grid> {
        self.grids.get(id).map(|grid| &**grid)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opposite_faces_are_opposite() {
        assert!((0..FACE_KINDS).all(|k| VOXEL_FACES[opposite(k)] == VOXEL_FACES[k].map(|v| -v)));
    }

    #[test]
    fn pieces_held_in_part_show_whole_as_often_as_they_stop_rays() {
        for alpha in [1, 64, 191, 254] {
            let look = partial([200, 100, 50], alpha, true);
            let shown = (0..64 * 64 * 64).filter(|&i| whole_or_none(look, i % 64, i / 64 % 64, i / 4096) != 0).count();
            let share = shown as f64 / (64 * 64 * 64) as f64;
            assert!((share - f64::from(alpha) / 256.0).abs() < 0.005, "alpha {alpha}: shown {share}");
        }
        assert_eq!(whole_or_none(partial([200, 100, 50], 64, false), 0, 0, 0), partial([200, 100, 50], 64, false), "a medium as it is");
    }

    #[test]
    fn blocks_hold_what_a_map_would_and_a_copy_changes_alone() {
        use std::collections::BTreeMap;
        // Tiles either side of zero and of blocks' edges: set, then of a copy one in three taken out, one set again.
        let ids = || (-9..9).flat_map(|i| (-9..9).map(move |j| [i, j, i * j]));
        let (mut blocks, mut map) = (Blocks::default(), BTreeMap::new());
        for (n, id) in ids().enumerate() {
            assert_eq!(blocks.insert(id, n), map.insert(id, n));
        }
        let before = (blocks.clone(), map.clone());
        for (n, id) in ids().enumerate() {
            match n % 3 {
                0 => assert_eq!(blocks.remove(id), map.remove(&id)),
                1 => assert_eq!(blocks.insert(id, 0), map.insert(id, 0)),
                _ => (),
            }
        }
        assert_eq!(blocks.remove([99; 3]), None);
        for (blocks, map) in [(&blocks, &map), (&before.0, &before.1)] {
            assert_eq!(blocks.iter().map(|(id, n)| (id, *n)).collect::<BTreeMap<_, _>>(), *map);
            assert!(blocks.len() == map.len() && ids().all(|id| blocks.get(id) == map.get(&id)));
        }
        ids().for_each(|id| _ = blocks.remove(id));
        assert!(blocks.is_empty() && blocks.iter().next().is_none());
    }

    #[test]
    fn tiles_hold_their_cubes_alike_along_every_axis() {
        // A voxel in each of two tiles, one above the other, as shift 3 makes them: a brick each.
        let tile = |min: [i32; 3]| {
            let mut grid = Grid::new(min, [8, 8, 8], 1);
            grid.bricks[0] = 0;
            grid.cells[0] = 1 + min[1] as u32 / 8;
            grid.solid[0] = min[1] as u32 / 8;
            Arc::new(grid)
        };
        let tiles = Tiles::new(
            3,
            [([0, 0, 0], tile([0, 0, 0])), ([0, 1, 0], tile([0, 8, 0])), ([1, 0, 0], Arc::new(Grid::new([8, 0, 0], [0; 3], 0)))],
        );
        assert_eq!(tiles.at(0, 0, 0), Some(1));
        assert_eq!(tiles.at(0, 8, 0), Some(2));
        assert_eq!((tiles.solid(0, 0, 0), tiles.solid(0, 8, 0)), (Some(false), Some(true)));
        assert_eq!(tiles.at(2, 0, 0), Some(0), "empty in its tile");
        assert_eq!(tiles.at(8, 0, 0), Some(0), "a tile whose grid holds nothing is empty");
        for unknown in [[0, 16, 0], [0, -1, 0], [0, 0, 8], [-1, 0, 0]] {
            assert_eq!(tiles.at(unknown[0], unknown[1], unknown[2]), None, "{unknown:?}: no tile there");
        }
        assert!(tiles.has_all([[0, 0, 0], [1, 0, 0]]) && tiles.has_all([[0, 0, 0], [0, 1, 0]]));
        assert!(!tiles.has_all([[0, 0, 0], [1, 1, 0]]) && !tiles.has([0, 0, 1]));
        // A copy with a tile more and one less: the first is as it was.
        let mut next = tiles.clone();
        next.insert([0, 0, 1], tile([0, 0, 8]));
        next.remove([0, 1, 0]);
        assert_eq!((next.at(0, 0, 8), next.at(0, 8, 0)), (Some(1), None));
        assert_eq!((tiles.at(0, 0, 8), tiles.at(0, 8, 0)), (None, Some(2)));
    }

    #[test]
    fn a_part_holds_its_boxs_sites_as_the_grid_does() {
        // Two bricks along y sharing sites, a third above with a site that glows and one unlike through its faces.
        let mut grid = Grid::new([0, 0, 0], [8, 24, 8], 2);
        grid.bricks = vec![0, 0, 1];
        let index = |[x, y, z]: [i32; 3]| ((z * BRICK + y % BRICK) * BRICK + x) as usize;
        let (rock, lamp, painted) = ([2, 2, 2], [2, 18, 2], [4, 20, 2]);
        grid.cells[index(rock)] = 7;
        grid.solid[index(rock) / 32] = 1 << (index(rock) % 32);
        for site in [lamp, painted] {
            grid.cells[BRICK_SITES + index(site)] = 1;
        }
        grid.glow = vec![(BRICK_SITES + index(lamp)) as u32, 1, 2, 3];
        grid.views = [(BRICK_SITES + index(painted)) as u32].into_iter().chain((0..FACE_KINDS as u32).map(|k| 10 + k)).collect();
        let held = |g: &Grid, [x, y, z]: [i32; 3]| (g.get(x, y, z), g.face(x, y, z, 3), g.solid(x, y, z), g.glow(x, y, z));
        for (y, n, bricks) in [(0, 16, 1), (8, 16, 2), (16, 8, 1), (0, 24, 2)] {
            let part = grid.part([0, y, 0], [8, n, 8]);
            assert_eq!(part.cells.len(), bricks * BRICK_SITES, "from y = {y}: bricks sharing sites still do");
            for site in [rock, [2, 10, 2], lamp, painted, [6, 6, 2]] {
                let inside = (y..y + n).contains(&site[1]);
                assert_eq!(held(&part, site), if inside { held(&grid, site) } else { (0, 0, false, [0; 3]) }, "{site:?} from y = {y}");
            }
        }
    }

    #[test]
    fn a_ray_sees_a_voxel_by_the_faces_turned_most_against_it() {
        let seen = |d: [f64; 3]| facing(d).map(|k| VOXEL_FACES[k]).collect::<Vec<_>>();
        assert_eq!(seen([0.0, -1.0, 0.0]), [[0, 1, 0]]);
        assert_eq!(seen([0.3, -1.0, 0.1]), [[0, 1, 0]]);
        assert_eq!(seen([1.0, 1.0, 0.0]), [[-1, 0, 0], [0, -1, 0]]);
    }
}
