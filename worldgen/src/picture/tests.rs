use super::*;
use crate::generator::{ANY_SIZE, KindDef, Layer, Node, Op, Population, SOLID, ball};
use fcc::light::linear_to_srgb;

#[test]
fn checksums_are_zlib_and_png_ones() {
    assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    assert_eq!(adler32(b"Wikipedia"), 0x11E6_0398);
    assert_eq!(adler32(&vec![255; 100_000]), {
        let (mut a, mut b) = (1u64, 0u64);
        for _ in 0..100_000 {
            a = (a + 255) % 65521;
            b = (b + a) % 65521;
        }
        ((b << 16) | a) as u32
    });
}

#[test]
fn a_png_holds_its_pixels() {
    let (width, height) = (300, 250); // more than one stored block
    let rgb: Vec<u8> = (0..3 * width * height).map(|i| (i * 7 % 251) as u8).collect();
    let picture = Picture {
        width,
        height,
        rgb: rgb.clone(),
        looks: vec![0; width * height],
        depth: vec![0.0; width * height],
        sites: vec![[0; 3]; width * height],
        tiles: 0,
        generating: Duration::ZERO,
        casting: Duration::ZERO,
    };
    let png = picture.png();
    assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
    assert_eq!(&png[png.len() - 12..], [0, 0, 0, 0, b'I', b'E', b'N', b'D', 0xAE, 0x42, 0x60, 0x82]);
    // Its chunks, each with its CRC; the image data inflated from its stored blocks.
    let (mut at, mut idat) = (8, Vec::new());
    while at < png.len() {
        let length = u32::from_be_bytes(png[at..at + 4].try_into().unwrap()) as usize;
        let body = &png[at + 4..at + 8 + length];
        assert_eq!(crc32(body), u32::from_be_bytes(png[at + 8 + length..at + 12 + length].try_into().unwrap()));
        match &body[..4] {
            b"IHDR" => assert_eq!(body[4..], [0, 0, 1, 44, 0, 0, 0, 250, 8, 2, 0, 0, 0]),
            b"IDAT" => idat.extend_from_slice(&body[4..]),
            _ => {}
        }
        at += 12 + length;
    }
    assert_eq!((u16::from_be_bytes([idat[0], idat[1]]) % 31, idat[0] & 15), (0, 8));
    let (mut raw, mut at) = (Vec::new(), 2);
    loop {
        let length = u16::from_le_bytes([idat[at + 1], idat[at + 2]]) as usize;
        assert_eq!(!length as u16, u16::from_le_bytes([idat[at + 3], idat[at + 4]]));
        raw.extend_from_slice(&idat[at + 5..at + 5 + length]);
        let last = idat[at] & 1 == 1;
        at += 5 + length;
        if last {
            break;
        }
    }
    assert_eq!(u32::from_be_bytes(idat[at..at + 4].try_into().unwrap()), adler32(&raw));
    let rows: Vec<u8> = raw
        .chunks(3 * width + 1)
        .flat_map(|row| {
            assert_eq!(row[0], 0);
            row[1..].to_vec()
        })
        .collect();
    assert_eq!(rows, rgb);
}

/** A ball of radius 40 at the origin, on the corner of four tiles. */
fn a_ball() -> Generator {
    Generator {
        kinds: vec![KindDef { name: "red", kind_type: 1, colors: &["#c04030"], walk_through: false, glows: false, medium: false }],
        seed: 1,
        layers: vec![Layer::Population(
            Population::new("ball", 1.0, 1024)
                .reach(64)
                .size(ANY_SIZE)
                .anchor(|cell| if cell.i == 0 && cell.j == 0 && cell.k == 0 { Some([0.0; 3]) } else { None })
                .make(|at, _| Some(Node::of(vec![ball(at, 40.0, 0, SOLID, Op::Fill)]))),
        )],
    }
}

