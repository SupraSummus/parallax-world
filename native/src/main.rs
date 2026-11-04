//! Spike of a native frontend: the forest of worlds/ generated in tiles, those
//! in view first (world.rs), the renderer picking the chunks in view
//! (renderer.rs), worker threads baking them with the baker in bake/,
//! sharing the tiles, and wgpu drawing them in an egui window.
//!
//! parallax-native [--threads N] [--seed S] [--leak DEG] [--shot out.png [--seams] [--colour voxels|layers|chunks] | --bench] [--walk D] [--stroll S] [--spin]
//! parallax-native [--seed S] --check-requests frame.bin
//!
//! --shot draws the first frame once the world has its tiles and all its
//! chunks are baked, at 1280×800, offscreen, and saves it; --bench only
//! times that, the main thread's frames after, and the GPU drawing the
//! last. --walk D moves the camera D units along the view after the first
//! frame, and does the same there. --stroll S then walks on for S seconds
//! as the window would, a frame every 60th, and times each frame's parts.
//! --leak sets how far hidden geometry may show between slices (renderer.rs).
//! --seams and --colour draw the shot as the panel's switches do.
//! --check-requests takes a view the TS renderer dumped (native/dump.test.ts):
//! the faces must be the same, and the chunks the first frame
//! wants from its grids and camera too, but for some the TS renderer wants
//! and this one does not: its cells' extents count only the tiles a chunk
//! reads, and those chunks draw nothing in view.

mod bakery;
mod compositor;
mod dump;
mod renderer;
mod shot;
mod slicing;
mod view;
mod world;

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Instant;

use bake::Baker;
use eframe::egui;
use eframe::egui_wgpu;

use bakery::Bakery;
use compositor::{Colours, Compositor};
use dump::Dump;
use renderer::Renderer;
use view::{Vec3, focal_length, view_basis};
use world::{Level, Make, World};
use worldgen::generator::materials_of;

/** The world shown. */
const WORLD: Make = worlds::forest::forest;

