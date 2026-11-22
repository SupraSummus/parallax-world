use super::*;

/** The world of a seed. */
type Make = fn(u64) -> worldgen::World;

#[test]
fn the_options_are_well_formed() {
    <Args as clap::CommandFactory>::command().debug_assert();
}

/** What --record writes and --replay reads, and a panic reports. */
#[test]
fn a_view_gives_back_the_options_it_takes() {
    let options = "--engine flat --camera 1.5,-2,3 --look -30,10 --fov 70 --distance 500 --detail 4 --shift 2 --colour layers \
                   --size 320x200 --light 0.5,1,0,3,0.01 --eye -1,0.5 --seams --unhidden --walk";
    let argv: Vec<String> =
        ["", "--world", "moor", "--threads", "1"].into_iter().chain(options.split_whitespace()).map(str::to_string).collect();
    assert_eq!(View::new(&parse(&argv)).options(), options.split_whitespace().collect::<Vec<_>>().join(" "));
}

/** Black, a medium; white, a wall. */
const MEDIUM: u8 = 0;
const WALL: u8 = 1;
/** The medium between planes x + z = NEAR and FAR, the wall from FAR on. */
const NEAR: i64 = 60;
const FAR: i64 = 160;

/**
 * A wall across the axis (1, 0, 1) of a face, behind a slab of black
 * medium of `MILLI` thousandths of extinction per lattice unit, around the origin.
 */
fn wall<const MILLI: u32>(_: u64) -> worldgen::World {
    use worldgen::generator::{Field, Generator, KindDef, Layer, Op};
    let kinds = vec![
        KindDef { name: "medium", kind_type: 8, colors: &["#000000"], walk_through: true, glows: false, medium: true },
        KindDef { name: "wall", kind_type: 10, colors: &["#ffffff"], walk_through: false, glows: false, medium: false },
    ];
    let slab = Field::at("slab", 0.0, Op::Fill, |x, y, z| match x + z {
        _ if y.abs() >= 48 => None,
        k if (NEAR..FAR).contains(&k) && MILLI > 0 => Some((MEDIUM, Some(f64::from(MILLI) / 1000.0))),
        k if (FAR..FAR + 16).contains(&k) => Some((WALL, None)),
        _ => None,
    });
    let generator = Generator { kinds, seed: 1, layers: vec![Layer::Field(slab)] };
    worldgen::World { generator, spawn: [0, 0, 0], heights: Some((-64, 64)), points: Vec::new() }
}

/** What `kind` draws of the world `make` makes, from its spawn, looking along the face (1, 0, 1), without fog: its width and pixels. */
fn along_a_face(kind: Kind, make: Make) -> (u32, Vec<u8>) {
    let mut world = World::new(make(1), None, 4);
    let spawn = make(1).spawn.map(|v| v as f64);
    let look = Look { fog: f64::INFINITY, detail: 8.0, ..Look::new(spawn, [45.0, 0.0], FOV, 1000.0, [320.0, 200.0]) };
    (320, drawn(&mut *kind.make(&world), &mut world, &look))
}

/** The pixels of the view of `look` as `engine` draws it once complete, at the look's size. */
fn drawn(engine: &mut dyn Engine, world: &mut World, look: &Look) -> Vec<u8> {
    let mut easel = Easel::offscreen(None);
    let frame = drawn_complete(engine, world, look, &mut easel);
    easel.pixels(&*frame.picture, look.size.map(|v| v as u32)).2
}

/** Red ground under y = 0, endless. */
fn ground(_: u64) -> worldgen::World {
    use worldgen::generator::{Field, Generator, KindDef, Layer, Op};
    let kinds = vec![KindDef { name: "ground", kind_type: 10, colors: &["#ff0000"], walk_through: false, glows: false, medium: false }];
    let ground = Field::at("ground", 0.0, Op::Fill, |_, y, _| (y < 0).then_some((0, None)));
    let generator = Generator { kinds, seed: 1, layers: vec![Layer::Field(ground)] };
    worldgen::World { generator, spawn: [0, 16, 0], heights: Some((-64, 64)), points: Vec::new() }
}

/**
 * Flat ground seen nearly along the axis of a face, (1, 0, 1), parallel
 * to it, where flat pictures of cells part most (engine::chunks's Flat),
 * shows no sky between them: wherever a ray meets the ground near
 * enough, the picture is the ground's red, not the sky's blue.
 */
