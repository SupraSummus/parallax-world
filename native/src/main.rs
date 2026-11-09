//! Spike of a native frontend: a world of worlds/ (the valley, or --world)
//! generated in tiles, those in view first (world.rs), the renderer picking
//! the chunks in view (renderer.rs), worker threads baking them with the
//! baker in bake/, sharing the tiles, and wgpu drawing them in an egui window
//! (window.rs).
//!
//! parallax-native [--world NAME] [--threads N] [--seed S] [--view QUERY] [--light SKY,SUN,GLOW] [--detail D] [--all-chunks] [--shot out.png [--size WxH] [--seams] [--colour voxels|layers|chunks] | --bench] [--stroll S [--on-ground]]
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
//! what glows this bright: 1,1,0 a clear day, 0.01,0,1 night.
//! --detail sets viewing distance over slice thickness (renderer.rs);
//! --all-chunks bakes every chunk in view, covered or not, as the panel's
//! switch does.
//! --seams and --colour draw as the panel's switches do.

mod bakery;
mod compositor;
mod occlusion;
mod renderer;
mod shot;
mod view;
mod window;
mod world;

use std::sync::Arc;
use std::time::Instant;

use bake::Baker;
use bake::walk::{COLLISION_LEVEL, Ground, WalkInput, Walker};

use bakery::Bakery;
use compositor::Colours;
use renderer::Renderer;
use view::{Vec3, focal_length, view_basis};
use world::{Make, World};
use worldgen::generator::materials_of;
use worldgen::light::{Brightness, Light, Shared};

/** The world shown unless --world names another (worlds::WORLDS). */
const WORLD: Make = worlds::valley::valley;

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
    /** Walking on the ground (bake's walk.rs): the body the camera is the eye of; None flying. */
    walker: Option<Walker>,
    /** By material id, whether a walking body goes through it (KindDef::walk_through). */
    walk_through: Vec<bool>,
    /** The world, as --view gives it: `seed=…` and `&world=…` if given. */
    world_query: String,
    /** How bright the sky is (compositor::View). */
    sky: f32,
    /** The frame the ui drew last, for the window to draw (window::Content). */
    picture: Option<(Arc<renderer::Frame>, compositor::View, egui::Rect)>,
}

impl window::Content for App {
    fn picture(&mut self) -> Option<(Arc<renderer::Frame>, compositor::View, egui::Rect)> {
        self.picture.take()
    }

    fn describe(&self) -> String {
        last_frame()
    }

    fn ui(&mut self, ui: &mut egui::Ui) {
        let start = Instant::now();
        egui::Panel::left("panel").show(ui, |ui| {
            ui.heading("Parallax (native spike)");
            ui.label(&self.stats);
            // The web's (App.tsx).
            ui.add(egui::Slider::new(&mut self.fov, 5.0..=120.0).step_by(1.0).text("Field of View (°)"));
            ui.add(egui::Slider::new(&mut self.renderer.render_distance, 40.0..=40000.0).logarithmic(true).text("Render Distance"));
            // In quarter octaves, as the web's: dragging re-slices only at each step.
            let mut octaves = self.renderer.detail.log2();
            let detail = egui::Slider::new(&mut octaves, 0.0..=6.0).step_by(0.25).custom_formatter(|v, _| format!("{:.1}", 2f64.powf(v)));
            if ui.add(detail.text("Slice Detail")).changed() {
                self.renderer.detail = 2f64.powf(octaves);
            }
            let mut walking = self.walker.is_some();
            if ui.checkbox(&mut walking, "Walk on Ground (F)").changed() {
                self.walker = walking.then(|| Walker::at(self.camera));
            }
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
            let (frame, stats) = self.renderer.render(self.camera, &view);
            // Drawn from the next frame on.
            // Walking, the ground's level too, whether drawn or not.
            let mut levels = stats.levels;
            levels[COLLISION_LEVEL as usize] |= self.walker.is_some();
            let made = self.world.update(self.camera, &levels, &stats.tiles);
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
                sky: self.sky,
            };
            let pixels = ui.ctx().pixels_per_point();
            self.remember([rect.width() * pixels, rect.height() * pixels]);
            self.picture = Some((Arc::new(frame), view, rect));
            ui.painter().add(egui_wgpu::Callback::new_paint_callback(rect, compositor::Shown));
            // While moving, the view changes; until all is made and baked, tiles and chunks keep arriving, often enough for a frame now and then.
            // A walking body moves on while it falls or its eye catches up with a step.
            let walking = self.walker.is_some_and(|w| !w.settled());
            if moving.iter().any(|m| *m != 0.0) || walking {
                ui.ctx().request_repaint();
            } else if arrived || !stats.wanted.is_empty() || stats.occluding || self.world.busy() {
                ui.ctx().request_repaint_after(std::time::Duration::from_millis(16));
            }
        });
        self.frame_ms = start.elapsed().as_secs_f64() * 1000.0;
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
        let mut frame = format!(
            "--view '{}&camera={x},{y},{z}&look={},{}&fov={}&distance={:.0}&detail={}' --size {:.0}x{:.0}",
            self.world_query, self.yaw, self.pitch, self.fov, r.render_distance, r.detail, size[0], size[1],
        );
        if !r.skip_covered {
            frame += " --all-chunks";
        }
        *LAST_FRAME.lock().unwrap_or_else(|e| e.into_inner()) = format!("{frame}\n{}", self.stats);
    }
}

