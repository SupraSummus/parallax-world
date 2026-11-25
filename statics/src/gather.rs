//! A level from the one below, every level by the same rule: each cell of
//! 2^l·FCC one voxel, as many numbers to each.
//!
//! A voxel keeps what its matter weighs, and where; and along each of the
//! lattice's six directions how stiff its matter is, as a share of a cell
//! full of it: found by straining the finer voxels in it evenly along that
//! direction, those just outside moving with the strain (homogenisation),
//! as the energy they store. A face between two voxels keeps the area,
//! centre and spread of the matter crossing it, as `patch` keeps a face,
//! and is as stiff as the two cells' matter in series. So a full cell is
//! its material as it is; a lattice tower, at levels whose cells are about
//! its members' size, is not (docs/statics.md).

use crate::bodies::{Work, motions, shift};
use crate::lattice::*;
use crate::model::*;
use std::collections::HashMap;

/// The six directions: the neighbours whose first coordinate not 0 is positive.
fn directions() -> Vec<[i64; 3]> {
    NEIGHBOURS.iter().copied().filter(|e| e.iter().find(|&&v| v != 0) > Some(&0)).collect()
}

/// A cell's matter along each direction: the energy of a unit strain along
/// it, per volume (its stiffness).
#[derive(Clone, Copy, Debug)]
struct Sponge {
    stiff: [f64; 6],
}

/// Strain the voxels `inside` (a cell of `volume`) evenly along each
/// direction, the voxels joined to them from outside moving with the strain.
fn sponge(fine: &Lattice, inside: &[usize], volume: f64) -> Sponge {
    let is_in: HashMap<usize, usize> = inside.iter().enumerate().map(|(k, &v)| (v, k)).collect();
    // The cell and the ring around it: those outside held, moving evenly.
    let mut voxels: Vec<Voxel> = inside.iter().map(|&v| Voxel { held: false, ..fine.voxels[v].clone() }).collect();
    let mut sites: Vec<[i64; 3]> = inside.iter().map(|&v| fine.sites[v]).collect();
    let mut ring: HashMap<usize, usize> = HashMap::new();
    let mut bonds = Vec::new();
    let mut share = Vec::new();
    for b in &fine.bonds {
        let (i, j) = (is_in.get(&b.i), is_in.get(&b.j));
        if i.is_none() && j.is_none() {
            continue;
        }
        let mut local = |v: usize, k: Option<&usize>| match k {
            Some(&k) => k,
            None => *ring.entry(v).or_insert_with(|| {
                voxels.push(Voxel { held: true, ..fine.voxels[v].clone() });
                sites.push(fine.sites[v]);
                voxels.len() - 1
            }),
        };
        let (li, lj) = (local(b.i, i), local(b.j, j));
        bonds.push(Bond { i: li, j: lj, ..b.clone() });
        // A bond across the cell's boundary is half the cell's.
        share.push(if i.is_some() && j.is_some() { 1.0 } else { 0.5 });
    }
    let cell = Lattice { level: fine.level, voxels, bonds, sites };
    let alive = vec![true; cell.bonds.len()];
    let gone = vec![false; cell.voxels.len()];
    let mut out = Sponge { stiff: [0.0; 6] };
    for (d, e) in directions().iter().enumerate() {
        let n = unit(e.map(|v| v as f64));
        // Evenly strained: each voxel moved by n·(n·x), not turned.
        let even = |v: usize| -> Six {
            let u = mul(n, dot(n, cell.voxels[v].x));
            [u[0], u[1], u[2], 0.0, 0.0, 0.0]
        };
        // What holds the free voxels where the even strain puts them.
        let mut held_even = vec![[0.0; 6]; cell.voxels.len()];
        for b in &cell.bonds {
            let f = b.force(sub(at_even(&cell, b.j, b.p, &even), at_even(&cell, b.i, b.p, &even)));
            for (v, sign) in [(b.j, -1.0), (b.i, 1.0)] {
                let t = cross(sub(b.p, cell.voxels[v].x), f);
                for s in 0..3 {
                    held_even[v][s] += sign * f[s];
                    held_even[v][3 + s] += sign * t[s];
                }
            }
        }
        let loads = |v: usize| held_even[v];
        let rest = motions(&cell, &alive, &gone, &loads, &mut Work(0.0));
        let moved: Vec<Six> = (0..cell.voxels.len()).map(|v| std::array::from_fn(|s| even(v)[s] + rest[v][s])).collect();
        let mut energy = 0.0;
        let mut force = vec![[0.0; 3]; cell.bonds.len()];
        for (k, b) in cell.bonds.iter().enumerate() {
            let at = |v: usize| -> V3 { shift(moved[v], sub(b.p, cell.voxels[v].x))[..3].try_into().unwrap() };
            let delta = sub(at(b.j), at(b.i));
            force[k] = b.force(delta);
            energy += 0.5 * share[k] * dot(force[k], delta);
        }
        out.stiff[d] = 2.0 * energy / volume;
    }
    out
}

