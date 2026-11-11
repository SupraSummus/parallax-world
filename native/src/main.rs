//! Spike of a native frontend: a world of worlds/ (the valley, or --world)
//! generated in tiles, those in view first (world.rs), the renderer picking
//! the chunks in view (renderer.rs), worker threads baking them with the
//! baker in bake/, sharing the tiles, and wgpu drawing them in an egui window
//! (window.rs).
//!
//! parallax-native [--engine splats|voxels|slabs] [--world NAME] [--threads N] [--seed S] [--view QUERY] [--light SKY,SUN,GLOW] [--detail D] [--shift S] [--shot out.png [--size WxH] [--seams] [--colour voxels|layers|chunks] | --bench] [--stroll S [--on-ground]]
//!
//! --view takes the web page's view query (parseViewQuery in renderer.ts),
//! overriding --seed and --world.
//! --shot draws the first frame once all the tiles and chunks in view are
//! in, at 1280×800 or --size, offscreen, and saves it; --bench only times
//! the first frame once the world has all its tiles, the main thread's
//! frames after, and the GPU drawing the last. --stroll S then flies on for S seconds
//! (walks, --on-ground) as the window would, a frame every 60th, and times each frame's parts.
//! When the window panics or loses its GPU device, it says what it showed as these arguments.
//! --light lights the world (worldgen's light.rs), the sky, the sun and
//! what glows this bright: 1,1,0 a clear day (as the panel's Computed
//! lighting is unless --light), 0.01,0,1 night.
//! --detail sets viewing distance over a cell's thickness, --shift how many
//! levels finer than a cell its voxels are (bake's lod.rs), as the panel does.
//! --seams and --colour draw as the panel's switches do.
//! --engine picks what draws, as the panel does (Engine): rays through the
//! tiles' voxels (march.rs, the default), which take no --detail,
//! --shift, --seams or --colour; chunks of splats, as the web draws; or rays
//! through slabs baked along each face (trace.rs), for --shot and --bench
//! as seen from where they are baked, which take no --light, --seams or
//! --colour.
//! The panel picks the lighting apart from the engine (Lighting): voxels
//! shaded by the way their faces face, or lit by light worked out across
//! the world, as --light does; each engine shows it its own way.

mod bakery;
mod compositor;
mod march;
mod renderer;
mod shot;
mod trace;
mod view;
mod window;
mod world;

use std::sync::Arc;
use std::time::Instant;

use bake::Baker;
use bake::slicing::GRID_LEVELS;
use bake::walk::{COLLISION_LEVEL, Ground, WalkInput, Walker};

use bakery::Bakery;
use compositor::Colours;
use renderer::Renderer;
use view::{Vec3, focal_length, view_basis};
use world::{Lit, Make, World};
use worldgen::generator::materials_of;
use worldgen::light::{Brightness, Light, Shared};

/** The world shown unless --world names another (worlds::WORLDS). */
const WORLD: Make = worlds::valley::valley;

/** What draws the world: each its own way, from the same tiles (window::Picture). */
#[derive(Clone, Copy, PartialEq, Debug)]
enum Engine {
    Splats,
    Voxels,
    Slabs,
}

impl Engine {
    const ALL: [(Engine, &str); 3] = [(Engine::Splats, "splats"), (Engine::Voxels, "voxels"), (Engine::Slabs, "slabs")];

    /** How the world's tiles are lit for it, under `lighting`: the slabs show none. */
    fn lit(self, lighting: Lighting) -> Lit {
        match (lighting, self) {
            (Lighting::Shaded, _) | (_, Engine::Slabs) => Lit::Not,
            (Lighting::Computed, Engine::Splats) => Lit::Reading,
            (Lighting::Computed, Engine::Voxels) => Lit::Tiles,
        }
    }
}

/**
 * How voxels are lit, whatever draws them: shaded by the way their faces
 * face, or by light worked out across the world (worldgen's Light), the
 * same for every engine.
 */
#[derive(Clone, Copy, PartialEq, Debug)]
enum Lighting {
    Shaded,
    Computed,
}

impl Lighting {
    const ALL: [(Lighting, &str); 2] = [(Lighting::Shaded, "shaded"), (Lighting::Computed, "computed")];
}

/**
 * Drawing slabs (trace.rs): the slabs baked last and their version, and
 * whether a bake is on its thread or the world has changed since. Slabs
 * are baked again when the eye leaves their margin, the view their faces,
 * or tiles come.
 */
struct Traced {
    slabs: Option<Arc<trace::Slabs>>,
    version: u64,
    baking: bool,
    dirty: bool,
    ask: std::sync::mpsc::Sender<Bake>,
    done: std::sync::mpsc::Receiver<trace::Slabs>,
}

/** What slabs to bake: around which eye, of which faces, from which tiles, how. */
type Bake = (Vec3, Vec<(usize, [f64; 4])>, Vec<Arc<bake::Tiles>>, trace::Params);

/** Layers of grid 0 around the eye slabs are baked with, in the window. */
const TRACE_MARGIN: f64 = 64.0;

