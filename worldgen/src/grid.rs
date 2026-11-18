//! Voxel storage as voxel-grid.ts lays it out: what each site of a box
//! looks like, in bricks BRICK units wide, only bricks with voxels stored.
//! The same layout as the baker's Grid, which `into_grid` hands over.

use std::collections::BTreeMap;
use std::sync::OnceLock;

use fcc::light::linear_to_srgb;
use fcc::{FACE_KINDS, Look};

// The baker's grids, as worlds hand them over (web/wasm/src/lib.rs), the voxel's faces, and a whole site's alpha: it stops every ray.
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

    /** The cube reaching `half` from `centre` along each axis. */
    pub fn around(centre: [i64; 3], half: i64) -> Box3 {
        Box3 { min: centre.map(|v| v - half), max: centre.map(|v| v + half) }
    }
}

pub fn is_site(x: i64, y: i64, z: i64) -> bool {
    (x + y + z) & 1 == 0
}

/**
 * The sites of a box: see VoxelGrid in voxel-grid.ts. A site holds what
 * it looks like, not what it is made of, in two ways: pieces smaller
 * than itself, which stop some of its rays whole, and a medium
 * (KindDef::medium), which stops some of each ray the pieces let
 * through. Hosts are shown its pieces as a whole voxel as often as they
 * stop rays (`show`, `into_grid`); light sees every site as it is
 * (`light`). Apart from how it looks, how much of it stops a walking body.
 */
#[derive(Clone)]
pub struct VoxelGrid {
    pub min: [i64; 3],
    pub n: [i64; 3],
    bx: i64,
    by: i64,
    /** Per brick, z-major, then y, then x: where its sites start in `shown`, in BRICK_SITES; −1 if empty. Once buried (`bury`), bricks may share sites. */
    pub bricks: Vec<i32>,
    /** Whether each site's pieces are shown to hosts, a whole voxel (`show`). */
    pub shown: Vec<bool>,
    /** What is in each site in pieces: how many of its rays they stop, whole. */
    pub pieces: Part,
    /** What is in each site as a medium: how much it stops of each ray the pieces let through. */
    pub medium: Part,
    /** Of ALPHA, how much of each site's rays what stops a walking body stops (KindDef::walk_through, not). */
    pub solid: Vec<u8>,
    /** By index in `shown`, of the sites that glow: how much, per channel, of GLOW_ONE. */
    pub glow: BTreeMap<usize, [u32; 3]>,
    /** By index in `shown`, the sites whose pieces look unlike along each direction: what they stop, show and glow along each. */
    pub views: BTreeMap<usize, Views>,
}

/**
 * What a site's pieces stop of the rays along each of the lattice's
 * directions, show and glow to them: by the face of VOXEL_FACES they head
 * into, along the opposite of its normal (fcc's `facing`).
 */
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Views {
    pub alpha: [u8; FACE_KINDS],
    pub colour: [[u16; 3]; FACE_KINDS],
    pub glow: [[u32; 3]; FACE_KINDS],
}

