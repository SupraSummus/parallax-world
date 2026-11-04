//! A small PNG encoder: RGB, 8 bits, deflated with LZ77 and the fixed
//! Huffman codes. Label maps are runs and repeats of the row above, which
//! that finds without dynamic codes.

/** Writes `rgb` (`w` × `h` pixels, rows top first, 3 bytes each) to `path` as a PNG. */
pub fn write(path: &str, w: usize, h: usize, rgb: &[u8]) {
    assert_eq!(rgb.len(), w * h * 3, "{path}: {w} × {h} pixels");
    let mut raw = Vec::with_capacity((w * 3 + 1) * h);
    for row in rgb.chunks(w * 3) {
        raw.push(0); // filter: none
        raw.extend_from_slice(row);
    }
    let mut ihdr = Vec::new();
    ihdr.extend((w as u32).to_be_bytes());
    ihdr.extend((h as u32).to_be_bytes());
    ihdr.extend([8, 2, 0, 0, 0]);
    let mut png = vec![0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1a, b'\n'];
    chunk(&mut png, b"IHDR", &ihdr);
    chunk(&mut png, b"IDAT", &zlib(&raw));
    chunk(&mut png, b"IEND", &[]);
    if let Some(dir) = std::path::Path::new(path).parent() {
        std::fs::create_dir_all(dir).expect("output directory");
    }
    std::fs::write(path, png).expect("PNG written");
}

fn chunk(png: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    png.extend((data.len() as u32).to_be_bytes());
    let start = png.len();
    png.extend_from_slice(kind);
    png.extend_from_slice(data);
    let crc = crc32(&png[start..]);
    png.extend(crc.to_be_bytes());
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &b in bytes {
        crc ^= b as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 { 0xedb88320 ^ (crc >> 1) } else { crc >> 1 };
        }
    }
    !crc
}

fn adler32(bytes: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for block in bytes.chunks(5552) {
        for &x in block {
            a += x as u32;
            b += a;
        }
        a %= 65521;
        b %= 65521;
    }
    (b << 16) | a
}

/** Bits packed from the least significant, as deflate wants. */
struct Bits {
    out: Vec<u8>,
    acc: u64,
    n: u32,
}

impl Bits {
    fn put(&mut self, value: u32, bits: u32) {
        self.acc |= (value as u64) << self.n;
        self.n += bits;
        while self.n >= 8 {
            self.out.push(self.acc as u8);
            self.acc >>= 8;
            self.n -= 8;
        }
    }

    /** A Huffman code, which deflate packs from its most significant bit. */
    fn code(&mut self, code: u32, bits: u32) {
        self.put(code.reverse_bits() >> (32 - bits), bits);
    }

    fn literal(&mut self, symbol: u32) {
        match symbol {
            0..=143 => self.code(0x30 + symbol, 8),
            144..=255 => self.code(0x190 + symbol - 144, 9),
            256..=279 => self.code(symbol - 256, 7),
            _ => self.code(0xc0 + symbol - 280, 8),
        }
    }
}

const LENGTH_BASE: [u32; 29] =
    [3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131, 163, 195, 227, 258];
const LENGTH_EXTRA: [u32; 29] = [0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0];
const DIST_BASE: [u32; 30] = [
    1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537, 2049, 3073, 4097, 6145, 8193, 12289, 16385,
    24577,
];
const DIST_EXTRA: [u32; 30] = [0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13, 13];

/** `data` as a zlib stream: one deflate block with the fixed codes, matches found by hash chains. */
fn zlib(data: &[u8]) -> Vec<u8> {
    const WINDOW: usize = 32768;
    const CHAIN: usize = 24;
    let mut bits = Bits { out: vec![0x78, 0x01], acc: 0, n: 0 };
    bits.put(1, 1); // the last block
    bits.put(1, 2); // fixed codes
    let key = |i: usize| ((((data[i] as usize) << 16 | (data[i + 1] as usize) << 8 | data[i + 2] as usize) * 2654435761) >> 17) & 0x7fff;
    let mut head = vec![usize::MAX; 1 << 15];
    let mut prev = vec![usize::MAX; data.len()];
    let insert = |i: usize, head: &mut [usize], prev: &mut [usize]| {
        if i + 2 < data.len() {
            let k = key(i);
            prev[i] = head[k];
            head[k] = i;
        }
    };
    let mut i = 0;
    while i < data.len() {
        let (mut best, mut at) = (0, 0);
        if i + 2 < data.len() {
            let mut j = head[key(i)];
            for _ in 0..CHAIN {
                if j == usize::MAX || i - j > WINDOW {
                    break;
                }
                let n = (0..258.min(data.len() - i)).take_while(|&n| data[j + n] == data[i + n]).count();
                if n > best {
                    (best, at) = (n, i - j);
                }
                j = prev[j];
            }
        }
        if best >= 3 {
            let c = LENGTH_BASE.iter().rposition(|&b| b as usize <= best).expect("a length code");
            bits.literal(257 + c as u32);
            bits.put(best as u32 - LENGTH_BASE[c], LENGTH_EXTRA[c]);
            let d = DIST_BASE.iter().rposition(|&b| b as usize <= at).expect("a distance code");
            bits.code(d as u32, 5);
            bits.put(at as u32 - DIST_BASE[d], DIST_EXTRA[d]);
            for k in i..i + best {
                insert(k, &mut head, &mut prev);
            }
            i += best;
        } else {
            bits.literal(data[i] as u32);
            insert(i, &mut head, &mut prev);
            i += 1;
        }
    }
    bits.literal(256);
    let mut out = bits.out;
    if bits.n > 0 {
        out.push(bits.acc as u8);
    }
    out.extend(adler32(data).to_be_bytes());
    out
}
