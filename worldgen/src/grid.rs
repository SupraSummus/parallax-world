//! Voxel storage: what each site of a box
//! looks like, in bricks BRICK units wide, only bricks with voxels stored.
//! The same layout as the baker's Grid, which `to_grid` hands over.

use std::sync::OnceLock;

use fcc::{FACE_KINDS, Look};

use crate::js;

// The baker's grids, as worlds hand them over, the voxel's faces, and a whole site's alpha: it stops every ray.
pub use fcc::{ALPHA, Grid, VOXEL_FACES};

pub const BRICK: i64 = 8;
const BRICK_SITES: usize = (BRICK * BRICK * BRICK / 2) as usize;

/** Half-open box of integer points: min ≤ p < max on each axis; lattice units, or a level's. */
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Box3 {
    pub min: [i64; 3],
    pub max: [i64; 3],
}

impl Box3 {
    pub fn holds(&self, p: [i64; 3]) -> bool {
        (0..3).all(|i| p[i] >= self.min[i] && p[i] < self.max[i])
    }

    pub fn overlaps(&self, b: &Box3) -> bool {
        (0..3).all(|i| self.min[i] < b.max[i] && b.min[i] < self.max[i])
    }

    pub fn inside(&self, b: &Box3) -> bool {
        (0..3).all(|i| self.min[i] >= b.min[i] && self.max[i] <= b.max[i])
    }

    pub fn intersect(&self, b: &Box3) -> Box3 {
        Box3 { min: [0, 1, 2].map(|i| self.min[i].max(b.min[i])), max: [0, 1, 2].map(|i| self.max[i].min(b.max[i])) }
    }

    pub fn widen(&self, m: i64) -> Box3 {
        Box3 { min: self.min.map(|v| v - m), max: self.max.map(|v| v + m) }
    }

    /** It with its sites from `from` to `to` along `axis`. */
    pub fn slab(&self, axis: usize, from: i64, to: i64) -> Box3 {
        let (mut min, mut max) = (self.min, self.max);
        (min[axis], max[axis]) = (from, to);
        Box3 { min, max }
    }

    /** The cube reaching `half` from `centre` along each axis. */
    pub fn around(centre: [i64; 3], half: i64) -> Box3 {
        Box3 { min: centre.map(|v| v - half), max: centre.map(|v| v + half) }
    }
}

pub fn is_site(x: i64, y: i64, z: i64) -> bool {
    (x + y + z) & 1 == 0
}

/**
 * The sites of a box. A site holds what
 * it looks like, not what it is made of, in two ways: pieces smaller
 * than itself, which stop some of its rays whole, and a medium
 * (KindDef::medium), which stops some of each ray the pieces let
 * through, of each channel its own share. Hosts and light see every
 * site as it is: pieces that stop every ray as a whole voxel (`show`),
 * else what it stops, held in part (`to_grid`, `light`). Apart from how
 * it looks, how much of it stops a walking body.
 */
#[derive(Clone)]
pub struct VoxelGrid {
    pub min: [i64; 3],
    pub n: [i64; 3],
    bx: i64,
    by: i64,
    /** Per brick, z-major, then y, then x: where its sites start in `shown`, in BRICK_SITES; −1 if empty. Once buried (`bury`), bricks may share sites. */
    pub bricks: Vec<i32>,
    /** Whether each site's pieces stop every ray, shown to hosts as a whole voxel (`show`). */
    pub shown: Vec<bool>,
    /** What is in each site in pieces: how many of its rays they stop, whole. */
    pub pieces: Part<u8>,
    /** What is in each site as a medium: how much it stops of each ray the pieces let through, channel by channel. */
    pub medium: Part<[u8; 3]>,
    /** Of ALPHA, how much of each site's rays what stops a walking body stops (KindDef::walk_through, not). */
    pub solid: Vec<u8>,
    /** Of the sites that glow: how much, per channel, of GLOW_ONE. */
    pub glow: Few<[u32; 3]>,
    /** Of the sites whose pieces look unlike along each direction: what they stop and show along each. */
    pub views: Sights,
    /** Of the sites in `views`, those that glow: how much along each direction. Apart, as few do. */
    pub glows: Few<Glows>,
}

/** All a site holds (`VoxelGrid::held`). */
pub type Held = (bool, (u8, [u16; 3]), ([u8; 3], [u16; 3]), u8, [u32; 3], Option<Views>, Option<Glows>);

/** What a site's pieces glow along each direction, as Views are. */
pub type Glows = [[u32; 3]; FACE_KINDS];

/**
 * What a site's pieces stop of the rays along each of the lattice's
 * directions and show to them: by the face of VOXEL_FACES they head
 * into, along the opposite of its normal (fcc's `facing`).
 */
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Views {
    pub alpha: [u8; FACE_KINDS],
    pub colour: [[u16; 3]; FACE_KINDS],
}

/**
 * What the sites hold one way, as pieces or as a medium, by index in
 * `shown`: only of the bricks that hold any, as few hold a medium, and
 * some no pieces. Pieces stop every channel alike (`A` a share), a
 * medium each its own (a share a channel).
 */
#[derive(Clone, Default)]
pub struct Part<A> {
    /** By where a brick's sites start in `shown` (in bricks), where they start here; −1 if it holds none. */
    at: Vec<i32>,
    /** Of ALPHA, how much they stop. */
    pub alpha: Vec<A>,
    /** What they show of the rays they stop, premultiplied: per channel, linear, of COLOUR_ONE. */
    pub colour: Vec<[u16; 3]>,
}