#[test]
#[ignore = "draws on the GPU: needs Vulkan, as Mesa's software one"]
fn ground_along_a_face_shows_no_gaps() {
    const PIXELS: [u32; 2] = [320, 200];
    for kind in [Kind::Reliefs, Kind::Flat] {
        let mut world = World::new(ground(1), None, 4);
        let look = Look { fog: f64::INFINITY, ..Look::new([0.0, 16.0, 0.0], [45.0, -10.0], FOV, 1000.0, PIXELS.map(f64::from)) };
        let (width, pixels) = (PIXELS[0], drawn(&mut *kind.make(&world), &mut world, &look));
        let [right, up, forward] = view::view_basis(look.yaw, look.pitch);
        let focal = look.view().focal;
        let (mut ground, mut gaps) = (0, 0);
        for j in 0..PIXELS[1] {
            for i in 0..PIXELS[0] {
                let [x, y] = [(f64::from(i) + 0.5 - 160.0) / focal, (100.0 - f64::from(j) - 0.5) / focal];
                let ray: [f64; 3] = std::array::from_fn(|d| forward[d] + x * right[d] + y * up[d]);
                // Where it meets y = 0, if within a fifth of the render distance.
                let length = ray.iter().map(|v| v * v).sum::<f64>().sqrt();
                if ray[1] >= 0.0 || 16.0 / -ray[1] * length > 200.0 {
                    continue;
                }
                let at = 4 * (j * width + i) as usize;
                ground += 1;
                gaps += usize::from(pixels[at + 2] >= pixels[at]);
            }
        }
        assert!(ground > 10_000 && gaps == 0, "{kind:?}: {gaps} of {ground} pixels of the ground show the sky");
    }
}

/** `ground`, under 2 units of blue medium that lets next to nothing through. */
fn flooded(seed: u64) -> worldgen::World {
    use worldgen::generator::{Field, KindDef, Layer, Op};
    let mut world = ground(seed);
    world.generator.kinds.push(KindDef {
        name: "water",
        kind_type: 8,
        colors: &["#0000ff"],
        walk_through: true,
        glows: false,
        medium: true,
    });
    world.generator.layers.push(Layer::Field(Field::at("water", 0.0, Op::Fill, |_, y, _| (0..2).contains(&y).then_some((1, Some(1.0))))));
    world
}

/**
 * A medium thin along a face's axis, on the ground, hides it from above,
 * though rays cross it between the planes its slabs' pictures are on,
 * many times its depth apart (veil_fragment), and meet the ground within
 * a slab (veil_vertex).
 */
#[test]
#[ignore = "draws on the GPU: needs Vulkan, as Mesa's software one"]
fn a_thin_medium_on_the_ground_hides_it_from_above() {
    const PIXELS: [u32; 2] = [320, 200];
    for kind in [Kind::Reliefs, Kind::Flat] {
        let mut world = World::new(flooded(1), None, 4);
        let look = Look { fog: f64::INFINITY, ..Look::new([0.0, 32.0, 0.0], [45.0, -25.0], FOV, 300.0, PIXELS.map(f64::from)) };
        let pixels = drawn(&mut *kind.make(&world), &mut world, &look);
        // The lower half of the picture is all ground; right under the eye the medium is a little thin (docs/todo.md).
        let ground = pixels.chunks(4).skip((PIXELS[0] * PIXELS[1] / 2) as usize);
        let bare = ground.clone().filter(|p| p[0] > p[2]).count();
        assert!(bare * 50 < ground.len(), "{kind:?}: {bare} of {} pixels of the ground show through", ground.len());
    }
}

#[test]
#[ignore = "draws on the GPU: needs Vulkan, as Mesa's software one"]
fn a_medium_of_reliefs_lets_through_as_much_as_its_extinction_says() {
    medium_lets_through_as_much_as_its_extinction_says(Kind::Reliefs);
}

#[test]
#[ignore = "draws on the GPU: needs Vulkan, as Mesa's software one"]
fn a_medium_of_voxels_lets_through_as_much_as_its_extinction_says() {
    medium_lets_through_as_much_as_its_extinction_says(Kind::Voxels);
}

