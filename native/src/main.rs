//! The native frontend: a world of worlds/ (the valley, or --world)
//! generated in tiles, those in view first (world.rs), drawn by an engine
//! (engine.rs) with wgpu in an egui window (window.rs), or offscreen (gpu.rs).
//!
//! parallax-native [--engine reliefs|flat|splats|voxels] [--world NAME] [--threads N] [--seed S] [--view QUERY] [--light SKY,SUN,GLOW] [--detail D] [--shift S] [--seams] [--colour voxels|layers|chunks] [--shot out.png [--size WxH] | --bench] [--stroll S [--on-ground]]
//!
//! --view takes the web page's view query (parseViewQuery in renderer.ts),
//! overriding --seed and --world.
//! --shot draws the first frame once all it shows is in, at 1280×800 or
//! --size, offscreen, and saves it; --bench only times the first frame
//! once the world has all its tiles, the main thread's frames after, and
//! the GPU drawing the last. --stroll S then flies on for S seconds
//! (walks, --on-ground) as the window would, a frame every 60th, and times each frame's parts.
//! When the window panics or loses its GPU device, it says what it showed as these arguments.
//! --light lights the world (worldgen's light.rs), the sky, the sun and
//! what glows this bright: 1,1,0 a clear day (as the panel's Computed
//! lighting is unless --light), 0.01,0,1 night.
//! --engine picks what draws, as the panel does (Kind): rays through the
//! tiles' voxels (the default), chunks of cells' reliefs or splats, as the
//! web draws, or reliefs drawn flat (an experiment).
//! --detail sets viewing distance over a cell's thickness, --shift how many
//! levels finer than a cell its voxels are (bake's lod.rs), for the
//! engines that draw cells; --seams and --colour draw chunks as the panel's
//! switches do.
//! The panel picks the lighting apart from the engine (Lighting): voxels
//! shaded by the way their faces face, or lit by light worked out across
//! the world, as --light does, as bright as its sliders say; each engine
//! shows it its own way.

mod engine;
mod gpu;
mod view;
mod window;
mod world;

use std::time::Instant;

use bake::walk::{COLLISION_LEVEL, Ground, WalkInput, Walker};

use engine::{Colours, Engine, Frame, Kind, Look, Picture};
use gpu::Easel;
use view::{Vec3, view_basis};
use world::{Make, TILE, World};
use worldgen::generator::materials_of;
use worldgen::light::{Brightness, Shared};

/** The world shown unless --world names another (worlds::WORLDS). */
const WORLD: Make = worlds::valley::valley;

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

struct App {
    kind: Kind,
    engine: Box<dyn Engine>,
    world: World,
    look: Look,
    lighting: Lighting,
    /** How bright the sky, the sun and what glows are, Computed, as the sliders say: the world is lit so once one is let go. */
    brightness: Brightness,
    stats: String,
    times: window::Times,
    /** How long the engine took to make the last frame, ms. */
    framing: f64,
    /** Units a second the camera flies at, as the panel's slider says (walking keeps walk.rs's own). */
    speed: f64,
    /** Walking on the ground (bake's walk.rs): the body the camera is the eye of; None flying. */
    walker: Option<Walker>,
    /** By material id, whether a walking body goes through it (KindDef::walk_through). */
    walk_through: Vec<bool>,
    /** The world, as --view gives it: `seed=…` and `&world=…` if given. */
    world_query: String,
    /** What the ui drew last, for the window to draw (window::Content). */
    picture: Option<(Box<dyn Picture>, egui::Rect)>,
}

