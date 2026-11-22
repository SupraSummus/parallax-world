//! What a chunk shows of a cell, seen along the face's axis, in relief
//! over its chunk's plane: its whole voxels a pillar per column, as far as
//! its side may show, and its see-through ones as pictures of its medium,
//! a column's slab by slab.

use std::collections::HashMap;
use std::f64::consts::FRAC_1_SQRT_2;

use crate::face::{LAYER, MAX_SLANT};
use crate::surface::Voxel;
use fcc::light::{FaceLight, colour};
use fcc::{ALPHA, FACE_KINDS, VOXEL_FACES};

use crate::tables::Geometry;

/**
 * A column of a cell's relief: its first voxel a ray can enter, at its
 * site (a, b, c), `depth` half layers past its cell's centre plane, its
 * outline seen along the axis a hexagon (the points (a·LAYER + u, b + v)
 * with |u| ≤ 1/√2 and |u| / √2 + |v| ≤ 1, in units of its grid), standing for its column's
 * voxels behind it too, `run` half layers deep: a pillar along the axis.
 * Seen along a ray slanting σ off the axis, a voxel's outline is |σx| / √2
 * wider (its tips a layer before and after move sideways), no higher in
 * the face's rays.
 */
pub struct Pillar {
    pub depth: i32,
    /** How many half layers it reaches past its site. */
    pub run: i32,
    /** RGBA (see voxel_colour), its alpha its voxel's (Voxel). */
    pub colour: u32,
    /** Its voxel's lattice coordinates. */
    pub site: [i32; 3],
}

impl Pillar {
    /**
     * The half width and height of the hexagon holding where it may be
     * drawn on its plane, for any ray of the face: seen along a ray
     * slanting σ off the axis, |σx| + √2·|σy| ≤ MAX_SLANT, a voxel `offset`
     * past the plane is drawn σ · offset off its site, |σx| / √2 wider; of
     * its ends, the one that moves more.
     */
    pub fn reach(&self) -> (f64, f64) {
        let offset = (self.depth.abs().max((self.depth + self.run).abs())) as f64 * LAYER / 2.0;
        (FRAC_1_SQRT_2 + MAX_SLANT * (FRAC_1_SQRT_2 + offset), 1.0 + MAX_SLANT * offset * FRAC_1_SQRT_2)
    }
}

/** The pillars of one chunk, and the lists that make them, kept for their memory. */
#[derive(Default)]
pub struct Pillars {
    pub pillars: Vec<Pillar>,
    /** Painting order: each surface voxel's layer, and its index. */
    order: Vec<(i32, usize)>,
    /** A voxel's index by column (a, b), to keep the first. */
    columns: HashMap<(i32, i32), usize>,
}

impl Pillars {
    /** The top-left corner of where its pillars may be drawn (Pillar::reach), in units of their grid; none, if it has none. */
    pub fn corner(&self) -> Option<[f64; 2]> {
        let corners = self.pillars.iter().map(|p| {
            let (width, height) = p.reach();
            [p.site[0] as f64 * LAYER - width, p.site[1] as f64 + height]
        });
        corners.reduce(|[x, y], [u, v]| [x.min(u), y.max(v)])
    }
}

/**
 * A voxel's colour: its look (through its face towards the camera, as
 * Reader reads it) shaded as the mean of its open faces' shades (or of
 * its cut faces', if it has none), so a voxel is shaded by the way its
 * surface faces; `shades` are a view face's (tables::shades). Opaque.
 */
pub fn voxel_colour(voxel: &Voxel, shades: &[f32; FACE_KINDS]) -> u32 {
    let faces = if voxel.open != 0 { voxel.open } else { voxel.cut };
    let kinds = (0..FACE_KINDS).filter(|k| faces & (1 << k) != 0);
    let shade = kinds.clone().map(|k| shades[k]).sum::<f32>() / kinds.count().max(1) as f32;
    let [r, g, b, _] = voxel.look.to_le_bytes().map(|c| (c as f32 * shade).round() as u8);
    u32::from_le_bytes([r, g, b, 255])
}

/**
 * A lit voxel's colour: the light from its open faces (from its cut
 * faces, if it has none), each as much as it faces the camera, along
 * `towards`; `kinds` takes its faces' kinds to the world's, as `light`
 * has them.
 */
