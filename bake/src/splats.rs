//! The voxels of a chunk that can show, each a hexagonal dab of paint
//! (a splat) at its site, in relief over its slice's centre plane.

use std::f64::consts::FRAC_1_SQRT_2;

use crate::slicing::{LAYER, MAX_SLANT, slice_centre};
use crate::surface::{Slice, Voxel};
use crate::world::{FACE_KINDS, Geometry};

/**
 * A voxel painted as a hexagon across the face's axis, at its site:
 * the points (x + u, y + v) with |u| ≤ width and |u| / √2 + |v| ≤ height,
 * its outline seen along the axis (width 1/√2, height 1, in units of its
 * grid), `depth` half layers past its slice's centre plane.
 * Seen along a ray slanting σ off the axis, the voxel's outline is
 * |σx| · width wider (its tips a layer before and after move sideways),
 * no higher in the face's rays; the compositor widens it so.
 */
pub struct Splat {
    pub x: f64,
    pub y: f64,
    pub depth: i32,
    pub width: f64,
    pub height: f64,
    /** Opaque RGBA (see voxel_colour). */
    pub colour: u32,
    /** Its voxel's lattice coordinates. */
    pub site: [i32; 3],
}

impl Splat {
    /** How far past its centre plane it is, in units of its grid. */
    pub fn offset(&self) -> f64 {
        self.depth as f64 * LAYER / 2.0
    }

    /** See reach. */
    pub fn reach(&self) -> (f64, f64) {
        reach(self.width, self.height, self.offset())
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
 * A voxel's colour: its material (that of its face towards the camera,
 * as Reader reads it) shaded as the mean of its open faces' shades (or
 * of its cut faces', if it has none), so a splat is shaded by the way its
 * surface faces. `palette` is a view face's (material · FACE_KINDS + kind
 * → RGBA); every view face's shades alike, by the way a face faces in the
 * world.
 */
pub fn voxel_colour(voxel: &Voxel, palette: &[u32]) -> u32 {
    let faces = if voxel.open != 0 { voxel.open } else { voxel.cut };
    let base = voxel.material as usize * FACE_KINDS;
    let mut sum = [0u32; 3];
    for kind in (0..FACE_KINDS).filter(|k| faces & (1 << k) != 0) {
        let colour = palette[base + kind];
        for (channel, total) in sum.iter_mut().enumerate() {
            *total += (colour >> (8 * channel)) & 255;
        }
    }
    let n = faces.count_ones().max(1);
    let [r, g, b] = sum.map(|total| (total + n / 2) / n);
    0xff00_0000 | (b << 16) | (g << 8) | r
}

/**
 * Whether a ray of a face (|x| + √2·|y| ≤ z, see inFace in view.ts) can
 * meet one of `faces` from outside: n · r < 0 at a corner of the rhombus
 * of rays, r = (±MAX_SLANT, 0, 1) or (0, ±MAX_SLANT · LAYER, 1).
 */
fn seen(geometry: &Geometry, faces: u16) -> bool {
    (0..FACE_KINDS).any(|kind| {
        let [nx, ny, nz] = geometry.normals[kind];
        faces & (1 << kind) != 0 && nz < MAX_SLANT * nx.abs().max(LAYER * ny.abs())
    })
}

/**
 * Fills `splats` with the voxels of a chunk of a slice that rays of the
 * face may reach, in painting order: far to near.
 * A voxel is painted if a ray of the face can meet one of its open or cut
 * faces (seen).
 *
 * Painting order: by layer, farthest first. A ray of the face passes from
 * a voxel into one of the next layer, never back: their common face's
 * normal is along the axis or (±1, ±√2, 1) / 2, and the rays are
 * |σx| + √2·|σy| ≤ 1. Voxels of one layer meet only side by side.
 *
 * `surface` is the cell's (cell_surface), with the margin around the cell
 * that its chunks may show; `palette` the view face's (see voxel_colour).
 */