/** Where site `i` of `shown` is among the sites of the bricks that `at` says hold any: by brick of `shown`, where its own start, in bricks; −1 if it holds none. */
#[inline]
fn within(at: &[i32], i: usize) -> Option<usize> {
    usize::try_from(at[i / BRICK_SITES]).ok().map(|at| at * BRICK_SITES + i % BRICK_SITES)
}

impl<A: Copy + Default + PartialEq> Part<A> {
    /** Where site `i` of `shown` is here, if its brick holds any. */
    #[inline]
    fn slot(&self, i: usize) -> Option<usize> {
        within(&self.at, i)
    }

    /** What site `i` of `shown` stops and shows: nothing, if its brick holds none. */
    #[inline]
    pub fn get(&self, i: usize) -> (A, [u16; 3]) {
        self.slot(i).map_or((A::default(), [0; 3]), |at| (self.alpha[at], self.colour[at]))
    }

    /** Has site `i` of `shown` stop `alpha` and show `colour`. */
    #[inline]
    pub fn set(&mut self, i: usize, alpha: A, colour: [u16; 3]) {
        let at = match self.slot(i) {
            Some(at) => at,
            None if (alpha, colour) == (A::default(), [0; 3]) => return,
            None => {
                self.at[i / BRICK_SITES] = (self.alpha.len() / BRICK_SITES) as i32;
                self.alpha.resize(self.alpha.len() + BRICK_SITES, A::default());
                self.colour.resize(self.colour.len() + BRICK_SITES, [0; 3]);
                self.alpha.len() - BRICK_SITES + i % BRICK_SITES
            }
        };
        (self.alpha[at], self.colour[at]) = (alpha, colour);
    }

    /** What the sites of the brick starting at `at` in `shown` (in bricks) stop, if it holds any. */
    fn brick(&self, at: usize) -> Option<&[A]> {
        usize::try_from(self.at[at]).ok().map(|at| &self.alpha[at * BRICK_SITES..][..BRICK_SITES])
    }

    /** Holds the sites of only the bricks `held` (VoxelGrid::hold), in that order, and of those only the ones that hold any. */
    fn hold(&mut self, held: &[usize]) {
        let mut kept = Part { at: Vec::new(), alpha: Vec::new(), colour: Vec::new() };
        for &from in held {
            let sites = usize::try_from(self.at[from]).ok().map(|at| at * BRICK_SITES..(at + 1) * BRICK_SITES);
            let sites = sites.filter(|sites| {
                self.alpha[sites.clone()].iter().any(|&a| a != A::default()) || self.colour[sites.clone()] != [[0; 3]; BRICK_SITES]
            });
            kept.at.push(sites.as_ref().map_or(-1, |_| (kept.alpha.len() / BRICK_SITES) as i32));
            if let Some(sites) = sites {
                kept.alpha.extend_from_slice(&self.alpha[sites.clone()]);
                kept.colour.extend_from_slice(&self.colour[sites]);
            }
        }
        kept.alpha.shrink_to_fit();
        kept.colour.shrink_to_fit();
        *self = kept;
    }
}

/**
 * What few sites hold, by index in `shown`: each site of a brick that
 * holds any has a mark, so most sites, in bricks that hold none, are read
 * at a glance. Marks, not room for every site as a Part has: what a site
 * holds is many times a mark, and most sites of such a brick hold nothing.
 */
#[derive(Clone)]
pub struct Few<T> {
    /** As Part's, of its marks. */
    at: Vec<i32>,
    /** By site of a brick that has them: one more than where what it holds is in `held`; 0 if it holds nothing. */
    marks: Vec<u32>,
    /** What sites hold; and, till `hold`, what those that hold nothing any more did. */
    held: Vec<T>,
}

impl<T> Default for Few<T> {
    fn default() -> Few<T> {
        Few { at: Vec::new(), marks: Vec::new(), held: Vec::new() }
    }
}

impl<T> Few<T> {
    /** What site `i` of `shown` holds, if anything. */
    #[inline]
    pub fn get(&self, i: usize) -> Option<&T> {
        let place = (self.marks[within(&self.at, i)?] as usize).checked_sub(1)?;
        Some(&self.held[place])
    }

    /** Has site `i` of `shown` hold `value`, or nothing. */
    pub fn set(&mut self, i: usize, value: Option<T>) {
        let mark = match (within(&self.at, i), &value) {
            (Some(mark), _) => mark,
            (None, None) => return,
            (None, Some(_)) => {
                self.at[i / BRICK_SITES] = (self.marks.len() / BRICK_SITES) as i32;
                self.marks.resize(self.marks.len() + BRICK_SITES, 0);
                self.marks.len() - BRICK_SITES + i % BRICK_SITES
            }
        };
        match (self.marks[mark], value) {
            (_, None) => self.marks[mark] = 0,
            (0, Some(value)) => {
                self.held.push(value);
                self.marks[mark] = self.held.len() as u32;
            }
            (place, Some(value)) => self.held[place as usize - 1] = value,
        }
    }

    /** Every site that holds anything, with what it holds, in their order. */
    pub fn iter(&self) -> impl Iterator<Item = (usize, &T)> {
        (0..self.at.len()).flat_map(|at| self.brick(at))
    }

    /** `iter`, of the brick whose sites start at `at` in `shown` (in bricks). */
    fn brick(&self, at: usize) -> impl Iterator<Item = (usize, &T)> {
        let marks = usize::try_from(self.at[at]).ok().map(|marks| &self.marks[marks * BRICK_SITES..][..BRICK_SITES]);
        let held = marks.into_iter().flatten().enumerate().filter_map(|(site, &mark)| Some((site, (mark as usize).checked_sub(1)?)));
        held.map(move |(site, place)| (at * BRICK_SITES + site, &self.held[place]))
    }
}

