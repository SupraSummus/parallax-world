//! Spike of a native frontend: bakes the chunks of a dumped view with the
//! baker in bake/ on every core, from one shared copy of the grids, and
//! draws them with wgpu in an egui window. The camera turns and moves over
//! the chunks baked at the start (no rebaking yet).
//!
//! parallax-native <dump> [--threads N] [--bench]

mod compositor;
mod dump;
mod view;

use std::time::Instant;

use bake::{Baker, Slice};
use eframe::egui;
use eframe::egui_wgpu;
use rayon::prelude::*;

use compositor::{Compositor, GpuChunk, GpuSplat, View};
use dump::Dump;
use view::{focal_length, to_local, view_basis};

/** A baked chunk: its numbers as ChunkSplats has them. */
struct Baked {
    request: usize,
    origin: [f64; 2],
    extent: [f64; 2],
    painted: usize,
    numbers: Vec<f32>,
}

/** Bakes every request of the dump on the current rayon pool, a baker per thread sharing the grids. */
fn bake_all(dump: &Dump) -> Vec<Baked> {
    dump.requests
        .par_iter()
        .enumerate()
        .map_init(
            || {
                let mut baker = Baker::new(dump.geometry(), dump.lattices, dump.palettes.clone());
                for (level, grid) in dump.grids.iter().enumerate() {
                    if let Some(grid) = grid {
                        baker.share_grid(level, grid.clone());
                    }
                }
                baker
            },
            |baker, (n, r)| {
                let slice = Slice { level: r.level, layer: r.layer, cutoff: r.cutoff };
                let b = baker.bake(r.face, &slice, r.i, r.j, r.eye, r.fill);
                Baked {
                    request: n,
                    origin: [b.origin_x, b.origin_y],
                    extent: [b.extent_width, b.extent_height],
                    painted: b.painted,
                    numbers: baker.numbers.clone(),
                }
            },
        )
        .filter(|b| !b.numbers.is_empty())
        .collect()
}

fn timed_bake(dump: &Dump, threads: usize) -> (Vec<Baked>, f64) {
    let pool = rayon::ThreadPoolBuilder::new().num_threads(threads).build().expect("thread pool");
    let start = Instant::now();
    let baked = pool.install(|| bake_all(dump));
    (baked, start.elapsed().as_secs_f64())
}

/**
 * The chunks and splats for the GPU: layers numbered face by face, near
 * to far from the dump's camera, as Frame.layers are.
 */
fn gpu_data(dump: &Dump, baked: &[Baked]) -> (Vec<GpuChunk>, Vec<GpuSplat>, usize) {
    let distance = |n: usize| {
        let r = &dump.requests[n];
        r.centre - to_local(&dump.faces[r.face], dump.camera)[2]
    };
    let key = |n: usize| {
        let r = &dump.requests[n];
        (r.face, r.level, r.layer, r.cutoff.to_bits())
    };
    let mut layers: Vec<(usize, usize)> = baked.iter().map(|b| (dump.requests[b.request].face, b.request)).collect();
    layers.sort_by(|a, b| a.0.cmp(&b.0).then(distance(a.1).total_cmp(&distance(b.1))));
    layers.dedup_by_key(|(_, n)| key(*n));
    let rank: std::collections::HashMap<_, _> = layers.iter().enumerate().map(|(i, (_, n))| (key(*n), i)).collect();
    let count = layers.len() as f32 + 1.5;

    let mut chunks = Vec::with_capacity(baked.len());
    let mut splats = Vec::new();
    for (c, b) in baked.iter().enumerate() {
        let r = &dump.requests[b.request];
        chunks.push(GpuChunk {
            origin: [b.origin[0] as f32, b.origin[1] as f32],
            extent: [b.extent[0] as f32, b.extent[1] as f32],
            centre: r.centre as f32,
            face: r.face as u32,
            depth: (rank[&key(b.request)] as f32 + 0.5) / count,
            pad: 0.0,
        });
        for (s, n) in b.numbers.as_chunks::<5>().0.iter().enumerate() {
            let grown = if s < b.painted { 0 } else { 1 << 31 };
            splats.push([n[0].to_bits(), n[1].to_bits(), n[2].to_bits(), n[3].to_bits(), n[4].to_bits(), c as u32 | grown]);
        }
    }
    (chunks, splats, layers.len())
}