impl Traced {
    /** Bakes on a thread of its own, with `threads` bakers. */
    fn new(colours: Vec<u32>, threads: usize) -> Traced {
        let (ask, asked) = std::sync::mpsc::channel::<Bake>();
        let (send, done) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            while let Ok(mut bake) = asked.recv() {
                // Only the latest.
                while let Ok(later) = asked.try_recv() {
                    bake = later;
                }
                let (camera, faces, levels, params) = bake;
                if send.send(trace::bake(&colours, &levels, camera, &faces, &params, threads)).is_err() {
                    return;
                }
            }
        });
        Traced { slabs: None, version: 0, baking: false, dirty: false, ask, done }
    }

    /** Takes slabs baked, and asks for new ones if need be, for an eye at `camera` seeing `view` with `fov`. */
    fn update(&mut self, arrived: bool, camera: Vec3, view: &view::View, fov: f64, renderer: &Renderer) {
        if let Ok(slabs) = self.done.try_recv() {
            (self.slabs, self.version, self.baking) = (Some(Arc::new(slabs)), self.version + 1, false);
        }
        self.dirty |= arrived;
        let stale = self.slabs.as_ref().is_none_or(|s| {
            let moved = (0..3).map(|d| (camera[d] - s.camera[d]).powi(2)).sum::<f64>().sqrt();
            moved > TRACE_MARGIN * bake::slicing::LAYER / 2.0 || !s.hold(&trace::faces_in(view))
        });
        if !self.baking && (stale || self.dirty) {
            // Room to turn.
            let wide = view::View { focal: focal_length((fov + 40.0).min(170.0), view.width, view.height), ..*view };
            let levels = (0..GRID_LEVELS).map(|l| renderer.level(l).grids.clone()).collect();
            if self.ask.send((camera, trace::faces_in(&wide), levels, trace_params(renderer, TRACE_MARGIN))).is_ok() {
                (self.baking, self.dirty) = (true, false);
            }
        }
    }

    fn describe(&self) -> String {
        let size = self.slabs.as_ref().map_or(0.0, |s| s.bytes() as f64 / 1e6);
        format!("slabs v{} ({size:.0} MB){}", self.version, if self.baking { ", baking" } else { "" })
    }
}

/** How slabs are made for `renderer`'s level of detail and render distance, with `margin` layers around the eye. */
fn trace_params(renderer: &Renderer, margin: f64) -> trace::Params {
    let q = ((2.0 * renderer.detail).ceil() as i32).max(4);
    trace::Params { shift: renderer.shift, q, far: renderer.render_distance, margin }
}

/** Has `world` make the tiles of `levels` around `camera`, and gives them to `renderer`, until none is left to make. */
fn make_tiles(world: &mut World, renderer: &mut Renderer, camera: Vec3, levels: &[bool; GRID_LEVELS]) {
    loop {
        for (level, tiles) in world.update(camera, levels, &[]) {
            renderer.set_tiles(level, tiles);
        }
        if !world.busy() {
            return;
        }
        world.wait();
    }
}

struct App {
    engine: Engine,
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
    times: window::Times,
    /** How long the renderer took to pick the last frame's chunks, ms. */
    picking: f64,
    /** Walking on the ground (bake's walk.rs): the body the camera is the eye of; None flying. */
    walker: Option<Walker>,
    /** By material id, whether a walking body goes through it (KindDef::walk_through). */
    walk_through: Vec<bool>,
    /** The world, as --view gives it: `seed=…` and `&world=…` if given. */
    world_query: String,
    lighting: Lighting,
    /** How bright the sky, the sun and what glows are, Computed. */
    brightness: Brightness,
    /** What the ui drew last, for the window to draw (window::Content). */
    picture: Option<(window::Picture, egui::Rect)>,
    /** By material id, its colour (the voxels' and the slabs'). */
    palette: Arc<Vec<u32>>,
    /** Drawing slabs, while it does; and the threads it may bake on. */
    traced: Option<Traced>,
    threads: usize,
}

impl window::Content for App {
    fn picture(&mut self) -> Option<(window::Picture, egui::Rect)> {
        self.picture.take()
    }

    fn describe(&self) -> String {
        last_frame()
    }

    fn timed(&mut self, times: window::Times) {
        self.times = times;
    }