impl<T: Clone> Few<T> {
    /** Holds the sites of only the bricks `held` (VoxelGrid::hold), in that order, with no room to spare. */
    fn hold(&mut self, held: &[usize]) {
        let mut kept = Few { at: vec![-1; held.len()], ..Few::default() };
        for (to, &from) in held.iter().enumerate() {
            for (i, value) in self.brick(from) {
                kept.set(to * BRICK_SITES + i % BRICK_SITES, Some(value.clone()));
            }
        }
        kept.marks.shrink_to_fit();
        kept.held.shrink_to_fit();
        *self = kept;
    }
}

/** What a site's pieces stop of the rays along one direction, and show of them (as Views', one face's). */
pub type Sight = (u8, [u16; 3]);

/** A site's views, as Sights holds them: where its sights start, and by face, which of them it sees. */
type Seen = (u32, [u8; FACE_KINDS]);

/**
 * The Views of the few sites that have them, by index in `shown`: the
 * sights a site's faces see, each once, and which of them each face
 * sees. Nine in ten of a valley's see two, and a sight is a twelfth of
 * a Views.
 */
#[derive(Clone, Default)]
pub struct Sights {
    seen: Few<Seen>,
    /** Sites' sights; and, till `hold`, those of what sites held before. */
    sights: Vec<Sight>,
}

/** `views` as a site holds them, their sights added to `sights`. */
fn seen(sights: &mut Vec<Sight>, views: &Views) -> Seen {
    let first = sights.len();
    let which = std::array::from_fn(|k| {
        let sight = (views.alpha[k], views.colour[k]);
        let at = sights[first..].iter().position(|s| *s == sight).unwrap_or_else(|| {
            sights.push(sight);
            sights.len() - first - 1
        });
        at as u8
    });
    (first as u32, which)
}

/** The views a site holds as `seen`, of `sights`. */
fn views(sights: &[Sight], (first, which): &Seen) -> Views {
    let sight = |k: usize| sights[*first as usize + which[k] as usize];
    Views { alpha: std::array::from_fn(|k| sight(k).0), colour: std::array::from_fn(|k| sight(k).1) }
}

impl Sights {
    /** What site `i` of `shown` stops and shows along face `kind`'s direction, if it has views. */
    #[inline]
    pub fn face(&self, i: usize, kind: usize) -> Option<Sight> {
        let (first, which) = self.seen.get(i)?;
        Some(self.sights[*first as usize + which[kind] as usize])
    }

    /** Site `i`'s views, if it has any. */
    pub fn get(&self, i: usize) -> Option<Views> {
        self.seen.get(i).map(|seen| views(&self.sights, seen))
    }

    /** Has site `i` of `shown` hold `views`, or none. */
    pub fn set(&mut self, i: usize, views: Option<Views>) {
        let seen = views.map(|views| seen(&mut self.sights, &views));
        self.seen.set(i, seen);
    }

    /** Every site that has views, with them, in their order. */
    pub fn iter(&self) -> impl Iterator<Item = (usize, Views)> {
        self.seen.iter().map(|(i, seen)| (i, views(&self.sights, seen)))
    }

    /** As Few's `hold`, of the sights too. */
    fn hold(&mut self, held: &[usize]) {
        self.seen.hold(held);
        let mut kept = Vec::new();
        for held in &mut self.seen.held {
            *held = seen(&mut kept, &views(&self.sights, held));
        }
        kept.shrink_to_fit();
        self.sights = kept;
    }
}

/** The glow of a site all of a glowing white: 255 per channel (linear), of a whole share (generator's SHARE). */
pub const GLOW_ONE: u32 = 255 << 16;

/** The colour of a site all of a white that stops every ray, per channel (linear). */
pub const COLOUR_ONE: u16 = u16::MAX;

impl VoxelGrid {
    /** A grid of the sites in `b`, widened by one in x if needed to start and end on an even x. */
    pub fn new(b: &Box3) -> VoxelGrid {
        let min_x = b.min[0].div_euclid(2) * 2;
        let nx = 0.max((b.max[0] + 1).div_euclid(2) * 2 - min_x);
        let ny = 0.max(b.max[1] - b.min[1]);
        let nz = 0.max(b.max[2] - b.min[2]);
        let (bx, by, bz) = ((nx + BRICK - 1) / BRICK, (ny + BRICK - 1) / BRICK, (nz + BRICK - 1) / BRICK);
        VoxelGrid {
            min: [min_x, b.min[1], b.min[2]],
            n: [nx, ny, nz],
            bx,
            by,
            bricks: vec![-1; (bx * by * bz) as usize],
            shown: Vec::new(),
            pieces: Part::default(),
            medium: Part::default(),
            solid: Vec::new(),
            glow: Few::default(),
            views: Sights::default(),
            glows: Few::default(),
        }
    }