pub fn chunk_splats(splats: &mut Splats, geometry: &Geometry, palette: &[u32], surface: &[Voxel], slice: &Slice) {
    splats.clear();
    // Twice the centre plane's depth, in layers: a whole number.
    let centre = (2.0 * slice_centre(slice.level, slice.layer) / LAYER).round() as i32;

    let mut order = std::mem::take(&mut splats.order);
    order.clear();
    order.extend(surface.iter().enumerate().map(|(k, voxel)| (voxel.site[2], k)));
    order.sort_unstable_by_key(|&(c, k)| (std::cmp::Reverse(c), k));

    for &(_, k) in &order {
        let voxel = &surface[k];
        if !seen(geometry, voxel.open | voxel.cut) {
            continue;
        }
        let [a, b, c] = voxel.site;
        splats.push(Splat {
            x: a as f64 * LAYER,
            y: b as f64,
            depth: 2 * c - centre,
            width: FRAC_1_SQRT_2,
            height: 1.0,
            colour: voxel_colour(voxel, palette),
            site: voxel.site,
        });
    }
    splats.order = order;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tables::geometry;
    use crate::world::lattices;

    #[test]
    fn a_slanted_ray_shows_the_voxel_it_meets_first() {
        // A solid block of level 2's slab of layers, its side a + c = 0
        // at 45° to the axis, as a tree trunk ahead does where four faces
        // meet; its inside buried, so only its surface is kept.
        let geometry = geometry(&lattices()[0]);
        let solid = |[a, b, c]: [i32; 3]| (0..4).contains(&c) && a.abs() <= 12 && b.abs() <= 12 && a + c >= 0;
        let sites: Vec<[i32; 3]> = (0..4)
            .flat_map(|c| (-12..=12).flat_map(move |b| (-12..=12).map(move |a| [a, b, c])))
            .filter(|&[a, b, c]| (a - b) % 2 == 0 && (b - c) % 2 == 0 && solid([a, b, c]))
            .collect();
        let surface: Vec<Voxel> = sites
            .iter()
            .map(|&site| {
                let open = (0..FACE_KINDS)
                    .filter(|&k| !solid(std::array::from_fn(|d| site[d] + geometry.offsets[k][d])))
                    .fold(0, |o, k| o | 1 << k);
                Voxel { site, material: 0, open, cut: 0 }
            })
            .filter(|v| v.open != 0)
            .collect();
        let slice = Slice { level: 2, layer: 0, cutoff: f64::INFINITY };
        let mut splats = Splats::new();
        chunk_splats(&mut splats, &geometry, &[0; FACE_KINDS], &surface, &slice);
        let centre = slice_centre(slice.level, slice.layer);

        let at = |[a, b, c]: [i32; 3]| [a as f64 * LAYER, b as f64, c as f64 * LAYER];
        // Where the ray through (px, py) on the centre plane along (sx, sy, 1)
        // enters a voxel, if it meets it: inside, n · (p − site) ≤ 1 for each
        // face's normal n.
        let enter = |px: f64, py: f64, [sx, sy]: [f64; 2], site: [i32; 3]| {
            let p = at(site);
            let (mut enter, mut exit) = (f64::NEG_INFINITY, f64::INFINITY);
            for n in geometry.normals {
                let along = n[0] * sx + n[1] * sy + n[2];
                let off = n[0] * (px - p[0]) + n[1] * (py - p[1]) + n[2] * (centre - p[2]);
                let t = (1.0 - off) / along;
                if along < 0.0 { enter = enter.max(t) } else { exit = exit.min(t) }
            }
            (enter < exit).then_some(enter)
        };
        let first = |px: f64, py: f64, slant: [f64; 2]| {
            let met = sites.iter().filter_map(|&s| enter(px, py, slant, s).map(|t| (t, s)));
            met.min_by(|a, b| a.0.total_cmp(&b.0)).map(|(_, site)| site)
        };
        let mut hits = 0;
        for slant in [[0.0, 0.0], [0.99, 0.0], [-0.99, 0.0], [0.5, 0.35], [-0.3, -0.45], [0.0, 0.7]] {
            for (px, py) in (0..60).flat_map(|i| (0..60).map(move |j| (-4.0 + i as f64 * 0.13, -4.0 + j as f64 * 0.13))) {
                let Some(site) = first(px, py, slant) else { continue };
                // Near an outline rays may disagree, as the hexagon holds the outline only.
                let near = [[0.2, 0.0], [-0.2, 0.0], [0.0, 0.2], [0.0, -0.2]];
                if near.iter().any(|[dx, dy]| first(px + dx, py + dy, slant) != Some(site)) {
                    continue;
                }
                hits += 1;
                // What shows there: the last splat whose hexagon, at its depth and
                // widened by the slant, holds where the ray crosses it.
                let shown = splats.splats.iter().rev().find(|s| {
                    let (x, y) = (px + slant[0] * s.offset(), py + slant[1] * s.offset());
                    let width = s.width * (1.0 + slant[0].abs());
                    (x - s.x).abs() <= width && (x - s.x).abs() * FRAC_1_SQRT_2 + (y - s.y).abs() <= s.height
                });
                let shown = shown.expect("a splat holds the point").site;
                // Or its neighbour in its layer: the order within a layer is the
                // same for every ray, and widened, their hexagons overlap.
                let beside = shown[2] == site[2] && shown[1] == site[1] && (shown[0] - site[0]).abs() == 2;
                assert!(shown == site || beside, "({px}, {py}) along {slant:?} shows {shown:?}, not {site:?}");
            }
        }
        assert!(hits > 5000, "{hits} rays met the block");
    }

    #[test]
    fn colours_a_voxel_by_its_open_faces_or_else_its_cut_ones() {
        // Material 1's kinds: kind k is grey k · 10.
        let palette: Vec<u32> = (0..2 * FACE_KINDS as u32).map(|i| 0xff00_0000 | ((i % 12 * 10) * 0x0001_0101)).collect();
        let voxel = |open: u16, cut: u16| Voxel { site: [0; 3], material: 1, open, cut };
        assert_eq!(voxel_colour(&voxel(0b1000_0100, 0b1), &palette), 0xff00_0000 | (45 * 0x0001_0101));
        assert_eq!(voxel_colour(&voxel(0, 0b11), &palette), 0xff00_0000 | (5 * 0x0001_0101));
    }
}
