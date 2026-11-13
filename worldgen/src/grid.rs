//! Voxel storage as voxel-grid.ts lays it out: one material id per site of
//! a box, in bricks BRICK units wide, only bricks with voxels stored. The
//! same layout as the baker's Grid, which `into_grid` hands over.

use std::collections::BTreeMap;

use fcc::{Faces, face_words};

// The baker's grids, as worlds hand them over (web/wasm/src/lib.rs), the voxel's faces, and a whole site's alpha: it stops every ray.
pub use fcc::{ALPHA, FACE_WORDS, Grid, VOXEL_FACES};

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
}

pub fn is_site(x: i64, y: i64, z: i64) -> bool {
    (x + y + z) & 1 == 0
}

/**
 * The sites of a box: see VoxelGrid in voxel-grid.ts. A voxel has a
 * material on each of its faces (fcc's Grid): its site's in `cells`,
 * unless `faces` gives them. A site holds things in two ways: pieces
 * smaller than itself, which stop some of its rays whole, and a medium
 * (KindDef::medium), which stops some of each ray the pieces let
 * through. The voxels are its pieces, shown as often as they stop rays
 * (`show`); light sees every site as it is (`light`).
 */
#[derive(Clone)]
pub struct VoxelGrid {
    pub min: [i64; 3],
    pub n: [i64; 3],
    bx: i64,
    by: i64,
    /** Per brick, z-major, then y, then x: where its sites start in `cells`, in BRICK_SITES; −1 if empty. Once buried (`bury`), bricks may share sites. */
    pub bricks: Vec<i32>,
    pub cells: Vec<u8>,
    /** By index in `cells`, the faces' materials of the sites whose faces are not all of their material there. */
    pub faces: BTreeMap<usize, Faces>,
    /** What is in each site in pieces: how many of its rays they stop, whole; their faces' materials in `faces` if they differ. */
    pub pieces: Part,
    /** What is in each site as a medium: how much it stops of each ray the pieces let through. */
    pub medium: Part,
    /** By index in `cells`, of the sites that glow: how much, per channel, of GLOW_ONE. */
    pub glow: BTreeMap<usize, [u32; 3]>,
}

/** What the sites hold one way, as pieces or as a medium, by index in `cells`. */
#[derive(Clone, Default)]
pub struct Part {
    /** Of ALPHA, how much they stop. */
    pub alpha: Vec<u8>,
    /** The material they show, one of theirs. */
    pub seen: Vec<u8>,
}