    /**
     * Keeps its bricks under `floor` along `axis`, by line along it the
     * first site of the line through a site not buried under, the same
     * over a brick's lines, as one of rock of `colour` (a whole site's,
     * premultiplied): each a whole voxel, as `bury` keeps them. Nothing was
     * put in them. Not emptied: a host shows it as rock, which rays from a
     * finer level's hollows meet and light doesn't pass.
     */
    pub fn on_rock(&mut self, axis: usize, floor: impl Fn([i64; 3]) -> i64, colour: [u16; 3]) {
        let at = (self.shown.len() / BRICK_SITES) as i32;
        let (bx, by, top) = (self.bx, self.by, self.min[axis] + self.n[axis]);
        let mut any = false;
        for (i, brick) in self.bricks.iter_mut().enumerate() {
            let first = [i as i64 % bx, i as i64 / bx % by, i as i64 / (bx * by)].map(|b| b * BRICK);
            let first: [i64; 3] = std::array::from_fn(|a| self.min[a] + first[a]);
            if (first[axis] + BRICK).min(top) <= floor(first) {
                debug_assert!(*brick < 0, "a brick under its floor holds sites");
                (*brick, any) = (at, true);
            }
        }
        if any {
            for i in self.grown() {
                (self.shown[i], self.solid[i]) = (true, ALPHA);
                self.pieces.set(i, ALPHA, colour);
            }
        }
    }

    /** Room for one more brick's sites, each holding nothing: where they are in `shown`. */
    fn grown(&mut self) -> std::ops::Range<usize> {
        let (first, len) = (self.shown.len(), self.shown.len() + BRICK_SITES);
        self.shown.resize(len, false);
        self.solid.resize(len, 0);
        self.pieces.at.push(-1);
        self.medium.at.push(-1);
        self.glow.at.push(-1);
        self.views.seen.at.push(-1);
        self.glows.at.push(-1);
        first..len
    }

    /** The box of its sites. */
    pub fn clip(&self) -> Box3 {
        Box3 { min: self.min, max: [0, 1, 2].map(|i| self.min[i] + self.n[i]) }
    }

    /** Where site (x, y, z) is in `shown`: Err(brick) if that brick is empty, None outside the box or off the sites. */
    fn slot(&self, x: i64, y: i64, z: i64) -> Option<Result<usize, usize>> {
        let (i, j, k) = (x - self.min[0], y - self.min[1], z - self.min[2]);
        if i < 0 || i >= self.n[0] || j < 0 || j >= self.n[1] || k < 0 || k >= self.n[2] || !is_site(x, y, z) {
            return None;
        }
        let brick = (((k >> 3) * self.by + (j >> 3)) * self.bx + (i >> 3)) as usize;
        let at = self.bricks[brick];
        Some(if at < 0 {
            Err(brick)
        } else {
            Ok(at as usize * BRICK_SITES + ((((k & 7) * BRICK + (j & 7)) * BRICK + (i & 7)) >> 1) as usize)
        })
    }

    /** What the voxel at (x, y, z) looks like if its pieces are whole (`look`); 0 if not, outside the box or not a site. */
    pub fn get(&self, x: i64, y: i64, z: i64) -> Look {
        match self.slot(x, y, z) {
            Some(Ok(i)) => self.look(i),
            _ => 0,
        }
    }

    /** As `get`, but None if the brick of (x, y, z) holds nothing: rays skip it. */
    pub fn get_or_empty_brick(&self, x: i64, y: i64, z: i64) -> Option<Look> {
        match self.slot(x, y, z) {
            Some(Ok(i)) => Some(self.look(i)),
            Some(Err(_)) => None,
            None => Some(0),
        }
    }

    /** What the voxel at (x, y, z) looks like to hosts through face `kind` (of fcc's VOXEL_FACES); 0 if none, outside the box or not a site. */
    pub fn face(&self, x: i64, y: i64, z: i64, kind: usize) -> Look {
        match self.slot(x, y, z) {
            Some(Ok(i)) => self.view(i, kind),
            _ => 0,
        }
    }

    /**
     * What the voxel at (x, y, z)'s pieces look like to a ray along
     * `direction`, whole: the mean of their looks through the faces turned
     * most against it (fcc's `facing`); 0 if none, outside the box or not a site.
     */
    pub fn along(&self, x: i64, y: i64, z: i64, direction: [f64; 3]) -> Look {
        match self.slot(x, y, z) {
            Some(Ok(i)) if self.pieces.get(i).0 > 0 => {
                let (mut alpha, mut colour, mut faces) = (0, [0; 3], 0);
                for k in fcc::facing(direction) {
                    let (a, c) = self.part(i, k);
                    alpha += a as u32;
                    (0..3).for_each(|j| colour[j] += c[j] as u32);
                    faces += 1;
                }
                let mean = |v: u32| (v + faces / 2) / faces;
                look_of(colour.map(|c| mean(c) as u16), mean(alpha) as u8, ALPHA)
            }
            _ => 0,
        }
    }

    /**
     * What is in site (x, y, z) looks like, shown or not: its pieces if
     * any, else its medium, each as much as it stops; 0 if nothing.
     */
    pub fn inside(&self, x: i64, y: i64, z: i64) -> Look {
        match self.slot(x, y, z) {
            Some(Ok(i)) => match self.pieces.get(i) {
                (0, _) => self.partial(i),
                (alpha, colour) => look_of(colour, alpha, alpha),
            },
            _ => 0,
        }
    }

    /** All site (x, y, z) holds, which every way of seeing it reads; nothing outside the box or off the sites. */
    pub fn held(&self, x: i64, y: i64, z: i64) -> Held {
        match self.slot(x, y, z) {
            Some(Ok(i)) => (
                self.shown[i],
                self.pieces.get(i),
                self.medium.get(i),
                self.solid[i],
                self.glow.get(i).copied().unwrap_or_default(),
                self.views.get(i),
                self.glows.get(i).copied(),
            ),
            _ => Held::default(),
        }
    }