    fn ui(&mut self, ui: &mut egui::Ui) {
        egui::Panel::left("panel").show(ui, |ui| {
            ui.heading("Parallax (native spike)");
            ui.label(&self.stats);
            ui.horizontal(|ui| {
                ui.label("Engine");
                for (engine, name) in Engine::ALL {
                    ui.radio_value(&mut self.engine, engine, name);
                }
            });
            ui.horizontal(|ui| {
                ui.label("Lighting");
                for (lighting, name) in Lighting::ALL {
                    ui.radio_value(&mut self.lighting, lighting, name);
                }
            });
            // The web's (App.tsx).
            ui.add(egui::Slider::new(&mut self.fov, 5.0..=120.0).step_by(1.0).text("Field of View (°)"));
            ui.add(egui::Slider::new(&mut self.renderer.render_distance, 40.0..=40000.0).logarithmic(true).text("Render Distance"));
            let mut walking = self.walker.is_some();
            if ui.checkbox(&mut walking, "Walk on Ground (F)").changed() {
                self.walker = walking.then(|| Walker::at(self.camera));
            }
            if self.engine != Engine::Voxels {
                // In quarter octaves, as the web's: dragging picks cells anew only at each step.
                let mut octaves = self.renderer.detail.log2();
                let detail =
                    egui::Slider::new(&mut octaves, 0.0..=6.0).step_by(0.25).custom_formatter(|v, _| format!("{:.1}", 2f64.powf(v)));
                if ui.add(detail.text("Detail")).changed() {
                    self.renderer.detail = 2f64.powf(octaves);
                }
                ui.add(egui::Slider::new(&mut self.renderer.shift, 1..=5).text("Voxels Finer than Cells (levels)"));
            }
            if self.engine == Engine::Splats {
                ui.checkbox(&mut self.seams, "Show Face Seams");
                ui.horizontal(|ui| {
                    ui.label("Colour by");
                    for (colours, name) in Colours::ALL {
                        ui.radio_value(&mut self.colours, colours, name);
                    }
                });
            }
            ui.label(format!(
                "camera {:.0} {:.0} {:.0}  yaw {:.0}  pitch {:.0}",
                self.camera[0], self.camera[1], self.camera[2], self.yaw, self.pitch
            ));
            let t = self.times;
            ui.label(format!(
                "frame {:.1} ms: ui {:.1} (chunks picked {:.1}), upload {:.1}, waiting for the GPU {:.1}, the rest {:.1}",
                t.since, t.ui, self.picking, t.upload, t.waiting, t.rest
            ));
            ui.label("WASD, Space/Shift, arrows or drag\nWalking: Space jumps, Shift sneaks");
        });
        egui::CentralPanel::default().frame(egui::Frame::NONE).show(ui, |ui| {
            let (rect, response) = ui.allocate_exact_size(ui.available_size(), egui::Sense::drag());
            let drag = response.drag_delta();
            let (dt, moving, toggle) = ui.input(|i| {
                let key = |k| if i.key_down(k) { 1.0 } else { 0.0 };
                let forward = key(egui::Key::W) - key(egui::Key::S);
                let right = key(egui::Key::D) - key(egui::Key::A);
                let (jump, sneak) = (key(egui::Key::Space), if i.modifiers.shift { 1.0 } else { 0.0 });
                let turn = key(egui::Key::ArrowRight) - key(egui::Key::ArrowLeft);
                let look = key(egui::Key::ArrowUp) - key(egui::Key::ArrowDown);
                (i.stable_dt.min(0.1) as f64, [forward, right, jump - sneak, turn, look, jump, sneak], i.key_pressed(egui::Key::F))
            });
            if toggle {
                self.walker = if self.walker.is_some() { None } else { Some(Walker::at(self.camera)) };
            }
            self.yaw += drag.x as f64 * 0.2 + moving[3] * 90.0 * dt;
            self.pitch = (self.pitch - drag.y as f64 * 0.2 + moving[4] * 90.0 * dt).clamp(-90.0, 90.0);
            let [right, _, _] = view_basis(self.yaw, 0.0);
            let level = [-right[2], 0.0, right[0]];
            if let Some(walker) = self.walker {
                let ground = Ground { tiles: &self.renderer.level(COLLISION_LEVEL as usize).grids, walk_through: &self.walk_through };
                let input =
                    WalkInput { forward: moving[0], right: moving[1], yaw: self.yaw, jump: moving[5] > 0.0, sneak: moving[6] > 0.0 };
                let walker = walker.walk(input, dt, &ground);
                self.camera = walker.eye();
                self.walker = Some(walker);
            } else {
                let step = SPEED * dt;
                for d in 0..3 {
                    self.camera[d] += (level[d] * moving[0] + right[d] * moving[1]) * step;
                }
                self.camera[1] += moving[2] * step;
            }
            let (width, height) = (rect.width() as f64, rect.height() as f64);
            let focal = focal_length(self.fov, width, height);
            let view = view::View { yaw: self.yaw, pitch: self.pitch, width, height, focal };
            let far = self.renderer.render_distance;
            let lit = self.engine.lit(self.lighting);
            self.renderer.lit = lit != Lit::Not;
            self.world.lit = lit;
            // The sky as bright as the light, if any.
            let lit_sky = (lit != Lit::Not).then_some(self.brightness.sky);
            let sky = lit_sky.unwrap_or(1.0);
            // How the splats and the slabs are seen.
            let faces_view = compositor::View {
                yaw: self.yaw,
                pitch: self.pitch,
                focal,
                fog_distance: far * FOG_PER_RENDER_DISTANCE,
                faces: self.faces,
                size: [rect.width(), rect.height()],
                pixels: [0, 0],
                seams: self.seams,
                colours: self.colours,
                sky,
            };
            // The tiles each engine reads, and what it draws: drawn from the next frame on.
            // And whether chunks or slabs are baking, for frames until they are in.
            let (mut levels, wanted, baking, picture) = match self.engine {
                Engine::Splats => {
                    let start = Instant::now();
                    let (frame, stats) = self.renderer.render(self.camera, &view);
                    self.picking = start.elapsed().as_secs_f64() * 1000.0;
                    self.stats =
                        format!("{} chunks\n{} baking, {} tiles wanted", frame.chunks.len(), stats.wanted.len(), stats.tiles.len());
                    (stats.levels, stats.tiles, !stats.wanted.is_empty(), window::Picture::Splats(Arc::new(frame), faces_view))
                }
                Engine::Slabs => {
                    let traced = self.traced.get_or_insert_with(|| Traced::new(self.palette.to_vec(), (self.threads / 2).max(1)));
                    self.stats = traced.describe();
                    let sky_fog = 1.0 - (-1.0 / FOG_PER_RENDER_DISTANCE).exp();
                    let picture = window::Picture::Slabs(traced.slabs.clone(), traced.version, self.camera, faces_view, sky_fog);
                    (trace_params(&self.renderer, TRACE_MARGIN).levels(), Vec::new(), traced.baking, picture)
                }
                Engine::Voxels => {
                    let levels = march::levels(far);
                    let tiles: Vec<_> = (0..levels).map(|l| self.renderer.level(l).clone()).collect();
                    self.stats = format!("{} tiles", tiles.iter().map(|l| l.tiles.len()).sum::<usize>());
                    let scene =
                        march::Scene::new(tiles, self.palette.clone(), lit_sky, self.camera, [self.yaw, self.pitch, self.fov], far, [0, 0]);
                    (std::array::from_fn(|l| l < levels), Vec::new(), false, window::Picture::Voxels(scene))
                }
            };
            // Walking, the ground's level too, whether drawn or not.
            levels[COLLISION_LEVEL as usize] |= self.walker.is_some();
            let made = self.world.update(self.camera, &levels, &wanted);
            let arrived = !made.is_empty();
            for (level, tiles) in made {
                self.renderer.set_tiles(level, tiles);
            }
            match (self.engine, &mut self.traced) {
                (Engine::Slabs, Some(traced)) => traced.update(arrived, self.camera, &view, self.fov, &self.renderer),
                // Their slabs, and their thread, gone with them.
                _ => self.traced = None,
            }
            let pixels = ui.ctx().pixels_per_point();
            self.remember([rect.width() * pixels, rect.height() * pixels]);
            self.picture = Some((picture, rect));
            match self.engine {
                Engine::Splats => ui.painter().add(egui_wgpu::Callback::new_paint_callback(rect, compositor::Shown)),
                Engine::Voxels => ui.painter().add(egui_wgpu::Callback::new_paint_callback(rect, march::Shown)),
                Engine::Slabs => ui.painter().add(egui_wgpu::Callback::new_paint_callback(rect, trace::Shown)),
            };
            // While moving, the view changes; until all is made and baked, tiles and chunks keep arriving, often enough for a frame now and then.
            // A walking body moves on while it falls or its eye catches up with a step.
            let walking = self.walker.is_some_and(|w| !w.settled());
            if moving.iter().any(|m| *m != 0.0) || walking {
                ui.ctx().request_repaint();
            } else if arrived || baking || self.world.busy() {
                ui.ctx().request_repaint_after(std::time::Duration::from_millis(16));
            }
        });
    }
}

