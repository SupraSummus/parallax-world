//! The voxels of a chunk that can show, each a hexagonal dab of paint
//! (a splat) on its slice's centre plane.

use std::f64::consts::FRAC_1_SQRT_2;

use crate::slicing::{LAYER, MAX_SLANT, slice_centre};
use crate::surface::{Slice, Voxel};
use crate::world::{FACE_KINDS, Geometry};

/**
 * A voxel painted as a hexagon on a slice's centre plane:
 * the points (x + u, y + v) with |u| ≤ width and |u| / √2 + |v| ≤ height,
 * its outline seen along the face's axis (width 1/√2, height 1, in units
 * of its grid). Grown, its sides move out.
 */
pub struct Splat {
    /** Its centre: its site, seen along the axis. */
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    /** Opaque RGBA (see voxel_colour). */
    pub colour: u32,
    /** Its voxel's lattice coordinates. */
    pub site: [i32; 3],
}

/**
 * The splats of one chunk and their extent.
 * `painted` are the voxels rays of the face may reach, far to near;
 * `grown` are voxels going on into another slice, their hexagons grown
 * to close the gaps between slices, also far to near:
 * drawn only where the chunk paints nothing (docs/architecture.md, section 3).
 */
pub struct Splats {
    pub painted: Vec<Splat>,
    pub grown: Vec<Splat>,
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
            painted: Vec::new(),
            grown: Vec::new(),
            order: Vec::new(),
            min_x: f64::INFINITY,
            max_x: f64::NEG_INFINITY,
            min_y: f64::INFINITY,
            max_y: f64::NEG_INFINITY,
        }
    }

    /** Empties it, keeping the lists' memory. */
    fn clear(&mut self) {
        self.painted.clear();
        self.grown.clear();
        (self.min_x, self.max_x, self.min_y, self.max_y) = (f64::INFINITY, f64::NEG_INFINITY, f64::INFINITY, f64::NEG_INFINITY);
    }

    fn extend(&mut self, splat: &Splat) {
        self.min_x = self.min_x.min(splat.x - splat.width);
        self.max_x = self.max_x.max(splat.x + splat.width);
        self.min_y = self.min_y.min(splat.y - splat.height);
        self.max_y = self.max_y.max(splat.y + splat.height);
    }
}

/**
 * A voxel's colour, the same from every side: the mean of its open faces'
 * shaded colours (or of its cut faces', if it has none), so a splat is
 * shaded by the way its surface faces. `palette` is a view face's
 * (material · FACE_KINDS + kind → RGBA); every view face's gives the same,
 * as it shades a face by the way it faces in the world.
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
 * Slices are drawn as seen along the face's axis (orthographically): a
 * voxel's splat is its outline along the axis, centred on its site.
 * A voxel is painted if a ray of the face can meet one of its open or cut
 * faces (seen); along the axis, nearer voxels cover what they hide.
 *
 * Painting order: by layer, farthest first. Voxels of one layer do not
 * overlap seen along the axis; of two that do, the one in the farther
 * layer is behind.
 *
 * If `fill`, voxels with an open face and a neighbour in another slice,
 * where gaps open between slices (docs/architecture.md, section 3),
 * are also grown.
 *
 * `surface` is the cell's (cell_surface), with the margin around the cell
 * that its chunks may show; `palette` the view face's (see voxel_colour).
 */
pub fn chunk_splats(splats: &mut Splats, geometry: &Geometry, palette: &[u32], surface: &[Voxel], slice: &Slice, fill: bool) {
    splats.clear();
    let centre = slice_centre(slice.level, slice.layer);

    let mut order = std::mem::take(&mut splats.order);
    order.clear();
    order.extend(surface.iter().enumerate().map(|(k, voxel)| (voxel.site[2], k)));
    order.sort_unstable_by_key(|&(c, k)| (std::cmp::Reverse(c), k));

    for &(_, k) in &order {
        let voxel = &surface[k];
        let painted = seen(geometry, voxel.open | voxel.cut);
        let grows = fill && voxel.cut != 0 && seen(geometry, voxel.open);
        if !painted && !grows {
            continue;
        }
        let [a, b, c] = voxel.site;
        let splat = Splat {
            x: a as f64 * LAYER,
            y: b as f64,
            width: FRAC_1_SQRT_2,
            height: 1.0,
            colour: voxel_colour(voxel, palette),
            site: voxel.site,
        };
        if grows {
            // The camera sees a point along a ray at most MAX_SLANT off the
            // axis (as a tangent), so one d ahead of the centre plane is
            // drawn at most MAX_SLANT · d off (section 2). Where its voxel
            // meets one in another slice, within a layer of its own, that
            // may pull it away from the other slice's drawing of the surface,
            // so its sides move out by that much: the slanted ones, with
            // normals (±1/√2, ±1) / √(3/2), by √(3/2) as much in `height`.
            let d = ((c + 1) as f64 * LAYER - centre).abs().max(((c - 1) as f64 * LAYER - centre).abs());
            let grow = MAX_SLANT * d;
            let grown = Splat { width: splat.width + grow, height: splat.height + grow * 1.5f64.sqrt(), site: voxel.site, ..splat };
            splats.extend(&grown);
            splats.grown.push(grown);
        }
        if painted {
            splats.extend(&splat);
            splats.painted.push(splat);
        }
    }
    splats.order = order;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tables::geometry;
    use crate::world::lattices;

    #[test]
    fn a_ray_along_the_axis_shows_the_voxel_it_meets_first() {
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
        chunk_splats(&mut splats, &geometry, &[0; FACE_KINDS], &surface, &slice, false);

        let at = |[a, b, c]: [i32; 3]| [a as f64 * LAYER, b as f64, c as f64 * LAYER];
        let mut hits = 0;
        for (px, py) in (0..80).flat_map(|i| (0..80).map(move |j| (-4.0 + i as f64 * 0.1, -4.0 + j as f64 * 0.1))) {
            // The voxel the ray (px, py, z) meets first, z growing: inside one,
            // n · (p − site) ≤ 1 for each face's normal n.
            let enter = |&site: &[i32; 3]| {
                let p = at(site);
                let (mut enter, mut exit) = (f64::NEG_INFINITY, f64::INFINITY);
                for n in geometry.normals {
                    let off = n[0] * (px - p[0]) + n[1] * (py - p[1]) - n[2] * p[2];
                    if n[2] == 0.0 {
                        if off > 1.0 {
                            return None;
                        }
                        continue;
                    }
                    let z = (1.0 - off) / n[2];
                    if n[2] < 0.0 { enter = enter.max(z) } else { exit = exit.min(z) }
                }
                (enter < exit).then_some(enter)
            };
            let Some((_, site)) = sites.iter().filter_map(|s| enter(s).map(|z| (z, *s))).min_by(|a, b| a.0.total_cmp(&b.0)) else {
                continue;
            };
            hits += 1;
            // What shows there: the last splat painted that holds the point.
            let holds =
                |s: &&Splat| (px - s.x).abs() <= s.width + 1e-9 && (px - s.x).abs() * FRAC_1_SQRT_2 + (py - s.y).abs() <= s.height + 1e-9;
            let shown = splats.painted.iter().rev().find(holds).expect("a splat holds the point");
            // On an edge between two outlines either may show.
            let edge = splats.painted.iter().filter(holds).filter(|s| s.site[2] == shown.site[2]).count() > 1;
            assert!(shown.site == site || edge, "({px}, {py}) shows {:?}, not {site:?}", shown.site);
        }
        assert!(hits > 3000, "{hits} rays met the block");
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
