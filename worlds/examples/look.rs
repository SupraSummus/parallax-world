//! A picture of a world at one level of detail (picture.rs), as a PNG:
//!
//! cargo run --release --example look -- forest 42 0 eye spawn yaw 0 pitch -4 out.png
//! cargo run --release --example look -- valley 42 2 eye 0 800 400 at 0 0 1500 fov 60 far 2600 out.png
//! cargo run --release --example look -- valley 7 2 eye windmill -400 200 -400 at windmill out.png
//! cargo run --release --example look -- valley 42 3 top -800 800 800 2400 size 960 out.png
//! cargo run --release --example look -- valley 7 3 top mountainside 400 out.png
//! cargo run --release --example look -- city 42 0 eye spawn -20 30 -10 at spawn -100 10 50 light 0.01 0 0.5 out.png
//!
//! for world, seed and level, then the camera: its eye and a target to
//! look at or a yaw and pitch (degrees; yaw 0 looks along +z), with `fov`
//! (degrees across), `far` (lattice units) and `size` (pixels) optional;
//! or `top` and an area (min x and z, max x and z) to look straight down
//! at; `light` and how bright the sky, the sun and glowing kinds are
//! (1 1 0: a clear day; 0.01 0 0.5: night) lights the picture by
//! worldgen::light instead of a fixed sun. Last, the file. A point is
//! x y z (lattice units), or a place the world names (`World::point`:
//! spawn, a town, a windmill…) and, optionally, x y z from it; `top`
//! takes one and how far around it.
//! It prints the time taken and the share of pixels of each material type.

use worldgen::generator::materials_of;
use worldgen::light::{Light, Sky};
use worldgen::picture::{Camera, picture_on_threads};
use worlds::{WORLDS, by_name};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let usage = "look <world> <seed> <level> (eye <point> (at <point>|yaw <d> pitch <d>) | top <x0 z0 x1 z1|place d>) \
                 [fov <d>] [far <units>] [size <w> [h]] [light <sky> <sun> <glow>] <out.png>, a point x y z or <place> [x y z]";
    let is_number = |i: usize| args.get(i).is_some_and(|a| a.parse::<f64>().is_ok());
    let number = |i: usize| args.get(i).and_then(|a| a.parse::<f64>().ok()).unwrap_or_else(|| panic!("a number at {i}: {usage}"));
    let seed = number(1) as u64;
    let Some(entry) = by_name(&args[0]) else {
        let names: Vec<&str> = WORLDS.iter().map(|e| e.name).collect();
        eprintln!("no world {}; the worlds are {}", args[0], names.join(", "));
        std::process::exit(1);
    };
    let world = (entry.make)(seed);
    // The place named at `i`, and how many arguments it took.
    let place = |i: usize| match world.point(&args[i]) {
        Some(p) => p.map(|v| v as f64),
        None => {
            let names: Vec<&str> = std::iter::once("spawn").chain(world.points.iter().map(|(n, _)| *n)).collect();
            eprintln!("no place {} in {}; it names {}", args[i], entry.name, names.join(", "));
            std::process::exit(1);
        }
    };
    // The point at `i`: x y z, or a place and x y z from it if given.
    let point = |i: usize| -> ([f64; 3], usize) {
        if is_number(i) {
            return ([number(i), number(i + 1), number(i + 2)], 3);
        }
        let p = place(i);
        if is_number(i + 1) && is_number(i + 3) && i + 4 < args.len() {
            ([p[0] + number(i + 1), p[1] + number(i + 2), p[2] + number(i + 3)], 4)
        } else {
            (p, 1)
        }
    };
    let level = number(2) as u32;
    let (mut eye, mut target, mut angles, mut area) = (None, None, [0.0, 0.0], None);
    let (mut fov, mut far, mut size, mut sky) = (70.0, 3000.0, [960, 600], None);
    let mut i = 3;
    while i + 1 < args.len() {
        let word = args[i].as_str();
        i += 1;
        match word {
            "eye" | "at" => {
                let (p, n) = point(i);
                if word == "eye" {
                    eye = Some(p)
                } else {
                    target = Some(p)
                }
                i += n;
            }
            "yaw" | "pitch" | "fov" | "far" => {
                match word {
                    "yaw" => angles[0] = number(i),
                    "pitch" => angles[1] = number(i),
                    "fov" => fov = number(i),
                    _ => far = number(i),
                }
                i += 1;
            }
            "light" => {
                let [sky_, sun, glow] = [0, 1, 2].map(|k| number(i + k) as f32);
                sky = Some(Sky { sky: sky_, sun, glow });
                i += 3;
            }
            "top" if is_number(i) => {
                area = Some(([number(i), number(i + 1)], [number(i + 2), number(i + 3)]));
                i += 4;
            }
            "top" => {
                let ([x, _, z], d) = (place(i), number(i + 1));
                area = Some(([x - d, z - d], [x + d, z + d]));
                i += 2;
            }
            "size" => {
                size[0] = number(i) as usize;
                i += 1;
                if let Some(h) = args.get(i).filter(|_| i + 1 < args.len()).and_then(|a| a.parse().ok()) {
                    size[1] = h;
                    i += 1;
                }
            }
            _ => panic!("{word}? {usage}"),
        }
    }
    let camera = match (area, eye, target) {
        (Some((min, max)), ..) => Camera::top(min, max, size[0]),
        (None, Some(eye), Some(target)) => Camera::looking_at(eye, target, fov.to_radians(), size[0], size[1], far),
        (None, Some(eye), None) => {
            let mut camera = Camera::looking_at(eye, [eye[0], eye[1], eye[2] + 1.0], fov.to_radians(), size[0], size[1], far);
            (camera.yaw, camera.pitch) = (angles[0].to_radians(), angles[1].to_radians());
            camera
        }
        _ => panic!("no camera: {usage}"),
    };
    let mut picture = picture_on_threads(&|| (entry.make)(seed).generator, &camera, level, world.heights_at(level));
    if let Some(sky) = sky {
        let start = std::time::Instant::now();
        let mut light = Light::new(&world, level, sky);
        light.relight(&mut picture);
        println!("lit in {:.2} s, the light of {:?} sites found by level", start.elapsed().as_secs_f64(), light.found());
    }
    std::fs::write(args.last().expect("a file"), picture.png()).expect("written");
    let middle = picture.height / 2 * picture.width + picture.width / 2;
    println!(
        "{}×{} at level {level}: {} tiles generated in {:.2} s, rays cast in {:.2} s; the middle pixel {:.0} units away",
        picture.width,
        picture.height,
        picture.tiles,
        picture.generating.as_secs_f64(),
        picture.casting.as_secs_f64(),
        picture.depth[middle]
    );
    // What the pixels show, by material type (0: nothing).
    let types: Vec<u8> = materials_of(&world.generator).iter().map(|m| m.material_type).collect();
    let mut counts = [0usize; 256];
    for id in &picture.material {
        counts[types[*id as usize] as usize] += 1;
    }
    let share = |n: usize| 100.0 * n as f64 / picture.material.len() as f64;
    let shares: Vec<String> = (0..256).filter(|t| counts[*t] > 0).map(|t| format!("{t}: {:.1}%", share(counts[t]))).collect();
    println!("pixels by material type: {}", shares.join(", "));
}