/** The last frame, as the arguments that draw it (--shot, --bench) and its stats: for a report, should the window panic or lose its GPU device. */
static LAST_FRAME: std::sync::Mutex<String> = std::sync::Mutex::new(String::new());

fn last_frame() -> String {
    LAST_FRAME.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

impl App {
    fn remember(&self, size: [f32; 2]) {
        let ([x, y, z], r) = (self.camera, &self.renderer);
        let frame = format!(
            "--engine {:?} --view '{}&camera={x},{y},{z}&look={},{}&fov={}&distance={:.0}&detail={}' --shift {} --size {:.0}x{:.0}",
            self.engine, self.world_query, self.yaw, self.pitch, self.fov, r.render_distance, r.detail, r.shift, size[0], size[1],
        );
        let b = self.brightness;
        let light = if self.lighting == Lighting::Computed { format!(" --light {},{},{}", b.sky, b.sun, b.glow) } else { String::new() };
        *LAST_FRAME.lock().unwrap_or_else(|e| e.into_inner()) = format!("{frame}{light}\n{}", self.stats);
    }
}

/** The world `make` makes of a seed, lit as `Shared` says, which holds its tiles. */
type Lights = (Make, u64, Shared);

/** The renderer and its bakery, for `world`; given `lights`, each baker can light the voxels it shows (a Light each), as the renderer's `lit` says. */
fn renderer(world: &worldgen::World, lights: Option<Lights>, threads: usize, render_distance: f64) -> Renderer {
    let faces = view::faces();
    let colours: Vec<u32> = materials_of(&world.generator).iter().map(|m| m.rgb()).collect();
    let bakery = Bakery::new(threads, move || {
        let mut baker = Baker::new(&colours);
        if let Some((make, seed, shared)) = lights.clone() {
            let mut light = Light::new(make(seed), shared);
            baker.light = Some(Box::new(move |level, site| light.faces(level as u32, site.map(i64::from))));
        }
        baker
    });
    Renderer::new(faces, bakery, render_distance, world::heights(world))
}

/**
 * Renders until the frame has every chunk it wants and, if `whole`, the
 * world has made every tile it would, else stops it making them; says
 * when it first draws any, and when it has all the tiles and chunks in view.
 */
fn complete(
    world: &mut World,
    renderer: &mut Renderer,
    camera: Vec3,
    view: &view::View,
    whole: bool,
) -> (renderer::Frame, renderer::Stats) {
    let start = Instant::now();
    let (mut drawn, mut in_view) = (false, false);
    loop {
        let (frame, stats) = renderer.render(camera, view);
        if !drawn && !frame.chunks.is_empty() {
            println!("first chunks drawn after {:.2} s", start.elapsed().as_secs_f64());
            drawn = true;
        }
        if !in_view && stats.tiles.is_empty() && stats.wanted.is_empty() {
            println!("the view complete after {:.2} s", start.elapsed().as_secs_f64());
            in_view = true;
            if !whole {
                // Tiles still to make are out of view: they would only take cores from the GPU's drawing.
                world.stop();
                return (frame, stats);
            }
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

/**
 * Moves on along the view for `seconds` at the window's speed, flying, or
 * walking on the ground with `walk_through`, a frame every 60th of a
 * second as the window draws it, the GPU a frame behind, and says what
 * the frames cost, and where the slowest were.
 */
fn stroll(
    seconds: f64,
    walk_through: Option<&[bool]>,
    world: &mut World,
    renderer: &mut Renderer,
    mut camera: Vec3,
    view: &view::View,
    picture: &compositor::View,
) {
    let mut gpu = shot::Offscreen::new();
    // All of the first frame is new to this GPU.
    gpu.prepare(&renderer.render(camera, view).0, picture);
    let mut last = gpu.render();
    let forward = view_basis(view.yaw, 0.0)[2];
    let (mut times, mut short) = (Vec::new(), 0);
    let ms = |a: Instant, b: Instant| (b - a).as_secs_f64() * 1000.0;
    let mut walker = Walker::at(camera);
    let mut cameras = Vec::new();
    for _ in 0..((seconds * 60.0) as usize).max(1) {
        camera = match walk_through {
            Some(walk_through) => {
                let ground = Ground { tiles: &renderer.level(COLLISION_LEVEL as usize).grids, walk_through };
                let input = WalkInput { forward: 1.0, right: 0.0, yaw: view.yaw, jump: false, sneak: false };
                walker = walker.walk(input, 1.0 / 60.0, &ground);
                walker.eye()
            }
            None => std::array::from_fn(|d| camera[d] + forward[d] * SPEED / 60.0),
        };
        cameras.push(camera);
        let start = Instant::now();
        let (frame, stats) = renderer.render(camera, view);
        let rendered = Instant::now();
        let mut levels = stats.levels;
        levels[COLLISION_LEVEL as usize] |= walk_through.is_some();
        for (level, tiles) in world.update(camera, &levels, &stats.tiles) {
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
    let mut slowest: Vec<usize> = (0..times.len()).collect();
    slowest.sort_by(|&a, &b| times[b][3].total_cmp(&times[a][3]));
    for &k in slowest.iter().take(5) {
        let [x, y, z] = cameras[k];
        println!("  frame {k}: GPU {:.1} ms at camera={x},{y},{z}", times[k][3]);
    }
}

/**
 * As stroll, drawing the voxels: each frame's tiles taken, the voxels
 * that came uploaded and the frame drawn and waited for.
 */
fn stroll_voxels(
    seconds: f64,
    walk_through: Option<&[bool]>,
    world: &mut World,
    renderer: &mut Renderer,
    offscreen: &mut march::Offscreen,
    first: march::Scene,
) {
    let mut scene = first;
    let forward = view_basis(scene.yaw, 0.0)[2];
    let mut wanted: [bool; GRID_LEVELS] = std::array::from_fn(|l| l < scene.levels.len());
    wanted[COLLISION_LEVEL as usize] |= walk_through.is_some();
    let ms = |a: Instant, b: Instant| (b - a).as_secs_f64() * 1000.0;
    let (mut times, mut cameras) = (Vec::new(), Vec::new());
    let mut walker = Walker::at(scene.eye);
    offscreen.render(&scene);
    for _ in 0..((seconds * 60.0) as usize).max(1) {
        let camera = match walk_through {
            Some(walk_through) => {
                let ground = Ground { tiles: &renderer.level(COLLISION_LEVEL as usize).grids, walk_through };
                let input = WalkInput { forward: 1.0, right: 0.0, yaw: scene.yaw, jump: false, sneak: false };
                walker = walker.walk(input, 1.0 / 60.0, &ground);
                walker.eye()
            }
            None => std::array::from_fn(|d| scene.eye[d] + forward[d] * SPEED / 60.0),
        };
        cameras.push(camera);
        let start = Instant::now();
        for (level, tiles) in world.update(camera, &wanted, &[]) {
            renderer.set_tiles(level, tiles);
        }
        let updated = Instant::now();
        scene.eye = camera;
        for (l, tiles) in scene.levels.iter_mut().enumerate() {
            *tiles = renderer.level(l).clone();
        }
        offscreen.render(&scene);
        let drawn = Instant::now();
        offscreen.wait();
        let end = Instant::now();
        times.push([ms(start, updated), ms(updated, drawn), ms(drawn, end), ms(start, end)]);
    }
    println!("{} frames; ms at the median, 90th and 99th percentile, and most:", times.len());
    for (k, name) in ["world", "upload", "GPU", "frame"].iter().enumerate() {
        let mut t: Vec<f64> = times.iter().map(|t| t[k]).collect();
        t.sort_by(f64::total_cmp);
        let at = |q: f64| t[((t.len() - 1) as f64 * q) as usize];
        println!("  {name:>6}: {:6.1} {:6.1} {:6.1} {:6.1}", at(0.5), at(0.9), at(0.99), at(1.0));
    }
    let mut slowest: Vec<usize> = (0..times.len()).collect();
    slowest.sort_by(|&a, &b| times[b][3].total_cmp(&times[a][3]));
    for &k in slowest.iter().take(5) {
        let [x, y, z] = cameras[k];
        let [w, u, g, f] = times[k];
        println!("  frame {k}: {f:.1} ms (world {w:.1}, upload {u:.1}, GPU {g:.1}) at camera={x},{y},{z}");
    }
}

/** The view the TS renderer starts with (renderer.ts), at the size --shot draws unless --size. */
const YAW: f64 = 0.0;
const PITCH: f64 = 0.0;
const FOV: f64 = 90.0;
const RENDER_DISTANCE: f64 = 8000.0;
const SHOT: [f64; 2] = [1280.0, 800.0];

/** Units a second the window moves at. */
const SPEED: f64 = 40.0;
/** renderer.ts's: the fog's distance, a share of the render distance. */
const FOG_PER_RENDER_DISTANCE: f64 = 0.7;

/** A view as the web's page query gives it (parseViewQuery in renderer.ts); what it leaves out or gets wrong is None. */
#[derive(Default)]
struct ViewQuery {
    seed: Option<u64>,
    world: Option<String>,
    camera: Option<Vec3>,
    look: Option<[f64; 2]>,
    fov: Option<f64>,
    distance: Option<f64>,
    detail: Option<f64>,
}

fn parse_view_query(query: &str) -> ViewQuery {
    // URLSearchParams writes commas as %2C.
    let pairs: Vec<(&str, String)> = query
        .trim_start_matches('?')
        .split('&')
        .filter_map(|p| p.split_once('='))
        .map(|(k, v)| (k, v.replace("%2C", ",").replace("%2c", ",")))
        .collect();
    let get = |name: &str| pairs.iter().find(|(k, _)| *k == name).map(|(_, v)| v.as_str());
    fn numbers<const N: usize>(value: Option<&str>) -> Option<[f64; N]> {
        let v: Vec<f64> = value?.split(',').map(|v| v.trim().parse::<f64>()).collect::<Result<_, _>>().ok()?;
        v.try_into().ok().filter(|v: &[f64; N]| v.iter().all(|x| x.is_finite()))
    }
    ViewQuery {
        // As worlds' WebAssembly takes the web's.
        seed: numbers::<1>(get("seed")).map(|[v]| v as i64 as u64),
        world: get("world").filter(|w| worlds::by_name(w).is_some()).map(str::to_string),
        camera: numbers::<3>(get("camera")),
        look: numbers::<2>(get("look")),
        fov: numbers::<1>(get("fov")).map(|[v]| v),
        distance: numbers::<1>(get("distance")).map(|[v]| v),
        detail: numbers::<1>(get("detail")).map(|[v]| v),
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let flag = |name: &str| args.iter().any(|a| a == name);
    let value = |name: &str| args.iter().position(|a| a == name).map(|i| args[i + 1].as_str());
    let threads = value("--threads").map_or_else(num_cpus, |v| v.parse().expect("threads"));
    let query = value("--view").map(parse_view_query).unwrap_or_default();
    let seed = query.seed.unwrap_or_else(|| value("--seed").map_or(12345, |v| v.parse().expect("seed")));
    let world_name = query.world.as_deref().or_else(|| value("--world"));
    let make = world_name.map_or(WORLD, |name| {
        let names: Vec<&str> = worlds::WORLDS.iter().map(|e| e.name).collect();
        worlds::by_name(name).unwrap_or_else(|| panic!("--world {}", names.join(", "))).make
    });
    let size = value("--size").map_or(SHOT, |v| {
        let size: Vec<f64> = v.split('x').map(|v| v.parse().expect("--size WxH")).collect();
        size.try_into().expect("--size WxH")
    });
    let [yaw, pitch] = query.look.unwrap_or([YAW, PITCH]);
    let (fov, render_distance) = (query.fov.unwrap_or(FOV), query.distance.unwrap_or(RENDER_DISTANCE));
    let focal = focal_length(fov, size[0], size[1]);
    let first = view::View { yaw, pitch, width: size[0], height: size[1], focal };
    // Sky, sun and lamps, as in look's `light`: 1,1,0 a clear day, 0.01,0,1 night.
    let light = value("--light").map(|v| {
        let v: Vec<f32> = v.split(',').map(|v| v.parse().expect("--light SKY,SUN,GLOW")).collect();
        let [sky, sun, glow] = v.try_into().expect("--light SKY,SUN,GLOW");
        Brightness { sky, sun, glow }
    });
    let lighting = if light.is_some() { Lighting::Computed } else { Lighting::Shaded };
    let brightness = light.unwrap_or(Brightness { sky: 1.0, sun: 1.0, glow: 0.0 });
    let shared = Shared::new(brightness);
    let shown = make(seed);
    let mut renderer = renderer(&shown, Some((make, seed, shared.clone())), threads, render_distance);
    if let Some(v) = value("--detail").map(|v| v.parse().expect("detail")).or(query.detail) {
        renderer.detail = v;
    }
    if let Some(v) = value("--shift") {
        renderer.shift = v.parse().expect("--shift S");
    }
    let mut world = World::new(make, seed, Some(shared), threads);
    let camera = query.camera.unwrap_or(shown.spawn.map(|v| v as f64));
    let faces: [[Vec3; 3]; 12] = std::array::from_fn(|f| view::faces()[f].axes);
    let fog_distance = render_distance * FOG_PER_RENDER_DISTANCE;
    let engine = value("--engine").map_or(Engine::Voxels, |v| {
        Engine::ALL.into_iter().find(|(_, name)| name.eq_ignore_ascii_case(v)).expect("--engine splats, voxels or slabs").0
    });
    let palette: Arc<Vec<u32>> = Arc::new(materials_of(&shown.generator).iter().map(|m| m.rgb()).collect());
    let lit = engine.lit(lighting);
    renderer.lit = lit != Lit::Not;
    world.lit = lit;
    let lit_sky = (lit != Lit::Not).then_some(brightness.sky);
    let sky = lit_sky.unwrap_or(1.0);
    if ["--bench", "--shot", "--stroll"].into_iter().any(flag) && engine == Engine::Voxels {
        let levels = march::levels(render_distance);
        let start = Instant::now();
        make_tiles(&mut world, &mut renderer, camera, &std::array::from_fn(|l| l < levels));
        println!("tiles made in {:.2} s on {threads} threads", start.elapsed().as_secs_f64());
        let scene = |renderer: &Renderer, eye| {
            let tiles = (0..levels).map(|l| renderer.level(l).clone()).collect();
            march::Scene::new(tiles, palette.clone(), lit_sky, eye, [yaw, pitch, fov], render_distance, size.map(|v| v as u32))
        };
        let mut offscreen = march::Offscreen::new(lit_sky.is_some());
        if let Some(v) = value("--stroll") {
            let walk_through: Vec<bool> = materials_of(&shown.generator).iter().map(|m| m.walk_through).collect();
            let on_ground = flag("--on-ground").then_some(walk_through.as_slice());
            let first = scene(&renderer, camera);
            stroll_voxels(v.parse().expect("seconds"), on_ground, &mut world, &mut renderer, &mut offscreen, first);
            return;
        }
        let first = scene(&renderer, camera);
        offscreen.render(&first);
        let [held, lacking, pool] = offscreen.bricks();
        println!("{held} bricks of {pool} on the GPU, {lacking} lacking room");
        if flag("--bench") {
            println!("the GPU marches the frame in {:.1} ms", offscreen.time(&first, 30));
        }
        if let Some(v) = value("--shot") {
            offscreen.save(&first, v);
            println!("saved {v}");
        }
        return;
    }
    if ["--bench", "--shot", "--stroll"].into_iter().any(flag) && engine == Engine::Slabs {
        assert!(!flag("--stroll"), "--stroll draws splats or voxels");
        let params = trace_params(&renderer, 0.0);
        let start = Instant::now();
        make_tiles(&mut world, &mut renderer, camera, &params.levels());
        println!("tiles made in {:.2} s on {threads} threads", start.elapsed().as_secs_f64());
        let levels: Vec<_> = (0..GRID_LEVELS).map(|l| renderer.level(l).grids.clone()).collect();
        let slabs = trace::bake(&palette, &levels, camera, &trace::faces_in(&first), &params, threads);
        let view = compositor::View {
            yaw,
            pitch,
            focal,
            fog_distance,
            faces,
            size: size.map(|v| v as f32),
            pixels: size.map(|v| v as u32),
            seams: false,
            colours: Colours::Voxels,
            sky,
        };
        let mut tracer = trace::Tracer::new(slabs, &view, 1.0 - (-1.0 / FOG_PER_RENDER_DISTANCE).exp());
        if flag("--bench") {
            println!("the GPU traces the frame in {:.1} ms", tracer.time(30));
        }
        if let Some(v) = value("--shot") {
            tracer.save(v);
            println!("saved {v}");
        }
        return;
    }
    if ["--bench", "--shot", "--stroll"].into_iter().any(flag) {
        let start = Instant::now();
        // A shot alone needs only the view; the timings, all the world would make around it.
        let whole = flag("--bench") || flag("--stroll");
        let (frame, _) = complete(&mut world, &mut renderer, camera, &first, whole);
        let (chunks, seconds) = (frame.chunks.len(), start.elapsed().as_secs_f64());
        println!("first frame complete in {seconds:.2} s on {threads} threads: {chunks} chunks");
        if flag("--bench") {
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
            yaw,
            pitch,
            focal,
            fog_distance,
            faces,
            size: size.map(|v| v as f32),
            pixels: size.map(|v| v as u32),
            seams: flag("--seams"),
            colours: value("--colour").map_or(Colours::Voxels, |v| {
                Colours::ALL.into_iter().find(|(_, name)| name.eq_ignore_ascii_case(v)).expect("--colour voxels, layers or chunks").0
            }),
            sky,
        };
        if flag("--bench") {
            let splats: usize = frame.chunks.iter().map(|p| p.chunk.baked.splats.len()).sum();
            let (ms, [first, next]) = shot::time(&frame, &view, 30);
            println!("the GPU draws the frame in {ms:.1} ms: of its {splats} splats, {first} that showed before and {next} more");
        }
        if let Some(v) = value("--stroll") {
            let walk_through: Vec<bool> = materials_of(&shown.generator).iter().map(|m| m.walk_through).collect();
            let on_ground = flag("--on-ground").then_some(walk_through.as_slice());
            stroll(v.parse().expect("seconds"), on_ground, &mut world, &mut renderer, camera, &first, &view);
        }
        if let Some(v) = value("--shot") {
            shot::save(&frame, &view, v);
            println!("saved {v}");
        }
        return;
    }
    let app = App {
        engine,
        world,
        renderer,
        faces,
        camera,
        yaw,
        pitch,
        fov,
        seams: false,
        colours: Colours::Voxels,
        stats: String::new(),
        times: window::Times::default(),
        picking: 0.0,
        walker: None,
        walk_through: materials_of(&shown.generator).iter().map(|m| m.walk_through).collect(),
        lighting,
        brightness,
        world_query: format!("seed={seed}{}", world_name.map(|w| format!("&world={w}")).unwrap_or_default()),
        picture: None,
        palette,
        traced: None,
        threads,
    };
    let panic = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        eprintln!("The last frame:\n{}", last_frame());
        panic(info);
    }));
    window::run("Parallax", [1580.0, 800.0], app);
}

fn num_cpus() -> usize {
    std::thread::available_parallelism().map_or(1, |n| n.get())
}

#[cfg(test)]
mod tests {
    use super::*;

    /**
     * World `name` of seed 1 from its spawn, looking `yaw` and `pitch`,
     * once the view is complete: the frame, and how it is seen.
     */
    fn complete_view(name: &str, look: [f64; 2], detail: f64) -> (renderer::Frame, compositor::View) {
        complete_view_of(worlds::by_name(name).expect("world").make, look, detail)
    }

    /** complete_view of the world `make` makes. */
    fn complete_view_of(make: world::Make, [yaw, pitch]: [f64; 2], detail: f64) -> (renderer::Frame, compositor::View) {
        let shown = make(1);
        let mut renderer = renderer(&shown, None, 4, 1000.0);
        renderer.detail = detail;
        let mut world = World::new(make, 1, None, 4);
        let size = [320.0, 200.0];
        let focal = focal_length(FOV, size[0], size[1]);
        let first = view::View { yaw, pitch, width: size[0], height: size[1], focal };
        let (frame, _) = complete(&mut world, &mut renderer, shown.spawn.map(|v| v as f64), &first, false);
        let view = compositor::View {
            yaw,
            pitch,
            focal,
            fog_distance: 1000.0 * FOG_PER_RENDER_DISTANCE,
            faces: std::array::from_fn(|f| view::faces()[f].axes),
            size: size.map(|v| v as f32),
            pixels: size.map(|v| v as u32),
            seams: false,
            colours: Colours::Voxels,
            sky: 1.0,
        };
        (frame, view)
    }

    /** World, look and detail as a power of two. The thickest cells too: in relief, their splats move most with the camera. */
    const VIEWS: [(&str, [f64; 2], f64); 3] =
        [("forest", [0.0, 0.0], 3.25), ("forest", [20.0, -25.0], 2.25), ("moor", [-30.0, 10.0], 3.25)];

    #[test]
    #[ignore = "draws on the GPU: needs Vulkan, as Mesa's software one"]
    fn culling_on_the_gpu_leaves_the_picture_unchanged() {
        for (name, look, octaves) in VIEWS {
            let (frame, view) = complete_view(name, look, 2f64.powf(octaves));
            let [culled, all] = [true, false].map(|culls| {
                let mut gpu = shot::Offscreen::with(None, culls);
                gpu.prepare(&frame, &view);
                (gpu.picture().2, gpu.drawn().iter().sum::<u32>())
            });
            assert!(culled.0 == all.0, "{name} {look:?}");
            assert!(culled.1 < all.1, "{name} {look:?}: culling drew {} splats of {}", culled.1, all.1);
        }
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
            KindDef { name: "medium", kind_type: 8, colors: &["#000000"], walk_through: true, glows: false },
            KindDef { name: "wall", kind_type: 10, colors: &["#ffffff"], walk_through: false, glows: false },
        ];
        let slab = Field::at("slab", 0.0, Op::Fill, |x, y, z, _| match x + z {
            _ if y.abs() >= 48 => None,
            k if (NEAR..FAR).contains(&k) && MILLI > 0 => Some((MEDIUM, Some(f64::from(MILLI) / 1000.0))),
            k if (FAR..FAR + 16).contains(&k) => Some((WALL, None)),
            _ => None,
        });
        let generator = Generator { kinds, seed: 1, layers: vec![Layer::Field(slab)] };
        worldgen::World { generator, spawn: [0, 0, 0], heights: (-64, 64), points: Vec::new() }
    }

    /** The splats of the world `make` makes, from its spawn, looking along the face (1, 0, 1), without fog: its width and pixels. */
    fn splats_along_a_face(make: world::Make) -> (u32, Vec<u8>) {
        let (frame, view) = complete_view_of(make, [45.0, 0.0], 8.0);
        let (width, _, pixels) = shot::picture(&frame, &compositor::View { fog_distance: f64::INFINITY, ..view });
        (width, pixels)
    }

    /** splats_along_a_face, of its voxels (march.rs). */
    fn voxels_along_a_face(make: world::Make) -> (u32, Vec<u8>) {
        let shown = make(1);
        let mut world = World::new(make, 1, None, 4);
        let far = 1000.0;
        let count = march::levels(far);
        let mut levels: Vec<Arc<world::Level>> = (0..count).map(|_| Arc::new(world::Level::new(Default::default()))).collect();
        let camera = shown.spawn.map(|v| v as f64);
        loop {
            for (level, tiles) in world.update(camera, &std::array::from_fn(|l| l < count), &[]) {
                levels[level] = tiles;
            }
            if !world.busy() {
                break;
            }
            world.wait();
        }
        let palette = Arc::new(materials_of(&shown.generator).iter().map(|m| m.rgb()).collect());
        let scene =
            march::Scene { fog: f64::INFINITY, ..march::Scene::new(levels, palette, None, camera, [45.0, 0.0, FOV], far, [320, 200]) };
        let (width, _, pixels) = march::Offscreen::new(false).picture(&scene);
        (width, pixels)
    }

    #[test]
    #[ignore = "draws on the GPU: needs Vulkan, as Mesa's software one"]
    fn a_medium_of_splats_lets_through_as_much_as_its_extinction_says() {
        medium_lets_through_as_much_as_its_extinction_says(splats_along_a_face);
    }

    #[test]
    #[ignore = "draws on the GPU: needs Vulkan, as Mesa's software one"]
    fn a_medium_of_voxels_lets_through_as_much_as_its_extinction_says() {
        medium_lets_through_as_much_as_its_extinction_says(voxels_along_a_face);
    }

    /** A wall behind a slab of medium, as `picture` draws it, lets through exp(−extinction · path), thin and thick, along a face's axis and slanted. */
    fn medium_lets_through_as_much_as_its_extinction_says(picture: fn(world::Make) -> (u32, Vec<u8>)) {
        // The mean of a box of 9 × 9 pixels around (x, y) of the wall, seen along the face's axis.
        let seen = |make: world::Make| {
            let (width, pixels) = picture(make);
            move |[x, y]: [usize; 2]| {
                let at = |i: usize, j: usize| f64::from(pixels[4 * (j * width as usize + i)]);
                (y - 4..=y + 4).flat_map(|j| (x - 4..=x + 4).map(move |i| (i, j))).map(|(i, j)| at(i, j)).sum::<f64>() / 81.0
            }
        };
        let bare = seen(wall::<0>);
        // The slab is (FAR − NEAR) / √2 thick along the axis; a ray at tangent t crosses √(1 + t²) times that.
        let thick = (FAR - NEAR) as f64 / std::f64::consts::SQRT_2;
        // Thin, a site stops a few SHARE-ths of a step of alpha.
        for (milli, make) in [(1, wall::<1> as world::Make), (4, wall::<4>), (15, wall::<15>)] {
            let veiled = seen(make);
            for (t, at) in [(0.0, [160, 100]), (0.5, [240, 100])] {
                let through = veiled(at) / bare(at);
                let expected = (-f64::from(milli) / 1000.0 * thick * (1.0f64 + t * t).sqrt()).exp();
                assert!((through - expected).abs() < 0.01, "{milli}‰ at tangent {t}: {through:.3} let through, {expected:.3} expected");
            }
        }
    }

    #[test]
    #[ignore = "draws on the GPU: needs Vulkan, as Mesa's software one"]
    fn what_the_buffers_cannot_hold_is_left_out_farthest_first() {
        let (mut frame, view) = complete_view("moor", [-30.0, 10.0], 2f64.powf(3.25));
        let all = shot::picture(&frame, &view).2;
        // Room for about half of the frame's splats: the chunks that come first, as many as fit.
        let size = size_of::<compositor::GpuSplat>();
        let room = frame.chunks.iter().map(|p| p.chunk.baked.splats.len()).sum::<usize>() / 2;
        let mut gpu = shot::Offscreen::with(Some((room * size) as u64), true);
        gpu.prepare(&frame, &view);
        let some = gpu.picture().2;
        let sums = frame.chunks.iter().scan(0, |sum, p| {
            *sum += p.chunk.baked.splats.len();
            Some(*sum)
        });
        let fit = sums.take_while(|&sum| sum <= room).count();
        assert!(fit > 0 && fit < frame.chunks.len(), "{fit} of {} chunks fit", frame.chunks.len());
        frame.chunks.truncate(fit);
        let first = shot::picture(&frame, &view).2;
        assert!(some != all, "some left out");
        assert!(some == first, "what is drawn shows as it would alone");
    }
}