impl window::Content for App {
    fn picture(&mut self) -> Option<(Box<dyn Picture>, egui::Rect)> {
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
            ui.heading("Parallax (native)");
            ui.label(&self.stats);
            ui.group(|ui| {
                ui.set_width(ui.available_width());
                ui.strong("Renderer");
                ui.horizontal(|ui| {
                    ui.label("Engine");
                    for (kind, name) in Kind::ALL {
                        if ui.radio_value(&mut self.kind, kind, name).changed() {
                            self.engine = kind.make(&self.world);
                        }
                    }
                });
                // The web's (App.tsx).
                ui.add(egui::Slider::new(&mut self.look.fov, 5.0..=120.0).step_by(1.0).text("Field of View (°)"));
                ui.add(egui::Slider::new(&mut self.look.far, 40.0..=40000.0).logarithmic(true).text("Render Distance"));
                self.engine.panel(ui, &mut self.look);
            });
            ui.group(|ui| {
                ui.set_width(ui.available_width());
                ui.strong("Light");
                ui.horizontal(|ui| {
                    for (lighting, name) in Lighting::ALL {
                        ui.radio_value(&mut self.lighting, lighting, name);
                    }
                });
                if self.lighting == Lighting::Computed {
                    self.brightness_sliders(ui);
                }
            });
            let mut walking = self.walker.is_some();
            if ui.checkbox(&mut walking, "Walk on Ground (F)").changed() {
                self.walker = walking.then(|| Walker::at(self.look.camera));
            }
            ui.add_enabled(
                self.walker.is_none(),
                egui::Slider::new(&mut self.speed, 1.0..=4000.0).logarithmic(true).text("Flying Speed (units/s)"),
            );
            let Look { camera: [x, y, z], yaw, pitch, .. } = self.look;
            ui.label(format!("camera {x:.0} {y:.0} {z:.0}  yaw {yaw:.0}  pitch {pitch:.0}"));
            let t = self.times;
            ui.label(format!(
                "frame {:.1} ms: ui {:.1} (the engine's frame {:.1}), upload {:.1}, waiting for the GPU {:.1}, the rest {:.1}",
                t.since, t.ui, self.framing, t.upload, t.waiting, t.rest
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
            let look = &mut self.look;
            if toggle {
                self.walker = if self.walker.is_some() { None } else { Some(Walker::at(look.camera)) };
            }
            look.yaw += drag.x as f64 * 0.2 + moving[3] * 90.0 * dt;
            look.pitch = (look.pitch - drag.y as f64 * 0.2 + moving[4] * 90.0 * dt).clamp(-90.0, 90.0);
            let [right, _, _] = view_basis(look.yaw, 0.0);
            let level = [-right[2], 0.0, right[0]];
            if let Some(walker) = self.walker {
                let ground = Ground { tiles: &self.world.level(COLLISION_LEVEL as usize).grids, walk_through: &self.walk_through };
                let input =
                    WalkInput { forward: moving[0], right: moving[1], yaw: look.yaw, jump: moving[5] > 0.0, sneak: moving[6] > 0.0 };
                let walker = walker.walk(input, dt, &ground);
                look.camera = walker.eye();
                self.walker = Some(walker);
            } else {
                let step = self.speed * dt;
                for d in 0..3 {
                    look.camera[d] += (level[d] * moving[0] + right[d] * moving[1]) * step;
                }
                look.camera[1] += moving[2] * step;
            }
            look.size = [rect.width() as f64, rect.height() as f64];
            look.light = self.world.brightness().filter(|_| self.lighting == Lighting::Computed);
            let start = Instant::now();
            let frame = self.engine.frame(look, &self.world);
            self.framing = start.elapsed().as_secs_f64() * 1000.0;
            self.stats = frame.stats;
            // Drawn from the next frame on. Walking, the ground's level too, whether drawn or not, a tile around at least.
            let mut wants = frame.wants;
            let ground = &mut wants.reach[COLLISION_LEVEL as usize];
            *ground = ground.or(self.walker.is_some().then_some(TILE as f64));
            let arrived = self.world.update(look.camera, &wants);
            let pixels = ui.ctx().pixels_per_point();
            self.remember([rect.width() * pixels, rect.height() * pixels]);
            self.picture = Some((frame.picture, rect));
            // While moving, the view changes; until all is made, tiles and what engines make of them keep arriving,
            // often enough for a frame now and then. A walking body moves on while it falls or its eye catches up with a step.
            let walking = self.walker.is_some_and(|w| !w.settled());
            if moving.iter().any(|m| *m != 0.0) || walking {
                ui.ctx().request_repaint();
            } else if arrived || frame.busy || self.world.busy() {
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
    /** Lit anew (World::relight) once a slider is let go: each step of a drag would start it over. */
    fn brightness_sliders(&mut self, ui: &mut egui::Ui) {
        let b = &mut self.brightness;
        let mut done = false;
        for (value, name) in [(&mut b.sky, "Sky"), (&mut b.sun, "Sun"), (&mut b.glow, "Glow")] {
            let slider = egui::Slider::new(value, 0.0..=10.0).logarithmic(true).smallest_positive(0.01).text(name);
            let response = ui.add(slider);
            done |= response.drag_stopped() || (response.changed() && !response.dragged());
        }
        if done && self.world.brightness() != Some(self.brightness) {
            self.world.relight(self.brightness);
            self.engine = self.kind.make(&self.world);
        }
    }

    fn remember(&self, size: [f32; 2]) {
        let Look { camera: [x, y, z], yaw, pitch, fov, far, detail, shift, .. } = self.look;
        let frame = format!(
            "--engine {:?} --view '{}&camera={x},{y},{z}&look={yaw},{pitch}&fov={fov}&distance={far:.0}&detail={detail}' --shift {shift} --size {:.0}x{:.0}",
            self.kind, self.world_query, size[0], size[1],
        );
        let b = self.world.brightness().expect("a world with light");
        let light = if self.lighting == Lighting::Computed { format!(" --light {},{},{}", b.sky, b.sun, b.glow) } else { String::new() };
        *LAST_FRAME.lock().unwrap_or_else(|e| e.into_inner()) = format!("{frame}{light}\n{}", self.stats);
    }
}

/**
 * Frames of `look` until all it shows is in: every tile it lacks made,
 * and all the engine makes of them; and, if `whole`, every tile the world
 * would make around it, else it makes no more: they would only take cores
 * from the GPU's drawing. Says when.
 */
fn complete(engine: &mut dyn Engine, world: &mut World, look: &Look, whole: bool) -> Frame {
    let start = Instant::now();
    loop {
        let frame = engine.frame(look, world);
        let shown = frame.wants.tiles.is_empty() && !frame.busy;
        if shown && !whole {
            world.stop();
        } else if world.update(look.camera, &frame.wants) || engine.wait() {
            continue;
        } else if world.busy() {
            world.wait();
            continue;
        }
        if shown {
            println!("the view complete after {:.2} s", start.elapsed().as_secs_f64());
        }
        return frame;
    }
}

/**
 * --bench, --stroll and --shot: the view of `look` once complete (with all
 * the world would make around it, if timed), timed, strolled on from for
 * `seconds`, walking on the ground with `walk_through` if given, and saved.
 */
fn offscreen(
    engine: &mut dyn Engine,
    world: &mut World,
    look: Look,
    bench: bool,
    stroll: Option<(f64, Option<&[bool]>)>,
    shot: Option<&str>,
) {
    let start = Instant::now();
    let frame = complete(engine, world, &look, bench || stroll.is_some());
    let (seconds, threads) = (start.elapsed().as_secs_f64(), world.threads);
    println!("first frame complete in {seconds:.2} s on {threads} threads: {}", frame.stats.replace('\n', ", "));
    let mut easel = Easel::offscreen(None);
    let pixels = look.size.map(|v| v as u32);
    if bench {
        // What a frame costs the main thread once all is made, still and turning.
        for (name, turn) in [("still", 0.0), ("turning", 1.0)] {
            let start = Instant::now();
            for k in 0..20 {
                engine.frame(&Look { yaw: look.yaw + turn * k as f64, ..look }, world);
            }
            println!("a frame {name}: {:.1} ms", start.elapsed().as_secs_f64() * 1000.0 / 20.0);
        }
        let ms = easel.time(&*frame.picture, pixels, 30);
        println!("drawn again and again, the frame takes {ms:.1} ms{}", easel.describe());
    }
    if let Some((seconds, walk_through)) = stroll {
        self::stroll(seconds, walk_through, engine, world, look, &mut easel);
    }
    if let Some(path) = shot {
        easel.save(&*frame.picture, pixels, path);
        println!("saved {path}");
    }
}

/**
 * Moves on along the view for `seconds` at SPEED, flying, or
 * walking on the ground with `walk_through`, a frame every 60th of a
 * second as the window draws it, the GPU a frame behind, and says what
 * the frames cost, and where the slowest were.
 */
fn stroll(seconds: f64, walk_through: Option<&[bool]>, engine: &mut dyn Engine, world: &mut World, mut look: Look, easel: &mut Easel) {
    let pixels = look.size.map(|v| v as u32);
    // All of the first frame is new to this GPU.
    let mut last = easel.draw(&*engine.frame(&look, world).picture, pixels);
    let forward = view_basis(look.yaw, 0.0)[2];
    let mut walker = Walker::at(look.camera);
    let ms = |a: Instant, b: Instant| (b - a).as_secs_f64() * 1000.0;
    let (mut times, mut cameras, mut short) = (Vec::new(), Vec::new(), 0);
    for _ in 0..((seconds * 60.0) as usize).max(1) {
        look.camera = match walk_through {
            Some(walk_through) => {
                let ground = Ground { tiles: &world.level(COLLISION_LEVEL as usize).grids, walk_through };
                let input = WalkInput { forward: 1.0, right: 0.0, yaw: look.yaw, jump: false, sneak: false };
                walker = walker.walk(input, 1.0 / 60.0, &ground);
                walker.eye()
            }
            None => std::array::from_fn(|d| look.camera[d] + forward[d] * SPEED / 60.0),
        };
        cameras.push(look.camera);
        let start = Instant::now();
        let frame = engine.frame(&look, world);
        let framed = Instant::now();
        let mut wants = frame.wants;
        let ground = &mut wants.reach[COLLISION_LEVEL as usize];
        *ground = ground.or(walk_through.is_some().then_some(TILE as f64));
        world.update(look.camera, &wants);
        let updated = Instant::now();
        let next = easel.draw(&*frame.picture, pixels);
        let drawn = Instant::now();
        easel.wait_for(std::mem::replace(&mut last, next));
        let end = Instant::now();
        times.push([ms(start, framed), ms(framed, updated), ms(updated, drawn), ms(drawn, end), ms(start, end)]);
        short += usize::from(frame.busy || !wants.tiles.is_empty());
    }
    println!("{} frames, {short} with what they show still coming; ms at the median, 90th and 99th percentile, and most:", times.len());
    for (k, name) in ["engine", "world", "upload", "GPU", "frame"].iter().enumerate() {
        let mut t: Vec<f64> = times.iter().map(|t| t[k]).collect();
        t.sort_by(f64::total_cmp);
        let at = |q: f64| t[((t.len() - 1) as f64 * q) as usize];
        println!("  {name:>6}: {:6.1} {:6.1} {:6.1} {:6.1}", at(0.5), at(0.9), at(0.99), at(1.0));
    }
    let mut slowest: Vec<usize> = (0..times.len()).collect();
    slowest.sort_by(|&a, &b| times[b][4].total_cmp(&times[a][4]));
    for &k in slowest.iter().take(5) {
        let [x, y, z] = cameras[k];
        let [e, w, u, g, f] = times[k];
        println!("  frame {k}: {f:.1} ms (engine {e:.1}, world {w:.1}, upload {u:.1}, GPU {g:.1}) at camera={x},{y},{z}");
    }
}

/** The view the TS renderer starts with (renderer.ts), at the size --shot draws unless --size. */
const YAW: f64 = 0.0;
const PITCH: f64 = 0.0;
const FOV: f64 = 90.0;
const RENDER_DISTANCE: f64 = 8000.0;
const SHOT: [f64; 2] = [1280.0, 800.0];

/** Units a second the window flies at to begin with, and --stroll flies at. */
const SPEED: f64 = 40.0;

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
        // As the web's WebAssembly takes it.
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
    // Tiles come and go, in blocks of MBs. glibc maps blocks that large apart, to give back once freed, but raises that
    // threshold past each it frees: later ones would stay in its heap.
    // SAFETY: mallopt only sets a parameter, before any thread starts.
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    unsafe {
        libc::mallopt(libc::M_MMAP_THRESHOLD, 128 << 10);
    }
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
    // Sky, sun and lamps, as in look's `light`: 1,1,0 a clear day, 0.01,0,1 night.
    let light = value("--light").map(|v| {
        let v: Vec<f32> = v.split(',').map(|v| v.parse().expect("--light SKY,SUN,GLOW")).collect();
        let [sky, sun, glow] = v.try_into().expect("--light SKY,SUN,GLOW");
        Brightness { sky, sun, glow }
    });
    let brightness = light.unwrap_or(Brightness { sky: 1.0, sun: 1.0, glow: 0.0 });
    let mut world = World::new(make, seed, Some(Shared::new(brightness)), threads);
    let shown = make(seed);
    let camera = query.camera.unwrap_or(shown.spawn.map(|v| v as f64));
    let far = query.distance.unwrap_or(RENDER_DISTANCE);
    let mut look = Look::new(camera, query.look.unwrap_or([YAW, PITCH]), query.fov.unwrap_or(FOV), far, size);
    look.light = light;
    if let Some(v) = value("--detail").map(|v| v.parse().expect("--detail D")).or(query.detail) {
        look.detail = v;
    }
    if let Some(v) = value("--shift") {
        look.shift = v.parse().expect("--shift S");
    }
    look.seams = flag("--seams");
    if let Some(v) = value("--colour") {
        look.colours =
            Colours::ALL.into_iter().find(|(_, name)| name.eq_ignore_ascii_case(v)).expect("--colour voxels, layers or chunks").0;
    }
    let kind = value("--engine").map_or(Kind::Voxels, |v| {
        Kind::ALL.into_iter().find(|(_, name)| name.eq_ignore_ascii_case(v)).expect("--engine reliefs, flat, splats or voxels").0
    });
    let mut engine = kind.make(&world);
    let walk_through: Vec<bool> = materials_of(&shown.generator).iter().map(|m| m.walk_through).collect();
    if ["--bench", "--shot", "--stroll"].into_iter().any(flag) {
        let stroll = value("--stroll").map(|v| (v.parse().expect("seconds"), flag("--on-ground").then_some(&walk_through[..])));
        offscreen(&mut *engine, &mut world, look, flag("--bench"), stroll, value("--shot"));
        return;
    }
    let app = App {
        kind,
        engine,
        world,
        look,
        lighting: if light.is_some() { Lighting::Computed } else { Lighting::Shaded },
        brightness,
        stats: String::new(),
        times: window::Times::default(),
        framing: 0.0,
        speed: SPEED,
        walker: None,
        walk_through,
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

#[cfg(test)]
mod tests {
    use super::*;

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
        let slab = Field::at("slab", 0.0, Op::Fill, |x, y, z, _| match x + z {
            _ if y.abs() >= 48 => None,
            k if (NEAR..FAR).contains(&k) && MILLI > 0 => Some((MEDIUM, Some(f64::from(MILLI) / 1000.0))),
            k if (FAR..FAR + 16).contains(&k) => Some((WALL, None)),
            _ => None,
        });
        let generator = Generator { kinds, seed: 1, layers: vec![Layer::Field(slab)] };
        worldgen::World { generator, spawn: [0, 0, 0], heights: (-64, 64), points: Vec::new() }
    }

    /** What `kind` draws of the world `make` makes, from its spawn, looking along the face (1, 0, 1), without fog: its width and pixels. */
    fn along_a_face(kind: Kind, make: Make) -> (u32, Vec<u8>) {
        let mut world = World::new(make, 1, None, 4);
        let spawn = make(1).spawn.map(|v| v as f64);
        let look = Look { fog: f64::INFINITY, detail: 8.0, ..Look::new(spawn, [45.0, 0.0], FOV, 1000.0, [320.0, 200.0]) };
        let mut engine = kind.make(&world);
        let frame = complete(&mut *engine, &mut world, &look, false);
        let (width, _, pixels) = Easel::offscreen(None).pixels(&*frame.picture, [320, 200]);
        (width, pixels)
    }

    /** Red ground under y = 0, endless. */
    fn ground(_: u64) -> worldgen::World {
        use worldgen::generator::{Field, Generator, KindDef, Layer, Op};
        let kinds = vec![KindDef { name: "ground", kind_type: 10, colors: &["#ff0000"], walk_through: false, glows: false, medium: false }];
        let ground = Field::at("ground", 0.0, Op::Fill, |_, y, _, _| (y < 0).then_some((0, None)));
        let generator = Generator { kinds, seed: 1, layers: vec![Layer::Field(ground)] };
        worldgen::World { generator, spawn: [0, 16, 0], heights: (-64, 64), points: Vec::new() }
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
            let mut world = World::new(ground, 1, None, 4);
            let look = Look { fog: f64::INFINITY, ..Look::new([0.0, 16.0, 0.0], [45.0, -10.0], FOV, 1000.0, PIXELS.map(f64::from)) };
            let frame = complete(&mut *kind.make(&world), &mut world, &look, false);
            let (width, _, pixels) = Easel::offscreen(None).pixels(&*frame.picture, PIXELS);
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

    #[test]
    #[ignore = "draws on the GPU: needs Vulkan, as Mesa's software one"]
    fn a_medium_of_splats_lets_through_as_much_as_its_extinction_says() {
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
        let slab = Field::at("slabs", 0.0, Op::Fill, move |x, y, z, _| match x + z {
            _ if y.abs() >= 48 => None,
            k if (NEAR..FAR).contains(&k) => Some((u8::from((k < middle) != RED_NEAR), Some(0.04))),
            k if (FAR..FAR + 16).contains(&k) => Some((2, None)),
            _ => None,
        });
        let generator = Generator { kinds, seed: 1, layers: vec![Layer::Field(slab)] };
        worldgen::World { generator, spawn: [0, 0, 0], heights: (-64, 64), points: Vec::new() }
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
        worldgen::World { generator, spawn: [0, -400, 0], heights: (-64, 512), points: Vec::new() }
    }

    #[test]
    #[ignore = "draws on the GPU: needs Vulkan, as Mesa's software one"]
    fn voxels_show_the_faces_rays_enter_by() {
        // From below, near and far: rays enter its voxels by faces that look down or sideways, white, never by
        // those that look up, red. Most of its coarse voxels have both, and their sites' own material is either.
        // Lit, near: only the light differs.
        let lit = Some(Brightness { sky: 0.0, sun: 0.0, glow: 1.0 });
        for (light, distance) in [(None, 400.0), (None, 1500.0), (None, 2500.0), (lit, 400.0)] {
            let mut world = World::new(sheeted_block, 1, light.map(Shared::new), 4);
            let at = Look::new([0.0, -distance, 0.0], [0.0, 80.0], FOV, 8000.0, [320.0, 200.0]);
            let look = Look { fog: f64::INFINITY, light, ..at };
            let frame = complete(&mut *Kind::Voxels.make(&world), &mut world, &look, false);
            let pixels = Easel::offscreen(None).pixels(&*frame.picture, [320, 200]).2;
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
        let field = Field::at("specks", 0.0, Op::Fill, move |x, y, z, _| {
            let [x, y, z] = [x - by[0], y - by[1], z - by[2]];
            let h = worldgen::noise::hash(1, x, y, z);
            let ground = 10 * y < 3 * x + 2 * z - 300;
            (ground || h.is_multiple_of(20000)).then_some(((h % 4) as u8, None))
        });
        let generator = Generator { kinds, seed: 1, layers: vec![Layer::Field(field)] };
        worldgen::World { generator, spawn: by, heights: (-200, 200), points: Vec::new() }
    }

    /** What the voxels engine draws of `make`'s world from `eye` past its spawn, within level 0's reach: its pixels. */
    fn specks_from(make: Make, eye: Vec3, angles: [f64; 2]) -> Vec<u8> {
        let mut world = World::new(make, 1, None, 4);
        let spawn = make(1).spawn.map(|v| v as f64);
        let camera = std::array::from_fn(|i| spawn[i] + eye[i]);
        let look = Look { fog: f64::INFINITY, ..Look::new(camera, angles, FOV, 200.0, [320.0, 200.0]) };
        let mut engine = Kind::Voxels.make(&world);
        let frame = complete(&mut *engine, &mut world, &look, false);
        Easel::offscreen(None).pixels(&*frame.picture, [320, 200]).2
    }

    #[test]
    #[ignore = "draws on the GPU: needs Vulkan, as Mesa's software one"]
    fn no_ray_of_the_voxels_runs_out_of_steps() {
        // As a shot looks, from the spawn, where rays once stuck crossing cells.
        for make in [WORLD, worlds::by_name("forest").expect("forest").make] {
            let mut world = World::new(make, 1, None, 4);
            let spawn = make(1).spawn.map(|v| v as f64);
            let look = Look::new(spawn, [YAW, PITCH], FOV, RENDER_DISTANCE, SHOT);
            let mut engine = Kind::Voxels.make(&world);
            let frame = complete(&mut *engine, &mut world, &look, false);
            let pixels = Easel::offscreen(None).pixels(&*frame.picture, SHOT.map(|v| v as u32)).2;
            // Lost rays show magenta (voxels.wgsl's LOST).
            let lost = pixels.chunks(4).filter(|p| p[..3] == [255, 0, 255]).count();
            assert_eq!(lost, 0, "pixels of rays that ran out of steps");
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
}