/** The glow of a site all of a glowing white: 255 per channel (linear), of a whole share (generator's SHARE). */
pub const GLOW_ONE: u32 = 255 << 16;

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
            cells: Vec::new(),
            faces: BTreeMap::new(),
            pieces: Part::default(),
            medium: Part::default(),
            glow: BTreeMap::new(),
        }
    }

    /** The box of its sites. */
    pub fn clip(&self) -> Box3 {
        Box3 { min: self.min, max: [0, 1, 2].map(|i| self.min[i] + self.n[i]) }
    }

    /** Where site (x, y, z) is in `cells`: Err(brick) if that brick is empty, None outside the box or off the sites. */
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

    /** Material id at (x, y, z); 0 if empty, outside the box or not a site. */
    pub fn get(&self, x: i64, y: i64, z: i64) -> u8 {
        match self.slot(x, y, z) {
            Some(Ok(i)) => self.cells[i],
            _ => 0,
        }
    }

    /** As `get`, but None if the brick of (x, y, z) holds nothing: rays skip it. */
    pub fn get_or_empty_brick(&self, x: i64, y: i64, z: i64) -> Option<u8> {
        match self.slot(x, y, z) {
            Some(Ok(i)) => Some(self.cells[i]),
            Some(Err(_)) => None,
            None => Some(0),
        }
    }

    /** The material of face `kind` (of fcc's VOXEL_FACES) of the voxel at (x, y, z); 0 if empty, outside the box or not a site. */
    pub fn face(&self, x: i64, y: i64, z: i64, kind: usize) -> u8 {
        match self.slot(x, y, z) {
            Some(Ok(i)) if self.cells[i] != 0 => self.faces.get(&i).map_or(self.cells[i], |faces| faces[kind]),
            _ => 0,
        }
    }

    /** The faces' materials of the voxel at (x, y, z), if they are not all its site's. */
    pub fn faces_at(&self, x: i64, y: i64, z: i64) -> Option<&Faces> {
        match self.slot(x, y, z) {
            Some(Ok(i)) if self.cells[i] != 0 => self.faces.get(&i),
            _ => None,
        }
    }

    /**
     * Sets the faces of the voxel at (x, y, z) to `faces`' materials,
     * none 0, its site to `id`: the material it stands for to whatever
     * does not look at its faces. Outside the box or off the sites, nothing.
     */
    pub fn set_faces(&mut self, x: i64, y: i64, z: i64, id: u8, faces: Faces) {
        debug_assert!(id != 0 && !faces.contains(&0), "a face of nothing");
        self.set(x, y, z, id);
        if let Some(Ok(i)) = self.slot(x, y, z)
            && faces.iter().any(|&m| m != id)
        {
            self.faces.insert(i, faces);
        }
    }

    /** Sets site (x, y, z) to material `id` (0 clears), all its faces; outside the box or off the sites, nothing. */
    pub fn set(&mut self, x: i64, y: i64, z: i64, id: u8) {
        let i = match self.slot(x, y, z) {
            None => return,
            Some(Ok(i)) => i,
            Some(Err(_)) if id == 0 => return,
            Some(Err(_)) => self.index(x, y, z).expect("in the box"),
        };
        self.cells[i] = id;
        if !self.faces.is_empty() {
            self.faces.remove(&i);
        }
    }

    /** Where site (x, y, z) is in `cells`, its brick added if empty; None outside the box or off the sites. */
    pub fn index(&mut self, x: i64, y: i64, z: i64) -> Option<usize> {
        match self.slot(x, y, z)? {
            Ok(i) => Some(i),
            Err(brick) => {
                self.bricks[brick] = (self.cells.len() / BRICK_SITES) as i32;
                let len = self.cells.len() + BRICK_SITES;
                self.cells.resize(len, 0);
                for part in [&mut self.pieces, &mut self.medium] {
                    part.alpha.resize(len, 0);
                    part.seen.resize(len, 0);
                }
                self.slot(x, y, z).and_then(Result::ok)
            }
        }
    }

    /** Sets what site `i`'s pieces show: material `id`, its faces' if they differ. */
    pub fn see(&mut self, i: usize, id: u8, faces: Option<Faces>) {
        self.pieces.seen[i] = id;
        match faces.filter(|faces| faces.iter().any(|&m| m != id)) {
            Some(faces) => self.faces.insert(i, faces),
            None if !self.faces.is_empty() => self.faces.remove(&i),
            None => None,
        };
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
                        let shown = self.pieces.alpha[site] > coin(x, y, z);
                        self.cells[site] = if shown { self.pieces.seen[site] } else { 0 };
                    }
                }
            }
        }
    }

    /**
     * Keeps its buried bricks as one: those whose every site's pieces stop
     * every ray, with the 26 around them so too. None of their voxels shows,
     * nor lets light by, so they all hold one's sites (`bricks` shares it),
     * its faces and glow dropped: rock is kept once, not by its volume.
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
        let Some(&first) = buried.first() else { return };
        let one = self.bricks[first];
        let sites = one as usize * BRICK_SITES..(one as usize + 1) * BRICK_SITES;
        self.faces.retain(|i, _| !sites.contains(i));
        self.glow.retain(|i, _| !sites.contains(i));
        for brick in buried {
            self.bricks[brick] = one;
        }
        self.hold(|_| true);
    }

    /**
     * Holds the sites of only the bricks that are in `bricks` and `keep`
     * (by where their sites start in `cells`, in bricks), together in
     * their order in `cells`; the others' bricks become empty.
     */
    fn hold(&mut self, keep: impl Fn(usize) -> bool) {
        let mut moved = vec![None; self.cells.len() / BRICK_SITES];
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
        let hold = |v: &mut Vec<u8>| {
            let mut kept = Vec::with_capacity(held.len() * BRICK_SITES);
            for &from in &held {
                kept.extend_from_slice(&v[from * BRICK_SITES..][..BRICK_SITES]);
            }
            *v = kept;
        };
        hold(&mut self.cells);
        for part in [&mut self.pieces, &mut self.medium] {
            hold(&mut part.alpha);
            hold(&mut part.seen);
        }
        let to = |i: usize| moved[i / BRICK_SITES].map(|to| to * BRICK_SITES + i % BRICK_SITES);
        self.faces = std::mem::take(&mut self.faces).into_iter().filter_map(|(i, f)| Some((to(i)?, f))).collect();
        self.glow = std::mem::take(&mut self.glow).into_iter().filter_map(|(i, g)| Some((to(i)?, g))).collect();
    }

    /** Of ALPHA, how much of its rays site `i` of `cells` stops: its pieces, and its medium of the rest. */
    fn stops(&self, i: usize) -> u8 {
        let (pieces, medium, all) = (self.pieces.alpha[i] as u32, self.medium.alpha[i] as u32, ALPHA as u32);
        ((pieces * all + (all - pieces) * medium + all / 2) / all) as u8
    }

    /**
     * How light sees site (x, y, z): what its pieces stop of its rays and
     * its medium of the rest, each how much (of ALPHA; together, `alpha`)
     * and of what material (the pieces', face `kind`'s); and its glow.
     */
    pub fn light(&self, x: i64, y: i64, z: i64, kind: usize) -> ([(u8, u8); 2], [u32; 3]) {
        match self.slot(x, y, z) {
            Some(Ok(i)) => {
                let glow = if self.glow.is_empty() { [0; 3] } else { self.glow.get(&i).copied().unwrap_or_default() };
                let pieces = self.pieces.alpha[i];
                let material = if self.faces.is_empty() {
                    self.pieces.seen[i]
                } else {
                    self.faces.get(&i).map_or(self.pieces.seen[i], |faces| faces[kind])
                };
                ([(pieces, material), (self.stops(i) - pieces, self.medium.seen[i])], glow)
            }
            _ => ([(0, 0); 2], [0; 3]),
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
     * site's pieces where shown (`show`), whole; and `see_through`, for a
     * host that draws alpha, elsewhere its medium, as much as it stops.
     * What glows, shown or not, for the light: its bricks are kept too.
     */
    pub fn into_grid(mut self, see_through: bool) -> Grid {
        // Faces are the pieces': none where they don't show, though its medium may.
        let cells = &self.cells;
        self.faces.retain(|&i, _| cells[i] != 0);
        if see_through {
            let medium = &mut self.medium;
            for ((cell, alpha), &seen) in self.cells.iter_mut().zip(&mut medium.alpha).zip(&medium.seen) {
                match *cell {
                    0 if *alpha > 0 => *cell = seen,
                    0 => {}
                    _ => *alpha = ALPHA,
                }
            }
        }
        let keep: Vec<bool> = (0..self.cells.len() / BRICK_SITES)
            .map(|at| {
                let sites = at * BRICK_SITES..(at + 1) * BRICK_SITES;
                self.cells[sites.clone()].iter().any(|&c| c != 0) || self.glow.range(sites).next().is_some()
            })
            .collect();
        self.hold(|at| keep[at]);
        let faces = self.faces.iter().flat_map(|(&i, faces)| face_words(i, faces));
        let glow = self.glow.iter().flat_map(|(&i, &[r, g, b])| [i as u32, r, g, b]);
        Grid {
            min: self.min.map(|v| v as i32),
            n: self.n.map(|v| v as i32),
            faces: faces.collect(),
            glow: glow.collect(),
            bricks: self.bricks,
            alpha: if see_through { self.medium.alpha } else { Vec::new() },
            cells: self.cells,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn faces_are_set_with_their_site_and_go_with_it() {
        let mut grid = VoxelGrid::new(&Box3 { min: [0; 3], max: [24, 8, 8] });
        let faces: Faces = std::array::from_fn(|k| 2 + (k % 2) as u8);
        grid.set_faces(2, 2, 2, 2, faces);
        grid.set_faces(4, 2, 2, 2, [2; 12]);
        grid.set_faces(10, 2, 2, 3, faces);
        assert_eq!((grid.faces.len(), grid.face(2, 2, 2, 1), grid.face(4, 2, 2, 1), grid.get(2, 2, 2)), (2, 3, 2, 2));
        grid.set(2, 2, 2, 5);
        assert_eq!((grid.faces.len(), grid.face(2, 2, 2, 1)), (1, 5));

        // Handed over with its first brick emptied, that brick is left out, and the next moves with its sites' faces.
        // The last, with no voxel but a site that glows, is kept for its glow.
        grid.set(2, 2, 2, 0);
        grid.set(4, 2, 2, 0);
        let lamp = grid.index(18, 2, 2).unwrap();
        grid.glow.insert(lamp, [1, 2, 3]);
        let whole = grid.into_grid(false);
        assert_eq!((whole.cells.len(), whole.get(2, 2, 2)), (2 * BRICK_SITES, (0, 0)));
        assert_eq!([0, 1].map(|k| whole.face(10, 2, 2, k)), [(2, ALPHA), (3, ALPHA)]);
        assert_eq!((whole.get(18, 2, 2), whole.glow(18, 2, 2), whole.glow(10, 2, 2)), ((0, 0), [1, 2, 3], [0; 3]));
    }

    #[test]
    fn buried_bricks_are_kept_as_one_and_the_rest_as_they_were() {
        // Solid but for a hole at (20, 20, 20), a lamp at (12, 12, 12) and odd faces at (44, 44, 44): 6 bricks a side.
        let b = Box3 { min: [0; 3], max: [48; 3] };
        let mut grid = VoxelGrid::new(&b);
        let sites =
            || (0..48).flat_map(|z| (0..48).flat_map(move |y| (0..48).map(move |x| [x, y, z]))).filter(|&[x, y, z]| is_site(x, y, z));
        for [x, y, z] in sites() {
            let i = grid.index(x, y, z).unwrap();
            (grid.cells[i], grid.pieces.alpha[i], grid.pieces.seen[i]) = (1 + (x + y) as u8 % 3, ALPHA, 1);
        }
        let i = grid.index(20, 20, 20).unwrap();
        (grid.cells[i], grid.pieces.alpha[i]) = (0, 0);
        let lamp = grid.index(12, 12, 12).unwrap();
        grid.glow.insert(lamp, [1, 2, 3]);
        grid.set_faces(44, 44, 44, 1, std::array::from_fn(|k| 1 + (k % 2) as u8));
        let before = grid.clone();
        grid.bury();
        // Of the inner 4³ bricks (1 to 4 each way), those not around the hole's (2, 2, 2): any 4 away, held once.
        let buried = |p: [i64; 3]| p.iter().all(|&v| (8..40).contains(&v)) && p.iter().any(|&v| v >= 32);
        assert_eq!(grid.cells.len(), (6 * 6 * 6 - (4 * 4 * 4 - 3 * 3 * 3) + 1) * BRICK_SITES);
        let handed = grid.clone().into_grid(false);
        for [x, y, z] in sites() {
            if buried([x, y, z]) {
                assert_eq!(grid.light(x, y, z, 1).0[0].0, ALPHA, "{x}, {y}, {z}");
                assert_ne!(handed.get(x as i32, y as i32, z as i32).0, 0);
            } else {
                assert_eq!(grid.light(x, y, z, 1), before.light(x, y, z, 1), "{x}, {y}, {z}");
                assert_eq!((grid.get(x, y, z), handed.get(x as i32, y as i32, z as i32).0), (before.get(x, y, z), before.get(x, y, z)));
            }
        }
        assert_eq!(handed.face(44, 44, 44, 1), (2, ALPHA));
    }

    #[test]
    fn a_site_is_handed_over_as_its_pieces_where_shown_and_elsewhere_its_medium() {
        let mut grid = VoxelGrid::new(&Box3 { min: [0; 3], max: [8, 8, 8] });
        // Pieces of material 2, under the coin and over it, without a medium of material 1 and in one.
        let sites: [(i32, i32, u8, u8); 5] = [(0, 0, 40, 0), (2, 0, 200, 0), (0, 2, 0, 60), (2, 2, 40, 60), (4, 2, 200, 60)];
        for (x, y, pieces, medium) in sites {
            let i = grid.index(x.into(), y.into(), 0).unwrap();
            (grid.pieces.alpha[i], grid.pieces.seen[i]) = (pieces, 2);
            (grid.medium.alpha[i], grid.medium.seen[i]) = (medium, 1);
        }
        grid.show(|_, _, _| 100);
        // Light sees all a site stops, each as it is: its pieces 40 of 255, its medium 60 of the rest.
        assert_eq!((grid.alpha(2, 2, 0), grid.light(2, 2, 0, 0).0), (91, [(40, 2), (51, 1)]));
        let handed = |grid: Grid| sites.map(|(x, y, ..)| grid.get(x, y, 0));
        assert_eq!(handed(grid.clone().into_grid(true)), [(0, 0), (2, ALPHA), (1, 60), (1, 60), (2, ALPHA)]);
        assert_eq!(handed(grid.into_grid(false)), [(0, 0), (2, ALPHA), (0, 0), (0, 0), (2, ALPHA)]);
    }
}
