//! The voxels of a chunk the face's axis sees, each a hexagonal dab of
//! paint (a splat) at its site, in relief over its chunk's plane:
//! a slice's swept over its column's voxels behind it,
//! a cell's whole ones one per column, as far as its side may show,
//! or each; and a cell's see-through ones as pictures of its medium,
//! a column's slab by slab.

use std::collections::HashMap;
use std::f64::consts::FRAC_1_SQRT_2;

use crate::face::{LAYER, MAX_SLANT};
use crate::surface::Voxel;
use fcc::light::{FaceLight, colour};
use fcc::{ALPHA, FACE_KINDS, VOXEL_FACES};

use crate::tables::Geometry;

/**
 * A voxel painted as a hexagon across the face's axis, at its site:
 * the points (x + u, y + v) with |u| ≤ width and |u| / √2 + |v| ≤ height,
 * its outline seen along the axis (width 1/√2, height 1, in units of its
 * grid), `depth` half layers past its slice's centre plane.
 * Seen along a ray slanting σ off the axis, the voxel's outline is
 * |σx| · width wider (its tips a layer before and after move sideways),
 * no higher in the face's rays; the compositor widens it so.
 * With `run` > 0 it stands for its column's voxels behind it too, that
 * many half layers deep: a pillar along the axis (see chunk_splats).
 */
pub struct Splat {
    pub x: f64,
    pub y: f64,
    pub depth: i32,
    /** How many half layers its pillar reaches past its site. */
    pub run: i32,
    pub width: f64,
    /** RGBA (see voxel_colour), its alpha its voxel's (Voxel). */
    pub colour: u32,
    /** Its voxel's lattice coordinates. */
    pub site: [i32; 3],
}

impl Splat {
    /** √2 times its width, as a voxel's outline is (Baker::numbers leaves it out). */
    pub fn height(&self) -> f64 {
        self.width * std::f64::consts::SQRT_2
    }

    /** How far past its centre plane it is, in units of its grid. */
    pub fn offset(&self) -> f64 {
        self.depth as f64 * LAYER / 2.0
    }

    /** See reach; its pillar's farther end, if it moves more. */
    pub fn reach(&self) -> (f64, f64) {
        let far = (self.depth + self.run) as f64 * LAYER / 2.0;
        reach(self.width, self.height(), if far.abs() > self.offset().abs() { far } else { self.offset() })
    }
}

/**
 * The width and height of the hexagon holding where a splat `offset` past
 * its centre plane may be drawn on that plane, for any ray of the face:
 * seen along a ray slanting σ off the axis, |σx| + √2·|σy| ≤ MAX_SLANT,
 * it is drawn σ · offset off its site, |σx| · width wider.
 */
pub fn reach(width: f64, height: f64, offset: f64) -> (f64, f64) {
    (width + MAX_SLANT * (width + offset.abs()), height + MAX_SLANT * offset.abs() * FRAC_1_SQRT_2)
}

/** The splats of one chunk, far to near, and the extent of their reach. */
pub struct Splats {
    pub splats: Vec<Splat>,
    /** Painting order: each surface voxel's layer, and its index. */
    order: Vec<(i32, usize)>,
    /** A voxel's index by column (a, b), to keep the first. */
    columns: HashMap<(i32, i32), usize>,
    pub min_x: f64,
    pub max_x: f64,
    pub min_y: f64,
    pub max_y: f64,
}

impl Splats {
    pub fn new() -> Splats {
        Splats {
            splats: Vec::new(),
            order: Vec::new(),
            columns: HashMap::new(),
            min_x: f64::INFINITY,
            max_x: f64::NEG_INFINITY,
            min_y: f64::INFINITY,
            max_y: f64::NEG_INFINITY,
        }
    }

    /** Empties it, keeping the lists' memory. */
    fn clear(&mut self) {
        self.splats.clear();
        (self.min_x, self.max_x, self.min_y, self.max_y) = (f64::INFINITY, f64::NEG_INFINITY, f64::INFINITY, f64::NEG_INFINITY);
    }

    fn push(&mut self, splat: Splat) {
        let (width, height) = splat.reach();
        self.min_x = self.min_x.min(splat.x - width);
        self.max_x = self.max_x.max(splat.x + width);
        self.min_y = self.min_y.min(splat.y - height);
        self.max_y = self.max_y.max(splat.y + height);
        self.splats.push(splat);
    }
}