    /** Of ALPHA, how much of site (x, y, z)'s rays what stops a walking body stops. */
    pub fn solid(&self, x: i64, y: i64, z: i64) -> u8 {
        match self.slot(x, y, z) {
            Some(Ok(i)) => self.solid[i],
            _ => 0,
        }
    }

    /** What site `i`'s pieces look like where shown, whole (fcc's Look); else 0. */
    fn look(&self, i: usize) -> Look {
        match self.shown[i] {
            true => {
                let (alpha, colour) = self.pieces.get(i);
                look_of(colour, alpha, ALPHA)
            }
            false => 0,
        }
    }

    /**
     * What site `i` looks like held in part (fcc's Look): its colour, each
     * channel what it shows of what it stops of it, and for alpha what it
     * stops of the channel it stops least, of pieces if it holds any, else
     * of a medium; 0 if it stops none of one. How much a medium stops of
     * each is apart (`to_grid`'s `stops`).
     */
    fn partial(&self, i: usize) -> Look {
        let (stops, shows) = self.seen(i);
        match least(stops) {
            0 => 0,
            alpha => fcc::partial(std::array::from_fn(|c| srgb(shows[c], stops[c])), alpha, self.pieces.get(i).0 > 0),
        }
    }

    /** What site `i` stops of each channel of its rays, its pieces and its medium of the rest, and what that shows (premultiplied, of COLOUR_ONE). */
    fn seen(&self, i: usize) -> ([u8; 3], [u16; 3]) {
        let ((pieces, p), (medium, m)) = self.with_medium(i, self.pieces.get(i));
        (medium.map(|m| pieces + m), std::array::from_fn(|c| (p[c] + m[c]) as u16))
    }

    /** Of site `i`, pieces that stop `pieces` of its rays and show `colour`, and its medium of the rest: each how much it stops and shows. */
    fn with_medium(&self, i: usize, (pieces, colour): (u8, [u16; 3])) -> (Stops<u8>, Stops<[u8; 3]>) {
        let rest = |v: u32| ((ALPHA - pieces) as u64 * v as u64 + ALPHA as u64 / 2) / ALPHA as u64;
        let (medium, of) = self.medium.get(i);
        ((pieces, colour.map(u32::from)), (medium.map(|a| rest(a as u32) as u8), of.map(|c| rest(c as u32) as u32)))
    }

    /** What site `i`'s shown pieces look like to hosts through face `kind`: as along its direction, where they stop any of it, else as a whole. */
    fn view(&self, i: usize, kind: usize) -> Look {
        let (alpha, colour) = self.part(i, kind);
        if self.shown[i] { look_of(colour, alpha, ALPHA) } else { 0 }
    }

    /** Site `i`'s pieces along face `kind`'s direction: what they stop of it and show (premultiplied), where they stop any of it, else as a whole. */
    fn part(&self, i: usize, kind: usize) -> (u8, [u16; 3]) {
        self.views.face(i, kind).filter(|sight| sight.0 > 0).unwrap_or_else(|| self.pieces.get(i))
    }

    /**
     * Puts at (x, y, z) pieces that stop every ray, shown, of `colours`
     * through each face (linear, of COLOUR_ONE), for tests.
     */
    #[cfg(test)]
    pub(crate) fn put_whole(&mut self, x: i64, y: i64, z: i64, colours: [[u16; 3]; FACE_KINDS]) {
        let i = self.index(x, y, z).expect("in the box");
        self.shown[i] = true;
        self.pieces.set(i, ALPHA, colours[0]);
        if colours.iter().any(|c| *c != colours[0]) {
            self.views.set(i, Some(Views { alpha: [ALPHA; FACE_KINDS], colour: colours }));
        }
    }

    /** Where site (x, y, z) is in `shown`, its brick added if empty; None outside the box or off the sites. */
    pub fn index(&mut self, x: i64, y: i64, z: i64) -> Option<usize> {
        match self.slot(x, y, z)? {
            Ok(i) => Some(i),
            Err(brick) => {
                self.bricks[brick] = (self.grown().start / BRICK_SITES) as i32;
                self.slot(x, y, z).and_then(Result::ok)
            }
        }
    }

    /** Shows each site its pieces as a whole voxel where they stop every ray; the rest are held in part (`to_grid`). */
    pub fn show(&mut self) {
        for i in 0..self.shown.len() {
            self.shown[i] = self.pieces.get(i).0 == ALPHA;
        }
    }

    /**
     * Keeps its buried bricks as one: those whose every site's pieces stop
     * every ray, with the 26 around them so too. None of their voxels shows,
     * nor lets light by, so they all hold one's sites (`bricks` shares it),
     * its views and glow dropped: rock is kept once, not by its volume.
     * Not emptied, as a hollow's walls would be a surface to bake and
     * light. Bricks on its sides are kept, whatever is past them unknown.
     * Its bricks sharing sites, it is to be read or handed over, not changed.
     */
    pub fn bury(&mut self) {
        // Every slot of a brick is a site; those past the box are 0, so a brick it cuts is not whole.
        let whole: Vec<bool> = self
            .bricks
            .iter()
            .map(|&at| usize::try_from(at).is_ok_and(|at| self.pieces.brick(at).is_some_and(|stops| stops.iter().all(|&a| a == ALPHA))))
            .collect();
        let count = [self.bx, self.by, self.bricks.len() as i64 / (self.bx * self.by).max(1)];
        let buried = |brick: usize| {
            let b = brick as i64;
            let at = [b % self.bx, b / self.bx % self.by, b / (self.bx * self.by)];
            (0..3).all(|c| at[c] > 0 && at[c] + 1 < count[c])
                && (0..27).all(|d| {
                    let [i, j, k] = [d % 3, d / 3 % 3, d / 9].map(|v| v - 1);
                    whole[(b + (k * self.by + j) * self.bx + i) as usize]
                })
        };
        let buried: Vec<usize> = (0..self.bricks.len()).filter(|&b| buried(b)).collect();
        // The last of their sites: on_rock's, if they are of it, so it is kept once.
        if let Some(one) = buried.iter().map(|&b| self.bricks[b]).max() {
            (self.glow.at[one as usize], self.views.seen.at[one as usize], self.glows.at[one as usize]) = (-1, -1, -1);
            for brick in buried {
                self.bricks[brick] = one;
            }
        }
        // With none buried too: its sites, grown brick by brick, are held with no room to spare.
        self.hold(|_| true);
    }