struct App {
    world: World,
    renderer: Renderer,
    faces: [[Vec3; 3]; 12],
    camera: Vec3,
    yaw: f64,
    pitch: f64,
    fov: f64,
    seams: bool,
    colours: Colours,
    stats: String,
    frame_ms: f64,
    /** --spin: turns on its own and prints frames per second. */
    spin: Option<(Instant, u32)>,
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let start = Instant::now();
        egui::Panel::left("panel").show(ui, |ui| {
            ui.heading("Parallax (native spike)");
            ui.label(&self.stats);
            // The web's (App.tsx).
            ui.add(egui::Slider::new(&mut self.fov, 5.0..=120.0).step_by(1.0).text("Field of View (°)"));
            ui.add(egui::Slider::new(&mut self.renderer.render_distance, 40.0..=40000.0).logarithmic(true).text("Render Distance"));
            ui.add(egui::Slider::new(&mut self.renderer.max_leak, 0.1..=6.0).step_by(0.1).text("Max Occlusion Leak (°)"));
            ui.checkbox(&mut self.renderer.fill, "Fill Gaps");
            ui.checkbox(&mut self.renderer.skip_covered, "Skip Covered Chunks");
            ui.checkbox(&mut self.seams, "Show Face Seams");
            ui.horizontal(|ui| {
                ui.label("Colour by");
                for (colours, name) in Colours::ALL {
                    ui.radio_value(&mut self.colours, colours, name);
                }
            });
            ui.label(format!(
                "camera {:.0} {:.0} {:.0}  yaw {:.0}  pitch {:.0}",
                self.camera[0], self.camera[1], self.camera[2], self.yaw, self.pitch
            ));
            ui.label(format!("render {:.1} ms", self.frame_ms));
            ui.label("WASD, Space/Shift, arrows or drag");
        });
        egui::CentralPanel::default().frame(egui::Frame::NONE).show(ui, |ui| {
            let (rect, response) = ui.allocate_exact_size(ui.available_size(), egui::Sense::drag());
            let drag = response.drag_delta();
            let (dt, moving) = ui.input(|i| {
                let key = |k| if i.key_down(k) { 1.0 } else { 0.0 };
                let forward = key(egui::Key::W) - key(egui::Key::S);
                let right = key(egui::Key::D) - key(egui::Key::A);
                let up = key(egui::Key::Space) - (if i.modifiers.shift { 1.0 } else { 0.0 });
                let turn = key(egui::Key::ArrowRight) - key(egui::Key::ArrowLeft);
                let look = key(egui::Key::ArrowUp) - key(egui::Key::ArrowDown);
                (i.stable_dt.min(0.1) as f64, [forward, right, up, turn, look])
            });
            self.yaw += drag.x as f64 * 0.2 + moving[3] * 90.0 * dt;
            self.pitch = (self.pitch - drag.y as f64 * 0.2 + moving[4] * 90.0 * dt).clamp(-90.0, 90.0);
            let [right, _, _] = view_basis(self.yaw, 0.0);
            let level = [-right[2], 0.0, right[0]];
            let step = SPEED * dt;
            for d in 0..3 {
                self.camera[d] += (level[d] * moving[0] + right[d] * moving[1]) * step;
            }
            self.camera[1] += moving[2] * step;
            if let Some((since, frames)) = &mut self.spin {
                self.yaw += 30.0 * dt;
                *frames += 1;
                if since.elapsed().as_secs_f64() >= 2.0 {
                    println!("{:.1} fps", *frames as f64 / since.elapsed().as_secs_f64());
                    *since = Instant::now();
                    *frames = 0;
                }
            }
            let (width, height) = (rect.width() as f64, rect.height() as f64);
            let focal = focal_length(self.fov, width, height);
            let view = view::View { yaw: self.yaw, pitch: self.pitch, width, height, focal };
            let (frame, stats) = self.renderer.render(self.camera, &view);
            // Drawn from the next frame on.
            let made = self.world.update(self.camera, &stats.levels, &stats.tiles);
            let arrived = !made.is_empty();
            for (level, tiles) in made {
                self.renderer.set_tiles(level, tiles);
            }
            self.stats = format!(
                "{} layers, {} chunks\n{} baking, {} tiles wanted, {} covered",
                frame.layers.len(),
                stats.chunks,
                stats.wanted.len(),
                stats.tiles.len(),
                stats.covered
            );
            let view = compositor::View {
                yaw: self.yaw,
                pitch: self.pitch,
                focal,
                fog_distance: self.renderer.render_distance * FOG_PER_RENDER_DISTANCE,
                faces: self.faces,
                size: [rect.width(), rect.height()],
                pixels: [0, 0],
                seams: self.seams,
                colours: self.colours,
            };
            let callback = compositor::Callback { frame: Arc::new(frame), view, rect };
            ui.painter().add(egui_wgpu::Callback::new_paint_callback(rect, callback));
            // While moving, the view changes; until all is made and baked, tiles and chunks keep arriving, often enough for a frame now and then.
            if moving.iter().any(|m| *m != 0.0) || self.spin.is_some() {
                ui.ctx().request_repaint();
            } else if arrived || !stats.wanted.is_empty() || self.world.busy() {
                ui.ctx().request_repaint_after(std::time::Duration::from_millis(16));
            }
        });
        self.frame_ms = start.elapsed().as_secs_f64() * 1000.0;
    }
}

/** The renderer and its bakery, for `world`. */
fn renderer(world: &worldgen::World, threads: usize, render_distance: f64) -> Renderer {
    let faces = view::faces();
    let colours: Vec<u32> = materials_of(&world.generator).iter().map(|m| m.rgb()).collect();
    let bakery = Bakery::new(threads, move || Baker::new(&colours));
    Renderer::new(faces, bakery, render_distance, world::heights(world))
}

/**
 * Renders until the world has made every tile it would and the frame has
 * every chunk it wants; says when it first draws any, and when it has all
 * the tiles and chunks in view.
 */