struct App {
    view: View,
    fov: f64,
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
                self.view.camera[0], self.view.camera[1], self.view.camera[2], self.view.yaw, self.view.pitch
            ));
            ui.label(format!("ui {:.2} ms", self.frame_ms));
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
            let v = &mut self.view;
            v.yaw += drag.x as f64 * 0.2 + moving[3] * 90.0 * dt;
            v.pitch = (v.pitch - drag.y as f64 * 0.2 + moving[4] * 90.0 * dt).clamp(-90.0, 90.0);
            let [right, _, _] = view_basis(v.yaw, 0.0);
            let level = [-right[2], 0.0, right[0]];
            let step = self.speed * dt;
            for d in 0..3 {
                v.camera[d] += (level[d] * moving[0] + right[d] * moving[1]) * step;
            }
            v.camera[1] += moving[2] * step;
            v.rect = rect;
            v.focal = focal_length(self.fov, rect.width() as f64, rect.height() as f64);
            if let Some((since, frames)) = &mut self.spin {
                v.yaw += 30.0 * dt;
                *frames += 1;
                if since.elapsed().as_secs_f64() >= 2.0 {
                    println!("{:.1} fps", *frames as f64 / since.elapsed().as_secs_f64());
                    *since = Instant::now();
                    *frames = 0;
                }
                ui.ctx().request_repaint();
            }
            if moving.iter().any(|m| *m != 0.0) {
                ui.ctx().request_repaint();
            }
            ui.painter().add(egui_wgpu::Callback::new_paint_callback(rect, self.view.clone()));
        });
        self.frame_ms = start.elapsed().as_secs_f64() * 1000.0;
    }
}

fn main() -> eframe::Result {
    let args: Vec<String> = std::env::args().collect();
    let path = args.get(1).expect("usage: parallax-native <dump> [--threads N] [--bench]");
    let threads = args.iter().position(|a| a == "--threads").map_or_else(num_cpus, |i| args[i + 1].parse().expect("threads"));
    let start = Instant::now();
    let dump = dump::read(&std::fs::read(path).expect("dump"));
    let grid_bytes: usize = dump.grids.iter().flatten().map(|g| g.cells.len() + g.bricks.len() * 4).sum();
    println!("read {} requests, {} MB of grids in {:.2} s", dump.requests.len(), grid_bytes >> 20, start.elapsed().as_secs_f64());

    if args.iter().any(|a| a == "--bench") {
        let mut counts: Vec<usize> = [1, 2, 4, threads].into_iter().filter(|t| *t <= threads).collect();
        counts.dedup();
        for t in counts {
            let (baked, seconds) = timed_bake(&dump, t);
            let splats: usize = baked.iter().map(|b| b.numbers.len() / 5).sum();
            println!(
                "{t} threads: {} chunks ({} with splats, {splats} splats) in {seconds:.2} s, {:.0} chunks/s",
                dump.requests.len(),
                baked.len(),
                dump.requests.len() as f64 / seconds
            );
        }
        return Ok(());
    }

    let (baked, seconds) = timed_bake(&dump, threads);
    let (chunks, splats, layers) = gpu_data(&dump, &baked);
    let stats = format!(
        "baked {} chunks on {threads} threads in {seconds:.2} s\n{} with splats, {} layers, {} splats ({} MB)",
        dump.requests.len(),
        chunks.len(),
        layers,
        splats.len(),
        (splats.len() * 24) >> 20
    );
    println!("{stats}");
    let view = View {
        camera: dump.camera,
        yaw: dump.yaw,
        pitch: dump.pitch,
        focal: 1.0,
        fog_distance: dump.render_distance * 0.7,
        sky_fog: 1.0 - (-1.0f64 / 0.7).exp(),
        layers,
        faces: dump.faces,
        rect: egui::Rect::NOTHING,
    };
    let fov = dump.fov;
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
    eframe::run_native(
        "Parallax",
        options,
        Box::new(move |cc| {
            let render_state = cc.wgpu_render_state.as_ref().expect("wgpu");
            let compositor = Compositor::new(render_state, &chunks, &splats);
            render_state.renderer.write().callback_resources.insert(compositor);
            Ok(Box::new(App { view, fov, speed: 40.0, stats, frame_ms: 0.0, spin }))
        }),
    )
}

fn num_cpus() -> usize {
    std::thread::available_parallelism().map_or(1, |n| n.get())
}