/** What the sites hold one way, as pieces or as a medium, by index in `shown`. */
#[derive(Clone, Default)]
pub struct Part {
    /** Of ALPHA, how much they stop. */
    pub alpha: Vec<u8>,
    /** What they show of the rays they stop, premultiplied: per channel, linear, of COLOUR_ONE. */
    pub colour: Vec<[u16; 3]>,
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
            glow: BTreeMap::new(),
            views: BTreeMap::new(),
        }
    }

    /**
     * It on box `under`, the same columns right below it, all buried in
     * rock of `colour` (a whole site's, premultiplied), as `bury` keeps it:
     * each brick of it one's sites, each a whole voxel. Not emptied: a host
     * shows it as rock, which rays from a finer level's hollows meet and
     * light doesn't pass. `under` is a whole number of bricks tall, unless
     * it has no sites itself.
     */
    pub fn on_buried(self, under: &Box3, colour: [u16; 3]) -> VoxelGrid {
        let below = (self.min[1] - under.min[1] + BRICK - 1).div_euclid(BRICK);
        if below <= 0 {
            return self;
        }
        assert!(self.n[1] == 0 || (self.min[1] - under.min[1]) % BRICK == 0, "{under:?} under {:?} is not of whole bricks", self.clip());
        let top = if self.n[1] == 0 { under.max[1] } else { self.min[1] + self.n[1] };
        let mut grid = VoxelGrid::new(&Box3 {
            min: [self.min[0], under.min[1], self.min[2]],
            max: [self.min[0] + self.n[0], top, self.min[2] + self.n[2]],
        });
        let at = (self.shown.len() / BRICK_SITES) as i32;
        let (bx, by) = (grid.bx, grid.by);
        for (i, brick) in grid.bricks.iter_mut().enumerate() {
            let [x, y, z] = [i as i64 % bx, i as i64 / bx % by, i as i64 / (bx * by)];
            *brick = if y < below { at } else { self.bricks[((z * self.by + y - below) * bx + x) as usize] };
        }
        let VoxelGrid { mut shown, mut pieces, mut medium, mut solid, glow, views, .. } = self;
        shown.extend([true; BRICK_SITES]);
        pieces.alpha.extend([ALPHA; BRICK_SITES]);
        pieces.colour.extend([colour; BRICK_SITES]);
        medium.alpha.extend([0; BRICK_SITES]);
        medium.colour.extend([[0; 3]; BRICK_SITES]);
        solid.extend([ALPHA; BRICK_SITES]);
        VoxelGrid { shown, pieces, medium, solid, glow, views, ..grid }
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

    /** What the voxel at (x, y, z) looks like to hosts (`look`); 0 if none, outside the box or not a site. */
    pub fn get(&self, x: i64, y: i64, z: i64) -> Look {
        match self.slot(x, y, z) {
            Some(Ok(i)) => self.look(i, false),
            _ => 0,
        }
    }

    /** As `get`, but None if the brick of (x, y, z) holds nothing: rays skip it. */
    pub fn get_or_empty_brick(&self, x: i64, y: i64, z: i64) -> Option<Look> {
        match self.slot(x, y, z) {
            Some(Ok(i)) => Some(self.look(i, false)),
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
     * What the voxel at (x, y, z) looks like to a ray along `direction`:
     * the mean of its looks through the faces turned most against it
     * (fcc's `facing`); 0 if none, outside the box or not a site.
     */
    pub fn along(&self, x: i64, y: i64, z: i64, direction: [f64; 3]) -> Look {
        match self.slot(x, y, z) {
            Some(Ok(i)) if self.shown[i] => {
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
            Some(Ok(i)) if self.pieces.alpha[i] > 0 => look_of(self.pieces.colour[i], self.pieces.alpha[i], self.pieces.alpha[i]),
            Some(Ok(i)) if self.medium.alpha[i] > 0 => look_of(self.medium.colour[i], self.medium.alpha[i], self.medium.alpha[i]),
            _ => 0,
        }
    }

    /** Of ALPHA, how much of site (x, y, z)'s rays what stops a walking body stops. */
    pub fn solid(&self, x: i64, y: i64, z: i64) -> u8 {
        match self.slot(x, y, z) {
            Some(Ok(i)) => self.solid[i],
            _ => 0,
        }
    }

    /**
     * What site `i` looks like to hosts (fcc's Look): its pieces where
     * shown, whole; else, `see_through`, its medium, as much as it stops.
     */
    fn look(&self, i: usize, see_through: bool) -> Look {
        match self.shown[i] {
            true => look_of(self.pieces.colour[i], self.pieces.alpha[i], ALPHA),
            false if see_through && self.medium.alpha[i] > 0 => look_of(self.medium.colour[i], self.medium.alpha[i], self.medium.alpha[i]),
            false => 0,
        }
    }

    /** What site `i`'s shown pieces look like to hosts through face `kind`: as along its direction, where they stop any of it, else as a whole. */
    fn view(&self, i: usize, kind: usize) -> Look {
        let (alpha, colour) = self.part(i, kind);
        if self.shown[i] { look_of(colour, alpha, ALPHA) } else { 0 }
    }

    /** Site `i`'s pieces along face `kind`'s direction: what they stop of it and show (premultiplied), where they stop any of it, else as a whole. */
    fn part(&self, i: usize, kind: usize) -> (u8, [u16; 3]) {
        match self.views.get(&i) {
            Some(v) if v.alpha[kind] > 0 => (v.alpha[kind], v.colour[kind]),
            _ => (self.pieces.alpha[i], self.pieces.colour[i]),
        }
    }

    /**
     * Puts at (x, y, z) pieces that stop every ray, shown, of `colours`
     * through each face (linear, of COLOUR_ONE), for tests.
     */
    #[cfg(test)]
    pub(crate) fn put_whole(&mut self, x: i64, y: i64, z: i64, colours: [[u16; 3]; FACE_KINDS]) {
        let i = self.index(x, y, z).expect("in the box");
        (self.shown[i], self.pieces.alpha[i], self.pieces.colour[i]) = (true, ALPHA, colours[0]);
        if colours.iter().any(|c| *c != colours[0]) {
            self.views.insert(i, Views { alpha: [ALPHA; FACE_KINDS], colour: colours, glow: [[0; 3]; FACE_KINDS] });
        }
    }

    /** Where site (x, y, z) is in `shown`, its brick added if empty; None outside the box or off the sites. */
    pub fn index(&mut self, x: i64, y: i64, z: i64) -> Option<usize> {
        match self.slot(x, y, z)? {
            Ok(i) => Some(i),
            Err(brick) => {
                self.bricks[brick] = (self.shown.len() / BRICK_SITES) as i32;
                let len = self.shown.len() + BRICK_SITES;
                self.shown.resize(len, false);
                self.solid.resize(len, 0);
                for part in [&mut self.pieces, &mut self.medium] {
                    part.alpha.resize(len, 0);
                    part.colour.resize(len, [0; 3]);
                }
                self.slot(x, y, z).and_then(Result::ok)
            }
        }
    }

    /** Shows each site its pieces, a whole voxel, where their alpha is over `coin` of it (under ALPHA), else nothing. */
    pub fn show(&mut self, coin: impl Fn(i64, i64, i64) -> u8) {
        for (brick, &at) in self.bricks.iter().enumerate() {
            let Ok(at) = usize::try_from(at) else { continue };
            let b = brick as i64;
            let corner = [b % self.bx, b / self.bx % self.by, b / (self.bx * self.by)].map(|v| v * BRICK);
            for k in 0..BRICK {
                for j in 0..BRICK {
                    for i in 0..BRICK {
                        let [x, y, z] = [0, 1, 2].map(|c| self.min[c] + corner[c] + [i, j, k][c]);
                        if !is_site(x, y, z) {
                            continue;
                        }
                        let site = at * BRICK_SITES + (((k * BRICK + j) * BRICK + i) >> 1) as usize;
                        self.shown[site] = self.pieces.alpha[site] > coin(x, y, z);
                    }
                }
            }
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
            .map(|&at| usize::try_from(at).is_ok_and(|at| self.pieces.alpha[at * BRICK_SITES..][..BRICK_SITES].iter().all(|&a| a == ALPHA)))
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
        // The last of their sites: on_buried's rock, if they are of it, so it is kept once.
        let Some(one) = buried.iter().map(|&b| self.bricks[b]).max() else { return };
        let sites = one as usize * BRICK_SITES..(one as usize + 1) * BRICK_SITES;
        self.glow.retain(|i, _| !sites.contains(i));
        self.views.retain(|i, _| !sites.contains(i));
        for brick in buried {
            self.bricks[brick] = one;
        }
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
        for part in [&mut self.pieces, &mut self.medium] {
            hold(&mut part.alpha, &held);
            hold(&mut part.colour, &held);
        }
        let to = |i: usize| moved[i / BRICK_SITES].map(|to| to * BRICK_SITES + i % BRICK_SITES);
        self.glow = std::mem::take(&mut self.glow).into_iter().filter_map(|(i, g)| Some((to(i)?, g))).collect();
        self.views = std::mem::take(&mut self.views).into_iter().filter_map(|(i, v)| Some((to(i)?, v))).collect();
    }

    /** Of ALPHA, how much of its rays site `i` of `shown` stops: its pieces, and its medium of the rest. */
    fn stops(&self, i: usize) -> u8 {
        let (pieces, medium, all) = (self.pieces.alpha[i] as u32, self.medium.alpha[i] as u32, ALPHA as u32);
        ((pieces * all + (all - pieces) * medium + all / 2) / all) as u8
    }

    /**
     * How light entering site (x, y, z) by face `kind` sees it: what its
     * pieces stop of its rays and its medium of the rest, each how much (of
     * ALPHA) and what that shows (premultiplied, of COLOUR_ONE); and its
     * glow. Each the face's, where they differ (`views`).
     */
    pub fn light(&self, x: i64, y: i64, z: i64, kind: usize) -> ([(u8, [u32; 3]); 2], [u32; 3]) {
        match self.slot(x, y, z) {
            Some(Ok(i)) => {
                let (pieces, colour, glow) = match self.views.get(&i) {
                    Some(view) => (view.alpha[kind], view.colour[kind], view.glow[kind]),
                    None => (self.pieces.alpha[i], self.pieces.colour[i], self.glow.get(&i).copied().unwrap_or_default()),
                };
                let colour = colour.map(u32::from);
                // The medium, of the rays the pieces let through.
                let rest = |v: u32| ((ALPHA - pieces) as u64 * v as u64 + ALPHA as u64 / 2) / ALPHA as u64;
                let medium = (rest(self.medium.alpha[i] as u32) as u8, self.medium.colour[i].map(|c| rest(c as u32) as u32));
                ([(pieces, colour), medium], glow)
            }
            _ => ([(0, [0; 3]); 2], [0; 3]),
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
     * The voxels as the baker takes them, bricks with none left out: each
     * site's pieces where shown (`show`), whole, as they look through each
     * face; and `see_through`, for a host that draws alpha, elsewhere its
     * medium, as much as it stops. What glows, shown or not, for the light:
     * its bricks are kept too.
     */
    pub fn into_grid(self, see_through: bool) -> Grid {
        let looks: Vec<Look> = (0..self.shown.len()).map(|i| self.look(i, see_through)).collect();
        let keep: Vec<bool> = (0..self.shown.len() / BRICK_SITES)
            .map(|at| {
                let sites = at * BRICK_SITES..(at + 1) * BRICK_SITES;
                looks[sites.clone()].iter().any(|&c| c != 0) || self.glow.range(sites).next().is_some()
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
        let view = |(&i, v): (&usize, &Views)| -> Option<Vec<u32>> {
            if !self.shown[i] {
                return None;
            }
            let look = |k: usize| if v.alpha[k] > 0 { look_of(v.colour[k], v.alpha[k], ALPHA) } else { looks[i] };
            let (to, views) = (to(i)?, std::array::from_fn::<Look, FACE_KINDS, _>(look));
            views.iter().any(|&v| v != views[0]).then(|| [&[to][..], &views].concat())
        };
        grid.views = sorted(&mut self.views.iter().filter_map(view));
        grid.glow = sorted(&mut self.glow.iter().filter_map(|(&i, g)| Some([&[to(i)?][..], g].concat())));
        grid
    }
}

/**
 * A look of colour `colour` (linear, premultiplied by `alpha`, of
 * COLOUR_ONE): its colour unpremultiplied, sRGB, and `shown` its alpha.
 */
pub(crate) fn look_of(colour: [u16; 3], alpha: u8, shown: u8) -> Look {
    let srgb = |c: u16, alpha: u8| {
        let of = alpha.max(1) as f32 / ALPHA as f32 * COLOUR_ONE as f32;
        (255.0 * linear_to_srgb((c as f32 / of).min(1.0))).round() as u8
    };
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
        grid.glow.insert(lamp, [1, 2, 3]);
        let whole = grid.into_grid(false);
        // A fifth of white, linear, is 124 of 255 in sRGB.
        let (dim, white) = (fcc::look([124; 3], ALPHA), fcc::look([255; 3], ALPHA));
        assert_eq!((whole.cells.len(), whole.get(2, 2, 2), whole.get(10, 2, 2)), (2 * BRICK_SITES, 0, dim));
        assert_eq!([2, 3].map(|k| whole.face(10, 2, 2, k)), [dim, white]);
        assert_eq!((whole.get(18, 2, 2), whole.glow(18, 2, 2), whole.glow(10, 2, 2)), (0, [1, 2, 3], [0; 3]));
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
        (grid.shown[i], grid.pieces.alpha[i]) = (false, 0);
        let lamp = grid.index(12, 12, 12).unwrap();
        grid.glow.insert(lamp, [1, 2, 3]);
        grid.put_whole(44, 44, 44, std::array::from_fn(|k| [(1 + k % 2) as u16 * 1000; 3]));
        let before = grid.clone();
        grid.bury();
        // Of the inner 4³ bricks (1 to 4 each way), those not around the hole's (2, 2, 2): any 4 away, held once.
        let buried = |p: [i64; 3]| p.iter().all(|&v| (8..40).contains(&v)) && p.iter().any(|&v| v >= 32);
        assert_eq!(grid.shown.len(), (6 * 6 * 6 - (4 * 4 * 4 - 3 * 3 * 3) + 1) * BRICK_SITES);
        let handed = grid.clone().into_grid(false);
        for [x, y, z] in sites() {
            if buried([x, y, z]) {
                assert_eq!(grid.light(x, y, z, 1).0[0].0, ALPHA, "{x}, {y}, {z}");
                assert_ne!(handed.get(x as i32, y as i32, z as i32), 0);
            } else {
                assert_eq!(grid.light(x, y, z, 1), before.light(x, y, z, 1), "{x}, {y}, {z}");
                assert_eq!(grid.get(x, y, z), before.get(x, y, z));
                assert_eq!(handed.get(x as i32, y as i32, z as i32), before.get(x, y, z));
            }
        }
    }

    #[test]
    fn a_site_is_handed_over_as_its_pieces_where_shown_and_elsewhere_its_medium() {
        let mut grid = VoxelGrid::new(&Box3 { min: [0; 3], max: [8, 8, 8] });
        // White pieces, as much of them stopping a walking body, under the coin and over it, without a black medium and in one.
        let sites: [(i32, i32, u8, u8); 5] = [(0, 0, 40, 0), (2, 0, 200, 0), (0, 2, 0, 60), (2, 2, 40, 60), (4, 2, 200, 60)];
        for (x, y, pieces, medium) in sites {
            let i = grid.index(x.into(), y.into(), 0).unwrap();
            let white = (COLOUR_ONE as u32 * pieces as u32 / ALPHA as u32) as u16;
            (grid.pieces.alpha[i], grid.pieces.colour[i], grid.solid[i]) = (pieces, [white; 3], pieces);
            grid.medium.alpha[i] = medium;
        }
        grid.show(|_, _, _| 100);
        // Light sees all a site stops, each as it is: its pieces 40 of 255, its medium 60 of the rest.
        assert_eq!((grid.alpha(2, 2, 0), grid.light(2, 2, 0, 0).0.map(|part| part.0)), (91, [40, 51]));
        let handed = |grid: Grid| sites.map(|(x, y, ..)| (grid.get(x, y, 0), grid.solid(x, y, 0)));
        let (pieces, medium) = ((fcc::look([255; 3], ALPHA), true), (fcc::look([0; 3], 60), false));
        assert_eq!(handed(grid.clone().into_grid(true)), [(0, false), pieces, medium, medium, pieces]);
        assert_eq!(handed(grid.into_grid(false)), [(0, false), pieces, (0, false), (0, false), pieces]);
    }
}
