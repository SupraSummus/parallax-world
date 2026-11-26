use super::*;
use lattice::light::colour;
use worldgen::generator::{Field, Generator, KindDef, Layer, Node, Op, Population, SOLID, cuboid};

const STONE: u8 = 0;
const LAMP_KIND: u8 = 1;
/** Lamps are cubes 4 units wide, as the towns' are: many sites, its glow on those inside. */
const LAMP: f64 = 4.0;

/**
 * Stone where `stone(x, y, z, spacing)` says (the sites' spacing), and white lamps at
 * `lamps`, as the towns' are; from y = −64 up to what the coarsest
 * levels' sites hold.
 */
fn world(lamps: &[[f64; 3]], stone: impl Fn(i64, i64, i64, i64) -> bool + Send + Sync + 'static) -> World {
    world_of(&["#808080"], lamps, stone)
}

/** As `world`, its stone coloured `colour`. */
fn world_of(
    colour: &'static [&'static str],
    lamps: &[[f64; 3]],
    stone: impl Fn(i64, i64, i64, i64) -> bool + Send + Sync + 'static,
) -> World {
    world_with(colour, lamps, &[], stone)
}

/** As `world_of`, with blocks of stone the lamps' size at `blocks`. */
fn world_with(
    colour: &'static [&'static str],
    lamps: &[[f64; 3]],
    blocks: &[[f64; 3]],
    stone: impl Fn(i64, i64, i64, i64) -> bool + Send + Sync + 'static,
) -> World {
    let kinds = vec![
        KindDef { name: "stone", kind_type: 1, colors: colour, walk_through: false, glows: false, medium: false },
        KindDef { name: "lamp", kind_type: 6, colors: &["#ffffff"], walk_through: false, glows: true, medium: false },
    ];
    let cube = |p: &[f64; 3], kind| cuboid(p.map(|c| c - LAMP / 2.0), p.map(|c| c + LAMP / 2.0), kind, SOLID, Op::Fill);
    let shapes: Vec<_> = lamps.iter().map(|p| cube(p, LAMP_KIND)).chain(blocks.iter().map(|p| cube(p, STONE))).collect();
    let lamps = Population::new("lamps", 1.0, 1024).reach(2048).at(|_| vec![([0.0; 3], ())], move |_, _| Some(Node::of(shapes.clone())));
    let ground = Field::at("stone", 0.0, Op::Fill, move |x, y, z| stone(x, y, z, worldgen::sites::spacing()).then_some((STONE, None)));
    let layers = vec![Layer::Field(ground), Layer::Population(lamps)];
    World { generator: Generator { kinds, seed: 1, layers }, spawn: [0; 3], heights: Some((-64, 1024)), points: Vec::new() }
}

/** A site on top of ground below y = 0: at y = −1. */
fn ground(x: i64, z: i64) -> [i64; 3] {
    [x, -1, z]
}

#[test]
fn a_light_packed_is_within_a_thousandth_and_packs_to_the_same() {
    for k in -40..40 {
        let v = 1.37 * 1.9f32.powi(k);
        let [back, ..] = unpack(pack([v; 3]));
        assert_eq!(pack([back; 3]), pack([v; 3]), "{v}");
        assert!((back / v.min(2f32.powi(31)) - 1.0).abs() < 1e-3 || v < 2f32.powi(-32) && back <= v, "{v} packed as {back}");
    }
    assert_eq!(unpack(pack([0.0, 1.0, 0.5])), [0.0, 1.0, 0.5]);
}

#[test]
fn walls_keep_the_sky_out() {
    // A closed box whose walls are one site thick at every level, as generators draw sheets.
    let world = world(&[], |x, y, z, s| {
        let wall = |c: i64| (48..48 + s).contains(&c.abs());
        let within = |c: i64| c.abs() < 48 + s;
        y < 0 || within(x) && within(z) && (0..48 + s).contains(&y) && (wall(x) || wall(z) || y >= 48)
    });
    let mut light = Light::new(world, Shared::new(Brightness { sky: 1.0, sun: 1.0, glow: 0.0 }));
    let inside = light.on_voxel(0, ground(1, 0));
    assert!(inside.iter().all(|c| *c < 0.02), "inside {inside:?}");
}