/** A wall behind a slab of medium, as `kind` draws it, lets through exp(−extinction · path), thin and thick, along a face's axis and slanted. */
fn medium_lets_through_as_much_as_its_extinction_says(kind: Kind) {
    // The mean of a box of 9 × 9 pixels around (x, y) of the wall, seen along the face's axis.
    let seen = |make: Make| {
        let (width, pixels) = along_a_face(kind, make);
        move |[x, y]: [usize; 2]| {
            let at = |i: usize, j: usize| f64::from(pixels[4 * (j * width as usize + i)]);
            (y - 4..=y + 4).flat_map(|j| (x - 4..=x + 4).map(move |i| (i, j))).map(|(i, j)| at(i, j)).sum::<f64>() / 81.0
        }
    };
    let bare = seen(wall::<0>);
    // The slab is (FAR − NEAR) / √2 thick along the axis; a ray at tangent t crosses √(1 + t²) times that.
    let thick = (FAR - NEAR) as f64 / std::f64::consts::SQRT_2;
    // Thin, a site stops a few SHARE-ths of a step of alpha.
    for (milli, make) in [(1, wall::<1> as Make), (4, wall::<4>), (15, wall::<15>)] {
        let veiled = seen(make);
        for (t, at) in [(0.0, [160, 100]), (0.5, [240, 100])] {
            let through = veiled(at) / bare(at);
            let expected = (-f64::from(milli) / 1000.0 * thick * (1.0f64 + t * t).sqrt()).exp();
            assert!((through - expected).abs() < 0.01, "{milli}‰ at tangent {t}: {through:.3} let through, {expected:.3} expected");
        }
    }
}

/** A wall, white if `WHITE`, else black, behind a slab of pink medium if `VEILED`, as `wall`'s. No channel of the pink at 255: thin, its look would clip it. */
fn pink<const WHITE: bool, const VEILED: bool>(_: u64) -> worldgen::World {
    use worldgen::generator::{Field, Generator, KindDef, Layer, Op};
    let kinds = vec![
        KindDef { name: "medium", kind_type: 8, colors: &["#bc8989"], walk_through: true, glows: false, medium: true },
        KindDef {
            name: "wall",
            kind_type: 10,
            colors: if WHITE { &["#ffffff"] } else { &["#000000"] },
            walk_through: false,
            glows: false,
            medium: false,
        },
    ];
    let slab = Field::at("slab", 0.0, Op::Fill, |x, y, z| match x + z {
        _ if y.abs() >= 48 => None,
        k if (NEAR..FAR).contains(&k) && VEILED => Some((MEDIUM, Some(0.015))),
        k if (FAR..FAR + 16).contains(&k) => Some((WALL, None)),
        _ => None,
    });
    let generator = Generator { kinds, seed: 1, layers: vec![Layer::Field(slab)] };
    worldgen::World { generator, spawn: [0, 0, 0], heights: Some((-64, 64)), points: Vec::new() }
}

/**
 * A medium stops each channel the more, the darker its colour there
 * (worldgen's stops_faster, as voxels.wgsl's lets_through draws it): pink, half as bright (linear) in green as in red,
 * lets through of red what its extinction says, and of green that twice over.
 * What a white wall behind it shows over a black one is what it lets through.
 */
#[test]
#[ignore = "draws on the GPU: needs Vulkan, as Mesa's software one"]
fn a_medium_of_voxels_stops_more_of_the_channels_its_colour_is_darker_in() {
    // The mean of red and of green over 9 × 9 pixels in the middle of the picture.
    let seen = |make: Make| {
        let (width, pixels) = along_a_face(Kind::Voxels, make);
        [0, 1].map(|c| {
            (96..=104)
                .flat_map(|j| (156..=164).map(move |i| (i, j)))
                .map(|(i, j)| f64::from(pixels[4 * (j * width as usize + i) + c]))
                .sum::<f64>()
                / 81.0
        })
    };
    let [bare, white, black] = [seen(pink::<true, false>), seen(pink::<true, true>), seen(pink::<false, true>)];
    let [red, green] = [0, 1].map(|c| (white[c] - black[c]) / bare[c]);
    let expected = (-0.015 * (FAR - NEAR) as f64 / std::f64::consts::SQRT_2).exp();
    assert!((red - expected).abs() < 0.01, "red: {red:.3} let through, {expected:.3} expected");
    let [most, less] = [0xbc, 0x89].map(|c| f64::from(fcc::light::srgb_to_linear(f32::from(c as u8) / 255.0)));
    let expected = expected.powf(most / less);
    assert!((green - expected).abs() < 0.01, "green: {green:.3} let through, {expected:.3} expected");
}