    /**
     * Holds the sites of only the bricks that are in `bricks` and `keep`
     * (by where their sites start in `shown`, in bricks), together in
     * their order in `shown`; the others' bricks become empty.
     */
    fn hold(&mut self, keep: impl Fn(usize) -> bool) {
        let mut moved = vec![None; self.shown.len() / BRICK_SITES];
        for &at in &self.bricks {
            if let Ok(at) = usize::try_from(at) {
                moved[at] = Some(0);
            }
        }
        let held: Vec<usize> = (0..moved.len()).filter(|&at| moved[at].is_some() && keep(at)).collect();
        moved.fill(None);
        for (to, &from) in held.iter().enumerate() {
            moved[from] = Some(to);
        }
        for at in &mut self.bricks {
            if let Ok(from) = usize::try_from(*at) {
                *at = moved[from].map_or(-1, |to| to as i32);
            }
        }
        fn hold<T: Copy>(v: &mut Vec<T>, held: &[usize]) {
            let mut kept = Vec::with_capacity(held.len() * BRICK_SITES);
            for &from in held {
                kept.extend_from_slice(&v[from * BRICK_SITES..][..BRICK_SITES]);
            }
            *v = kept;
        }
        hold(&mut self.shown, &held);
        hold(&mut self.solid, &held);
        self.pieces.hold(&held);
        self.medium.hold(&held);
        self.glow.hold(&held);
        self.views.hold(&held);
        self.glows.hold(&held);
    }

    /** Of ALPHA, how much of its rays site `i` of `shown` stops: its pieces, and its medium of the rest, in the channel it stops least. */
    fn stops(&self, i: usize) -> u8 {
        let (pieces, medium, all) = (self.pieces.get(i).0 as u32, least(self.medium.get(i).0) as u32, ALPHA as u32);
        ((pieces * all + (all - pieces) * medium + all / 2) / all) as u8
    }

    /**
     * How light entering site (x, y, z) by face `kind` sees it: what its
     * pieces stop of its rays and its medium of the rest, channel by
     * channel, each how much (of ALPHA) and what that shows (premultiplied,
     * of COLOUR_ONE); and its glow. Each the face's, where they differ
     * (`views`).
     */
    pub fn light(&self, x: i64, y: i64, z: i64, kind: usize) -> (Stops<u8>, Stops<[u8; 3]>, [u32; 3]) {
        match self.slot(x, y, z) {
            Some(Ok(i)) => {
                let (pieces, glow) = match self.views.face(i, kind) {
                    Some(pieces) => (pieces, self.glows.get(i).map_or([0; 3], |g| g[kind])),
                    None => (self.pieces.get(i), self.glow.get(i).copied().unwrap_or_default()),
                };
                let (pieces, medium) = self.with_medium(i, pieces);
                (pieces, medium, glow)
            }
            _ => ((0, [0; 3]), ([0; 3], [0; 3]), [0; 3]),
        }
    }

    /** Of ALPHA, how much of the rays through site (x, y, z) it stops. */
    pub fn alpha(&self, x: i64, y: i64, z: i64) -> u8 {
        match self.slot(x, y, z) {
            Some(Ok(i)) => self.stops(i),
            _ => 0,
        }
    }