#[test]
fn a_medium_stops_more_of_the_channels_its_colour_is_darker_in() {
    // Ground under a thin slab of blue medium: lit in blue nearly as in the open, in red hardly.
    let under = |veiled: bool| {
        let kinds = vec![
            KindDef { name: "stone", kind_type: 1, colors: &["#808080"], walk_through: false, glows: false, medium: false },
            KindDef { name: "medium", kind_type: 8, colors: &["#0000ff"], walk_through: true, glows: false, medium: true },
        ];
        let field = Field::at("slab", 0.0, Op::Fill, move |_, y, _| match y {
            ..0 => Some((0, None)),
            0..32 if veiled => Some((1, Some(0.005))),
            _ => None,
        });
        let world = World {
            generator: Generator { kinds, seed: 1, layers: vec![Layer::Field(field)] },
            spawn: [0; 3],
            heights: Some((-64, 1024)),
            points: Vec::new(),
        };
        Light::new(world, Shared::new(Brightness { sky: 1.0, sun: 1.0, glow: 0.0 })).on_voxel(0, ground(1, 0))
    };
    let (open, veiled) = (under(false), under(true));
    let [red, _, blue] = std::array::from_fn(|c| veiled[c] / open[c]);
    assert!(red < 0.05 && blue > 0.6, "of the open's, red {red}, blue {blue}");
}

#[test]
fn a_lamp_glows_as_much_at_every_level_it_is_smaller_than_a_site() {
    // Its glow is its share of its site, which is its area over the site's: times the site's area, the same.
    let world = world(&[[5.0, 31.0, 6.0]], |_, _, _, _| false);
    let glows: Vec<f64> = (3..=8)
        .map(|level| {
            let grid = worldgen::generator::generate(&world.generator, &worldgen::grid::Box3 { min: [-8; 3], max: [8; 3] }, level);
            grid.glow.iter().map(|(_, g)| g[0] as f64).sum::<f64>() * (1u64 << (2 * level)) as f64
        })
        .collect();
    assert!(glows.iter().all(|g| (g / glows[0] - 1.0).abs() < 0.1), "glow times the site's area, by level from 3: {glows:?}");
}

#[test]
fn a_closed_room_keeps_its_lamps_light_in() {
    // A box whose walls are one site thick at every level, a lamp inside, and the ground outside.
    let room = |x: i64, y: i64, z: i64, s: i64| {
        let wall = |c: i64| (16..16 + s).contains(&c.abs());
        let within = |c: i64| c.abs() < 16 + s;
        y < 0 || within(x) && within(z) && (0..32 + s).contains(&y) && (wall(x) || wall(z) || y >= 32)
    };
    let outside = |closed: bool| {
        let world = world(&[[0.0, 24.0, 0.0]], move |x, y, z, s| if closed { room(x, y, z, s) } else { y < 0 });
        let mut light = Light::new(world, Shared::new(Brightness { sky: 0.0, sun: 0.0, glow: 1.0 }));
        light.on_voxel(0, ground(25, 0))[0]
    };
    let (open, closed) = (outside(false), outside(true));
    assert!(open > 0.0 && closed < 0.01 * open, "outside, open {open}, closed {closed}");
}

#[test]
fn a_wall_is_lit_on_the_lamps_side() {
    // A wall one site thick across x, the lamp on its +x side.
    let world = world(&[[80.0, 10.0, 64.0]], |x, y, z, s| y < 0 || (64..64 + s).contains(&x) && y < 32 && (32..96).contains(&z));
    let light = Light::new(world, Shared::new(Brightness { sky: 0.0, sun: 0.0, glow: 1.0 })).faces(0, [64, 10, 64]);
    let side = |sign: i64| FACES.iter().zip(&light).filter(|(f, _)| f[0] == sign).map(|(_, l)| l[0]).sum::<f32>();
    assert!(side(1) > 100.0 * side(-1), "towards the lamp {}, away {}", side(1), side(-1));
}

/** A scene to hold Light to the reference: its world, how bright, the ground sites to probe, rays per face, and the bound on Light's error there. */
struct Scene {
    name: String,
    world: World,
    brightness: Brightness,
    probes: Vec<[i64; 3]>,
    samples: u64,
    bound: f64,
}