fn complete(world: &mut World, renderer: &mut Renderer, camera: Vec3, view: &view::View) -> (renderer::Frame, renderer::Stats) {
    let start = Instant::now();
    let (mut drawn, mut in_view) = (false, false);
    loop {
        let (frame, stats) = renderer.render(camera, view);
        if !drawn && stats.chunks > 0 {
            println!("first chunks drawn after {:.2} s", start.elapsed().as_secs_f64());
            drawn = true;
        }
        if !in_view && stats.tiles.is_empty() && stats.wanted.is_empty() {
            println!("the view complete after {:.2} s", start.elapsed().as_secs_f64());
            in_view = true;
        }
        let made = world.update(camera, &stats.levels, &stats.tiles);
        if !made.is_empty() {
            for (level, tiles) in made {
                renderer.set_tiles(level, tiles);
            }
        } else if !stats.wanted.is_empty() {
            renderer.wait();
        } else if world.busy() {
            world.wait();
        } else {
            return (frame, stats);
        }
    }
}

/** Compares the chunks the first frame wants, coverage off, with those the TS renderer wanted for the dump. */
fn check_requests(dump: &Dump, seed: u64, threads: usize) {
    let faces = view::faces();
    for (face, l) in faces.iter().zip(&dump.lattices) {
        assert_eq!(face.lattice, *l, "the faces are not the TS renderer's");
    }
    let focal = focal_length(dump.fov, dump.width, dump.height);
    let view = &view::View { yaw: dump.yaw, pitch: dump.pitch, width: dump.width, height: dump.height, focal };
    let shown = WORLD(seed);
    let mut renderer = renderer(&shown, threads, dump.render_distance);
    let heights = world::heights(&shown);
    let start = Instant::now();
    for (level, grid) in dump.grids.iter().enumerate() {
        if let Some(grid) = grid {
            renderer.set_tiles(level, Arc::new(Level::cut(&heights, level, grid)));
        }
    }
    println!("indexed the dump's grids in {:.2} s", start.elapsed().as_secs_f64());
    renderer.skip_covered = false;
    let (_, stats) = renderer.render(dump.camera, view);
    type Key = (usize, i32, i32, u64, i32, i32, [i32; 3], bool);
    let ours: HashSet<Key> =
        stats.wanted.iter().map(|r| (r.face, r.slice.level, r.slice.layer, r.slice.cutoff.to_bits(), r.i, r.j, r.eye, r.fill)).collect();
    let theirs: HashSet<Key> =
        dump.requests.iter().map(|r| (r.face, r.level, r.layer, r.cutoff.to_bits(), r.i, r.j, r.eye, r.fill)).collect();
    println!("ours {}, the dump's {}, in both {}", ours.len(), theirs.len(), ours.intersection(&theirs).count());
    for (name, a, b) in [("only ours", &ours, &theirs), ("only the dump's", &theirs, &ours)] {
        let mut only: Vec<_> = a.difference(b).collect();
        only.sort();
        println!("{name}: {}{}", only.len(), only.iter().take(8).map(|k| format!("\n  {k:?}")).collect::<String>());
    }
}

/**
 * Walks on along the view for `seconds` at the window's speed, a frame
 * every 60th of a second as the window draws it, the GPU a frame behind,
 * and says what the frames cost.
 */
