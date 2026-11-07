//! Label images: a label per cell of a w × h grid (x fastest), drawn with
//! +z (rows) up as worlds/examples/map.rs draws the world.

use crate::png;

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Image {
    pub w: usize,
    pub h: usize,
    pub data: Vec<u16>,
}

impl Image {
    pub fn new(w: usize, h: usize, data: Vec<u16>) -> Image {
        assert_eq!(data.len(), w * h);
        Image { w, h, data }
    }

    pub fn filled(w: usize, h: usize, label: u16) -> Image {
        Image { w, h, data: vec![label; w * h] }
    }

    pub fn at(&self, x: usize, y: usize) -> u16 {
        self.data[y * self.w + x]
    }

    pub fn set(&mut self, x: usize, y: usize, label: u16) {
        self.data[y * self.w + x] = label;
    }

    /** The number of labels: one more than the largest. */
    pub fn labels(&self) -> usize {
        self.data.iter().max().map_or(0, |m| *m as usize + 1)
    }

    /** The part `w` × `h` from (x, y). */
    pub fn crop(&self, x: usize, y: usize, w: usize, h: usize) -> Image {
        Image::new(w, h, (y..y + h).flat_map(|j| (x..x + w).map(move |i| (i, j))).map(|(i, j)| self.at(i, j)).collect())
    }

    /** As RGB, each label's colour from `rgb`, each cell `scale` pixels wide; +y up. */
    pub fn to_rgb(&self, rgb: &dyn Fn(u16) -> u32, scale: usize) -> Vec<u8> {
        let (w, h) = (self.w * scale, self.h * scale);
        let mut out = vec![0u8; w * h * 3];
        for py in 0..h {
            for px in 0..w {
                let c = rgb(self.at(px / scale, self.h - 1 - py / scale));
                out[(py * w + px) * 3..][..3].copy_from_slice(&[(c >> 16) as u8, (c >> 8) as u8, c as u8]);
            }
        }
        out
    }
}

/** Pictures side by side, all `h` pixels tall, a gap of `gap` white pixels between them, as one PNG. */
pub fn write_side_by_side(path: &str, pictures: &[(usize, usize, Vec<u8>)], gap: usize) {
    let h = pictures.iter().map(|p| p.1).max().unwrap_or(0);
    let w = pictures.iter().map(|p| p.0).sum::<usize>() + gap * pictures.len().saturating_sub(1);
    let mut out = vec![255u8; w * h * 3];
    let mut x0 = 0;
    for (pw, ph, rgb) in pictures {
        for y in 0..*ph {
            out[(y * w + x0) * 3..][..pw * 3].copy_from_slice(&rgb[y * pw * 3..][..pw * 3]);
        }
        x0 += pw + gap;
    }
    png::write(path, w, h, &out);
}

/** Rows of pictures side by side, the rows one above another, `gap` white pixels between. */
pub fn write_grid(path: &str, rows: &[Vec<(usize, usize, Vec<u8>)>], gap: usize) {
    let w = rows.iter().map(|r| r.iter().map(|p| p.0).sum::<usize>() + gap * (r.len() - 1)).max().unwrap_or(0);
    let heights: Vec<usize> = rows.iter().map(|r| r.iter().map(|p| p.1).max().unwrap_or(0)).collect();
    let h = heights.iter().sum::<usize>() + gap * (rows.len() - 1);
    let mut out = vec![255u8; w * h * 3];
    let mut y0 = 0;
    for (row, rh) in rows.iter().zip(&heights) {
        let mut x0 = 0;
        for (pw, ph, rgb) in row {
            for y in 0..*ph {
                out[((y0 + y) * w + x0) * 3..][..pw * 3].copy_from_slice(&rgb[y * pw * 3..][..pw * 3]);
            }
            x0 += pw + gap;
        }
        y0 += rh + gap;
    }
    png::write(path, w, h, &out);
}
