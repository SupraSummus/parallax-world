//! The voxels of a chunk the face's axis sees, each a hexagonal dab of
//! paint (a splat) at its site, in relief over its slice's centre plane,
//! swept over its column's voxels behind it.

use std::collections::HashMap;
use std::f64::consts::FRAC_1_SQRT_2;

use crate::light::{FaceLight, colour};
use crate::slicing::{LAYER, MAX_SLANT, slice_centre};
use crate::surface::{Slice, Voxel};
use crate::tables::VOXEL_FACES;
use crate::world::{FACE_KINDS, Geometry};

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
    /** Opaque RGBA (see voxel_colour). */
    pub colour: u32,
    /** Its voxel's lattice coordinates. */
    pub site: [i32; 3],
}

impl Splat {
    /** √2 times its width, as a voxel's outline is (the numbers of ChunkSplats leave it out). */
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
    /** A voxel's index by column (a, b), for run_ends and to keep the first. */
    columns: HashMap<(i32, i32), usize>,
    /** By surface voxel, the layer its run ends at (run_ends). */
    ends: Vec<i32>,
    /** By surface voxel, the voxel of its run its pillar shows (run_ends). */
    showing: Vec<usize>,
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
            ends: Vec::new(),
            showing: Vec::new(),
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
 * The layer at which each surface voxel's run ends: the last of the
 * voxels behind it in its column (a, b), every two layers, with nothing
 * between them but solid voxels of the slice, that a ray may see by an
 * open face (seen); its own layer if none. A voxel whose face away from
 * the camera (towards c + 2) is open or cut ends its column's solid part;
 * one that isn't has a solid voxel of the slice behind it, buried (so not
 * in the surface, and with solid behind it again) or the next surface
 * voxel of its column. Inside a solid, behind a cut through it, nothing
 * shows from the side, so its columns have no run. `order` is the
 * surface's (c, index), near to far.
 *
 * And the voxel each pillar shows (`showing`): its own, or if no ray sees
 * it by an open face, the first of its run a ray does. A voxel kept only
 * for a cut through it shows nothing of its own where the solid goes on in
 * a nearer slice, which hides the cut: what shows of its pillar is the run
 * behind it, where the face's rays enter its column from the side. (Where
 * cells interlock, the solid may go on in a farther slice instead, and the
 * cut stands for it; the run's voxel stands for it as well as the cut.)
 */
fn run_ends(splats: &mut Splats, geometry: &Geometry, surface: &[Voxel], order: &[(i32, usize)]) {
    let behind = geometry.offsets.iter().position(|&o| o == [0, 0, 2]).expect("a face along the axis");
    let Splats { ends, showing, columns, .. } = splats;
    ends.clear();
    ends.resize(surface.len(), 0);
    showing.clear();
    showing.resize(surface.len(), 0);
    columns.clear();
    for &(c, k) in order.iter().rev() {
        let voxel = &surface[k];
        let goes_on = (voxel.open | voxel.cut) & (1 << behind) == 0;
        let next = match columns.insert((voxel.site[0], voxel.site[1]), k) {
            Some(next) if goes_on && (ends[next] > surface[next].site[2] || seen(geometry, surface[next].open)) => Some(next),
            _ => None,
        };
        ends[k] = next.map_or(c, |next| ends[next]);
        showing[k] = match next {
            Some(next) if !seen(geometry, voxel.open) => showing[next],
            _ => k,
        };
    }
}

/**
 * Fills `splats` with the voxels of a chunk of a slice that the face's
 * axis sees: of each column (a, b), the nearest voxel a ray of the face
 * can meet by an open or cut face (seen), in painting order, far to near
 * (docs/architecture.md, section 3). A chunk is a picture along the axis
 * with a depth per point.
 * Each stands for its run (run_ends) as a pillar, so that a slanted ray
 * meets the column where its voxels are, and no gap opens behind it.
 * A pillar spans several layers, so no painting order is right for every
 * ray: the compositor draws pillars by the depth where a ray enters them,
 * and where they tie, the later painted.
 *
 * `surface` is the cell's (cell_surface), with the margin around the cell
 * that its chunks may show; `colour` gives each voxel's (voxel_colour),
 * a pillar the colour of the voxel it shows (run_ends).
 */
