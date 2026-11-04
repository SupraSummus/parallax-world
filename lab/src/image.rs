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

    /** Each block of b × b cells' most common label (whole blocks only). */
    pub fn majority(&self, b: usize) -> Image {
        let (w, h) = (self.w / b, self.h / b);
        let labels = self.labels();
        let mut out = Image::filled(w, h, 0);
        let mut count = vec![0u32; labels];
        for y in 0..h {
            for x in 0..w {
                count.iter_mut().for_each(|c| *c = 0);
                for j in 0..b {
                    for i in 0..b {
                        count[self.at(x * b + i, y * b + j) as usize] += 1;
                    }
                }
                let best = (0..labels).max_by_key(|l| (count[*l], std::cmp::Reverse(*l))).unwrap_or(0);
                out.set(x, y, best as u16);
            }
        }
        out
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

    pub fn write_png(&self, path: &str, rgb: &dyn Fn(u16) -> u32, scale: usize) {
        png::write(path, self.w * scale, self.h * scale, &self.to_rgb(rgb, scale));
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

/** Distinct colours for labels with no meaning of their own (biomes): golden-angle hues. */
pub fn palette(label: u16) -> u32 {
    let hue = (label as f64 * 0.381966 + 0.07).fract() * 6.0;
    let (l, s) = (if label.is_multiple_of(2) { 0.55 } else { 0.4 }, 0.65);
    let c = (1.0 - (2.0 * l - 1.0f64).abs()) * s;
    let x = c * (1.0 - (hue % 2.0 - 1.0).abs());
    let (r, g, b) = match hue as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = l - c / 2.0;
    let byte = |v: f64| ((v + m) * 255.0).round() as u32;
    byte(r) << 16 | byte(g) << 8 | byte(b)
}
