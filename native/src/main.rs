//! Spike of a native frontend: worldgen's forest generated in tiles around
//! the camera (world.rs), the renderer picking the chunks in view
//! (renderer.rs), worker threads baking them with the baker in bake/ from
//! one shared copy of the grids, and wgpu drawing them in an egui window.
//! The baker's tables and the starting view still come from a view dumped
//! by the TS renderer (native/dump.test.ts).
//!
//! parallax-native <dump> [--threads N] [--seed S] [--shot out.png | --bench] [--walk D] [--check-requests | --spin]
//!
//! --shot draws the first frame once the grids are in and all its chunks
//! are baked, at the dump's size, offscreen, and saves it; --bench only
//! times that, and the main thread's frames after. --walk D moves the
//! camera D units along the view after the first frame, and does the same
//! there. --check-requests
//! compares the chunks the first frame wants, from the dump's grids and
//! camera, with those the TS renderer wanted.

mod bakery;
mod compositor;
mod dump;
mod layer_index;
mod renderer;
mod shot;
mod slicing;
mod view;
mod world;

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Instant;

use bake::{Baker, Geometry};
use eframe::egui;
use eframe::egui_wgpu;

use bakery::Bakery;
use compositor::Compositor;
use dump::Dump;
use layer_index::LayerIndex;
use renderer::Renderer;
use view::{Vec3, focal_length, view_basis};
use world::World;

struct App {
    world: World,
    renderer: Renderer,
    faces: [[Vec3; 3]; 12],
    camera: Vec3,
    yaw: f64,
    pitch: f64,
    fov: f64,
    fog_distance: f64,
    speed: f64,
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
            ui.add(egui::Slider::new(&mut self.fov, 5.0..=120.0).text("FOV"));
            ui.add(egui::Slider::new(&mut self.speed, 1.0..=400.0).logarithmic(true).text("Speed (units/s)"));
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
            let step = self.speed * dt;
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
            for g in self.world.update(self.camera) {
                self.renderer.set_grid(g.level, g.grid, g.index);
            }
            let (frame, stats) = self.renderer.render(self.camera, &view);
            self.stats =
                format!("{} layers, {} chunks\n{} baking, {} covered", frame.layers.len(), stats.chunks, stats.wanted.len(), stats.covered);
            let view = compositor::View {
                yaw: self.yaw,
                pitch: self.pitch,
                focal,
                fog_distance: self.fog_distance,
                faces: self.faces,
                rect: [rect.min.x, rect.min.y, rect.width(), rect.height()],
                pixels_per_point: 1.0,
            };
            let callback = compositor::Callback { frame: Arc::new(frame), view };
            ui.painter().add(egui_wgpu::Callback::new_paint_callback(rect, callback));
            // Until all is baked, chunks keep arriving; while moving, the view changes.
            if !stats.wanted.is_empty() || self.world.busy() || moving.iter().any(|m| *m != 0.0) || self.spin.is_some() {
                ui.ctx().request_repaint();
            }
        });
        self.frame_ms = start.elapsed().as_secs_f64() * 1000.0;
    }
}

/** The renderer and its bakery, from the dump's tables; checks that the faces are the TS renderer's. */
fn renderer(dump: &Dump, threads: usize) -> Renderer {
    let faces = view::faces();
    for (face, l) in faces.iter().zip(&dump.lattices) {
        assert_eq!(face.lattice, [l.r, l.u, l.f], "the dump's faces are not ours");
    }
    let (table, lattices, palettes) = (dump.geometry.clone(), dump.lattices, dump.palettes.clone());
    let bakery = Bakery::new(threads, move || Baker::new(Geometry::from_table(&table), lattices, palettes.clone()));
    Renderer::new(faces, bakery, dump.render_distance)
}

fn lattices() -> Vec<[[i32; 3]; 3]> {
    view::faces().iter().map(|f| f.lattice).collect()
}