    /**
     * The voxels as hosts take them, bricks with none left out: each
     * site's pieces where shown (`show`), whole, as they look through each
     * face; elsewhere what it stops, its pieces and its medium, and of each
     * channel where they differ (the grid's `stops`). What glows, shown or
     * not, for the light: its bricks are kept too.
     */
    pub fn to_grid(&self) -> Grid {
        let looks: Vec<Look> = (0..self.shown.len()).map(|i| if self.shown[i] { self.look(i) } else { self.partial(i) }).collect();
        let keep: Vec<bool> = (0..self.shown.len() / BRICK_SITES)
            .map(|at| {
                let sites = at * BRICK_SITES..(at + 1) * BRICK_SITES;
                looks[sites].iter().any(|&c| c != 0) || self.glow.brick(at).next().is_some()
            })
            .collect();
        let mut moved = vec![None; keep.len()];
        let mut grid = Grid::new(self.min.map(|v| v as i32), self.n.map(|v| v as i32), 0);
        for (place, &at) in self.bricks.iter().enumerate() {
            let Ok(at) = usize::try_from(at) else { continue };
            if !keep[at] {
                continue;
            }
            let to = *moved[at].get_or_insert_with(|| {
                let to = grid.cells.len() / BRICK_SITES;
                grid.cells.extend_from_slice(&looks[at * BRICK_SITES..][..BRICK_SITES]);
                // More than half of its rays stopped by what stops a body: one stands on it.
                let solid = |i: usize| self.solid[at * BRICK_SITES + i] > ALPHA / 2;
                grid.solid.extend((0..BRICK_SITES / 32).map(|w| (0..32).filter(|b| solid(32 * w + b)).fold(0, |word, b| word | 1 << b)));
                to
            });
            grid.bricks[place] = to as i32;
        }
        let to = |i: usize| moved[i / BRICK_SITES].map(|to| (to * BRICK_SITES + i % BRICK_SITES) as u32);
        let sorted = |words: &mut dyn Iterator<Item = Vec<u32>>| -> Vec<u32> {
            let mut sites: Vec<Vec<u32>> = words.collect();
            sites.sort_unstable_by_key(|w| w[0]);
            sites.concat()
        };
        // As `view` gives them, without looking each face's up again.
        let view = |(i, v): (usize, Views)| -> Option<Vec<u32>> {
            if !self.shown[i] {
                return None;
            }
            let look = |k: usize| if v.alpha[k] > 0 { look_of(v.colour[k], v.alpha[k], ALPHA) } else { looks[i] };
            let (to, views) = (to(i)?, std::array::from_fn::<Look, FACE_KINDS, _>(look));
            views.iter().any(|&v| v != views[0]).then(|| [&[to][..], &views].concat())
        };
        grid.views = sorted(&mut self.views.iter().filter_map(view));
        grid.glow = sorted(&mut self.glow.iter().filter_map(|(i, g)| Some([&[to(i)?][..], g].concat())));
        // Of the bricks that hold a medium only: few do.
        let held = (0..self.shown.len() / BRICK_SITES).filter(|&at| self.medium.brick(at).is_some());
        let stops = |i: usize| {
            let [r, g, b] = self.seen(i).0;
            (fcc::medium(looks[i]) && (r != g || g != b)).then(|| Some([to(i)?, fcc::look([r, g, b], 0)])).flatten()
        };
        let mut stops: Vec<[u32; 2]> = held.flat_map(|at| at * BRICK_SITES..(at + 1) * BRICK_SITES).filter_map(stops).collect();
        stops.sort_unstable_by_key(|w| w[0]);
        grid.stops = stops.into_flattened();
        grid
    }
}

/** What is seen of a site one way: how much it stops (of ALPHA) and what that shows (premultiplied, of COLOUR_ONE). */
pub type Stops<A> = (A, [u32; 3]);

/** Of what a medium stops of each channel, the least: its alpha, to what takes one. */
pub fn least(stops: [u8; 3]) -> u8 {
    stops[0].min(stops[1]).min(stops[2])
}

/** A channel `c` (linear, premultiplied by `alpha`, of COLOUR_ONE) unpremultiplied, sRGB. */
fn srgb(c: u16, alpha: u8) -> u8 {
    let of = alpha.max(1) as f64 / ALPHA as f64 * COLOUR_ONE as f64;
    let c = (c as f64 / of).min(1.0);
    (255.0 * if c <= 0.003_130_8 { c * 12.92 } else { 1.055 * js::pow(c, 1.0 / 2.4) - 0.055 }).round() as u8
}

/** A channel of sRGB (of 1) as linear light: fcc::light's, by libm's power (js.rs), as generating is the same on every target. */
pub(crate) fn to_linear(c: f64) -> f64 {
    if c <= 0.04045 { c / 12.92 } else { js::pow((c + 0.055) / 1.055, 2.4) }
}

/**
 * A look of colour `colour` (linear, premultiplied by `alpha`, of
 * COLOUR_ONE): its colour unpremultiplied, sRGB, and `shown` its alpha.
 */
