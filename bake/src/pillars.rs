//! What a chunk shows of a cell, seen along its face's axis over its
//! chunk's plane: its whole voxels a pillar per column, as far as its side
//! may show, and its see-through ones as pictures of its medium, a
//! column's slab by slab.

use std::collections::HashMap;

use crate::face::Face;
use lattice::light::{FaceLight, colour};
use lattice::{ALPHA, FACE_KINDS, Look, VOXEL_FACES, alpha_of};

/** A voxel of a cell that shows: its site, its look along the face, and which of its faces show. Bit k of `open` and `cut` is face kind k. */
pub struct Voxel {
    /** Its lattice coordinates (a, b, c) along the face. */
    pub site: [i32; 3],
    /** Its look along the face: the mean of how it looks through the faces turned towards the camera (lattice's `facing`). */
    pub look: Look,
    /** Faces whose neighbour across them lets rays through: none, or a see-through one. */
    pub open: u8,
    /** Faces drawn because the chunk is cut there: their neighbour is in another chunk. */
    pub cut: u8,
}

impl Voxel {
    /** Of ALPHA, how much of the rays through its site it stops. */
    pub fn alpha(&self) -> u8 {
        alpha_of(self.look)
    }
}

/**
 * A column of a cell's relief: its first voxel a ray can enter, at its
 * site (a, b, c), `depth` half layers past its cell's middle plane,
 * standing for its column's voxels behind it too, `run` half layers deep:
 * a pillar along the axis.
 */
pub struct Pillar {
    pub depth: i32,
    /** How many half layers it reaches past its site. */
    pub run: i32,
    /** RGBA (see voxel_colour), its alpha its voxel's. */
    pub colour: u32,
    /** Its voxel's lattice coordinates. */
    pub site: [i32; 3],
}

/**
 * Brightness of a face by its outward normal in the world: the axes'
 * shades, so the top is brightest and the bottom darkest.
 */
fn face_shade(kind: usize) -> f32 {
    // −x, +x, −y, +y, −z, +z.
    [0.7, 0.8, 0.55, 1.0, 0.9, 0.65][kind]
}

/**
 * A voxel's colour: its look shaded as the mean of its open faces' shades
 * (or of its cut faces', if it has none), so a voxel is shaded by the way
 * its surface faces. Opaque.
 */
pub fn voxel_colour(voxel: &Voxel) -> u32 {
    let faces = if voxel.open != 0 { voxel.open } else { voxel.cut };
    let kinds = (0..FACE_KINDS).filter(|k| faces & (1 << k) != 0);
    let shade = kinds.clone().map(face_shade).sum::<f32>() / kinds.count().max(1) as f32;
    let [r, g, b, _] = voxel.look.to_le_bytes().map(|c| (c as f32 * shade).round() as u8);
    u32::from_le_bytes([r, g, b, 255])
}

/**
 * A lit voxel's colour: the light from its open faces (from its cut
 * faces, if it has none), each as much as it faces the camera, along
 * `towards`.
 */
pub fn lit_voxel_colour(voxel: &Voxel, light: &FaceLight, towards: [i32; 3]) -> u32 {
    let faces = if voxel.open != 0 { voxel.open } else { voxel.cut };
    let (mut sum, mut total) = ([0.0f32; 3], 0.0);
    for kind in (0..FACE_KINDS).filter(|k| faces & (1 << k) != 0) {
        let n = VOXEL_FACES[kind];
        // Faced away, it still counts a little: a voxel all of whose faces do keeps their light.
        let weight = (n[0] * towards[0] + n[1] * towards[1] + n[2] * towards[2]).max(0) as f32 + 0.25;
        (0..3).for_each(|c| sum[c] += weight * light[kind][c]);
        total += weight;
    }
    colour(sum.map(|c| if total > 0.0 { c / total } else { c }))
}

/**
 * The faces of a voxel a ray of `face` can meet from outside, as bits of
 * kinds: n · r < 0 for a ray r at a corner of its rays' tangents.
 */