fn scenes() -> Vec<Scene> {
    let day = Brightness { sky: 1.0, sun: 1.0, glow: 0.0 };
    let sky = Brightness { sky: 1.0, sun: 0.0, glow: 0.0 };
    let night = Brightness { sky: 0.0, sun: 0.0, glow: 1.0 };
    let scene =
        |name: &str, world, brightness, probes, samples, bound| Scene { name: name.into(), world, brightness, probes, samples, bound };
    let mut scenes = vec![
        scene("open ground", world(&[], |_, y, _, _| y < 0), sky, vec![ground(1, 0)], 4096, 0.1),
        scene(
            "wall, sun",
            world(&[], |x, y, z, s| y < 0 || (0..2 * s).contains(&x) && y < 32 && z.abs() < 96),
            day,
            [-9, -1, 3, 17, 29, 41].map(|x| ground(x, 0)).to_vec(),
            4096,
            0.3,
        ),
        scene(
            "lamp",
            world(&[[0.0, 24.0, 0.0]], |_, y, _, _| y < 0),
            night,
            vec![ground(1, 0), ground(16, 7), ground(33, 0)],
            1 << 16,
            0.2,
        ),
    ];
    // A roof 2 sites thick and 128 units wide at y = 24, turned about y, probed across it: the same light at every turn.
    for (turn, bound) in [(0.0, 0.15), (22.5, 0.15), (45.0, 0.3)] {
        let (sin, cos) = f64::to_radians(turn).sin_cos();
        let roof = world(&[], move |x, y, z, s| {
            let (u, v) = (cos * x as f64 + sin * z as f64, cos * z as f64 - sin * x as f64);
            y < 0 || u.abs() <= 64.0 && v.abs() <= 64.0 && (24..24 + 2 * s).contains(&y)
        });
        let probes = [0.0, 40.0, 56.0, 72.0, 96.0].map(|d: f64| ground_near(d * cos, d * sin)).to_vec();
        scenes.push(scene(&format!("roof turned {turn}°"), roof, sky, probes, 4096, bound));
    }
    // A yard walled 48 high, open to the sky, and past a wall with a low door a room roofed at 48: from 32 past the wall, the sky does not show through the door, so only light that bounced comes there.
    let yard = world(&[], |x, y, z, s| {
        let inside = x.abs() < 64 && z.abs() < 48 && y < 48;
        let shell = x.abs() < 64 + 2 * s && z.abs() < 48 + 2 * s && y < 48 + if x >= 0 { 2 * s } else { 0 };
        let wall = (0..2 * s).contains(&x) && !(z.abs() < 12 && y < 16);
        y < 0 || shell && (!inside || wall)
    });
    let probes = [-48.0, -24.0, 32.0, 44.0, 56.0].map(|x| ground_near(x, 0.0)).to_vec();
    scenes.push(scene("yard, past a wall", yard, sky, probes, 1 << 16, 1.25));
    // A tunnel 40 wide and high through a hill 384 long, under a roof 16 thick at every level, thinner than the coarse
    // levels' sites: probed along its floor, from its middle out past a portal. The sun is side on: none of its light comes in.
    let tunnel = world(&[], |x, y, z, _| y < 0 || z.abs() < 192 && y < 56 && !(x.abs() < 20 && y < 40));
    let probes = [0.0, 96.0, 160.0, 224.0].map(|z| ground_near(0.0, z)).to_vec();
    scenes.push(scene("tunnel under a thin roof", tunnel, day, probes, 1 << 16, 0.65));
    // Blocks the lamps' size strewn over the ground, two to a square 8 wide, from 24 to 40 up: whole voxels near, a
    // share of a site each far, stopping about two thirds of the sky's light.
    let strewn = |k: i64, v: i64| (worldgen::noise::hash(k as u64, v, 0, 0) % 1000) as f64 / 1000.0;
    let blocks: Vec<[f64; 3]> = (0..2 * 23 * 23)
        .map(|k| {
            [
                8.0 * (k % 23 - 11) as f64 + 4.0 * strewn(1, k),
                26.0 + 12.0 * strewn(2, k),
                8.0 * (k / 23 % 23 - 11) as f64 + 4.0 * strewn(3, k),
            ]
        })
        .collect();
    let canopy = world_with(&["#808080"], &[], &blocks, |_, y, _, _| y < 0);
    scenes.push(scene("blocks over the ground", canopy, sky, [0.0, 24.0, 48.0].map(|x| ground_near(x, 0.0)).to_vec(), 4096, 0.1));
    // A closed room, white, a lamp in it: most of the light on its floor has bounced, many times.
    let room = world_of(&["#f0f0f0"], &[[0.0, 24.0, 0.0]], |x, y, z, s| {
        let wall = |c: i64| (32..32 + s).contains(&c.abs());
        let within = |c: i64| c.abs() < 32 + s;
        y < 0 || within(x) && within(z) && (0..48 + s).contains(&y) && (wall(x) || wall(z) || y >= 48)
    });
    let probes = vec![ground(1, 0), ground(16, 7), ground(29, 0), ground(28, 27)];
    scenes.push(scene("white room, lamp", room, night, probes, 1 << 16, 1.35));
    // A cave, a ball in rock the cheese's colour, a lamp by its wall: probed across its floor.
    let ball = |x: i64, y: i64, z: i64| x * x + (y - CAVE.1) * (y - CAVE.1) + z * z < CAVE.0 * CAVE.0;
    let cave = world_of(&["#f3d46c"], &[[0.0, 20.0, 20.0]], move |x, y, z, _| y < 80 && !ball(x, y, z));
    let probes = [0, 12, 24].map(|x| {
        let y = (0..CAVE.1).find(|&y| ball(x, y, 0)).expect("the cave's floor") - 1;
        [x, y, 0]
    });
    scenes.push(scene("yellow cave, lamp", cave, night, probes.to_vec(), 1 << 16, 1.3));
    scenes
}

