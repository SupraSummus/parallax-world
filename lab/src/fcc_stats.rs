//! Statistics and pictures of volumes on the FCC lattice (masked_fcc.rs's):
//! what the FCC examples set samples against held-out volumes by.

use crate::masked_fcc::{Model, is_site};
use crate::scenes::{self, AIR, label3_rgb};
use crate::stats;
use crate::vol::Vol;

/** The lattice's 12 nearest neighbours, half of them (the other half their opposites). */
const NEAREST: [[i64; 3]; 6] = [[1, 1, 0], [1, 0, 1], [0, 1, 1], [1, -1, 0], [1, 0, -1], [0, 1, -1]];

pub fn sites(v: &Vol) -> impl Iterator<Item = [i64; 3]> + '_ {
    (0..v.n[1])
        .flat_map(move |y| (0..v.n[2]).flat_map(move |z| (0..v.n[0]).map(move |x| [x as i64, y as i64, z as i64])))
        .filter(|p| is_site(*p))
}

/** Shares over sites, and the joint of labels d nearest-neighbour steps apart along the lattice's 12 directions. */
pub fn pooled(vols: &[Vol], n: usize, d: i64) -> (Vec<f64>, Vec<f64>) {
    let (mut shares, mut joint, mut total) = (vec![0.0; n], vec![0.0; n * n], 0.0f64);
    for v in vols {
        for p in sites(v) {
            let a = v.get(p).unwrap_or(0) as usize;
            shares[a] += 1.0;
            for u in NEAREST {
                if let Some(b) = v.get([p[0] + d * u[0], p[1] + d * u[1], p[2] + d * u[2]]) {
                    joint[a * n + b as usize] += 1.0;
                    total += 1.0;
                }
            }
        }
    }
    let cells: f64 = shares.iter().sum();
    shares.iter_mut().for_each(|s| *s /= cells);
    joint.iter_mut().for_each(|j| *j /= total.max(1.0));
    (shares, joint)
}

/** Parts of sites `wanted` joined through nearest neighbours: their number (of at least 8 sites) per volume, the largest's share. */
pub fn parts(vols: &[Vol], wanted: &dyn Fn(u16) -> bool) -> (f64, f64) {
    let (mut count, mut largest) = (0.0, 0.0);
    for v in vols {
        let mut seen = vec![false; v.data.len()];
        let (mut big, mut cells, mut k) = (0usize, 0usize, 0usize);
        for s in sites(v) {
            let i = v.index(s.map(|c| c as usize));
            if seen[i] || !wanted(v.data[i]) {
                continue;
            }
            seen[i] = true;
            let mut stack = vec![s];
            let mut size = 0;
            while let Some(p) = stack.pop() {
                size += 1;
                for u in NEAREST {
                    for sign in [-1, 1] {
                        let q = [p[0] + sign * u[0], p[1] + sign * u[1], p[2] + sign * u[2]];
                        if let Some(l) = v.get(q)
                            && wanted(l)
                        {
                            let j = v.index(q.map(|c| c as usize));
                            if !seen[j] {
                                seen[j] = true;
                                stack.push(q);
                            }
                        }
                    }
                }
            }
            big = big.max(size);
            cells += size;
            k += (size >= 8) as usize;
        }
        count += k as f64;
        largest += big as f64 / cells.max(1) as f64;
    }
    (count / vols.len() as f64, largest / vols.len() as f64)
}

/**
 * Seams at `level`: of nearest neighbours, the share whose labels differ
 * where their owners at that level (the octree-like partition, owner of
 * owner) differ, over the share where they are one.
 */
pub fn seams(vols: &[Vol], level: usize) -> f64 {
    let owner = |p: [i64; 3]| (0..level).fold(p, |q, _| Model::owner(q));
    let ([mut cross, mut cross_diff], [mut same, mut same_diff]) = ([0f64; 2], [0f64; 2]);
    for v in vols {
        for p in sites(v) {
            let (a, op) = (v.get(p).unwrap_or(0), owner(p));
            for u in NEAREST {
                let q = [p[0] + u[0], p[1] + u[1], p[2] + u[2]];
                let Some(b) = v.get(q) else { continue };
                let differ = (a != b) as u8 as f64;
                if owner(q) == op {
                    same += 1.0;
                    same_diff += differ;
                } else {
                    cross += 1.0;
                    cross_diff += differ;
                }
            }
        }
    }
    (cross_diff / cross.max(1.0)) / (same_diff / same.max(1.0)).max(1e-9)
}

pub type Picture = (usize, usize, Vec<u8>);

pub fn picture(v: &Vol) -> Vec<Picture> {
    let d = scenes::densify(v);
    vec![d.top_rgb(&label3_rgb, AIR, 2), d.slice_rgb(d.n[2] / 2, &label3_rgb, AIR, 2), d.level_rgb(d.n[1] / 3, &label3_rgb, AIR, 2)]
}

pub fn report(rows: &[(String, Vec<Vol>)], n: usize) {
    let dists = [1, 2, 4, 8];
    let reference: Vec<(Vec<f64>, Vec<f64>)> = dists.iter().map(|d| pooled(&rows[0].1, n, *d)).collect();
    println!(
        "| volume | TV shares | {} | solid parts | largest solid | air parts | largest air | seams L2 | seams L3 |",
        dists.iter().map(|d| format!("TV joint d={d}")).collect::<Vec<_>>().join(" | ")
    );
    println!("|---|---|---|---|---|---|---|---|---|---|---|---|---|");
    for (name, vols) in rows {
        let tvs: Vec<String> =
            dists.iter().zip(&reference).map(|(d, r)| format!("{:.3}", stats::tv(&pooled(vols, n, *d).1, &r.1))).collect();
        let (sp, sl) = parts(vols, &|l| l != AIR);
        let (ap, al) = parts(vols, &|l| l == AIR);
        println!(
            "| {name} | {:.3} | {} | {sp:.1} | {sl:.2} | {ap:.1} | {al:.2} | {:.2} | {:.2} |",
            stats::tv(&pooled(vols, n, 1).0, &reference[0].0),
            tvs.join(" | "),
            seams(vols, 2),
            seams(vols, 3)
        );
    }
}