pub fn seen(face: &Face) -> u8 {
    let mut faces = 0;
    for (k, n) in VOXEL_FACES.iter().enumerate() {
        let [x, y, z] = face.axes_of(n.map(f64::from));
        if face.corners().iter().any(|t| x * t[0] + y * t[1] + z < -1e-9) {
            faces |= 1 << k;
        }
    }
    faces
}

/**
 * The pillars of a relief of a cell's whole voxels `whole` along `face`,
 * `centre` twice the c of its plane: a pillar per column (a, b), from its
 * first voxel a ray of the face can enter (`seen`, by kinds), as deep as
 * its side may show. A ray leaving the column's side enters the column
 * beside it, so it sees the side only before that one's first whole voxel
 * there (`is_whole`, of any site, in the cell or not, as far as `limit`
 * along the axis): the pillar reaches the farthest of those, but no
 * farther than its own solid runs on from its front, past the cell too,
 * as the next cell's relief starts at what a ray enters, not at its cut.
 * What is see-through is no part of it, as if empty: cell_medium.
 */
pub fn cell_relief(
    face: &Face,
    seen: u8,
    colour: &mut dyn FnMut(&Voxel) -> u32,
    whole: &[Voxel],
    centre: i32,
    limit: i32,
    is_whole: &dyn Fn([i32; 3]) -> bool,
) -> Vec<Pillar> {
    let k = face.k();
    // Each column's first voxel a ray can enter: by column, near to far.
    let mut order: Vec<&Voxel> = whole.iter().filter(|v| v.open & seen != 0).collect();
    order.sort_unstable_by_key(|v| v.site);
    // The first whole site of a column from layer c on, to `limit`.
    let first_whole = |[a, b]: [i32; 2], c: i32| (0..).map(|j| c + j * k).take_while(|&c| c <= limit).find(|&c| is_whole([a, b, c]));
    // Beside a column: the neighbours across its voxels' faces, in lattice coordinates.
    let beside: Vec<[i32; 3]> = VOXEL_FACES.iter().map(|&n| face.of(n)).filter(|o| o[..2] != [0, 0]).collect();
    order
        .chunk_by(|v, w| v.site[..2] == w.site[..2])
        .map(|column| {
            let first = column[0];
            let [a, b, c] = first.site;
            // Its own solid, from its front on.
            let own = (1..).map(|j| c + j * k).take_while(|&c| c <= limit && is_whole([a, b, c])).last().unwrap_or(c);
            let covered = beside
                .iter()
                .map(|o| first_whole([a + o[0], b + o[1]], c + o[2]).map_or(i32::MAX, |there| there - o[2]))
                .max()
                .unwrap_or(i32::MAX);
            let end = own.min(covered).max(c);
            Pillar { depth: 2 * c - centre, run: 2 * (end - c), colour: colour(first), site: first.site }
        })
        .collect()
}

/**
 * A column of a picture of a cell's medium (Baker::bake_cell): a slab of
 * a column of its grid, its voxels in `slab` layers of it, drawn on the
 * slab's middle plane see-through, as much as they stop.
 * Each slab of a column is one cell's, the one holding its middle, so that
 * a slab's pictures, one per cell, tile its plane once.
 */
pub struct Medium {
    /** Its column (a, b), from its cell's column (0, 0). */
    pub column: [i32; 2],
    /** Its slab's first layer, past the cell's plane, in half layers: every cell's slab there starts there. */
    pub first: i32,
    /**
     * How much its voxels stop of a ray crossing each along an axis, as
     * optical depth: −ln of what gets through each, added up; of those in
     * its slab's nearer half and its farther.
     */
    pub depth: [f32; 2],
    /** Its voxels' colour, their mean by how much each stops (RGBA, see voxel_colour, its alpha whole). */
    pub colour: u32,
}

/** The middle site's layer of the slab of `slab` layers holding layer c of column a, whose sites are c ≡ a (mod k). */
pub fn slab_middle(c: i32, a: i32, slab: i32, k: i32) -> i32 {
    let m = (c - 1).div_euclid(slab) * slab + slab / 2;
    m + (a - m).rem_euclid(k)
}