/** A white wall behind two slabs of medium, a red one and a green one, the red nearer if `RED_NEAR`. */
fn red_and_green<const RED_NEAR: bool>(_: u64) -> worldgen::World {
    use worldgen::generator::{Field, Generator, KindDef, Layer, Op};
    let medium = |name, colour| KindDef { name, kind_type: 8, colors: colour, walk_through: true, glows: false, medium: true };
    let kinds = vec![
        medium("red", &["#ff0000"]),
        medium("green", &["#00ff00"]),
        KindDef { name: "wall", kind_type: 10, colors: &["#ffffff"], walk_through: false, glows: false, medium: false },
    ];
    let middle = (NEAR + FAR) / 2;
    let slab = Field::at("slabs", 0.0, Op::Fill, move |x, y, z| match x + z {
        _ if y.abs() >= 48 => None,
        k if (NEAR..FAR).contains(&k) => Some((u8::from((k < middle) != RED_NEAR), Some(0.04))),
        k if (FAR..FAR + 16).contains(&k) => Some((2, None)),
        _ => None,
    });
    let generator = Generator { kinds, seed: 1, layers: vec![Layer::Field(slab)] };
    worldgen::World { generator, spawn: [0, 0, 0], heights: Some((-64, 64)), points: Vec::new() }
}

/** The nearer of two media shows over the other, not mixed with it as if in any order. */
#[test]
#[ignore = "draws on the GPU: needs Vulkan, as Mesa's software one"]
fn a_nearer_medium_shows_over_one_behind() {
    for kind in [Kind::Reliefs, Kind::Voxels] {
        // Red less green in the middle of the picture, with red nearer and with green nearer.
        let [red_near, green_near] = [red_and_green::<true> as Make, red_and_green::<false>].map(|make| {
            let (width, pixels) = along_a_face(kind, make);
            let at = 4 * (100 * width as usize + 160);
            i32::from(pixels[at]) - i32::from(pixels[at + 1])
        });
        // Each slab stops 76 %, so the nearer shows 0.76 of its colour, the one behind 0.24 · 0.76 = 0.18.
        assert!(red_near > 60 && green_near < -60, "{kind:?}: red less green {red_near} with red nearer, {green_near} with green");
    }
}

/**
 * A white block with a red sheet on top, drawn measured from afar
 * (Far::measured): its coarse voxels are red on the faces that look up,
 * through it to the sheet, and white on the others. Both glow, so that
 * lit from below, where no light comes, they show.
 */
fn sheeted_block(_: u64) -> worldgen::World {
    use worldgen::generator::{Far, Generator, KindDef, Layer, Node, Population, Shape};
    let kind = |colors| KindDef { name: "block", kind_type: 10, colors, walk_through: false, glows: true, medium: false };
    let block = Population::new("block", 1.0, 4096).anchor(|cell| (cell.centre == [0; 3]).then_some([0.0; 3])).make(|_, _| {
        let shapes = vec![
            Shape::cuboid([-256.0, 0.0, -256.0], [256.0, 256.0, 256.0], 0),
            Shape::cuboid([-256.0, 248.0, -256.0], [256.0, 256.0, 256.0], 1).paint(),
        ];
        Some(Node::of(shapes).with_far(Far::measured(worldgen::noise::fx(2.0))))
    });
    let generator = Generator { kinds: vec![kind(&["#ffffff"]), kind(&["#ff0000"])], seed: 1, layers: vec![Layer::Population(block)] };
    worldgen::World { generator, spawn: [0, -400, 0], heights: Some((-64, 512)), points: Vec::new() }
}

#[test]
#[ignore = "draws on the GPU: needs Vulkan, as Mesa's software one"]
fn voxels_show_the_faces_rays_enter_by() {
    // From below, near and far: rays enter its voxels by faces that look down or sideways, white, never by
    // those that look up, red. Most of its coarse voxels have both, and look as either as a whole.
    // Lit, near: only the light differs.
    let lit = Some(Brightness { sky: 0.0, sun: 0.0, glow: 1.0 });
    for (light, distance) in [(None, 400.0), (None, 1500.0), (None, 2500.0), (lit, 400.0)] {
        let mut world = World::new(sheeted_block(1), light.map(Shared::new), 4);
        let at = Look::new([0.0, -distance, 0.0], [0.0, 80.0], FOV, 8000.0, [320.0, 200.0]);
        let look = Look { fog: f64::INFINITY, light, ..at };
        let pixels = drawn(&mut *Kind::Voxels.make(&world), &mut world, &look);
        let (white, red) = pixels.chunks(4).fold((0, 0), |(w, r), p| {
            let [red, green, blue] = [p[0], p[1], p[2]].map(i32::from);
            (w + usize::from(red > 100 && (red - green).abs() < 8 && (red - blue).abs() < 8), r + usize::from(red > green + 60))
        });
        assert!(white > 500 && red == 0, "from {distance} below, lit {light:?}: {white} pixels of it white, {red} red");
    }
}