pub fn lit_voxel_colour(voxel: &Voxel, light: &FaceLight, kinds: &[usize; FACE_KINDS], towards: [i32; 3]) -> u32 {
    let faces = if voxel.open != 0 { voxel.open } else { voxel.cut };
    let (mut sum, mut total) = ([0.0f32; 3], 0.0);
    for kind in (0..FACE_KINDS).filter(|k| faces & (1 << k) != 0) {
        let n = VOXEL_FACES[kinds[kind]];
        // Faced away, it still counts a little: a voxel all of whose faces do keeps their light.
        let weight = (n[0] * towards[0] + n[1] * towards[1] + n[2] * towards[2]).max(0) as f32 + 0.25;
        (0..3).for_each(|c| sum[c] += weight * light[kinds[kind]][c]);
        total += weight;
    }
    colour(sum.map(|c| if total > 0.0 { c / total } else { c }))
}

/**
 * Whether a ray of a face (|x| + √2·|y| ≤ z, see in_face in native's view.rs) can
 * meet one of `faces` from outside: n · r < 0 at a corner of the rhombus
 * of rays, r = (±MAX_SLANT, 0, 1) or (0, ±MAX_SLANT · LAYER, 1). The
 * normal is (a · LAYER, b, c · LAYER) of the face's offset, so LAYER
 * drops out.
 */
fn seen(geometry: &Geometry, faces: u16) -> bool {
    (0..FACE_KINDS).any(|kind| {
        let [a, b, c] = geometry.offsets[kind];
        faces & (1 << kind) != 0 && f64::from(c) < MAX_SLANT * f64::from(a.abs().max(b.abs()))
    })
}

/**
 * Fills `pillars` with a relief of a cell's whole voxels (lod.rs), `centre`
 * twice the layer of its plane: a pillar per column (a, b), its first voxel a
 * ray of the face can enter (seen), a pillar from there as deep as its
 * side may show: at most to the column's last voxel, and on as far as the
 * solid behind it `runs_on` past the cell, as the next cell's relief
 * starts at what a ray enters, not at its cut. A slanted ray sees the
 * column as the axis does, wherever it meets the pillar. What is
 * see-through is no part of it, as if empty: cell_medium.
 */
pub fn cell_relief(
    pillars: &mut Pillars,
    geometry: &Geometry,
    colour: &mut dyn FnMut(&Voxel) -> u32,
    whole: &[Voxel],
    centre: i32,
    runs_on: &dyn Fn([i32; 3]) -> i32,
) {
    pillars.pillars.clear();
    let mut order = std::mem::take(&mut pillars.order);
    order.clear();
    // Column by column, near to far.
    order.extend(whole.iter().enumerate().map(|(k, voxel)| (voxel.site[2], k)));
    order.sort_unstable_by_key(|&(c, k)| (whole[k].site[0], whole[k].site[1], c));
    // Each column's first voxel, its last, and its colour.
    let mut reliefs = Vec::new();
    for column in order.chunk_by(|&(_, j), &(_, k)| whole[j].site[..2] == whole[k].site[..2]) {
        let mut voxels = column.iter().map(|&(_, k)| &whole[k]).skip_while(|v| !seen(geometry, v.open));
        let Some(first) = voxels.next() else { continue };
        reliefs.push((first.site, voxels.last().unwrap_or(first).site, colour(first)));
    }
    let mut fronts = std::mem::take(&mut pillars.columns);
    fronts.clear();
    fronts.extend(reliefs.iter().enumerate().map(|(k, (site, ..))| ((site[0], site[1]), k)));
    for &(site, last, paint) in &reliefs {
        let [a, b, c] = site;
        // A ray leaving the column's side enters a column of its voxels'
        // neighbours, so it sees the side only before that one's front: the
        // pillar reaches the farthest of theirs, or where one has none here,
        // as far as its solid goes.
        let farthest = geometry
            .offsets
            .iter()
            .filter(|o| o[..2] != [0, 0])
            .map(|o| fronts.get(&(a + o[0], b + o[1])).map_or(i32::MAX, |&k| reliefs[k].0[2]));
        let reach = farthest.max().unwrap_or(i32::MAX);
        let end = if reach <= last[2] { reach.max(c) } else { runs_on(last).min(reach) };
        pillars.pillars.push(Pillar { depth: 2 * c - centre, run: 2 * (end - c), colour: paint, site });
    }
    pillars.columns = fronts;
    pillars.order = order;
}