/** The world `make` makes of a seed, lit as `Shared` says, which holds its tiles. */
type Lit = (Make, u64, Shared);

/** The renderer and its bakery, for `world`; if `lit`, each baker lights the voxels it shows (a Light each). */
fn renderer(world: &worldgen::World, lit: Option<Lit>, threads: usize, render_distance: f64) -> Renderer {
    let faces = view::faces();
    let colours: Vec<u32> = materials_of(&world.generator).iter().map(|m| m.rgb()).collect();
    let bakery = Bakery::new(threads, move || {
        let mut baker = Baker::new(&colours);
        if let Some((make, seed, shared)) = lit.clone() {
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
        if !drawn && stats.chunks > 0 {
            println!("first chunks drawn after {:.2} s", start.elapsed().as_secs_f64());
            drawn = true;
        }
        if !in_view && stats.tiles.is_empty() && stats.wanted.is_empty() && !stats.occluding {
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
        } else if stats.occluding {
            renderer.wait_for_occlusion();
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
        short += usize::from(!stats.wanted.is_empty() || !stats.tiles.is_empty() || stats.occluding);
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
    let brightness = value("--light").map(|v| {
        let v: Vec<f32> = v.split(',').map(|v| v.parse().expect("--light SKY,SUN,GLOW")).collect();
        let [sky, sun, glow] = v.try_into().expect("--light SKY,SUN,GLOW");
        Brightness { sky, sun, glow }
    });
    let lit = brightness.map(|b| (make, seed, Shared::new(b)));
    let shown = make(seed);
    let mut renderer = renderer(&shown, lit.clone(), threads, render_distance);
    if let Some(v) = value("--detail").map(|v| v.parse().expect("detail")).or(query.detail) {
        renderer.detail = v;
    }
    renderer.skip_covered = !flag("--all-chunks");
    let sky = brightness.map_or(1.0, |b| b.sky);
    let mut world = World::new(make, seed, lit.map(|(.., shared)| shared), threads);
    let camera = query.camera.unwrap_or(shown.spawn.map(|v| v as f64));
    let faces: [[Vec3; 3]; 12] = std::array::from_fn(|f| view::faces()[f].axes);
    let fog_distance = render_distance * FOG_PER_RENDER_DISTANCE;
    if ["--bench", "--shot", "--stroll"].into_iter().any(flag) {
        let start = Instant::now();
        // A shot alone needs only the view; the timings, all the world would make around it.
        let whole = flag("--bench") || flag("--stroll");
        let (frame, stats) = complete(&mut world, &mut renderer, camera, &first, whole);
        let (layers, chunks, covered, seconds) = (frame.layers.len(), stats.chunks, stats.covered, start.elapsed().as_secs_f64());
        println!("first frame complete in {seconds:.2} s on {threads} threads: {layers} layers, {chunks} chunks, {covered} covered");
        if flag("--bench") {
            // What a frame costs the main thread once everything is baked, still and turning.
            for (name, turn) in [("still", 0.0), ("turning", 1.0)] {
                let start = Instant::now();
                for k in 0..20 {
                    renderer.render(camera, &view::View { yaw: first.yaw + turn * k as f64, ..first });
                }
                println!("a frame {name}: {:.1} ms", start.elapsed().as_secs_f64() * 1000.0 / 20.0);
            }
            renderer.wait_for_occlusion();
            println!("an occlusion pass, on its own thread: {:.1} ms", renderer.occlusion_seconds() * 1000.0);
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
            let splats: usize = frame.layers.iter().flat_map(|l| &l.placements).map(|p| p.chunk.baked.splats.len()).sum();
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
        frame_ms: 0.0,
        walker: None,
        walk_through: materials_of(&shown.generator).iter().map(|m| m.walk_through).collect(),
        sky,
        world_query: format!("seed={seed}{}", world_name.map(|w| format!("&world={w}")).unwrap_or_default()),
        picture: None,
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