/** A vector of the lattice, not of the bricks' (BRICK in voxels.rs) nor of their blocks'. */
const SHIFT: [i64; 3] = [3, 2, 1];

/**
 * Specks of a few colours, few enough that cells of each scale lie
 * empty between them, over a ground sloping across the lattice's axes,
 * which rays come to through the cells at every slant. Moved by SHIFT
 * if `MOVED`.
 */
fn specks<const MOVED: bool>(_: u64) -> worldgen::World {
    use worldgen::generator::{Field, Generator, KindDef, Layer, Op};
    let kind = |colors| KindDef { name: "speck", kind_type: 10, colors, walk_through: false, glows: false, medium: false };
    let kinds = vec![kind(&["#ff0000"]), kind(&["#00ff00"]), kind(&["#0000ff"]), kind(&["#ffff00"])];
    let by = if MOVED { SHIFT } else { [0; 3] };
    let field = Field::at("specks", 0.0, Op::Fill, move |x, y, z| {
        let [x, y, z] = [x - by[0], y - by[1], z - by[2]];
        let h = worldgen::noise::hash(1, x, y, z);
        let ground = 10 * y < 3 * x + 2 * z - 300;
        (ground || h.is_multiple_of(20000)).then_some(((h % 4) as u8, None))
    });
    let generator = Generator { kinds, seed: 1, layers: vec![Layer::Field(field)] };
    worldgen::World { generator, spawn: by, heights: Some((-200, 200)), points: Vec::new() }
}

/** What the voxels engine draws of `make`'s world from `eye` past its spawn, within level 0's reach: its pixels. */
fn specks_from(make: Make, eye: Vec3, angles: [f64; 2]) -> Vec<u8> {
    let mut world = World::new(make(1), None, 4);
    let spawn = make(1).spawn.map(|v| v as f64);
    let camera = std::array::from_fn(|i| spawn[i] + eye[i]);
    let look = Look { fog: f64::INFINITY, ..Look::new(camera, angles, FOV, 200.0, [320.0, 200.0]) };
    let mut engine = Kind::Voxels.make(&world);
    drawn(&mut *engine, &mut world, &look)
}

#[test]
#[ignore = "draws on the GPU: needs Vulkan, as Mesa's software one"]
fn no_ray_of_the_voxels_runs_out_of_steps() {
    // As a shot looks, from the spawn, where rays once stuck crossing cells.
    for make in [WORLD, "forest"].map(|name| worlds::by_name(name).expect("a world").make) {
        let mut world = World::new(make(1), None, 4);
        let spawn = make(1).spawn.map(|v| v as f64);
        let look = Look::new(spawn, [0.0, 0.0], FOV, RENDER_DISTANCE, SHOT);
        let mut engine = Kind::Voxels.make(&world);
        let pixels = drawn(&mut *engine, &mut world, &look);
        // Lost rays show magenta (voxels.wgsl's LOST).
        let lost = pixels.chunks(4).filter(|p| p[..3] == [255, 0, 255]).count();
        assert_eq!(lost, 0, "pixels of rays that ran out of steps");
    }
}

/**
 * The window's frames, still, as App::ui asks for them: one after
 * another while tiles arrive, the engine is busy or the world makes
 * any, each drawn; then whether the painter showed all it was given.
 */
fn frames_till_still(engine: &mut dyn Engine, world: &mut World, look: &Look, easel: &mut Easel) -> bool {
    for _ in 0..10_000 {
        let frame = engine.frame(look, world);
        let arrived = world.update(look.camera, &frame.wants);
        let drawn = easel.draw(&*frame.picture, look.size.map(|v| v as u32));
        easel.wait_for(drawn);
        if !arrived && !frame.busy {
            if world.making() == 0 {
                return easel.settled();
            }
            world.wait();
        }
    }
    panic!("frames never still");
}