/** The cave's radius and how high its middle is. */
const CAVE: (i64, i64) = (32, 40);

/** The ground site nearest (x, z). */
fn ground_near(x: f64, z: f64) -> [i64; 3] {
    ground(x.floor() as i64, z.floor() as i64)
}

/** What the open faces of the voxel at `site` (level 0) send out by Light, on average. */
fn lit(light: &mut Light, site: [i64; 3]) -> Rgb {
    let open: Vec<usize> = (0..F).filter(|&f| !light.whole(0, add(site, FACES[f]))).collect();
    let faces = light.faces(0, site);
    std::array::from_fn(|c| open.iter().map(|&f| faces[f][c]).sum::<f32>() / open.len().max(1) as f32)
}

/** Where the reference's light at the scenes' probes is kept (`reference_values`), as lines of scene, site, light and its standard error. */
const VALUES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/data/light-reference.txt");

fn line(name: &str, [x, y, z]: [i64; 3]) -> String {
    format!("{name}; {x} {y} {z}")
}

/** The reference's light at the scenes' probes, written to VALUES: run by hand when the scenes change, as it takes a while. It finds only the probes VALUES lacks; delete a changed scene's lines first. */
#[test]
#[ignore]
fn reference_values() {
    let kept = std::fs::read_to_string(VALUES).unwrap_or_default();
    let mut lines = String::new();
    for Scene { name, world, brightness, probes, samples, .. } in scenes() {
        let reference = reference::Reference::new(&world, brightness);
        for site in probes {
            let key = format!("{}; ", line(&name, site));
            lines += &match kept.lines().find(|l| l.starts_with(&key)) {
                Some(l) => format!("{l}\n"),
                None => {
                    let ([r, g, b], error) = reference.voxel(site, samples);
                    format!("{key}{r} {g} {b}; {error}\n")
                }
            };
        }
    }
    std::fs::write(VALUES, lines).expect("written");
}

/**
 * Light against the reference (reference.rs, as `reference_values`
 * kept it), scene by scene: how far off the light from each probe's
 * open faces is, as the root mean square of ln(light / reference)
 * over the probes. The bounds are Light's errors as they were; lower
 * one as Light gets closer. `-- --nocapture` prints each probe.
 */
#[test]
fn against_the_reference() {
    let kept = std::fs::read_to_string(VALUES).expect("the reference's values: run reference_values");
    let value = |name: &str, site| {
        let key = line(name, site);
        let found =
            kept.lines().find_map(|l| l.strip_prefix(&key)?.strip_prefix("; ")).unwrap_or_else(|| panic!("{key}: run reference_values"));
        let [light, error] = [0, 1].map(|k| found.split("; ").nth(k).expect("light; error"));
        (light.split(' ').map(|v| v.parse::<f32>().expect("a number")).sum::<f32>(), error.parse::<f32>().expect("a number"))
    };
    let mut over = Vec::new();
    for Scene { name, world, brightness, probes, bound, .. } in scenes() {
        let mut light = Light::new(world, Shared::new(brightness));
        let mut squares = 0.0;
        for &site in &probes {
            let (m, (r, error)) = (lit(&mut light, site).iter().sum::<f32>(), value(&name, site));
            println!("{name}, {site:?}: light {m:.4}, reference {r:.4} ± {error:.4}, light / reference {:.2}", m / r);
            squares += ((m / r) as f64).ln().powi(2);
        }
        let rms = (squares / probes.len() as f64).sqrt();
        println!("{name}: Light is off by {rms:.2} (ln), at most {bound}");
        if rms > bound {
            over.push(name);
        }
    }
    assert!(over.is_empty(), "Light is further off than it was in {over:?}");
}