fn stroll(seconds: f64, world: &mut World, renderer: &mut Renderer, mut camera: Vec3, view: &view::View, picture: &compositor::View) {
    let mut gpu = shot::Offscreen::new();
    // All of the first frame is new to this GPU.
    gpu.prepare(&renderer.render(camera, view).0, picture);
    let mut last = gpu.render();
    let forward = view_basis(view.yaw, 0.0)[2];
    let (mut times, mut short) = (Vec::new(), 0);
    let ms = |a: Instant, b: Instant| (b - a).as_secs_f64() * 1000.0;
    for _ in 0..((seconds * 60.0) as usize).max(1) {
        camera = std::array::from_fn(|d| camera[d] + forward[d] * SPEED / 60.0);
        let start = Instant::now();
        let (frame, stats) = renderer.render(camera, view);
        let rendered = Instant::now();
        for (level, tiles) in world.update(camera, &stats.levels, &stats.tiles) {
            renderer.set_tiles(level, tiles);
        }
        let updated = Instant::now();
        gpu.prepare(&frame, picture);
        let prepared = Instant::now();
        gpu.wait_for(last);
        last = gpu.render();
        let end = Instant::now();
        times.push([ms(start, rendered), ms(rendered, updated), ms(updated, prepared), ms(prepared, end), ms(start, end)]);
        short += usize::from(!stats.wanted.is_empty() || !stats.tiles.is_empty());
    }
    println!("{} frames walking, {short} with chunks or tiles missing; ms at the median, 90th and 99th percentile, and most:", times.len());
    for (k, name) in ["render", "world", "upload", "GPU", "frame"].iter().enumerate() {
        let mut t: Vec<f64> = times.iter().map(|t| t[k]).collect();
        t.sort_by(f64::total_cmp);
        let at = |q: f64| t[((t.len() - 1) as f64 * q) as usize];
        println!("  {name:>6}: {:6.1} {:6.1} {:6.1} {:6.1}", at(0.5), at(0.9), at(0.99), at(1.0));
    }
}

/** The view the TS renderer starts with (renderer.ts), at the size --shot draws. */
const YAW: f64 = 0.0;
const PITCH: f64 = 0.0;
const FOV: f64 = 90.0;
const RENDER_DISTANCE: f64 = 8000.0;
const SHOT: [f64; 2] = [1280.0, 800.0];
/** Units a second the window moves at. */
const SPEED: f64 = 40.0;
/** renderer.ts's: the fog's distance, a share of the render distance. */
const FOG_PER_RENDER_DISTANCE: f64 = 0.7;