pub(crate) fn look_of(colour: [u16; 3], alpha: u8, shown: u8) -> Look {
    // Most sites shown are whole, and a power costs more than all else here: theirs are kept.
    static WHOLE: OnceLock<Vec<u8>> = OnceLock::new();
    match alpha {
        ALPHA => {
            let whole = WHOLE.get_or_init(|| (0..=COLOUR_ONE).map(|c| srgb(c, ALPHA)).collect());
            fcc::look(colour.map(|c| whole[c as usize]), shown)
        }
        _ => fcc::look(colour.map(|c| srgb(c, alpha)), shown),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_site_is_handed_over_as_it_looks_through_each_face_and_moves_with_its_brick() {
        // Pieces in the second brick, a grey through every face but one; a lamp, nothing shown, in the third.
        let mut grid = VoxelGrid::new(&Box3 { min: [0; 3], max: [24, 8, 8] });
        grid.index(2, 2, 2);
        let grey = |c: u16| [c; 3];
        let mut colour = [grey(COLOUR_ONE / 5); FACE_KINDS];
        colour[3] = grey(COLOUR_ONE);
        grid.put_whole(10, 2, 2, colour);
        let lamp = grid.index(18, 2, 2).unwrap();
        grid.glow.set(lamp, Some([1, 2, 3]));
        let whole = grid.to_grid();
        // A fifth of white, linear, is 124 of 255 in sRGB.
        let (dim, white) = (fcc::look([124; 3], ALPHA), fcc::look([255; 3], ALPHA));
        assert_eq!((whole.cells.len(), whole.get(2, 2, 2), whole.get(10, 2, 2)), (2 * BRICK_SITES, 0, dim));
        assert_eq!([2, 3].map(|k| whole.face(10, 2, 2, k)), [dim, white]);
        assert_eq!((whole.get(18, 2, 2), whole.glow(18, 2, 2), whole.glow(10, 2, 2)), (0, [1, 2, 3], [0; 3]));
    }

    #[test]
    fn few_sites_hold_what_they_were_given_as_others_let_go_and_bricks_move() {
        // Two lamps in the first brick, one in each of the next two.
        let mut grid = VoxelGrid::new(&Box3 { min: [0; 3], max: [24, 8, 8] });
        let lamps = [[0, 0, 0], [2, 0, 0], [10, 2, 2], [18, 2, 2]];
        let sites = lamps.map(|[x, y, z]| grid.index(x, y, z).unwrap());
        let glows = |grid: &VoxelGrid| lamps.map(|[x, y, z]| grid.light(x, y, z, 0).2[0]);
        for (k, i) in sites.into_iter().enumerate() {
            grid.glow.set(i, Some([k as u32 + 1; 3]));
        }
        grid.glow.set(sites[0], None);
        grid.glow.set(sites[1], Some([5; 3]));
        assert_eq!(glows(&grid), [0, 5, 3, 4]);
        // The second brick let go, the third's sites take its place, with what they hold.
        grid.hold(|at| at != 1);
        assert_eq!(glows(&grid), [0, 5, 0, 4]);
        assert_eq!(grid.glow.iter().map(|(i, _)| i).collect::<Vec<_>>(), [sites[1], sites[3] - BRICK_SITES]);
    }

    #[test]
    fn buried_bricks_are_kept_as_one_and_the_rest_as_they_were() {
        // Solid but for a hole at (20, 20, 20), a lamp at (12, 12, 12) and odd views at (44, 44, 44): 6 bricks a side.
        let b = Box3 { min: [0; 3], max: [48; 3] };
        let mut grid = VoxelGrid::new(&b);
        let sites =
            || (0..48).flat_map(|z| (0..48).flat_map(move |y| (0..48).map(move |x| [x, y, z]))).filter(|&[x, y, z]| is_site(x, y, z));
        for [x, y, z] in sites() {
            grid.put_whole(x, y, z, [[(1 + (x + y) % 3) as u16 * 1000; 3]; FACE_KINDS]);
        }
        let i = grid.index(20, 20, 20).unwrap();
        grid.shown[i] = false;
        grid.pieces.set(i, 0, [0; 3]);
        let lamp = grid.index(12, 12, 12).unwrap();
        grid.glow.set(lamp, Some([1, 2, 3]));
        grid.put_whole(44, 44, 44, std::array::from_fn(|k| [(1 + k % 2) as u16 * 1000; 3]));
        let before = grid.clone();
        grid.bury();
        // Of the inner 4³ bricks (1 to 4 each way), those not around the hole's (2, 2, 2): any 4 away, held once.
        let buried = |p: [i64; 3]| p.iter().all(|&v| (8..40).contains(&v)) && p.iter().any(|&v| v >= 32);
        assert_eq!(grid.shown.len(), (6 * 6 * 6 - (4 * 4 * 4 - 3 * 3 * 3) + 1) * BRICK_SITES);
        let handed = grid.to_grid();
        for [x, y, z] in sites() {
            if buried([x, y, z]) {
                assert_eq!(grid.light(x, y, z, 1).0.0, ALPHA, "{x}, {y}, {z}");
                assert_ne!(handed.get(x as i32, y as i32, z as i32), 0);
            } else {
                assert_eq!(grid.light(x, y, z, 1), before.light(x, y, z, 1), "{x}, {y}, {z}");
                assert_eq!(grid.get(x, y, z), before.get(x, y, z));
                assert_eq!(handed.get(x as i32, y as i32, z as i32), before.get(x, y, z));
            }
        }
    }

    #[test]
    fn hosts_see_a_site_as_light_does() {
        let mut grid = VoxelGrid::new(&Box3 { min: [0; 3], max: [8, 8, 8] });
        // White pieces, as much of them stopping a walking body: in part, whole, none; each without a black medium and in one.
        let sites: [(i32, i32, u8, u8); 5] = [(0, 0, 40, 0), (2, 0, ALPHA, 0), (0, 2, 0, 60), (2, 2, 40, 60), (4, 2, ALPHA, 60)];
        for (x, y, pieces, medium) in sites {
            let i = grid.index(x.into(), y.into(), 0).unwrap();
            let white = (COLOUR_ONE as u32 * pieces as u32 / ALPHA as u32) as u16;
            grid.solid[i] = pieces;
            grid.pieces.set(i, pieces, [white; 3]);
            grid.medium.set(i, [medium; 3], [0; 3]);
        }
        grid.show();
        // Light sees all a site stops, each as it is: its pieces 40 of 255, its medium 60 of the rest.
        let (pieces, medium, _) = grid.light(2, 2, 0, 0);
        assert_eq!((grid.alpha(2, 2, 0), pieces.0, medium.0), (91, 40, [51; 3]));
        // Hosts see whole pieces whole, what hides behind them not; the rest as much as light sees it stop,
        // pieces held in part told from a medium alone (fcc's in_part).
        let (handed, whole) = (grid.to_grid(), fcc::look([255; 3], ALPHA));
        assert_eq!([(2, 0), (4, 2)].map(|(x, y)| (handed.get(x, y, 0), handed.solid(x, y, 0))), [(whole, true); 2]);
        for (x, y, pieces, _) in [sites[0], sites[2], sites[3]] {
            let (look, light) = (handed.get(x, y, 0), grid.alpha(x.into(), y.into(), 0));
            assert!(fcc::alpha_of(look) == light && fcc::in_part(look) == (pieces > 0), "at ({x}, {y}): {look:x}");
        }
    }
}
