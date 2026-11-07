//! Does a map have a town's geometry? Measures of straight streets,
//! rectangular roofs and straight edges between labels, to set samples
//! against the towns they learned from.

use crate::image::Image;

/** The 4-connected parts of label `l`: each one's cells. */
pub fn components(image: &Image, l: u16) -> Vec<Vec<usize>> {
    let mut seen = vec![false; image.data.len()];
    let mut out = Vec::new();
    for start in 0..image.data.len() {
        if seen[start] || image.data[start] != l {
            continue;
        }
        seen[start] = true;
        let mut stack = vec![start];
        let mut cells = Vec::new();
        while let Some(i) = stack.pop() {
            cells.push(i);
            let (x, y) = (i % image.w, i / image.w);
            let neighbours = [
                (x > 0).then(|| i - 1),
                (x + 1 < image.w).then(|| i + 1),
                (y > 0).then(|| i - image.w),
                (y + 1 < image.h).then(|| i + image.w),
            ];
            for j in neighbours.into_iter().flatten() {
                if !seen[j] && image.data[j] == l {
                    seen[j] = true;
                    stack.push(j);
                }
            }
        }
        out.push(cells);
    }
    out
}

/** A map's geometry. */
#[derive(Clone, Copy, Default, Debug)]
pub struct Geometry {
    /** Cells of street. */
    pub street: f64,
    /** The mean, over street cells, of the longer straight run of street through it (along x or y), metres. */
    pub street_run: f64,
    /** Parts of street of at least MIN_PART cells; the largest's share of the street. */
    pub street_parts: f64,
    pub street_largest: f64,
    /** Roofs: parts of at least MIN_PART cells, their median size, and their area over their bounding box's (area-weighted). */
    pub roofs: f64,
    pub roof_size: f64,
    pub roof_rectangular: f64,
    /** Of the edges between two labels, the share whose next edge along the same line divides the same two labels. */
    pub edges_straight: f64,
}

const MIN_PART: usize = 8;

impl Geometry {
    pub fn of(image: &Image, street: u16, roof: u16) -> Geometry {
        let (w, h) = (image.w, image.h);
        // Runs of street along x and y, each cell's run length.
        let mut run_x = vec![0u32; w * h];
        let mut run_y = vec![0u32; w * h];
        for y in 0..h {
            let mut x = 0;
            while x < w {
                let start = x;
                while x < w && image.at(x, y) == street {
                    x += 1;
                }
                (start..x).for_each(|i| run_x[y * w + i] = (x - start) as u32);
                x += 1;
            }
        }
        for x in 0..w {
            let mut y = 0;
            while y < h {
                let start = y;
                while y < h && image.at(x, y) == street {
                    y += 1;
                }
                (start..y).for_each(|j| run_y[j * w + x] = (y - start) as u32);
                y += 1;
            }
        }
        let streets: Vec<usize> = (0..w * h).filter(|i| image.data[*i] == street).collect();
        let n = streets.len().max(1) as f64;
        let street_run = streets.iter().map(|i| run_x[*i].max(run_y[*i]) as f64).sum::<f64>() / n;
        let parts: Vec<usize> = components(image, street).iter().map(|c| c.len()).filter(|s| *s >= MIN_PART).collect();
        let largest = parts.iter().copied().max().unwrap_or(0) as f64 / n;

        let mut roofs: Vec<(usize, f64)> = components(image, roof)
            .iter()
            .filter(|c| c.len() >= MIN_PART)
            .map(|c| {
                let (xs, ys) = (c.iter().map(|i| i % w), c.iter().map(|i| i / w));
                let (x0, x1) = (xs.clone().min().unwrap_or(0), xs.max().unwrap_or(0));
                let (y0, y1) = (ys.clone().min().unwrap_or(0), ys.max().unwrap_or(0));
                (c.len(), c.len() as f64 / ((x1 - x0 + 1) * (y1 - y0 + 1)) as f64)
            })
            .collect();
        let roof_area: f64 = roofs.iter().map(|r| r.0 as f64).sum();
        let roof_rectangular = roofs.iter().map(|r| r.0 as f64 * r.1).sum::<f64>() / roof_area.max(1.0);
        roofs.sort_unstable_by_key(|r| r.0);
        let roof_size = roofs.get(roofs.len() / 2).map_or(0.0, |r| r.0 as f64);

        // Edges between labels: across x at (x, y)|(x + 1, y), continued at y + 1; across y likewise along x.
        let (mut edges, mut straight) = (0u64, 0u64);
        for y in 0..h {
            for x in 0..w {
                let a = image.at(x, y);
                if x + 1 < w && a != image.at(x + 1, y) && y + 1 < h {
                    edges += 1;
                    straight += (image.at(x, y + 1) == a && image.at(x + 1, y + 1) == image.at(x + 1, y)) as u64;
                }
                if y + 1 < h && a != image.at(x, y + 1) && x + 1 < w {
                    edges += 1;
                    straight += (image.at(x + 1, y) == a && image.at(x + 1, y + 1) == image.at(x, y + 1)) as u64;
                }
            }
        }
        Geometry {
            street: streets.len() as f64,
            street_run,
            street_parts: parts.len() as f64,
            street_largest: largest,
            roofs: roofs.len() as f64,
            roof_size,
            roof_rectangular,
            edges_straight: straight as f64 / edges.max(1) as f64,
        }
    }

    pub fn mean(all: &[Geometry]) -> Geometry {
        let k = all.len().max(1) as f64;
        let avg = |f: &dyn Fn(&Geometry) -> f64| all.iter().map(f).sum::<f64>() / k;
        Geometry {
            street: avg(&|g| g.street),
            street_run: avg(&|g| g.street_run),
            street_parts: avg(&|g| g.street_parts),
            street_largest: avg(&|g| g.street_largest),
            roofs: avg(&|g| g.roofs),
            roof_size: avg(&|g| g.roof_size),
            roof_rectangular: avg(&|g| g.roof_rectangular),
            edges_straight: avg(&|g| g.edges_straight),
        }
    }
}