/**
 * A column of a picture of a cell's medium (Baker::bake_cell): a slab of
 * a column of its grid, its voxels in `slab` layers of it, drawn on the
 * slab's middle plane see-through, as much as they stop.
 * Each slab of a column is one cell's, the one holding its middle, so that
 * a slab's pictures, one per cell, tile its plane once.
 */
pub struct Medium {
    /** Its column (a, b), from the least of its cell's bounds. */
    pub column: [i32; 2],
    /** Its slab's first layer, past the cell's plane, in half layers: every cell's slab there starts there. */
    pub first: i32,
    /**
     * How much its voxels stop of a ray crossing each along its mean
     * chord (worldgen's CHORD), as optical depth: −ln of what gets through
     * each, added up; of those in its slab's nearer half and its farther.
     */
    pub depth: [f32; 2],
    /** Its voxels' colour, their mean by how much each stops (RGBA, see voxel_colour, its alpha whole). */
    pub colour: u32,
}

/** The middle site of the slab of `slab` layers holding layer c of a column of row b, whose sites are c of b's parity. */
pub(crate) fn slab_middle(c: i32, b: i32, slab: i32) -> i32 {
    let c = (c - 1).div_euclid(slab) * slab + slab / 2;
    c + ((c - b) & 1)
}

/**
 * Fills `medium` with the see-through voxels of `voxels` a ray of the face
 * can enter (seen), their slabs of `slab` layers by column, the first from
 * layer 1, those whose middle site `owns` says is the cell's, `centre`
 * as cell_relief takes it; by slab, row b and a.
 */
pub fn cell_medium(
    medium: &mut Vec<Medium>,
    geometry: &Geometry,
    colour: &mut dyn FnMut(&Voxel) -> u32,
    voxels: &[Voxel],
    centre: i32,
    slab: i32,
    owns: &dyn Fn(i32, i32, i32) -> bool,
) {
    medium.clear();
    // By slab and column: the optical depths of its halves, and the colours times them.
    let mut slabs: HashMap<[i32; 3], ([f64; 2], [f64; 3])> = HashMap::new();
    for v in voxels.iter().filter(|v| v.alpha() != ALPHA && v.alpha() != 0 && seen(geometry, v.open)) {
        let [a, b, c] = v.site;
        let depth = -(1.0 - f64::from(v.alpha()) / f64::from(ALPHA)).ln();
        let rgb = colour(v);
        let (k, into) = ((c - 1).div_euclid(slab), (c - 1).rem_euclid(slab));
        let (depths, colours) = slabs.entry([k, b, a]).or_default();
        depths[usize::from(2 * into >= slab)] += depth;
        for (k, sum) in colours.iter_mut().enumerate() {
            *sum += depth * f64::from((rgb >> (8 * k)) & 0xff);
        }
    }
    // By slab, row and column.
    let mut kept: Vec<[i32; 3]> = slabs.keys().copied().filter(|&[k, b, a]| owns(a, b, slab_middle(k * slab + 1, b, slab))).collect();
    kept.sort_unstable();
    for [k, b, a] in kept {
        let (depths, colours) = slabs[&[k, b, a]];
        let all: f64 = depths.iter().sum();
        let channel = |k: usize| ((colours[k] / all.max(f64::MIN_POSITIVE)).round() as u32).min(255) << (8 * k);
        medium.push(Medium {
            column: [a, b],
            first: 2 * (k * slab + 1) - centre,
            depth: depths.map(|d| d as f32),
            colour: channel(0) | channel(1) | channel(2) | 0xff << 24,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colours_a_voxel_by_its_open_faces_or_else_its_cut_ones() {
        // Kind k shades by k / 10, a grey of 100.
        let shades: [f32; FACE_KINDS] = std::array::from_fn(|k| k as f32 / 10.0);
        let voxel = |open: u16, cut: u16| Voxel { site: [0; 3], look: 0xff00_0000 | (100 * 0x0001_0101), open, cut };
        assert_eq!(voxel_colour(&voxel(0b1000_0100, 0b1), &shades), 0xff00_0000 | (45 * 0x0001_0101));
        assert_eq!(voxel_colour(&voxel(0, 0b11), &shades), 0xff00_0000 | (5 * 0x0001_0101));
    }
}