fn at_even(cell: &Lattice, v: usize, p: V3, even: &dyn Fn(usize) -> Six) -> V3 {
    shift(even(v), sub(p, cell.voxels[v].x))[..3].try_into().unwrap()
}

/// The next level up: one voxel to each cell of it with matter.
pub fn gathered(fine: &Lattice) -> Lattice {
    let level = fine.level + 1;
    let s = spacing(level);
    let volume = 2.0 * s * s * s;
    let mut index: HashMap<[i64; 3], usize> = HashMap::new();
    let mut cells: Vec<Vec<usize>> = Vec::new();
    let mut sites = Vec::new();
    let cell_of: Vec<usize> = fine
        .sites
        .iter()
        .enumerate()
        .map(|(v, site)| {
            let key = coarse_site(*site, level);
            let c = *index.entry(key).or_insert_with(|| {
                cells.push(Vec::new());
                sites.push(key);
                cells.len() - 1
            });
            cells[c].push(v);
            c
        })
        .collect();
    // Each cell's material: what most of its mass is.
    let mat: Vec<Mat> = cells
        .iter()
        .map(|members| {
            let mut by: Vec<(Mat, f64)> = Vec::new();
            for &v in members {
                let f = &fine.voxels[v];
                match by.iter_mut().find(|(m, _)| m.name == f.mat.name) {
                    Some((_, w)) => *w += f.m,
                    None => by.push((f.mat, f.m)),
                }
            }
            by.iter().max_by(|a, b| a.1.total_cmp(&b.1)).unwrap().0
        })
        .collect();
    // A cell full of each material, at this level, to measure shares by.
    let mut full: HashMap<&str, Sponge> = HashMap::new();
    for m in &mat {
        full.entry(m.name).or_insert_with(|| {
            let fill = move |_: V3| Some((*m, false));
            let block = build(fine.level, [-2.0 * s; 3], [2.0 * s; 3], &fill);
            let inside: Vec<usize> = (0..block.sites.len()).filter(|&v| coarse_site(block.sites[v], level) == [0, 0, 0]).collect();
            sponge(&block, &inside, volume)
        });
    }
    let voxels: Vec<Voxel> = cells
        .iter()
        .map(|members| {
            let m: f64 = members.iter().map(|&v| fine.voxels[v].m).sum();
            let x = mul(members.iter().fold([0.0; 3], |a, &v| add(a, mul(fine.voxels[v].x, fine.voxels[v].m))), 1.0 / m);
            (x, m, members.iter().any(|&v| fine.voxels[v].held))
        })
        .zip(&mat)
        .map(|((x, m, held), mat)| Voxel { x, m, volume, mat: *mat, held })
        .collect();
    // Each cell's matter as a sponge: a share of its full material's
    // stiffness along each direction.
    let sponges: Vec<[f64; 6]> = cells
        .iter()
        .enumerate()
        .map(|(c, members)| {
            let (own, whole) = (sponge(fine, members, volume), full[mat[c].name]);
            std::array::from_fn(|d| own.stiff[d] / whole.stiff[d])
        })
        .collect();
    // What crosses each face between two cells: area, where, how spread.
    struct Crossing {
        area: f64,
        first: V3,
        second: M3,
        mats: Vec<(Mat, f64)>,
    }
    let mut faces: HashMap<(usize, usize, usize), Crossing> = HashMap::new();
    for b in &fine.bonds {
        let (ci, cj) = (cell_of[b.i].min(cell_of[b.j]), cell_of[b.i].max(cell_of[b.j]));
        let step = [0, 1, 2].map(|k| (sites[cj][k] - sites[ci][k]) >> level);
        let Some(e) = NEIGHBOURS.iter().position(|e| *e == step) else { continue };
        if ci == cj || (voxels[ci].held && voxels[cj].held) {
            continue;
        }
        let w = b.a * dot(b.n, unit(step.map(|v| v as f64))).abs();
        let f = faces.entry((ci, cj, e)).or_insert(Crossing { area: 0.0, first: [0.0; 3], second: [[0.0; 3]; 3], mats: Vec::new() });
        f.area += w;
        f.first = add(f.first, mul(b.p, w));
        for r in 0..3 {
            for c in 0..3 {
                f.second[r][c] += w * b.p[r] * b.p[c];
            }
        }
        match f.mats.iter_mut().find(|(m, _)| m.name == b.mat.name) {
            Some((_, a)) => *a += w,
            None => f.mats.push((b.mat, w)),
        }
    }
    let dirs = directions();
    // What crosses a face between two full cells, along each direction: a
    // full face is as much, wherever some of it crosses at the cells' corners.
    let full_area: [f64; 6] = {
        let block = build(fine.level, [-3.0 * s; 3], [3.0 * s; 3], &|_| Some((STONE, false)));
        let mut area = [0.0; 6];
        for b in &block.bonds {
            let (a, c) = (coarse_site(block.sites[b.i], level), coarse_site(block.sites[b.j], level));
            for (d, o) in dirs.iter().enumerate() {
                let o = o.map(|v| v << level);
                if a == [0, 0, 0] && c == o || c == [0, 0, 0] && a == o {
                    area[d] += b.a * dot(b.n, unit(o.map(|v| v as f64))).abs();
                }
            }
        }
        area
    };
    let mut keys: Vec<_> = faces.keys().copied().collect();
    keys.sort();
    let mut bonds = Vec::new();
    for key in keys {
        let (i, j, e) = key;
        let f = &faces[&key];
        let step = NEIGHBOURS[e];
        let n = unit(step.map(|v| v as f64));
        let d = dirs.iter().position(|o| *o == step || *o == step.map(|v| -v)).unwrap();
        let series = |x: f64, y: f64| if x > 0.0 && y > 0.0 { 2.0 / (1.0 / x + 1.0 / y) } else { 0.0 };
        // Stiff as the two cells' sponge in series; strong as what crosses.
        let stiff = series(sponges[i][d], sponges[j][d]);
        let (_, rhombus, _) = face(step, s);
        let whole = rhombus / full_area[d];
        let area = f.area * whole;
        let centre = mul(f.first, 1.0 / f.area);
        let flat: M3 = std::array::from_fn(|r| std::array::from_fn(|c| if r == c { 1.0 } else { 0.0 } - n[r] * n[c]));
        let about: M3 = std::array::from_fn(|r| std::array::from_fn(|c| f.second[r][c] / f.area - centre[r] * centre[c]));
        let spread: M3 = std::array::from_fn(|r| {
            std::array::from_fn(|c| area * (0..3).map(|a| (0..3).map(|b| flat[r][a] * about[a][b] * flat[b][c]).sum::<f64>()).sum::<f64>())
        });
        let m = f.mats.iter().max_by(|a, b| a.1.total_cmp(&b.1)).unwrap().0;
        let k = stiff * rhombus / area;
        let mat = Mat { e: m.e * k, g: m.g * k, ..m };
        if stiff > 0.0 {
            bonds.extend(patch(i, j, n, 2f64.sqrt() * s, area, centre, &spread, mat));
        }
    }
    Lattice { level, voxels, bonds, sites }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A level gathered from a solid block is the same solid, built coarser.
    #[test]
    fn solid_gathers_to_itself() {
        let fill = |x: V3| (norm(x) < 1.6).then_some((STONE, false));
        let coarse = gathered(&build(0, [-2.0; 3], [2.0; 3], &fill));
        let middle = coarse.voxels.iter().position(|v| norm(v.x) < 0.1).unwrap();
        for b in coarse.bonds.iter().filter(|b| b.i == middle || b.j == middle) {
            for (got, want) in [(b.mat.e, STONE.e), (b.mat.g, STONE.g), (b.mat.ft, STONE.ft), (b.mat.fs, STONE.fs)] {
                assert!((got / want - 1.0).abs() < 1e-6, "{got} against {want}");
            }
        }
    }
}
