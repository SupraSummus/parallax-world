//! Voxel storage as voxel-grid.ts lays it out: one material id per site of
//! a box, in bricks BRICK units wide, only bricks with voxels stored. The
//! same layout as the baker's Grid, which `into_grid` hands over.

use std::collections::BTreeMap;

use bake::{Faces, face_words};

// The baker's grids, as worlds hand them over (worlds/src/wasm.rs), and the voxel's faces.
pub use bake::{FACE_WORDS, Grid, VOXEL_FACES};

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
 * material on each of its faces (bake's Grid): its site's in `cells`,
 * unless `faces` gives them.
 */
#[derive(Clone)]
pub struct VoxelGrid {
    pub min: [i64; 3],
    pub n: [i64; 3],
    bx: i64,
    by: i64,
    /** Per brick, z-major, then y, then x: where its sites start in `cells`, in BRICK_SITES; −1 if empty. */
    pub bricks: Vec<i32>,
    pub cells: Vec<u8>,
    /** By index in `cells`, the faces' materials of the sites whose faces are not all of their material there. */
    pub faces: BTreeMap<usize, Faces>,
    /**
     * What glows at sites: each glowing shape's, shared between the sites
     * around its centre (light::around), whether or not they keep voxels,
     * with its material and its mean shadow (a quarter of its area,
     * lattice units²).
     */
    pub glows: Vec<([i64; 3], u8, f32)>,
}

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
            glows: Vec::new(),
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

    /** The material of face `kind` (of bake's VOXEL_FACES) of the voxel at (x, y, z); 0 if empty, outside the box or not a site. */
    pub fn face(&self, x: i64, y: i64, z: i64, kind: usize) -> u8 {
        match self.slot(x, y, z) {
            Some(Ok(i)) if self.cells[i] != 0 => self.faces.get(&i).map_or(self.cells[i], |faces| faces[kind]),
            _ => 0,
        }
    }

    /** The faces' materials of the voxel at (x, y, z), if they are not all its site's. */
    pub fn faces_at(&self, x: i64, y: i64, z: i64) -> Option<&Faces> {
        match self.slot(x, y, z) {
            Some(Ok(i)) => self.faces.get(&i),
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
            Some(Err(brick)) => {
                self.bricks[brick] = (self.cells.len() / BRICK_SITES) as i32;
                self.cells.resize(self.cells.len() + BRICK_SITES, 0);
                self.slot(x, y, z).and_then(Result::ok).expect("brick just added")
            }
        };
        self.cells[i] = id;
        if !self.faces.is_empty() {
            self.faces.remove(&i);
        }
    }

    pub fn into_grid(self) -> Grid {
        Grid {
            min: self.min.map(|v| v as i32),
            n: self.n.map(|v| v as i32),
            bricks: self.bricks,
            cells: self.cells,
            faces: self.faces.iter().flat_map(|(i, faces)| face_words(*i, faces)).collect(),
        }
    }
}

/**
 * The grid of `b`, its bricks taken from `parts`: each a whole number of
 * bricks from the box's corner, no two with voxels in one brick of it
 * (VoxelGrid.compose in voxel-grid.ts). The rest of the box is empty.
 */
pub fn compose(b: &Box3, parts: &[&Grid]) -> Grid {
    let VoxelGrid { min, n, bx, by, mut bricks, mut cells, .. } = VoxelGrid::new(b);
    let mut faces: Vec<[u32; FACE_WORDS]> = Vec::new();
    let bz = (n[2] + BRICK - 1) / BRICK;
    for part in parts {
        let offset: [i64; 3] = std::array::from_fn(|i| {
            assert_eq!((part.min[i] as i64 - min[i]) % BRICK, 0, "a part is not a whole number of bricks from the box");
            (part.min[i] as i64 - min[i]) / BRICK
        });
        let [pbx, pby, pbz] = part.n.map(|v| (v as i64 + BRICK - 1) / BRICK);
        // Where each of the part's bricks of sites went, if it did.
        let mut moved = vec![None; part.cells.len() / BRICK_SITES];
        for k in 0..pbz {
            for j in 0..pby {
                for i in 0..pbx {
                    let at = part.bricks[((k * pby + j) * pbx + i) as usize];
                    let [gi, gj, gk] = [i + offset[0], j + offset[1], k + offset[2]];
                    if at < 0 || gi < 0 || gj < 0 || gk < 0 || gi >= bx || gj >= by || gk >= bz {
                        continue;
                    }
                    moved[at as usize] = Some(cells.len());
                    bricks[((gk * by + gj) * bx + gi) as usize] = (cells.len() / BRICK_SITES) as i32;
                    let from = at as usize * BRICK_SITES;
                    cells.extend_from_slice(&part.cells[from..from + BRICK_SITES]);
                }
            }
        }
        for &[i, words @ ..] in part.faces.as_chunks::<FACE_WORDS>().0 {
            if let Some(to) = moved[i as usize / BRICK_SITES] {
                faces.push([(to + i as usize % BRICK_SITES) as u32, words[0], words[1], words[2]]);
            }
        }
    }
    faces.sort_unstable_by_key(|site| site[0]);
    let (min, n) = (min.map(|v| v as i32), n.map(|v| v as i32));
    Grid { min, n, bricks, cells, faces: faces.concat() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn faces_are_set_with_their_site_and_go_with_it() {
        let mut grid = VoxelGrid::new(&Box3 { min: [0; 3], max: [16, 8, 8] });
        let faces: Faces = std::array::from_fn(|k| 2 + (k % 2) as u8);
        grid.set_faces(2, 2, 2, 2, faces);
        grid.set_faces(4, 2, 2, 2, [2; 12]);
        grid.set_faces(10, 2, 2, 3, faces);
        assert_eq!((grid.faces.len(), grid.face(2, 2, 2, 1), grid.face(4, 2, 2, 1), grid.get(2, 2, 2)), (2, 3, 2, 2));
        grid.set(2, 2, 2, 5);
        assert_eq!((grid.faces.len(), grid.face(2, 2, 2, 1)), (1, 5));

        // Composed into a box a brick wider, the bricks move, and their sites' faces with them.
        let whole = compose(&Box3 { min: [-8, 0, 0], max: [16, 8, 8] }, &[&grid.into_grid()]);
        assert_eq!([0, 1].map(|k| whole.face(10, 2, 2, k)), [2, 3]);
        assert_eq!([0, 1].map(|k| whole.face(2, 2, 2, k)), [5, 5]);
    }
}
