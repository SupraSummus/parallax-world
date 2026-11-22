use super::*;

#[test]
fn checksums_are_zlib_and_png_ones() {
    // Values from zlib; a long input, as adler32 reduces its sums only every so often.
    assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    assert_eq!(adler32(&[255; 100_000]), 0x149A_302C);
}

/** Bits read from the least significant, as deflate packs them. */
struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl Reader<'_> {
    fn bits(&mut self, n: u32) -> u32 {
        let mut value = 0;
        for i in 0..n {
            value |= u32::from(self.bytes[self.at / 8] >> (self.at % 8) & 1) << i;
            self.at += 1;
        }
        value
    }

    /** A Huffman code of `n` bits more, read from its most significant. */
    fn code(&mut self, code: u32, n: u32) -> u32 {
        (0..n).fold(code, |code, _| code << 1 | self.bits(1))
    }

    /** A symbol of the fixed literal and length codes. */
    fn literal(&mut self) -> u32 {
        let code = self.code(0, 7);
        if code <= 23 {
            return 256 + code;
        }
        let code = self.code(code, 1);
        match code {
            48..=191 => code - 48,
            192..=199 => 280 + code - 192,
            _ => 144 + self.code(code, 1) - 400,
        }
    }
}

/** A zlib stream of one block of fixed codes, inflated: by the encoder's tables, so it was checked against zlib once too. */
fn inflate(zlib: &[u8]) -> Vec<u8> {
    assert_eq!((u16::from_be_bytes([zlib[0], zlib[1]]) % 31, zlib[0] & 15), (0, 8), "a zlib header");
    let mut reader = Reader { bytes: &zlib[2..], at: 0 };
    assert_eq!((reader.bits(1), reader.bits(2)), (1, 1), "the last block, of fixed codes");
    let mut out: Vec<u8> = Vec::new();
    loop {
        match reader.literal() {
            256 => break,
            symbol @ 0..=255 => out.push(symbol as u8),
            symbol => {
                let c = (symbol - 257) as usize;
                let length = LENGTH_BASE[c] + reader.bits(LENGTH_EXTRA[c]);
                let d = reader.code(0, 5) as usize;
                let distance = (DIST_BASE[d] + reader.bits(DIST_EXTRA[d])) as usize;
                for _ in 0..length {
                    out.push(out[out.len() - distance]);
                }
            }
        }
    }
    let end = 2 + reader.at.div_ceil(8);
    assert_eq!(u32::from_be_bytes(zlib[end..end + 4].try_into().unwrap()), adler32(&out));
    out
}

#[test]
fn a_png_holds_its_pixels() {
    for (width, height, channels) in [(300, 250, 3), (17, 5, 4)] {
        // Runs and noise, so it is both matched and literal.
        let pixels: Vec<u8> =
            (0..channels * width * height).map(|i| if i % 997 < 500 { (i / 300 % 7) as u8 } else { (i * 7 % 251) as u8 }).collect();
        let png = encode(width, height, channels, &pixels);
        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
        let (mut at, mut chunks) = (8, Vec::new());
        while at < png.len() {
            let length = u32::from_be_bytes(png[at..at + 4].try_into().unwrap()) as usize;
            let (body, crc) = (&png[at + 4..at + 8 + length], &png[at + 8 + length..at + 12 + length]);
            assert_eq!(crc32(body).to_be_bytes(), crc);
            chunks.push(body);
            at += 12 + length;
        }
        let [header, data, end] = chunks[..] else { panic!("{} chunks", chunks.len()) };
        assert_eq!((&header[..4], &data[..4], end), (&b"IHDR"[..], &b"IDAT"[..], &b"IEND"[..]));
        assert_eq!(header[4..14], [&(width as u32).to_be_bytes()[..], &(height as u32).to_be_bytes(), &[8, [2, 6][channels - 3]]].concat());
        let rows = inflate(&data[4..]);
        assert!(rows.chunks(channels * width + 1).all(|row| row[0] == 0), "no filter");
        let unfiltered: Vec<u8> = rows.chunks(channels * width + 1).flat_map(|row| row[1..].to_vec()).collect();
        assert!(unfiltered == pixels, "{width} × {height} × {channels}: the pixels come back");
    }
}