/**
 * Do cross streets cross? Long straight streets (runs of street at least
 * LONG, side by side in a band at most WIDE across) and the cross streets
 * meeting them: street REACH out from the band's side. Of the long
 * streets met from both sides, the cross streets per side and the share
 * with one on the other side within ALIGNED of it (centre to centre).
 */
#[derive(Clone, Copy, Default, Debug)]
pub struct Crossings {
    pub streets: usize,
    pub cross: usize,
    pub aligned: usize,
}

const LONG: usize = 48;
const WIDE: usize = 16;
const REACH: usize = 8;
const ALIGNED: f64 = 2.0;

impl Crossings {
    pub fn of(image: &Image, street: u16) -> Crossings {
        let t = Image::new(image.h, image.w, (0..image.w * image.h).map(|i| image.at(i / image.h, i % image.h)).collect());
        let (a, b) = (Self::along_y(image, street), Self::along_y(&t, street));
        Crossings { streets: a.streets + b.streets, cross: a.cross + b.cross, aligned: a.aligned + b.aligned }
    }

    /** Of the streets along y (bands of columns). */
    fn along_y(image: &Image, street: u16) -> Crossings {
        let (w, h) = (image.w, image.h);
        // Bands: (x first, x last, y from, y to), open while their last column is the one before.
        let mut bands: Vec<(usize, usize, usize, usize)> = Vec::new();
        for x in 0..w {
            let mut y = 0;
            while y < h {
                let start = y;
                while y < h && image.at(x, y) == street {
                    y += 1;
                }
                let (y0, y1) = (start, y);
                y += 1;
                if y1 - y0 < LONG {
                    continue;
                }
                let open = bands.iter_mut().find(|b| b.1 + 1 == x && b.2.max(y0) + LONG <= b.3.min(y1));
                match open {
                    Some(b) => *b = (b.0, x, b.2.max(y0), b.3.min(y1)),
                    None => bands.push((x, x, y0, y1)),
                }
            }
        }
        let mut out = Crossings::default();
        for (x0, x1, y0, y1) in bands {
            if x1 - x0 + 1 > WIDE {
                continue;
            }
            // Cross streets each side: runs of y where the REACH cells beyond the band are all street; their centres.
            let side = |xs: &dyn Fn(usize) -> Option<usize>| {
                let mut centres = Vec::new();
                let mut run: Option<usize> = None;
                for y in y0..=y1 {
                    let cross = y < y1 && (0..REACH).all(|k| xs(k).is_some_and(|x| image.at(x, y) == street));
                    match (cross, run) {
                        (true, None) => run = Some(y),
                        (false, Some(s)) => {
                            centres.push((s + y - 1) as f64 / 2.0);
                            run = None;
                        }
                        _ => {}
                    }
                }
                centres
            };
            let left = side(&|k| (x0 > k).then(|| x0 - 1 - k));
            let right = side(&|k| (x1 + 1 + k < w).then_some(x1 + 1 + k));
            if left.is_empty() || right.is_empty() {
                continue;
            }
            let matched = |c: &f64, other: &[f64]| other.iter().any(|o| (o - c).abs() <= ALIGNED);
            out.streets += 1;
            out.cross += left.len() + right.len();
            out.aligned += left.iter().filter(|c| matched(c, &right)).count() + right.iter().filter(|c| matched(c, &left)).count();
        }
        out
    }

    /** Summed over maps: long streets met from both sides, and the share of their cross streets aligned. */
    pub fn sum(all: &[Crossings]) -> Crossings {
        all.iter().fold(Crossings::default(), |a, c| Crossings {
            streets: a.streets + c.streets,
            cross: a.cross + c.cross,
            aligned: a.aligned + c.aligned,
        })
    }

    pub fn share(&self) -> f64 {
        self.aligned as f64 / self.cross.max(1) as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cross_streets_that_cross_and_that_do_not() {
        // A street along y at x 30..36; cross streets along x, one crossing (y 20..25), one each side apart (y 60, 70).
        let mut im = Image::filled(80, 100, 0);
        for y in 0..100 {
            (30..36).for_each(|x| im.set(x, y, 1));
        }
        for x in 0..80 {
            (20..25).for_each(|y| im.set(x, y, 1));
        }
        for x in 0..30 {
            (60..64).for_each(|y| im.set(x, y, 1));
        }
        for x in 36..70 {
            (70..74).for_each(|y| im.set(x, y, 1));
        }
        let c = Crossings::of(&im, 1);
        // Along y: 4 cross streets, 2 aligned; the street along x at y 20 (run 80) has the y street crossing it.
        assert_eq!((c.streets, c.cross, c.aligned), (2, 6, 4));
    }

    #[test]
    fn a_street_grid_and_a_rectangle() {
        // A cross of streets (1) 2 wide in 20 × 20, a 3 × 4 roof (2).
        let mut im = Image::filled(20, 20, 0);
        for i in 0..20 {
            for t in 9..11 {
                im.set(i, t, 1);
                im.set(t, i, 1);
            }
        }
        for (x, y) in (2..5).flat_map(|x| (2..6).map(move |y| (x, y))) {
            im.set(x, y, 2);
        }
        let g = Geometry::of(&im, 1, 2);
        assert_eq!((g.street_parts, g.street_largest, g.roofs, g.roof_size, g.roof_rectangular), (1.0, 1.0, 1.0, 12.0, 1.0));
        assert_eq!(g.street_run, 20.0);
    }
}
