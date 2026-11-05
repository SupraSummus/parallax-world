//! Label volumes: a label per cell of an nx × ny × nz grid, y up, x
//! fastest, then z, then y; a flat one (ny = 1) is a map seen from above,
//! laid out as Image is. And pictures of them: from above, and a slice.

use crate::image::Image;

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Vol {
    pub n: [usize; 3],
    pub data: Vec<u16>,
}

impl Vol {
    pub fn new(n: [usize; 3], data: Vec<u16>) -> Vol {
        assert_eq!(data.len(), n[0] * n[1] * n[2]);
        Vol { n, data }
    }

    pub fn filled(n: [usize; 3], label: u16) -> Vol {
        Vol { n, data: vec![label; n[0] * n[1] * n[2]] }
    }

    pub fn index(&self, [x, y, z]: [usize; 3]) -> usize {
        x + self.n[0] * (z + self.n[2] * y)
    }

    pub fn at(&self, p: [usize; 3]) -> u16 {
        self.data[self.index(p)]
    }

    pub fn set(&mut self, p: [usize; 3], label: u16) {
        let i = self.index(p);
        self.data[i] = label;
    }

    /** The label at `p` or None outside. */
    pub fn get(&self, p: [i64; 3]) -> Option<u16> {
        (0..3).all(|a| p[a] >= 0 && (p[a] as usize) < self.n[a]).then(|| self.at(p.map(|v| v as usize)))
    }

    pub fn of_image(image: &Image) -> Vol {
        Vol::new([image.w, 1, image.h], image.data.clone())
    }

    pub fn to_image(&self) -> Image {
        assert_eq!(self.n[1], 1, "a flat volume");
        Image::new(self.n[0], self.n[2], self.data.clone())
    }

    /** The part of size `n` from `at`. */
    pub fn crop(&self, at: [usize; 3], n: [usize; 3]) -> Vol {
        let mut out = Vol::filled(n, 0);
        for y in 0..n[1] {
            for z in 0..n[2] {
                for x in 0..n[0] {
                    out.set([x, y, z], self.at([at[0] + x, at[1] + y, at[2] + z]));
                }
            }
        }
        out
    }

    /** Turned about y by `t` quarter turns (t mod 4), mirrored along x if t ≥ 4; x and z must be as long. */
    pub fn turned(&self, t: usize) -> Vol {
        let [nx, ny, nz] = self.n;
        assert_eq!(nx, nz, "a square base");
        let mut out = Vol::filled(self.n, 0);
        for y in 0..ny {
            for z in 0..nz {
                for x in 0..nx {
                    let m = if t >= 4 { nx - 1 - x } else { x };
                    let (a, b) = match t % 4 {
                        0 => (m, z),
                        1 => (nz - 1 - z, m),
                        2 => (nx - 1 - m, nz - 1 - z),
                        _ => (z, nx - 1 - m),
                    };
                    out.set([a, y, b], self.at([x, y, z]));
                }
            }
        }
        out
    }

    pub fn is_flat(&self) -> bool {
        self.n[1] == 1
    }

    /**
     * As RGB from above, `scale` pixels a cell, +z up: each column's
     * topmost label not `air`, lit by its height (higher lighter) and by the
     * slope towards the north-west, so relief and overhangs read.
     */
    pub fn top_rgb(&self, rgb: &dyn Fn(u16) -> u32, air: u16, scale: usize) -> (usize, usize, Vec<u8>) {
        let [nx, ny, nz] = self.n;
        let top = |x: usize, z: usize| (0..ny).rev().find(|y| self.at([x, *y, z]) != air);
        let (w, h) = (nx * scale, nz * scale);
        let mut out = vec![0u8; w * h * 3];
        for z in 0..nz {
            for x in 0..nx {
                let (c, light) = match top(x, z) {
                    None => (0xf4f4f8, 1.0),
                    Some(y) if ny == 1 => (rgb(self.at([x, y, z])), 1.0),
                    Some(y) => {
                        let up = |x: usize, z: usize| top(x, z).map_or(-4.0, |y| y as f64);
                        let slope = up(x, z) - up(x.saturating_sub(1), (z + 1).min(nz - 1));
                        let height = 0.75 + 0.35 * y as f64 / ny as f64;
                        (rgb(self.at([x, y, z])), (height + 0.06 * slope.clamp(-4.0, 4.0)).clamp(0.3, 1.25))
                    }
                };
                let px = [(c >> 16) as u8, (c >> 8) as u8, c as u8].map(|v| (v as f64 * light).clamp(0.0, 255.0) as u8);
                for j in 0..scale {
                    for i in 0..scale {
                        let (px_x, px_y) = (x * scale + i, (nz - 1 - z) * scale + j);
                        out[(px_y * w + px_x) * 3..][..3].copy_from_slice(&px);
                    }
                }
            }
        }
        (w, h, out)
    }

    /** As RGB, the level y seen from above: +z up, `scale` pixels a cell; air pale. */
    pub fn level_rgb(&self, y: usize, rgb: &dyn Fn(u16) -> u32, air: u16, scale: usize) -> (usize, usize, Vec<u8>) {
        let [nx, _, nz] = self.n;
        let (w, h) = (nx * scale, nz * scale);
        let mut out = vec![0u8; w * h * 3];
        for z in 0..nz {
            for x in 0..nx {
                let l = self.at([x, y, z]);
                let c = if l == air { 0xf4f4f8 } else { rgb(l) };
                for j in 0..scale {
                    for i in 0..scale {
                        let (px, py) = (x * scale + i, (nz - 1 - z) * scale + j);
                        out[(py * w + px) * 3..][..3].copy_from_slice(&[(c >> 16) as u8, (c >> 8) as u8, c as u8]);
                    }
                }
            }
        }
        (w, h, out)
    }

    /** As RGB, the slice at z: x across, y up, `scale` pixels a cell; air pale. */
    pub fn slice_rgb(&self, z: usize, rgb: &dyn Fn(u16) -> u32, air: u16, scale: usize) -> (usize, usize, Vec<u8>) {
        let [nx, ny, _] = self.n;
        let (w, h) = (nx * scale, ny * scale);
        let mut out = vec![0u8; w * h * 3];
        for y in 0..ny {
            for x in 0..nx {
                let l = self.at([x, y, z]);
                let c = if l == air { 0xf4f4f8 } else { rgb(l) };
                for j in 0..scale {
                    for i in 0..scale {
                        let (px, py) = (x * scale + i, (ny - 1 - y) * scale + j);
                        out[(py * w + px) * 3..][..3].copy_from_slice(&[(c >> 16) as u8, (c >> 8) as u8, c as u8]);
                    }
                }
            }
        }
        (w, h, out)
    }
}