fn main() -> eframe::Result {
    let args: Vec<String> = std::env::args().collect();
    let threads = args.iter().position(|a| a == "--threads").map_or_else(num_cpus, |i| args[i + 1].parse().expect("threads"));
    let seed = args.iter().position(|a| a == "--seed").map_or(12345, |i| args[i + 1].parse().expect("seed"));
    if let Some(i) = args.iter().position(|a| a == "--check-requests") {
        check_requests(&dump::read(&std::fs::read(&args[i + 1]).expect("dump")), seed, threads);
        return Ok(());
    }
    let size = SHOT;
    let focal = focal_length(FOV, size[0], size[1]);
    let first = view::View { yaw: YAW, pitch: PITCH, width: size[0], height: size[1], focal };
    let shown = WORLD(seed);
    let mut renderer = renderer(&shown, threads, RENDER_DISTANCE);
    if let Some(i) = args.iter().position(|a| a == "--leak") {
        renderer.max_leak = args[i + 1].parse().expect("leak");
    }
    let mut world = World::new(WORLD, seed, threads);
    let mut camera = shown.spawn.map(|v| v as f64);
    let faces: [[Vec3; 3]; 12] = std::array::from_fn(|f| view::faces()[f].axes);
    let fog_distance = RENDER_DISTANCE * FOG_PER_RENDER_DISTANCE;
    if args.iter().any(|a| ["--bench", "--shot", "--stroll"].contains(&a.as_str())) {
        let start = Instant::now();
        let (mut frame, mut stats) = complete(&mut world, &mut renderer, camera, &first);
        let report = |what: &str, start: Instant, frame: &renderer::Frame, stats: &renderer::Stats| {
            let seconds = start.elapsed().as_secs_f64();
            let (layers, chunks, covered) = (frame.layers.len(), stats.chunks, stats.covered);
            println!("{what} complete in {seconds:.2} s on {threads} threads: {layers} layers, {chunks} chunks, {covered} covered");
        };
        report("first frame", start, &frame, &stats);
        if let Some(i) = args.iter().position(|a| a == "--walk") {
            // On along the view, into tiles not made yet.
            let distance: f64 = args[i + 1].parse().expect("distance");
            let start = Instant::now();
            let forward = view_basis(YAW, 0.0)[2];
            camera = std::array::from_fn(|d| camera[d] + forward[d] * distance);
            (frame, stats) = complete(&mut world, &mut renderer, camera, &first);
            report(&format!("{distance} units on, the frame"), start, &frame, &stats);
        }
        if args.iter().any(|a| a == "--bench") {
            // What a frame costs the main thread once everything is baked, still and turning.
            for (name, turn) in [("still", 0.0), ("turning", 1.0)] {
                let start = Instant::now();
                for k in 0..20 {
                    renderer.render(camera, &view::View { yaw: first.yaw + turn * k as f64, ..first });
                }
                println!("a frame {name}: {:.1} ms", start.elapsed().as_secs_f64() * 1000.0 / 20.0);
            }
        }
        let view = compositor::View {
            yaw: YAW,
            pitch: PITCH,
            focal,
            fog_distance,
            faces,
            size: size.map(|v| v as f32),
            pixels: size.map(|v| v as u32),
            seams: args.iter().any(|a| a == "--seams"),
            colours: args.iter().position(|a| a == "--colour").map_or(Colours::Voxels, |i| {
                Colours::ALL
                    .into_iter()
                    .find(|(_, name)| name.eq_ignore_ascii_case(&args[i + 1]))
                    .expect("--colour voxels, layers or chunks")
                    .0
            }),
        };
        if args.iter().any(|a| a == "--bench") {
            let splats: usize = frame.layers.iter().flat_map(|l| &l.placements).map(|p| p.chunk.baked.splats.len()).sum();
            let (ms, [first, next]) = shot::time(&frame, &view, 30);
            println!("the GPU draws the frame in {ms:.1} ms: of its {splats} splats, {first} that showed before and {next} more");
        }
        if let Some(i) = args.iter().position(|a| a == "--stroll") {
            stroll(args[i + 1].parse().expect("seconds"), &mut world, &mut renderer, camera, &first, &view);
        }
        if let Some(i) = args.iter().position(|a| a == "--shot") {
            shot::save(&frame, &view, &args[i + 1]);
            println!("saved {}", args[i + 1]);
        }
        return Ok(());
    }
    let spin = args.iter().any(|a| a == "--spin").then(|| (Instant::now(), 0));
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([1580.0, 800.0]),
        wgpu_options: egui_wgpu::WgpuConfiguration {
            wgpu_setup: egui_wgpu::WgpuSetup::CreateNew(egui_wgpu::WgpuSetupCreateNew {
                device_descriptor: Arc::new(|adapter| eframe::wgpu::DeviceDescriptor {
                    required_limits: compositor::limits(adapter),
                    required_features: compositor::FEATURES,
                    ..Default::default()
                }),
                instance_descriptor: eframe::wgpu::InstanceDescriptor {
                    backends: compositor::BACKENDS,
                    ..eframe::wgpu::InstanceDescriptor::new_without_display_handle()
                },
                ..egui_wgpu::WgpuSetupCreateNew::without_display_handle()
            }),
            surface: egui_wgpu::SurfaceConfig {
                present_mode: eframe::wgpu::PresentMode::AutoNoVsync,
                ..egui_wgpu::SurfaceConfig::HIGH_THROUGHPUT
            },
            ..Default::default()
        },
        ..Default::default()
    };
    let app = App {
        world,
        renderer,
        faces,
        camera,
        yaw: YAW,
        pitch: PITCH,
        fov: FOV,
        seams: false,
        colours: Colours::Voxels,
        stats: String::new(),
        frame_ms: 0.0,
        spin,
    };
    eframe::run_native(
        "Parallax",
        options,
        Box::new(move |cc| {
            let render_state = cc.wgpu_render_state.as_ref().expect("wgpu");
            let compositor = Compositor::new(&render_state.device, render_state.target_format);
            render_state.renderer.write().callback_resources.insert(compositor);
            Ok(Box::new(app))
        }),
    )
}

fn num_cpus() -> usize {
    std::thread::available_parallelism().map_or(1, |n| n.get())
}