/**
 * The medium of the see-through voxels of `voxels` a ray of the face can
 * enter (`seen`), their slabs of `slab` layers by column, the first from
 * layer 1, those whose middle site `owns` says is the cell's, `centre` as
 * cell_relief takes it, `least` the cell's column (0, 0); by slab, row b and a.
 */
#[allow(clippy::too_many_arguments)]
pub fn cell_medium(
    seen: u8,
    k: i32,
    colour: &mut dyn FnMut(&Voxel) -> u32,
    voxels: &[Voxel],
    centre: i32,
    slab: i32,
    least: [i32; 2],
    owns: &dyn Fn(i32, i32, i32) -> bool,
) -> Vec<Medium> {
    // By slab and column: the optical depths of its halves, and the colours times them.
    let mut slabs: HashMap<[i32; 3], ([f64; 2], [f64; 3])> = HashMap::new();
    for v in voxels.iter().filter(|v| v.alpha() != ALPHA && v.alpha() != 0 && v.open & seen != 0) {
        let [a, b, c] = v.site;
        let depth = -(1.0 - f64::from(v.alpha()) / f64::from(ALPHA)).ln();
        let rgb = colour(v);
        let (s, into) = ((c - 1).div_euclid(slab), (c - 1).rem_euclid(slab));
        let (depths, colours) = slabs.entry([s, b, a]).or_default();
        depths[usize::from(2 * into >= slab)] += depth;
        for (n, sum) in colours.iter_mut().enumerate() {
            *sum += depth * f64::from((rgb >> (8 * n)) & 0xff);
        }
    }
    // By slab, row and column.
    let mut kept: Vec<[i32; 3]> = slabs.keys().copied().filter(|&[s, b, a]| owns(a, b, slab_middle(s * slab + 1, a, slab, k))).collect();
    kept.sort_unstable();
    kept.into_iter()
        .map(|[s, b, a]| {
            let (depths, colours) = slabs[&[s, b, a]];
            let all: f64 = depths.iter().sum();
            let channel = |n: usize| ((colours[n] / all.max(f64::MIN_POSITIVE)).round() as u32).min(255) << (8 * n);
            Medium {
                column: [a - least[0], b - least[1]],
                first: 2 * (s * slab + 1) - centre,
                depth: depths.map(|d| d as f32),
                colour: channel(0) | channel(1) | channel(2) | 0xff << 24,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::face::faces;

    #[test]
    fn colours_a_voxel_by_its_open_faces_or_else_its_cut_ones() {
        let voxel = |open: u8, cut: u8| Voxel { site: [0; 3], look: 0xff00_0000 | (100 * 0x0001_0101), open, cut };
        // Its top and its −x side: their mean; else its cut bottom.
        let mean = ((face_shade(3) + face_shade(0)) / 2.0 * 100.0).round() as u32;
        assert_eq!(voxel_colour(&voxel(0b1001, 0b100)), 0xff00_0000 | (mean * 0x0001_0101));
        assert_eq!(voxel_colour(&voxel(0, 0b100)), 0xff00_0000 | (55 * 0x0001_0101));
    }

    #[test]
    fn a_face_sees_the_faces_turned_towards_it_and_beside_it_not_away() {
        for face in faces() {
            let seen = seen(&face);
            for (k, n) in VOXEL_FACES.iter().enumerate() {
                let towards = n[0] * face.f[0] + n[1] * face.f[1] + n[2] * face.f[2];
                // Turned towards the camera, seen; away, not; beside the axis, seen by the rays slanting towards it.
                assert_eq!(seen & (1 << k) != 0, towards <= 0, "{face:?} kind {k}");
            }
        }
    }

    #[test]
    fn a_slabs_middle_is_a_site_of_its_column_within_it() {
        for k in [1i32, 2] {
            for slab in [2, 4, 8] {
                for c in -20i32..20 {
                    for a in -3i32..3 {
                        if (a - c).rem_euclid(k) != 0 {
                            continue;
                        }
                        let m = slab_middle(c, a, slab, k);
                        let first = (c - 1).div_euclid(slab) * slab + 1;
                        assert!((m - a).rem_euclid(k) == 0 && m >= first && m < first + slab, "k {k} slab {slab} c {c}: {m}");
                    }
                }
            }
        }
    }
}