/**
 * A voxel's colour: its look (through its face towards the camera, as
 * Reader reads it) shaded as the mean of its open faces' shades (or of
 * its cut faces', if it has none), so a splat is shaded by the way its
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
 * Fills `splats` with a relief of a cell's whole voxels (lod.rs), `centre`
 * as chunk_splats takes it: a splat per column (a, b), its first voxel a
 * ray of the face can enter (seen), a pillar from there as deep as its
 * side may show: at most to the column's last voxel, and on as far as the
 * solid behind it `runs_on` past the cell, as the next cell's relief
 * starts at what a ray enters, not at its cut. A slanted ray sees the
 * column as the axis does, wherever it meets the pillar. What is
 * see-through is no part of it, as if empty: cell_splats.
 */
pub fn cell_relief(
    splats: &mut Splats,
    geometry: &Geometry,
    colour: &mut dyn FnMut(&Voxel) -> u32,
    whole: &[Voxel],
    centre: i32,
    runs_on: &dyn Fn([i32; 3]) -> i32,
) {
    splats.clear();
    let mut order = std::mem::take(&mut splats.order);
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
    let mut fronts = std::mem::take(&mut splats.columns);
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
        splats.push(Splat {
            x: a as f64 * LAYER,
            y: b as f64,
            depth: 2 * c - centre,
            run: 2 * (end - c),
            width: FRAC_1_SQRT_2,
            colour: paint,
            site,
        });
    }
    splats.columns = fronts;
    splats.order = order;
}

/**
 * Fills `splats` with the voxels of a cell (lod.rs), of `voxels`, a ray of
 * the face can enter, by an open face (seen), each a hexagon at its site,
 * `centre` as chunk_splats takes it, no run: its see-through ones, which a
 * ray meets one after another, so each is drawn in the order it does (the
 * native compositor); or all of them, where its whole ones are no relief.
 * A ray's first whole voxel is entered so and is in one cell: its hexagon,
 * holding its outline, meets the ray, and depth decides between cells'.
 */
pub fn cell_splats(splats: &mut Splats, geometry: &Geometry, colour: &mut dyn FnMut(&Voxel) -> u32, voxels: &[Voxel], centre: i32) {
    splats.clear();
    for voxel in voxels.iter().filter(|v| seen(geometry, v.open)) {
        let [a, b, c] = voxel.site;
        splats.push(Splat {
            x: a as f64 * LAYER,
            y: b as f64,
            depth: 2 * c - centre,
            run: 0,
            width: FRAC_1_SQRT_2,
            colour: colour(voxel),
            site: voxel.site,
        });
    }
}

/**
 * A column of a picture of a cell's medium (Baker::bake_cell): a slab of
 * a column of its grid, its voxels in `slab` layers of it, drawn on the
 * slab's middle plane see-through, as much as they stop.
 * Each slab of a column is one cell's, the one holding its middle, so that
 * a slab's pictures, one per cell, tile its plane once; a picture holds
 * its neighbours around its own too, so that its pixels may take each
 * column's and its neighbours' by how near they are, across cells.
 */
pub struct Medium {
    /** Its column (a, b), from the least of its cell's bounds. */
    pub column: [i32; 2],
    /** Its slab's first layer, past the cell's plane, in half layers: every cell's slab there starts there. */
    pub first: i32,
    /** Whether the cell's own, or its neighbour's. */
    pub own: bool,
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
 * layer 1, those whose middle site `owns` says is the cell's, and their
 * neighbours, `centre` as cell_splats takes it; by slab, row b and a.
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
    let own = |[k, b, a]: [i32; 3]| owns(a, b, slab_middle(k * slab + 1, b, slab));
    // Its own slabs of columns where the field is: by any with medium, which reaches as far as their neighbours' middles (veil_fragment); their neighbours, for their pixels' field.
    const NEIGHBOURS: [[i32; 2]; 6] = [[2, 0], [-2, 0], [1, 1], [-1, 1], [1, -1], [-1, -1]];
    let near = |[k, b, a]: [i32; 3]| std::iter::once([k, b, a]).chain(NEIGHBOURS.map(|[da, db]| [k, b + db, a + da]));
    let mut kept: Vec<([i32; 3], bool)> = slabs.keys().flat_map(|&key| near(key)).filter(|&key| own(key)).map(|key| (key, true)).collect();
    kept.sort_unstable();
    kept.dedup();
    let owned: Vec<[i32; 3]> = kept.iter().map(|k| k.0).collect();
    kept.extend(owned.into_iter().flat_map(near).filter(|key| slabs.contains_key(key) && !own(*key)).map(|key| (key, false)));
    // By slab, row and column.
    kept.sort_unstable();
    kept.dedup();
    for ([k, b, a], own) in kept {
        let (depths, colours) = slabs.get(&[k, b, a]).copied().unwrap_or_default();
        let all: f64 = depths.iter().sum();
        let channel = |k: usize| ((colours[k] / all.max(f64::MIN_POSITIVE)).round() as u32).min(255) << (8 * k);
        medium.push(Medium {
            column: [a, b],
            first: 2 * (k * slab + 1) - centre,
            own,
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