#[test]
fn a_ray_sees_a_voxel_along_its_direction_wherever_it_enters() {
    // A voxel white through its faces up, black through the others.
    let mut grid = VoxelGrid::new(&Box3 { min: [-4; 3], max: [4; 3] });
    let white = |k: usize| if FACES[k][1] > 0 { COLOUR_ONE } else { 0 };
    grid.put_whole(0, 0, 0, std::array::from_fn(|k| [white(k); 3]));
    let seen = |origin: [f64; 3], direction: [f64; 3]| {
        let direction = normalize(direction);
        first_hit(&grid, origin, direction, 8.0).map(|(_, [x, y, z], face)| (grid.along(x, y, z, direction) & 255, FACES[face][1]))
    };
    assert_eq!(seen([0.1, 3.0, 0.2], [0.0, -1.0, 0.0]), Some((255, 1)));
    // Along +x, by its four faces towards −x, one of them up: a quarter of it.
    let srgb = |linear: f32| (255.0 * linear_to_srgb(linear)).round() as u32;
    assert_eq!(seen([-3.0, 0.1, 0.2], [1.0, 0.0, 0.0]), Some((srgb(0.25), 0)));
    // A little down, it enters by a face down and sees it by the face turned most against it, up.
    assert_eq!(seen([-3.0, -0.2, 0.1], [1.0, -0.1, 0.0]), Some((255, -1)));
    // Straight down onto a ridge of two faces up, as much of each.
    let mut ridge = VoxelGrid::new(&Box3 { min: [-4; 3], max: [4; 3] });
    let half = |k: usize| if FACES[k] == [1, 1, 0] || FACES[k] == [0, 1, 1] { COLOUR_ONE } else { 0 };
    ridge.put_whole(0, 0, 0, std::array::from_fn(|k| [half(k); 3]));
    let grey = ridge.along(0, 0, 0, [0.0, -1.0, 0.0]) & 255;
    assert_eq!(grey, srgb(0.5));
}

#[test]
fn a_ray_cut_in_two_stops_what_it_stops_whole() {
    // A medium all through: what gets through a stretch is what gets through its two parts, wherever it is cut,
    // the voxel cut in two counted by the part of it each crosses.
    let mut grid = VoxelGrid::new(&Box3 { min: [-8; 3], max: [8; 3] });
    for (x, y, z) in (-8..8).flat_map(|z| (-8..8).flat_map(move |y| (-8..8).map(move |x| (x, y, z)))) {
        if let Some(i) = grid.index(x, y, z) {
            grid.medium.set(i, [ALPHA / 2; 3], [0; 3]);
        }
    }
    let (origin, direction) = ([-3.1, -2.3, -1.7], normalize([0.8, 0.5, 0.3]));
    let through = |from: f64, to: f64| {
        let start = [0, 1, 2].map(|i| origin[i] + from * direction[i]);
        1.0 - composited(&grid, start, direction, to - from, 1.0).iter().map(|(_, seen)| seen.stopped).sum::<f64>()
    };
    for cut in [0.3, 1.1, 2.5, 3.7, 4.9] {
        let (whole, parts) = (through(0.0, 6.0), through(0.0, cut) * through(cut, 6.0));
        assert!((whole - parts).abs() < 1e-9, "cut at {cut}: {whole} whole, {parts} in parts");
    }
}

#[test]
fn a_picture_of_a_ball_shows_it_in_the_middle() {
    let generator = a_ball();
    for level in [0, 2] {
        let heights = (-64 >> level, 64 >> level);
        // From 200 units away, 60° across: the ball spans about a fifth of the picture.
        let camera = Camera::looking_at([0.0, 0.0, -200.0], [0.0; 3], 60f64.to_radians(), 64, 48, 1000.0);
        let seen = picture(&generator, &camera, level, heights);
        let middle = 24 * 64 + 32;
        assert_ne!(seen.looks[middle], 0, "level {level}: the ball is in the middle");
        assert!((155.0..=162.0).contains(&seen.depth[middle]), "level {level}: {} away", seen.depth[middle]);
        assert_eq!((seen.looks[0], seen.depth[0]), (0, f32::INFINITY), "level {level}: the corner misses");
        let threaded = picture_on_threads(&generator, &camera, level, heights);
        assert!(threaded.rgb == seen.rgb && threaded.depth == seen.depth, "level {level}: the same on threads");
        let from_above = picture(&generator, &Camera::top([-100.0, -100.0], [100.0, 100.0], 32), level, heights);
        assert_eq!((from_above.looks[16 * 32 + 16] != 0, from_above.looks[0]), (true, 0), "level {level}: from above");
    }
}