/** Lit anew, as the panel's sliders do (light_sliders), the voxels' frames keep coming till they show it all. */
#[test]
#[ignore = "draws on the GPU: needs Vulkan, as Mesa's software one"]
fn frames_come_till_a_world_lit_anew_shows() {
    let (day, dusk) = (Brightness { sky: 1.0, sun: 1.0, glow: 0.0 }, Brightness { sky: 0.2, sun: 0.0, glow: 0.0 });
    let mut world = World::new(ground(1), Some(Shared::new(day)), 4);
    let mut look = Look::new([0.0, 16.0, 0.0], [0.0, -20.0], FOV, 1000.0, [160.0, 100.0]);
    look.light = Some(day);
    let (mut engine, mut easel) = (Kind::Voxels.make(&world), Easel::offscreen(None));
    assert!(frames_till_still(&mut *engine, &mut world, &look, &mut easel), "lit at first");
    for shared in [world.shared().expect("light").under(dusk), world.shared().expect("light").under(dusk).stepping(3)] {
        look.light = Some(shared.brightness());
        world.relight(shared);
        engine = Kind::Voxels.make(&world);
        assert!(frames_till_still(&mut *engine, &mut world, &look, &mut easel), "lit anew");
    }
}

#[test]
#[ignore = "draws on the GPU: needs Vulkan, as Mesa's software one"]
fn voxels_look_the_same_wherever_the_bricks_fall() {
    // Eyes on dyadic fractions, so that moved by whole units they stay exact.
    for (eye, angles) in [([0.25, 0.625, 0.125], [30.0, -10.0]), ([-3.5, 1.75, 2.375], [-100.0, -25.0])] {
        let (here, there) = (specks_from(specks::<false>, eye, angles), specks_from(specks::<true>, eye, angles));
        assert!(here.chunks(4).any(|p| p[..3] != here[..3]), "something seen from {eye:?}");
        let differ = here.chunks(4).zip(there.chunks(4)).filter(|(a, b)| a != b).count();
        assert_eq!(differ, 0, "pixels that differ moved by {SHIFT:?}, from {eye:?}");
    }
}

/**
 * The pointer's ray (inspect.rs) meets what the picture shows along it: of
 * a white wall behind a slab of medium, seen along a face's axis, the
 * wall, as far as it is, and before it the slab, its path through it and
 * what it lets through as its extinction says.
 */
#[test]
#[ignore = "draws on the GPU: needs Vulkan, as Mesa's software one"]
fn the_pointers_ray_meets_the_wall_behind_the_medium() {
    const PIXELS: [u32; 2] = [320, 200];
    let mut world = World::new(wall::<15>(1), None, 4);
    let look = Look { fog: f64::INFINITY, pointer: Some([0.5; 2]), ..Look::new([0.0; 3], [45.0, 0.0], FOV, 1000.0, PIXELS.map(f64::from)) };
    let (mut engine, mut easel) = (Kind::Voxels.make(&world), Easel::offscreen(None));
    let frame = drawn_complete(&mut *engine, &mut world, &look, &mut easel);
    // Read back a frame or two late.
    let probed = (0..100).find_map(|_| {
        let drawn = easel.draw(&*frame.picture, PIXELS);
        easel.wait_for(drawn);
        easel.held::<Marcher>().and_then(|m| m.probed)
    });
    let probe = probed.expect("read back").1;
    let lines = inspect::lines(&probe, &world, false);
    assert!(lines.contains(&("look", "#ffffff, opaque".into(), vec![[255; 4]])), "the wall: {lines:?}");
    // Entered where it begins, x + z = FAR.
    let along = |k: i64| k as f64 / std::f64::consts::SQRT_2;
    assert!((f64::from(probe.distance) - along(FAR)).abs() < 1.0, "the wall {} away", probe.distance);
    assert!(probe.media().iter().all(|m| (NEAR..FAR).contains(&i64::from(m.site[0] + m.site[2]))), "{:?}", probe.media());
    assert!((f64::from(probe.path) - along(FAR - NEAR)).abs() < 1.0, "{} across the slab", probe.path);
    let expected = (-0.015 * along(FAR - NEAR)).exp();
    assert!(
        probe.through.iter().all(|&t| (f64::from(t) - expected).abs() < 0.01),
        "{:?} let through, {expected:.3} expected",
        probe.through
    );
}
