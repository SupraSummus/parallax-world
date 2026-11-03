//! The voxels of a chunk that can show, each a hexagonal dab of paint
//! (a splat) on its slice's centre plane.

use std::f64::consts::FRAC_1_SQRT_2;

use crate::slicing::{LAYER, eye_error, slice_centre, spacing};
use crate::surface::{Slice, Voxel};
use crate::world::{FACE_KINDS, Geometry};

/**
 * A voxel painted as a hexagon on a slice's centre plane:
 * the points (x + u, y + v) with |u| ≤ width and |u| / √2 + |v| ≤ height,
 * the least that holds the voxel's outline seen from the eye.
 * Seen straight along the face's axis, from afar, that is the outline
 * (width 1/√2, height 1, in units of its grid);
 * at a slant it is wider: twice where four faces meet.
 * Grown, its sides move out.
 */
pub struct Splat {
    /** Its centre: its site, projected from the eye onto the plane. */
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
 * The rhombic dodecahedron's corners around its site, in face units, by
 * depth: a layer before it, level with it (its outline along the axis),
 * a layer after.
 */
const CORNERS: [&[[f64; 2]]; 3] = [
    &[[LAYER, 0.0], [-LAYER, 0.0], [0.0, 0.5], [0.0, -0.5]],
    &[[0.0, 1.0], [0.0, -1.0], [LAYER, 0.5], [LAYER, -0.5], [-LAYER, 0.5], [-LAYER, -0.5]],
    &[[LAYER, 0.0], [-LAYER, 0.0], [0.0, 0.5], [0.0, -0.5]],
];

/**
 * The splats of one chunk and their extent.
 * `painted` are the voxels rays from the eye may reach, far to near;
 * `grown` are voxels going on into another slice, their hexagons grown
 * to close the gaps between slices, also far to near:
 * drawn only where the chunk paints nothing (docs/architecture.md, section 3).
 */
pub struct Splats {
    pub painted: Vec<Splat>,
    pub grown: Vec<Splat>,
    /** Painting order: each surface voxel's squared distance from the eye, and its index. */
    order: Vec<(i64, usize)>,
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
 * Fills `splats` with the voxels of a chunk of a slice that rays from the
 * eye (a site, in lattice coordinates, at least 2N layers before every
 * voxel, N = 2^level) reach, in painting order: far to near.
 * A voxel is painted if one of its open or cut faces faces the eye
 * (n · (p − eye) < 0 for its points p);
 * its splat is centred where the line from the eye through its site meets
 * the centre plane, and holds where the lines through its corners do.
 *
 * Painting order: voxels by their site's distance from the eye, farthest
 * first. Voxels are the cells of their sites' Voronoi diagram, so a ray
 * from the eye that passes from one voxel into another crosses where
 * their sites are equally far, from the nearer site's side: along every
 * ray, a voxel it meets first is nearer the eye.
 *
 * If `fill`, voxels with an open face and a neighbour in another slice,
 * where gaps open between slices (docs/architecture.md, section 3),
 * are also grown: including ones only some camera in the eye's cell
 * faces, which are not painted.
 *
 * `surface` is the cell's (cell_surface), with the margin around the cell
 * that its chunks may show; `palette` the view face's (see voxel_colour).
 */
pub fn chunk_splats(
    splats: &mut Splats,
    geometry: &Geometry,
    palette: &[u32],
    surface: &[Voxel],
    slice: &Slice,
    eye: [i32; 3],
    fill: bool,
) {
    splats.clear();
    let centre = slice_centre(slice.level, slice.layer);
    let n = spacing(slice.level) as f64;
    let (ex, ey, ez) = (eye[0] as f64 * LAYER, eye[1] as f64, eye[2] as f64 * LAYER);

    let mut order = std::mem::take(&mut splats.order);
    order.clear();
    order.extend(surface.iter().enumerate().map(|(k, voxel)| {
        let [a, b, c] = voxel.site;
        let [da, db, dc] = [(a - eye[0]) as i64, (b - eye[1]) as i64, (c - eye[2]) as i64];
        // Twice the squared distance: a and c are LAYER per unit, b is 1.
        (da * da + 2 * db * db + dc * dc, k)
    }));
    order.sort_unstable_by_key(|&(distance, _)| std::cmp::Reverse(distance));

    for &(_, k) in &order {
        let voxel = &surface[k];
        let [a, b, c] = voxel.site;
        let (x, y, z) = (a as f64 * LAYER, b as f64, c as f64 * LAYER);
        // Whether one of `faces` has n · (p − eye) < below for its points p,
        // as n · (p − site) = 1 on a face (|n| = √2).
        let facing = |faces: u16, below: f64| {
            (0..FACE_KINDS).any(|kind| {
                let [na, nb, nc] = geometry.normals[kind];
                faces & (1 << kind) != 0 && na * (x - ex) + nb * (y - ey) + nc * (z - ez) + 1.0 < below
            })
        };
        let painted = facing(voxel.open | voxel.cut, 0.0);
        // A voxel going on into another slice grows where some camera in the
        // eye's cell faces one of its open faces: the cell reaches N / √2
        // along a face's normal n, n · δ ≤ N.
        let grows = fill && voxel.cut != 0 && facing(voxel.open, n);
        if !painted && !grows {
            continue;
        }

        let t = (centre - ez) / (z - ez);
        let (sx, sy) = (ex + (x - ex) * t, ey + (y - ey) * t);
        let (mut width, mut height) = (0.0f64, 0.0f64);
        for (corners, depth) in CORNERS.iter().zip([-LAYER, 0.0, LAYER]) {
            let at = (centre - ez) / (z + depth - ez);
            for [u, v] in corners.iter() {
                let (du, dv) = ((ex + (x + u - ex) * at - sx).abs(), (ey + (y + v - ey) * at - sy).abs());
                width = width.max(du);
                height = height.max(du * FRAC_1_SQRT_2 + dv);
            }
        }
        let splat = Splat { x: sx, y: sy, width, height, colour: voxel_colour(voxel, palette), site: voxel.site };
        if grows {
            // A camera in the eye's cell sees the voxel's points at most
            // `error` off the eye's rays (as tangents); a point d ahead of
            // the centre plane is drawn (σ − τ) · d off, σ the camera's ray
            // and τ the eye's (section 2). Where its voxel meets one in
            // another slice, within a layer of its own, that may pull it
            // away from the other slice's drawing of the surface, so its
            // sides move out by that much: the slanted ones, with normals
            // (±1/√2, ±1) / √(3/2), by √(3/2) as much in `height`.
            let error = eye_error(slice.level, (c - 1 - eye[2]) as f64);
            let d = ((c + 1) as f64 * LAYER - centre).abs().max(((c - 1) as f64 * LAYER - centre).abs());
            let grow = error * d;
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
    fn a_ray_from_the_eye_lands_in_the_splat_of_the_voxel_it_meets_first() {
        // A solid block of level 2's slab of layers, its side a + c = 0
        // facing the eye at 45° to the axis, as a tree trunk ahead does where
        // four faces meet; its inside buried, so only that side shows.
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
        // 2N layers before the nearest voxel's front, a layer before its site.
        let (slice, eye) = (Slice { level: 2, layer: 0, cutoff: f64::INFINITY }, [-12, -4, -12]);
        let mut splats = Splats::new();
        chunk_splats(&mut splats, &geometry, &[0; FACE_KINDS], &surface, &slice, eye, false);

        let centre = slice_centre(slice.level, slice.layer);
        let from = [eye[0] as f64 * LAYER, eye[1] as f64, eye[2] as f64 * LAYER];
        let at = |[a, b, c]: [i32; 3]| [a as f64 * LAYER, b as f64, c as f64 * LAYER];
        let mut hits = 0;
        for (px, py) in (0..60).flat_map(|i| (0..60).map(move |j| (-1.5 + i as f64 * 0.05, -1.5 + j as f64 * 0.05))) {
            // The voxel the ray meets first: inside one, n · (p − site) ≤ 1 for each face's normal n.
            let ray = [px - from[0], py - from[1], centre - from[2]];
            let enter = |&site: &[i32; 3]| {
                let p = at(site);
                let (mut enter, mut exit) = (f64::NEG_INFINITY, f64::INFINITY);
                for n in geometry.normals {
                    let (along, off) = ((0..3).map(|d| n[d] * ray[d]).sum::<f64>(), (0..3).map(|d| n[d] * (from[d] - p[d])).sum::<f64>());
                    let t = (1.0 - off) / along;
                    if along < 0.0 { enter = enter.max(t) } else { exit = exit.min(t) }
                }
                (enter < exit).then_some(enter)
            };
            let Some((_, site)) = sites.iter().filter_map(|s| enter(s).map(|t| (t, *s))).min_by(|a, b| a.0.total_cmp(&b.0)) else {
                continue;
            };
            hits += 1;
            let s = splats.painted.iter().find(|s| s.site == site).expect("the voxel met is painted");
            let (u, v) = ((px - s.x).abs(), (py - s.y).abs());
            assert!(u <= s.width + 1e-9 && u * FRAC_1_SQRT_2 + v <= s.height + 1e-9, "({px}, {py}) misses the splat of {site:?}");
        }
        assert!(hits > 1000, "{hits} rays met the block");
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