pub fn chunk_splats(splats: &mut Splats, geometry: &Geometry, colour: &mut dyn FnMut(&Voxel) -> u32, surface: &[Voxel], slice: &Slice) {
    splats.clear();
    // Twice the centre plane's depth, in layers: a whole number.
    let centre = (2.0 * slice_centre(slice) / LAYER).round() as i32;

    // Near to far.
    let mut order = std::mem::take(&mut splats.order);
    order.clear();
    order.extend(surface.iter().enumerate().map(|(k, voxel)| (voxel.site[2], k)));
    order.sort_unstable_by_key(|&(c, k)| (c, std::cmp::Reverse(k)));
    run_ends(splats, geometry, surface, &order);
    let (ends, showing) = (std::mem::take(&mut splats.ends), std::mem::take(&mut splats.showing));
    // The nearest of each column that a ray may meet.
    let columns = &mut splats.columns;
    columns.clear();
    order.retain(|&(_, k)| {
        seen(geometry, surface[k].open | surface[k].cut) && columns.insert((surface[k].site[0], surface[k].site[1]), k).is_none()
    });

    for &(_, k) in order.iter().rev() {
        let voxel = &surface[k];
        let [a, b, c] = voxel.site;
        splats.push(Splat {
            x: a as f64 * LAYER,
            y: b as f64,
            depth: 2 * c - centre,
            run: 2 * (ends[k] - c),
            width: FRAC_1_SQRT_2,
            colour: colour(&surface[showing[k]]),
            site: voxel.site,
        });
    }
    splats.order = order;
    splats.ends = ends;
    splats.showing = showing;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tables::geometry;
    use crate::world::lattices;

    /** The sites of layers `layers`, |a| and |b| ≤ 12, where `solid` holds, and their surface: those with an open face. */
    fn blob(geometry: &Geometry, layers: std::ops::Range<i32>, solid: impl Fn([i32; 3]) -> bool) -> (Vec<[i32; 3]>, Vec<Voxel>) {
        let sites: Vec<[i32; 3]> = layers
            .flat_map(|c| (-12..=12).flat_map(move |b| (-12..=12).map(move |a| [a, b, c])))
            .filter(|&[a, b, c]| (a - b) % 2 == 0 && (b - c) % 2 == 0 && solid([a, b, c]))
            .collect();
        let surface = sites
            .iter()
            .map(|&site| {
                let open = (0..FACE_KINDS)
                    .filter(|&k| !solid(std::array::from_fn(|d| site[d] + geometry.offsets[k][d])))
                    .fold(0, |o, k| o | 1 << k);
                Voxel { site, material: 0, open, cut: 0 }
            })
            .filter(|v| v.open != 0)
            .collect();
        (sites, surface)
    }

    #[test]
    fn a_slanted_ray_shows_the_voxel_it_meets_first() {
        // A solid block of level 2's slab of layers, its side a + c = 0
        // at 45° to the axis, as a tree trunk ahead does where four faces
        // meet; its inside buried, so only its surface is kept.
        let geometry = geometry(&lattices()[0]);
        let (sites, surface) = blob(&geometry, 0..4, |[a, b, c]| (0..4).contains(&c) && a.abs() <= 12 && b.abs() <= 12 && a + c >= 0);
        let slice = Slice { level: 2, atoms: 0, layer: 0, cutoff: f64::INFINITY };
        let mut splats = Splats::new();
        chunk_splats(&mut splats, &geometry, &mut |v| voxel_colour(v, &[0; FACE_KINDS]), &surface, &slice);
        let centre = slice_centre(&slice);

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
                // widened by the slant, holds where the ray crosses it. Runs
                // are left out: in this block they change nothing that shows
                // (pillars.test.ts draws a lawn where they do).
                let shown = splats.splats.iter().rev().find(|s| {
                    let (x, y) = (px + slant[0] * s.offset(), py + slant[1] * s.offset());
                    let width = s.width * (1.0 + slant[0].abs());
                    (x - s.x).abs() <= width && (x - s.x).abs() * FRAC_1_SQRT_2 + (y - s.y).abs() <= s.height()
                });
                let shown = shown.expect("a splat holds the point").site;
                // Its column's nearest voxel, or its neighbour in its layer: the
                // order within a layer is the same for every ray, and widened,
                // their hexagons overlap.
                let column = shown[..2] == site[..2] && shown[2] <= site[2];
                let beside = shown[2] == site[2] && shown[1] == site[1] && (shown[0] - site[0]).abs() == 2;
                assert!(column || beside, "({px}, {py}) along {slant:?} shows {shown:?}, not {site:?}");
            }
        }
        assert!(hits > 5000, "{hits} rays met the block");
    }

    #[test]
    fn keeps_what_the_axis_sees() {
        // A porous blob of level 3's slab: columns with several voxels that show.
        let geometry = geometry(&lattices()[0]);
        let hash = |[a, b, c]: [i32; 3]| (a.wrapping_mul(73_856_093) ^ b.wrapping_mul(19_349_663) ^ c.wrapping_mul(83_492_791)) as u32;
        let (_, surface) = blob(&geometry, 0..8, |site| (0..8).contains(&site[2]) && hash(site) % 3 == 0);
        let slice = Slice { level: 3, atoms: 0, layer: 0, cutoff: f64::INFINITY };
        let mut splats = Splats::new();
        chunk_splats(&mut splats, &geometry, &mut |v| voxel_colour(v, &[0; FACE_KINDS]), &surface, &slice);
        let kept = splats.splats;
        // Every voxel a ray may meet, far to near.
        let mut all: Vec<[i32; 3]> = surface.iter().filter(|v| seen(&geometry, v.open | v.cut)).map(|v| v.site).collect();
        all.sort_by_key(|site| std::cmp::Reverse(site[2]));
        assert!(kept.len() * 3 < all.len() * 2, "{} of {}", kept.len(), all.len());
        let columns: std::collections::HashSet<_> = kept.iter().map(|s| (s.site[0], s.site[1])).collect();
        assert_eq!(columns.len(), kept.len());

        // Along the axis each point shows the same voxel: the last splat holding it.
        let holds = |site: [i32; 3], x: f64, y: f64| {
            let dx = (x - site[0] as f64 * LAYER).abs();
            dx <= FRAC_1_SQRT_2 && dx * FRAC_1_SQRT_2 + (y - site[1] as f64).abs() <= 1.0
        };
        for (x, y) in (0..80).flat_map(|i| (0..80).map(move |j| (-8.0 + i as f64 * 0.2 + 0.013, -8.0 + j as f64 * 0.2 + 0.007))) {
            let shown = kept.iter().rev().map(|s| s.site).find(|&site| holds(site, x, y));
            assert_eq!(shown, all.iter().rev().copied().find(|&site| holds(site, x, y)), "at ({x}, {y})");
        }
    }

    #[test]
    fn a_pillar_whose_voxel_is_cut_shows_the_first_of_its_run_a_ray_sees() {
        // Columns of voxels cut by the slice's near side (material 1), each
        // in its own material behind: 2 open across the axis, a ray sees;
        // 3 open only away from the camera, no ray sees, its run going on to
        // 4, which a ray sees; 5 cut by the far side, no ray sees, so the
        // column has no run and its voxel shows itself.
        let geometry = geometry(&lattices()[0]);
        let kind = |offset: [i32; 3]| 1 << geometry.offsets.iter().position(|&o| o == offset).expect("a face");
        let (near, far, across, away) = (kind([0, 0, -2]), kind([0, 0, 2]), kind([-2, 0, 0]), kind([1, 1, 1]));
        let voxel = |site: [i32; 3], material: u8, open: u16, cut: u16| Voxel { site, material, open, cut };
        let surface = [
            voxel([0, 0, 0], 1, 0, near),
            voxel([0, 0, 4], 2, across, 0),
            voxel([2, 0, 0], 1, 0, near),
            voxel([2, 0, 2], 3, away, 0),
            voxel([2, 0, 6], 4, across, 0),
            voxel([4, 0, 0], 1, 0, near),
            voxel([4, 0, 4], 5, 0, far),
        ];
        let slice = Slice { level: 3, atoms: 0, layer: 0, cutoff: f64::INFINITY };
        let mut splats = Splats::new();
        chunk_splats(&mut splats, &geometry, &mut |v| 0xff00_0000 | v.material as u32, &surface, &slice);
        let mut pillars: Vec<_> = splats.splats.iter().map(|s| (s.site, s.run, s.colour & 255)).collect();
        pillars.sort_unstable();
        assert_eq!(pillars, [([0, 0, 0], 8, 2), ([2, 0, 0], 12, 4), ([4, 0, 0], 0, 1)]);
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