/**
 * Maps of the ground around each scene, from above, as PNGs in the
 * directory OUT: Light's, the reference's (a sixty-fourth of the rays,
 * kept in OUT for the next run) and where Light is brighter (red) or
 * darker (blue), up to e^1.5 times; and how far off Light is over
 * each map, as `against_the_reference` reckons.
 * Run by hand: OUT=dir cargo test --release maps -- --ignored
 */
#[test]
#[ignore]
fn maps() {
    const HALF: i64 = 96;
    let out = std::env::var("OUT").expect("OUT, a directory");
    let (w, scale, gap) = (HALF as usize, 4, 4);
    for Scene { name, world, brightness, samples, .. } in scenes() {
        let file = format!("{out}/{}", name.replace([' ', ',', '°'], ""));
        let sites: Vec<[i64; 3]> =
            (0..w * w).map(|k| ground_near(2.0 * (k % w) as f64 - HALF as f64, HALF as f64 - 2.0 * (k / w) as f64)).collect();
        let kept: Vec<f32> = std::fs::read(format!("{file}.reference"))
            .unwrap_or_default()
            .as_chunks::<4>()
            .0
            .iter()
            .map(|b| f32::from_le_bytes(*b))
            .collect();
        let references: Vec<Rgb> = if kept.len() == 3 * sites.len() {
            kept.as_chunks::<3>().0.to_vec()
        } else {
            let reference = reference::Reference::new(&world, brightness);
            let found: Vec<Rgb> = sites.iter().map(|&site| reference.voxel(site, samples / 64).0).collect();
            std::fs::write(format!("{file}.reference"), found.iter().flatten().flat_map(|v| v.to_le_bytes()).collect::<Vec<u8>>())
                .expect("kept");
            found
        };
        let mut light = Light::new(world, Shared::new(brightness));
        let lights: Vec<Rgb> = sites.iter().map(|&site| lit(&mut light, site)).collect();
        let logs: Vec<f64> = lights
            .iter()
            .zip(&references)
            .map(|(m, r)| (m.iter().sum::<f32>() / r.iter().sum::<f32>()).ln() as f64)
            .filter(|v| v.is_finite())
            .collect();
        println!("{name}: Light is off by {:.2} (ln, over the map)", (logs.iter().map(|v| v * v).sum::<f64>() / logs.len() as f64).sqrt());
        // Exposed so that the reference's brightest tenth is white.
        let mut sums: Vec<f32> = references.iter().map(|r| r.iter().sum()).collect();
        sums.sort_by(f32::total_cmp);
        let exposure = 3.0 / sums[sums.len() * 9 / 10].max(1e-9);
        let shown = |l: Rgb| colour(l.map(|c| c * exposure)).to_le_bytes();
        let off = |m: Rgb, r: Rgb| {
            let ratio = (m.iter().sum::<f32>() / r.iter().sum::<f32>()).ln();
            let ratio = if ratio.is_nan() { 0.0 } else { ratio.clamp(-1.5, 1.5) / 1.5 };
            let pale = (255.0 * (1.0 - ratio.abs())) as u8;
            if ratio > 0.0 { [255, pale, pale, 255] } else { [pale, pale, 255, 255] }
        };
        let width = 3 * w + 2 * gap;
        let mut rgb = vec![255; width * w * 3];
        for (k, (&m, &r)) in lights.iter().zip(&references).enumerate() {
            for (panel, pixel) in [shown(m), shown(r), off(m, r)].into_iter().enumerate() {
                let at = 3 * ((k / w) * width + panel * (w + gap) + k % w);
                rgb[at..at + 3].copy_from_slice(&pixel[..3]);
            }
        }
        let rgb = (0..w * scale)
            .flat_map(|y| (0..width * scale).flat_map(move |x| [0, 1, 2].map(|c| ((y / scale) * width + x / scale) * 3 + c)))
            .map(|at| rgb[at])
            .collect();
        let picture = Picture { width: width * scale, height: w * scale, rgb, ..Picture::default() };
        std::fs::write(format!("{file}.png"), picture.png()).expect("written");
    }
}
