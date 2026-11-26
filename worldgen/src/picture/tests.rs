use super::*;
use crate::generator::{ANY_SIZE, KindDef, Layer, Node, Op, Population, SOLID, ball};
use lattice::light::linear_to_srgb;

/** A ball of radius 40 at the origin, on the corner of four tiles. */
fn a_ball() -> Generator {
    Generator {
        kinds: vec![KindDef { name: "red", kind_type: 1, colors: &["#c04030"], walk_through: false, glows: false, medium: false }],
        seed: 1,
        layers: vec![Layer::Population(
            Population::new("ball", 1.0, 1024)
                .reach(64)
                .size(ANY_SIZE)
                .at(|_| vec![([0.0; 3], ())], |_, anchor| Some(Node::of(vec![ball(anchor.at, 40.0, 0, SOLID, Op::Fill)]))),
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
        lattice::ray::first_hit(origin, direction, 8.0, |[x, y, z]| grid.get(x, y, z) != 0)
            .map(|(_, [x, y, z], face)| (grid.along(x, y, z, direction) & 255, FACES[face][1]))
    };
    assert_eq!(seen([0.1, 3.0, 0.2], [0.0, -1.0, 0.0]), Some((255, 1)));
    // Along +x, by its face towards −x.
    assert_eq!(seen([-3.0, 0.1, 0.2], [1.0, 0.0, 0.0]), Some((0, 0)));
    // A little down, it enters by its face up and sees it by the face turned most against it, towards −x.
    assert_eq!(seen([-3.0, 2.3, 0.5], [1.0, -0.4, 0.0]), Some((0, 1)));
    // Down along a diagonal, by two faces alike, as much of each.
    let srgb = |linear: f32| (255.0 * linear_to_srgb(linear)).round() as u32;
    assert_eq!(grid.along(0, 0, 0, [1.0, -1.0, 0.0]) & 255, srgb(0.5));
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
        1.0 - composited(&grid, start, direction, to - from).iter().map(|(_, seen)| seen.stopped).sum::<f64>()
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
