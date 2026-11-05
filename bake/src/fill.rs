//! Which pixels of a chunk's mask its splats cover, and which blocks of
//! them are solid: what coverage.rs knows of a chunk.

use crate::slicing::{LAYER, MAX_SLANT};
use crate::splats::Splats;

/**
 * Rays of a face the mask is filled for, as slants off its axis: the
 * axis and the corners of their rhombus (inFace in view.ts).
 */
const SLANTS: [[f64; 2]; 5] = [[0.0, 0.0], [MAX_SLANT, 0.0], [-MAX_SLANT, 0.0], [0.0, MAX_SLANT * LAYER], [0.0, -MAX_SLANT * LAYER]];
/** A pixel's bits in the mask: drawn seen along every one of SLANTS, and maybe drawn along some ray. */
pub const COVERED: u8 = (1 << SLANTS.len()) - 1;
pub const REACHED: u8 = 1 << SLANTS.len();

/**
 * Marks the pixels of a width × height mask showing the centre plane from
 * (origin_x, origin_y) (top-left) at `scale` pixels per unit whose centre
 * is in a splat's hexagon as drawn seen along each of SLANTS (a bit each),
 * and those in the hexagon holding where it may be drawn along any ray
 * (Splat::reach; REACHED).
 */
pub fn fill_mask(splats: &Splats, mask: &mut [u8], width: usize, height: usize, origin_x: f64, origin_y: f64, scale: f64) {
    let mut hexagon = |bit: u8, x: f64, y: f64, w: f64, h: f64| {
        // In pixel coordinates, y down.
        let (cx, cy) = ((x - origin_x) * scale, (origin_y - y) * scale);
        let (w, h) = (w * scale, h * scale);
        let first = (cy - h - 0.5).ceil().max(0.0) as usize;
        let last = ((cy + h - 0.5).floor() + 1.0).clamp(0.0, height as f64) as usize;
        for row in first..last {
            // |u| < width and |u| / √2 + |v| < height.
            let half = w.min((h - (row as f64 + 0.5 - cy).abs()) * std::f64::consts::SQRT_2);
            if half <= 0.0 {
                continue;
            }
            let from = (cx - half - 0.5).ceil().max(0.0) as usize;
            let to = ((cx + half - 0.5).floor() + 1.0).clamp(0.0, width as f64) as usize;
            for pixel in mask[row * width..].iter_mut().take(to).skip(from) {
                *pixel |= bit;
            }
        }
    };
    for splat in &splats.splats {
        let off = splat.offset();
        for (k, [sx, sy]) in SLANTS.iter().enumerate() {
            hexagon(1 << k, splat.x + sx * off, splat.y + sy * off, splat.width * (1.0 + sx.abs()), splat.height());
        }
        let (w, h) = splat.reach();
        hexagon(REACHED, splat.x, splat.y, w, h);
    }
}

/** Side of a block of pixels in solid_blocks. */
pub const BLOCK: usize = 4;
/** A block's bits in solid_blocks: all its pixels drawn, some drawn. */
pub const SOLID: u8 = 1;
pub const DRAWN: u8 = 2;

/**
 * Which BLOCK × BLOCK blocks of a mask are drawn all over seen along every
 * ray (SOLID) and which may have a pixel drawn (DRAWN), a byte of bits per
 * block, in rows, for finding what a chunk hides and whether it is hidden
 * (coverage.rs). A block reaching past the mask's edge counts its pixels
 * in the mask. SOLID looks along the axis and the rhombus's corners only:
 * a block covered from each may still open between them (docs/todo.md);
 * occlusion.test.ts finds the pictures the same with and without skipping.
 */
pub fn solid_blocks(blocks: &mut Vec<u8>, mask: &[u8], width: usize, height: usize) {
    let cols = width.div_ceil(BLOCK);
    let rows = height.div_ceil(BLOCK);
    blocks.clear();
    blocks.resize(cols * rows, SOLID);
    for y in 0..height {
        let row = &mask[y * width..(y + 1) * width];
        let block_row = &mut blocks[y / BLOCK * cols..(y / BLOCK + 1) * cols];
        for (block, run) in block_row.iter_mut().zip(row.chunks(BLOCK)) {
            if run.iter().any(|&p| p & COVERED != COVERED) {
                *block &= !SOLID;
            }
            if run.iter().any(|&p| p & REACHED != 0) {
                *block |= DRAWN;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::f64::consts::FRAC_1_SQRT_2;

    use crate::splats::Splat;

    #[test]
    fn marks_the_pixels_whose_centre_is_in_a_hexagon_seen_along_each_slant() {
        // 80 × 80 pixels showing [0, 8] × [0, 8] at 10 pixels per unit.
        let mut splats = Splats::new();
        let splat = Splat { x: 4.1, y: 3.9, depth: 3, run: 0, width: FRAC_1_SQRT_2, colour: 0, site: [0; 3] };
        let inside = |x: f64, y: f64, s: &Splat, (w, h): (f64, f64)| {
            let (u, v) = ((x - s.x).abs(), (y - s.y).abs());
            u < w && u * FRAC_1_SQRT_2 + v < h
        };
        let mut mask = vec![0; 80 * 80];
        splats.splats.push(splat);
        fill_mask(&splats, &mut mask, 80, 80, 0.0, 8.0, 10.0);
        let s = &splats.splats[0];
        let mut marked = [0; SLANTS.len() + 1];
        for row in 0..80 {
            for col in 0..80 {
                let (x, y) = ((col as f64 + 0.5) / 10.0, 8.0 - (row as f64 + 0.5) / 10.0);
                let pixel = mask[row * 80 + col];
                for (k, [sx, sy]) in SLANTS.iter().enumerate() {
                    let seen = inside(x - sx * s.offset(), y - sy * s.offset(), s, (s.width * (1.0 + sx.abs()), s.height()));
                    assert_eq!(pixel & 1 << k != 0, seen, "pixel ({col}, {row}) along {sx}, {sy}");
                    marked[k] += seen as usize;
                }
                assert_eq!(pixel & REACHED != 0, inside(x, y, s, s.reach()), "pixel ({col}, {row}) reached");
                marked[SLANTS.len()] += (pixel & REACHED != 0) as usize;
                // Every slant's hexagon is within the reach.
                assert!(pixel & COVERED == 0 || pixel & REACHED != 0);
            }
        }
        assert!(marked.iter().all(|&m| m > 100), "{marked:?}");
    }

    #[test]
    fn marks_blocks_all_covered_or_maybe_drawn() {
        // 6 × 5 pixels: blocks of 4 × 4, the right and bottom ones cut by the edge.
        let mut mask = vec![REACHED; 6 * 5];
        for pixel in &mut mask[..4 * 6] {
            *pixel = COVERED | REACHED;
        }
        // The right block's pixels in the mask all covered, but one only along some slants.
        mask[3] = COVERED & !1 | REACHED;
        mask[4 * 6 + 5] = COVERED | REACHED;
        mask[4 * 6] = 0;
        let mut blocks = Vec::new();
        solid_blocks(&mut blocks, &mask, 6, 5);
        assert_eq!(blocks, vec![DRAWN, SOLID | DRAWN, DRAWN, DRAWN]);
    }
}
