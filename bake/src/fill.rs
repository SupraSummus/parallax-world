//! Which pixels of a chunk's mask its splats cover, and which blocks of
//! them are solid: what coverage.rs knows of a chunk.

use crate::splats::Splats;

/**
 * Marks the pixels of a width × height mask showing the centre plane from
 * (origin_x, origin_y) (top-left) at `scale` pixels per unit whose centre
 * is in a splat's hexagon, painted or grown: everything the chunk draws.
 */
pub fn fill_mask(splats: &Splats, mask: &mut [bool], width: usize, height: usize, origin_x: f64, origin_y: f64, scale: f64) {
    for splat in splats.painted.iter().chain(&splats.grown) {
        // In pixel coordinates, y down.
        let (cx, cy) = ((splat.x - origin_x) * scale, (origin_y - splat.y) * scale);
        let (w, h) = (splat.width * scale, splat.height * scale);
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
            if from < to {
                mask[row * width + from..row * width + to].fill(true);
            }
        }
    }
}

/** Side of a block of pixels in solid_blocks. */
pub const BLOCK: usize = 4;
/** A block's bits in solid_blocks: all its pixels drawn, some drawn. */
pub const SOLID: u8 = 1;
pub const DRAWN: u8 = 2;

/**
 * Which BLOCK × BLOCK blocks of a mask are all drawn (SOLID) and which
 * have a pixel drawn (DRAWN), a byte of bits per block, in rows, for
 * finding what a chunk hides and whether it is hidden (coverage.rs).
 * A block reaching past the mask's edge counts its pixels in the mask.
 */
pub fn solid_blocks(blocks: &mut Vec<u8>, mask: &[bool], width: usize, height: usize) {
    let cols = width.div_ceil(BLOCK);
    let rows = height.div_ceil(BLOCK);
    blocks.clear();
    blocks.resize(cols * rows, SOLID);
    for y in 0..height {
        let row = &mask[y * width..(y + 1) * width];
        let block_row = &mut blocks[y / BLOCK * cols..(y / BLOCK + 1) * cols];
        for (block, run) in block_row.iter_mut().zip(row.chunks(BLOCK)) {
            let drawn = run.iter().filter(|&&p| p).count();
            if drawn < run.len() {
                *block &= !SOLID;
            }
            if drawn > 0 {
                *block |= DRAWN;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::splats::Splat;

    #[test]
    fn marks_the_pixels_whose_centre_is_in_a_hexagon() {
        // 40 × 40 pixels showing [0, 4] × [0, 4] at 10 pixels per unit.
        let mut splats = Splats::new();
        splats.painted.push(Splat { x: 1.23, y: 2.71, width: 0.61, height: 0.87, colour: 0, site: [0; 3] });
        splats.grown.push(Splat { x: 3.9, y: 0.2, width: 0.5, height: 0.6, colour: 0, site: [0; 3] });
        let mut mask = vec![false; 40 * 40];
        fill_mask(&splats, &mut mask, 40, 40, 0.0, 4.0, 10.0);
        let mut marked = 0;
        for row in 0..40 {
            for col in 0..40 {
                let (x, y) = ((col as f64 + 0.5) / 10.0, 4.0 - (row as f64 + 0.5) / 10.0);
                let inside = splats.painted.iter().chain(&splats.grown).any(|s| {
                    let (u, v) = ((x - s.x).abs(), (y - s.y).abs());
                    u < s.width && u / std::f64::consts::SQRT_2 + v < s.height
                });
                assert_eq!(mask[row * 40 + col], inside, "pixel ({col}, {row})");
                marked += mask[row * 40 + col] as usize;
            }
        }
        assert!(marked > 150);
    }

    #[test]
    fn marks_blocks_all_drawn_or_partly_drawn() {
        // 6 × 5 pixels: blocks of 4 × 4, the right and bottom ones cut by the edge.
        let mut mask = vec![false; 6 * 5];
        for y in 0..4 {
            for x in 0..6 {
                mask[y * 6 + x] = true;
            }
        }
        // The right block's pixels in the mask all drawn, the bottom right one partly.
        mask[4 * 6 + 5] = true;
        let mut blocks = Vec::new();
        solid_blocks(&mut blocks, &mask, 6, 5);
        assert_eq!(blocks, vec![SOLID | DRAWN, SOLID | DRAWN, 0, DRAWN]);
    }
}