/** Renders until the world's grids are in and the frame has every chunk it wants. */
fn complete(world: &mut World, renderer: &mut Renderer, camera: Vec3, view: &view::View) -> (renderer::Frame, renderer::Stats) {
    loop {
        for g in world.update(camera) {
            renderer.set_grid(g.level, g.grid, g.index);
        }
        let (frame, stats) = renderer.render(camera, view);
        if !stats.wanted.is_empty() {
            renderer.wait();
        } else if world.busy() {
            world.wait();
        } else {
            return (frame, stats);
        }
    }
}

/** Compares the chunks the first frame wants, coverage off, with those the TS renderer wanted for the dump. */
fn check_requests(dump: &Dump, renderer: &mut Renderer, view: &view::View) {
    let start = Instant::now();
    let lattices = lattices();
    for (level, grid) in dump.grids.iter().enumerate() {
        if let Some(grid) = grid {
            renderer.set_grid(level, grid.clone(), LayerIndex::new(grid, &lattices));
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

fn main() -> eframe::Result {
    let args: Vec<String> = std::env::args().collect();
    let path = args.get(1).expect("usage: parallax-native <dump> [options]: see main.rs");
    let threads = args.iter().position(|a| a == "--threads").map_or_else(num_cpus, |i| args[i + 1].parse().expect("threads"));
    let dump = dump::read(&std::fs::read(path).expect("dump"));
    let seed = args.iter().position(|a| a == "--seed").map_or(12345, |i| args[i + 1].parse().expect("seed"));
    let size = [dump.width, dump.height];
    let focal = focal_length(dump.fov, size[0], size[1]);
    let first = view::View { yaw: dump.yaw, pitch: dump.pitch, width: size[0], height: size[1], focal };
    let mut renderer = renderer(&dump, threads);
    if args.iter().any(|a| a == "--check-requests") {
        check_requests(&dump, &mut renderer, &first);
        return Ok(());
    }
    let mut world = World::new(seed, lattices(), threads);
    let mut camera = worldgen::forest::forest(seed).1.map(|v| v as f64);
    let faces: [[Vec3; 3]; 12] = std::array::from_fn(|f| view::faces()[f].axes);
    let fog_distance = dump.render_distance * 0.7;
    if args.iter().any(|a| a == "--bench") || args.iter().any(|a| a == "--shot") {
        let start = Instant::now();
        let (mut frame, mut stats) = complete(&mut world, &mut renderer, camera, &first);
        let report = |what: &str, start: Instant, frame: &renderer::Frame, stats: &renderer::Stats| {
            let seconds = start.elapsed().as_secs_f64();
            let (layers, chunks, covered) = (frame.layers.len(), stats.chunks, stats.covered);
            println!("{what} complete in {seconds:.2} s on {threads} threads: {layers} layers, {chunks} chunks, {covered} covered");
        };
        report("first frame", start, &frame, &stats);
        if let Some(i) = args.iter().position(|a| a == "--walk") {
            // On along the view, the grids moving with the camera.
            let distance: f64 = args[i + 1].parse().expect("distance");
            let start = Instant::now();
            let forward = view_basis(dump.yaw, 0.0)[2];
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
        if let Some(i) = args.iter().position(|a| a == "--shot") {
            let view = compositor::View {
                yaw: dump.yaw,
                pitch: dump.pitch,
                focal,
                fog_distance,
                faces,
                rect: [0.0, 0.0, size[0] as f32, size[1] as f32],
                pixels_per_point: 1.0,
            };
            shot::save(&frame, &view, size.map(|v| v as u32), &args[i + 1]);
            println!("saved {}", args[i + 1]);
        }
        return Ok(());
    }
    let spin = args.iter().any(|a| a == "--spin").then(|| (Instant::now(), 0));
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([1580.0, 800.0]),
        depth_buffer: 32,
        wgpu_options: egui_wgpu::WgpuConfiguration {
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
        yaw: dump.yaw,
        pitch: dump.pitch,
        fov: dump.fov,
        fog_distance,
        speed: 40.0,
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
