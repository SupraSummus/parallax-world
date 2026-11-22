//! The native frontend: a world of worlds/ generated in tiles, those in
//! view first (world.rs), drawn by an engine (engine.rs) with wgpu, in an
//! egui window (window.rs) or headless (gpu.rs).
//!
//! Its options (Args, --help) set what is shown (View), which gives them
//! back for --record and a panic's report; in a browser, the page's query
//! gives them, each `key=value` as `--key value`. A pilot steers the
//! camera a frame at a time till it ends (App::end); then the keys steer,
//! or, headless, it ends too.

mod engine;
mod eye;
mod gpu;
mod inspect;
mod threads;
mod view;
mod window;
mod world;

use std::io::Write;
use std::sync::Arc;
use web_time::Instant;

use clap::{Parser, ValueEnum};
use fcc::walk::{COLLISION_LEVEL, Ground, WalkInput, Walker};

use engine::voxels::{Marcher, Probe};
use engine::{Colours, Engine, Frame, Kind, Look, Picture};
use eye::Eye;
use gpu::Easel;
use light::{Brightness, Shared};
use view::{Vec3, view_basis};
use window::Times;
use world::{Piece, TILE, Wants, World};

/** Shows a world as an engine draws it, in a window or headless. The panel changes much the same as these options; should the window panic or lose its GPU device, it says what it showed last as them. */
#[derive(Parser, Clone)]
#[command(name = "parallax-native", args_override_self = true, max_term_width = 100)]
#[command(group(clap::ArgGroup::new("pilot").args(["stroll", "replay"])))]
struct Args {
    /** The world (worlds::all). */
    #[arg(long, default_value = WORLD, value_parser = clap::builder::PossibleValuesParser::new(worlds::all().map(|w| w.name)))]
    world: String,
    /** Its seed. */
    #[arg(long, default_value_t = 12345)]
    seed: u64,
    /** Threads the world is made on [default: one a core]. */
    #[arg(long, value_name = "N", value_parser = clap::builder::RangedU64ValueParser::<usize>::new().range(1..))]
    threads: Option<usize>,

    /** Where the camera is, in lattice units [default: the world's spawn]. */
    #[arg(long, value_name = "X,Y,Z", value_parser = numbers::<3>, allow_hyphen_values = true, help_heading = "View")]
    camera: Option<Vec3>,
    /** Where it looks, in degrees: yaw, and pitch up. */
    #[arg(long, value_name = "YAW,PITCH", value_parser = numbers::<2>, default_value = "0,0", allow_hyphen_values = true, help_heading = "View")]
    look: [f64; 2],
    /** The field of view, in degrees. */
    #[arg(long, value_name = "DEGREES", default_value_t = FOV, help_heading = "View")]
    fov: f64,
    /** The render distance, in lattice units. */
    #[arg(long, value_name = "UNITS", default_value_t = RENDER_DISTANCE, help_heading = "View")]
    distance: f64,
    /** Light worked out across the world (light/): how bright the sky, the sun and what glows are (1,1,0 a clear day, 0.01,0,1 night); the steps light takes along each level before the coarser one takes over; how little of the light on a site is not followed (light's TOLERANCE). Without it, voxels are shaded by the way their faces face. */
    #[arg(long, value_name = "SKY,SUN,GLOW[,STEPS[,TOLERANCE]]", value_parser = parse_light, help_heading = "View")]
    light: Option<Light>,
    /** How that light shows, in the voxels (eye.rs): stops of exposure added, and how far the eye adapts to what it sees, 0 not at all, 1 a dark place as bright as day. */
    #[arg(long, value_name = "EXPOSURE,ADAPTING", value_parser = parse_eye, allow_hyphen_values = true, help_heading = "View")]
    eye: Option<Eye>,

    /** What draws. */
    #[arg(long, value_enum, default_value_t = Kind::Voxels, help_heading = "Engine")]
    engine: Kind,
    /** Viewing distance over a cell's thickness, for the engines that draw cells. */
    #[arg(long, value_name = "D", help_heading = "Engine")]
    detail: Option<f64>,
    /** How many levels finer than a cell its voxels are (bake's lod.rs), for the engines that draw cells. */
    #[arg(long, value_name = "S", allow_hyphen_values = true, help_heading = "Engine")]
    shift: Option<i32>,
    /** What chunks are painted in. */
    #[arg(long, value_enum, default_value_t = Colours::Voxels, help_heading = "Engine")]
    colour: Colours,
    /** Draws the chunks' seams. */
    #[arg(long, help_heading = "Engine")]
    seams: bool,
    /** Bakes the cells that what is in front hides too. */
    #[arg(long, help_heading = "Engine")]
    unhidden: bool,

    /** Walks on the ground (fcc's walk.rs), as F does, rather than flying. */
    #[arg(long, help_heading = "Steering")]
    walk: bool,
    /** How fast it flies to begin with, in units a second, as the panel's slider. */
    #[arg(long, value_name = "UNITS", default_value_t = SPEED, help_heading = "Steering")]
    speed: f64,
    /** Steers for SECONDS, from the first frame that shows all it is to, as keys held would, a frame every 60th of a second: forward, or --heading degrees off it, turning --turn degrees a second. */
    #[arg(long, value_name = "SECONDS", help_heading = "Steering")]
    stroll: Option<f64>,
    /** Which way a stroll goes, in degrees off the way the camera looks (90 to the right). */
    #[arg(long, value_name = "DEGREES", default_value_t = 0.0, requires = "stroll", allow_hyphen_values = true, help_heading = "Steering")]
    heading: f64,
    /** How fast a stroll turns, in degrees a second (to the right). */
    #[arg(long, value_name = "DEGREES", default_value_t = 0.0, requires = "stroll", allow_hyphen_values = true, help_heading = "Steering")]
    turn: f64,
    /** Steers as a --record's frames say, each at its time; its world, seed and threads unless given. */
    #[arg(long, value_name = "FILE", help_heading = "Steering")]
    replay: Option<String>,
    /** Gives the world's threads each frame's time on their cores before the next, as if drawing took none: a software GPU takes most. */
    #[arg(long, help_heading = "Steering")]
    by_work: bool,

    /** Draws without a window, and ends once the steering does. */
    #[arg(long, help_heading = "Output")]
    headless: bool,
    /** The picture's size headless, in pixels, or the window's, in points [default: 1280x800 headless, 1580x800 a window]. */
    #[arg(long, value_name = "WxH", value_parser = parse_size, help_heading = "Output")]
    size: Option<[f64; 2]>,
    /** Once the steering ends, saves the view complete as a PNG. */
    #[arg(long, value_name = "FILE", help_heading = "Output")]
    shot: Option<String>,
    /** Writes each frame to FILE as the options that show it, for --replay. */
    #[arg(long, value_name = "FILE", help_heading = "Output")]
    record: Option<String>,
    /** While a stroll or a replay steers, keeps pictures of frames whose view was still coming; once it ends, saves each into DIR, with that view complete beside it. */
    #[arg(long, value_name = "DIR", requires = "pilot", help_heading = "Output")]
    shots: Option<String>,
}

/** N numbers, comma-separated. */
fn numbers<const N: usize>(v: &str) -> Result<[f64; N], String> {
    let v: Vec<f64> = v.split(',').map(|v| v.trim().parse::<f64>().map_err(|e| format!("{v:?}: {e}"))).collect::<Result<_, _>>()?;
    v.try_into().map_err(|v: Vec<f64>| format!("{N} numbers, not {}", v.len()))
}

/** --size's WxH. */
fn parse_size(v: &str) -> Result<[f64; 2], String> {
    numbers::<2>(&v.replace('x', ","))
}

/** --light: how bright, and if given, the steps and the tolerance. */
#[derive(Clone, Copy)]
struct Light {
    brightness: Brightness,
    steps: Option<usize>,
    tolerance: Option<f32>,
}

fn parse_light(v: &str) -> Result<Light, String> {
    let v: Vec<f32> = v.split(',').map(|v| v.trim().parse::<f32>().map_err(|e| format!("{v:?}: {e}"))).collect::<Result<_, _>>()?;
    let [sky, sun, glow, ..] = v[..] else { return Err("at least 3 numbers".into()) };
    if v.len() > 5 {
        return Err("at most 5 numbers".into());
    }
    Ok(Light { brightness: Brightness { sky, sun, glow }, steps: v.get(3).map(|&s| s as usize), tolerance: v.get(4).copied() })
}

impl Light {
    /**
     * `shared` lit so, what it leaves out as `shared` has it; None if it
     * lights so already, as most frames find: a light made anew copies
     * all the world made.
     */
    fn on(self, shared: &Shared) -> Option<Shared> {
        let (steps, tolerance) = (self.steps.unwrap_or(shared.steps()), self.tolerance.unwrap_or(shared.tolerance()));
        let same = (self.brightness, steps, tolerance) == (shared.brightness(), shared.steps(), shared.tolerance());
        (!same).then(|| shared.under(self.brightness).stepping(steps).settling(tolerance))
    }
}

fn parse_eye(v: &str) -> Result<Eye, String> {
    let [exposure, adapting] = numbers::<2>(v)?.map(|v| v as f32);
    Ok(Eye { exposure, adapting })
}

/** How `shared` lights a world, as --light takes it. */
fn light_arg(shared: &Shared) -> String {
    let b = shared.brightness();
    format!("{},{},{},{},{}", b.sky, b.sun, b.glow, shared.steps(), shared.tolerance())
}

/** `value` as an option names it. */
fn named(value: impl ValueEnum) -> String {
    value.to_possible_value().expect("named").get_name().to_string()
}

/** The options of `argv`; those it gets wrong said, and the program ended (in a browser, panicking). */
fn parse(argv: &[String]) -> Args {
    Args::try_parse_from(argv).unwrap_or_else(|e| {
        // A browser has no process to end.
        if cfg!(target_arch = "wasm32") {
            panic!("{e}")
        }
        e.exit()
    })
}

/** The world unless --world names another. */
const WORLD: &str = "valley";

/**
 * How voxels are lit, whatever draws them: shaded by the way their faces
 * face, or by light worked out across the world (light's Light), the
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
 * Keys held, each 0 or 1 (a stroll's, between): forward, right, up,
 * turning right, looking up, jumping, sneaking; how far the view was
 * dragged, in points; and whether walking was toggled (F).
 */
#[derive(Clone, Copy, Default)]
struct Keys {
    held: [f64; 7],
    drag: [f64; 2],
    toggle: bool,
}

/** What is shown, as the options say and the panel changes: the world, what draws it and how, the camera and how it moves. */
struct View {
    kind: Kind,
    engine: Box<dyn Engine>,
    world: World,
    /** The world, as the options that make it. */
    world_args: String,
    look: Look,
    lighting: Lighting,
    /** How bright the sky, the sun and what glows are, Computed, the steps light takes along each level and how little it may change to be found, as the sliders say: the world is lit so once one is let go. */
    brightness: Brightness,
    steps: usize,
    tolerance: f32,
    /** Units a second the camera flies at (walking keeps walk.rs's own). */
    speed: f64,
    /** Walking on the ground (fcc's walk.rs): the body the camera is the eye of; None flying. */
    walker: Option<Walker>,
    /** Whether the window tells of the voxel under the pointer (inspect.rs), as the voxels engine finds it. */
    inspecting: bool,
}

impl View {
    fn new(args: &Args) -> View {
        let made = (worlds::by_name(&args.world).expect("a world").make)(args.seed);
        let spawn = made.spawn.map(|v| v as f64);
        // Sky, sun and lamps, as in look's `light`: 1,1,0 a clear day.
        let day = Shared::new(Brightness { sky: 1.0, sun: 1.0, glow: 0.0 });
        let shared = args.light.and_then(|light| light.on(&day)).unwrap_or(day.clone());
        let world = World::new(made, Some(shared.clone()), args.threads.unwrap_or_else(threads::cores));
        let mut view = View {
            kind: args.engine,
            engine: args.engine.make(&world),
            world,
            world_args: format!("--world {} --seed {}", args.world, args.seed),
            look: Look::new(spawn, args.look, args.fov, args.distance, SHOT),
            lighting: Lighting::Shaded,
            brightness: shared.brightness(),
            steps: shared.steps(),
            tolerance: shared.tolerance(),
            speed: args.speed,
            walker: None,
            inspecting: false,
        };
        view.apply(args);
        view
    }

    /** Takes what `args` say of it, but the world and the speed: the engine, the light, the look, walking. */
    fn apply(&mut self, args: &Args) {
        let shared = self.world.shared().expect("a world with light");
        if let Some(lit) = args.light.and_then(|light| light.on(shared)) {
            self.world.relight(lit);
            self.engine = args.engine.make(&self.world);
        } else if args.engine != self.kind {
            self.engine = args.engine.make(&self.world);
        }
        self.kind = args.engine;
        let shared = self.world.shared().expect("a world with light");
        (self.brightness, self.steps, self.tolerance) = (shared.brightness(), shared.steps(), shared.tolerance());
        self.lighting = if args.light.is_some() { Lighting::Computed } else { Lighting::Shaded };
        self.light_look();
        let look = &mut self.look;
        if let Some(camera) = args.camera {
            look.camera = camera;
        }
        ([look.yaw, look.pitch], look.fov, look.far) = (args.look, args.fov, args.distance);
        if let Some(detail) = args.detail {
            look.detail = detail;
        }
        if let Some(shift) = args.shift {
            look.shift = shift;
        }
        (look.colours, look.seams, look.hides) = (args.colour, args.seams, !args.unhidden);
        look.eye = args.eye.unwrap_or_default();
        look.size = args.size.unwrap_or(SHOT);
        self.walker = args.walk.then(|| Walker::at(look.camera));
    }

    /** Its look lit as the lighting says, by the world's light: a shot with nothing to steer is drawn with no frame before. */
    fn light_look(&mut self) {
        self.look.light = self.world.shared().map(Shared::brightness).filter(|_| self.lighting == Lighting::Computed);
    }

    /** What `apply` takes, as the options that say it. */
    fn options(&self) -> String {
        let Look { camera: [x, y, z], yaw, pitch, fov, far, detail, shift, seams, colours, hides, size: [w, h], eye, .. } = self.look;
        let mut options = format!(
            "--engine {} --camera {x},{y},{z} --look {yaw},{pitch} --fov {fov} --distance {far} --detail {detail} --shift {shift} --colour {} --size {w}x{h}",
            named(self.kind),
            named(colours),
        );
        if self.lighting == Lighting::Computed {
            options += &format!(" --light {}", light_arg(self.world.shared().expect("a world with light")));
            let Eye { exposure, adapting } = eye;
            options += &format!(" --eye {exposure},{adapting}");
        }
        for (set, flag) in [(seams, " --seams"), (!hides, " --unhidden"), (self.walker.is_some(), " --walk")] {
            if set {
                options += flag;
            }
        }
        options
    }

    /** Moves the camera as `keys` say, for `dt` seconds: walking on the ground, or flying. */
    fn steer(&mut self, keys: Keys, dt: f64) {
        let Keys { held, drag, toggle } = keys;
        let look = &mut self.look;
        if toggle {
            self.walker = if self.walker.is_some() { None } else { Some(Walker::at(look.camera)) };
        }
        look.yaw += drag[0] * 0.2 + held[3] * TURN * dt;
        look.pitch = (look.pitch - drag[1] * 0.2 + held[4] * TURN * dt).clamp(-90.0, 90.0);
        let [right, _, _] = view_basis(look.yaw, 0.0);
        let level = [-right[2], 0.0, right[0]];
        if let Some(walker) = self.walker {
            let ground = Ground { tiles: &self.world.level(COLLISION_LEVEL as usize).grids };
            let input = WalkInput { forward: held[0], right: held[1], yaw: look.yaw, jump: held[5] > 0.0, sneak: held[6] > 0.0 };
            let walker = walker.walk(input, dt, &ground);
            look.camera = walker.eye();
            self.walker = Some(walker);
        } else {
            let step = self.speed * dt;
            for d in 0..3 {
                look.camera[d] += (level[d] * held[0] + right[d] * held[1]) * step;
            }
            look.camera[1] += held[2] * step;
        }
    }

    /** The panel's part that sets it. */
    fn panel(&mut self, ui: &mut egui::Ui) {
        ui.group(|ui| {
            ui.set_width(ui.available_width());
            ui.strong("Renderer");
            ui.horizontal(|ui| {
                ui.label("Engine");
                for (kind, name) in Kind::offered() {
                    if ui.radio_value(&mut self.kind, kind, name).changed() {
                        self.engine = kind.make(&self.world);
                    }
                }
            });
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
                self.light_sliders(ui);
                if self.kind == Kind::Voxels {
                    self.look.eye.panel(ui);
                }
            }
        });
        if self.kind == Kind::Voxels {
            ui.checkbox(&mut self.inspecting, "The Voxel under the Pointer (I)");
        }
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
    }

    /** Lit anew (World::relight) once a slider is let go: each step of a drag would start it over. */
    fn light_sliders(&mut self, ui: &mut egui::Ui) {
        let b = &mut self.brightness;
        let mut responses = Vec::new();
        for (value, name) in [(&mut b.sky, "Sky"), (&mut b.sun, "Sun"), (&mut b.glow, "Glow")] {
            responses.push(ui.add(egui::Slider::new(value, 0.0..=10.0).logarithmic(true).smallest_positive(0.01).text(name)));
        }
        // More steps a level, the thicker a wall must be to let light through: about its distance over them.
        // Few: past three, each direction reads too little of what it stands for, and lighting takes many times longer (docs/light.md).
        responses.push(ui.add(egui::Slider::new(&mut self.steps, 1..=3).text("Steps a Level")));
        // Less, the more rounds of bounces where light bounces back and forth.
        responses.push(ui.add(egui::Slider::new(&mut self.tolerance, 1e-4..=0.5).logarithmic(true).text("Tolerance")));
        let done = responses.iter().any(|r| r.drag_stopped() || (r.changed() && !r.dragged()));
        let shared = self.world.shared().expect("a world with light");
        let light = Light { brightness: self.brightness, steps: Some(self.steps), tolerance: Some(self.tolerance) };
        if done && let Some(relit) = light.on(shared) {
            self.world.relight(relit);
            self.engine = self.kind.make(&self.world);
        }
    }
}

/** What steers the camera till it ends (App::end); the keys after. */
enum Pilot {
    /** Nothing: it ends at once. */
    Still,
    /** --stroll: keys held for so many frames more, a 60th of a second each, from a frame that showed all it was to. */
    Stroll {
        held: Keys,
        left: usize,
        started: bool,
    },
    Replay(Replay),
}

impl Pilot {
    fn ended(&self) -> bool {
        match self {
            Pilot::Still => true,
            Pilot::Stroll { left, .. } => *left == 0,
            Pilot::Replay(replay) => replay.next == replay.frames.len(),
        }
    }
}

/**
 * --replay: the frames of a --record, each the time it was drawn at, since
 * the first, and the options that show it; the next; and when the first
 * was drawn here.
 */
struct Replay {
    frames: Vec<(f64, Args)>,
    next: usize,
    start: Option<Instant>,
}

impl Replay {
    fn read(text: &str) -> Replay {
        let mut frames: Vec<(f64, Args)> = text
            .lines()
            .filter(|l| !l.starts_with('#'))
            .map(|line| {
                let [t, options, _stats] = line.split('\t').collect::<Vec<_>>()[..] else { panic!("a recording's line: {line}") };
                let argv: Vec<String> = std::iter::once("").chain(options.split_whitespace()).map(str::to_string).collect();
                (t.parse().expect("its time"), Args::try_parse_from(argv).unwrap_or_else(|e| panic!("a recording's line: {e}")))
            })
            .collect();
        let first = frames.first().map_or(0.0, |f| f.0);
        frames.iter_mut().for_each(|f| f.0 -= first);
        Replay { frames, next: 0, start: None }
    }

    /** The next frame, once its time has come. */
    fn next(&mut self) -> (f64, Args) {
        let start = *self.start.get_or_insert_with(Instant::now);
        let (t, args) = self.frames[self.next].clone();
        self.next += 1;
        if let Some(wait) = std::time::Duration::from_secs_f64(t).checked_sub(start.elapsed()) {
            std::thread::sleep(wait);
        }
        (t, args)
    }
}

/**
 * A frame a stroll or a replay steered: ms in the engine, the world
 * (World::update), uploading, waiting for the GPU, and in all; when, in
 * seconds steered; where the camera was; what it asked for, and the world
 * owed it (World::owed); and whether what it showed was still coming.
 */
struct Sample {
    ms: [f64; 5],
    t: f64,
    camera: Vec3,
    owed: [usize; 3],
    still: bool,
}

/** --shots: where to; how far apart, in seconds steered, pictures are kept; and those kept, with when and their look. */
struct Shots {
    dir: String,
    every: f64,
    kept: Vec<(f64, Look, Box<dyn Picture>)>,
}

/** Pictures --shots keeps at most: as far apart as the steering is long for them. */
const KEPT: usize = 60;

/** A view steered, a frame at a time, and what the options ask of its frames. */
struct App {
    view: View,
    pilot: Option<Pilot>,
    /** Seconds a stroll or a replay steered. */
    steered: f64,
    /** Whether the last frame showed all it was to: a stroll starts from one. */
    shown: bool,
    stats: String,
    times: Times,
    /** How long the engine took to make the last frame, ms. */
    framing: f64,
    /** What the last frame drew, till drawn, and where the window shows it. */
    picture: Option<Box<dyn Picture>>,
    rect: egui::Rect,
    /** What the voxels engine last found the pointer's ray meets, and that pointer. */
    probed: Option<([f64; 2], Probe)>,
    /** The frames a stroll or a replay steered, the last till drawn. */
    samples: Vec<Sample>,
    sample: Option<Sample>,
    by_work: bool,
    headless: bool,
    shot: Option<String>,
    shots: Option<Shots>,
    /** --record: where each frame is written, and since when. */
    recording: Option<(std::io::BufWriter<std::fs::File>, Instant)>,
}

impl window::Content for App {
    fn picture(&self) -> Option<(&dyn Picture, egui::Rect)> {
        self.picture.as_deref().map(|p| (p, self.rect))
    }

    fn describe(&self) -> String {
        last_frame()
    }

    fn drawn(&mut self, times: Times, easel: &mut Easel) {
        App::drawn(self, times);
        self.probed = easel.held::<Marcher>().and_then(|m| m.probed);
        if self.ended() {
            self.end(easel);
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui) {
        egui::Panel::left("panel").show(ui, |ui| {
            ui.heading("Parallax");
            ui.label(&self.stats);
            ui.label(format!("{} tiles being made", self.view.world.making()));
            self.view.panel(ui);
            let t = self.times;
            ui.label(format!(
                "frame {:.1} ms: ui {:.1} (the engine's frame {:.1}), upload {:.1}, waiting for the GPU {:.1}, the rest {:.1}",
                t.since, t.ui, self.framing, t.upload, t.waiting, t.rest
            ));
            ui.label("WASD, Space/Shift, arrows or drag\nWalking: Space jumps, Shift sneaks\nI: the voxel under the pointer");
        });
        egui::CentralPanel::default().frame(egui::Frame::NONE).show(ui, |ui| {
            let (rect, response) = ui.allocate_exact_size(ui.available_size(), egui::Sense::drag());
            let drag = response.drag_delta();
            let (dt, keys) = ui.input(|i| {
                let key = |k| if i.key_down(k) { 1.0 } else { 0.0 };
                let forward = key(egui::Key::W) - key(egui::Key::S);
                let right = key(egui::Key::D) - key(egui::Key::A);
                let (jump, sneak) = (key(egui::Key::Space), if i.modifiers.shift { 1.0 } else { 0.0 });
                let turn = key(egui::Key::ArrowRight) - key(egui::Key::ArrowLeft);
                let look = key(egui::Key::ArrowUp) - key(egui::Key::ArrowDown);
                let held = [forward, right, jump - sneak, turn, look, jump, sneak];
                let drag = [drag.x, drag.y].map(f64::from);
                (i.stable_dt.min(0.1) as f64, Keys { held, drag, toggle: i.key_pressed(egui::Key::F) })
            });
            if ui.input(|i| i.key_pressed(egui::Key::I)) {
                self.view.inspecting = !self.view.inspecting;
            }
            // Where the pointer is on the picture, as a share of its sides, while what its ray meets is told of.
            let telling = self.view.inspecting && self.view.kind == Kind::Voxels && !response.dragged();
            let pointer = response.hover_pos().filter(|_| telling).map(|p| {
                let at = (p - rect.min) / rect.size();
                [at.x, at.y].map(f64::from)
            });
            self.view.look.pointer = pointer;
            let more = self.frame(keys, dt, Some([rect.width(), rect.height()].map(f64::from)));
            self.rect = rect;
            // Read back a frame or two late: frames come till it is of where the pointer is.
            let probed = self.probed.filter(|_| pointer.is_some());
            let waiting = pointer.is_some() && probed.map(|p| p.0) != pointer;
            if let Some((_, probe)) = probed {
                let lines = inspect::lines(&probe, &self.view.world, self.view.look.light.is_some());
                egui::Tooltip::for_widget(&response).at_pointer().gap(12.0).show(|ui| inspect::show(ui, &lines));
            }
            // While moving, the view changes; until all is made, tiles and what engines make of them keep arriving,
            // often enough for a frame now and then. A walking body moves on while it falls or its eye catches up with a step.
            let walking = self.view.walker.is_some_and(|w| !w.settled());
            if keys.held.iter().any(|m| *m != 0.0) || walking || waiting || self.pilot.is_some() {
                ui.ctx().request_repaint();
            } else if more {
                ui.ctx().request_repaint_after(std::time::Duration::from_millis(16));
            }
        });
    }
}

impl App {
    /** As `args` say; steered by a replay of `recording`, if given. */
    fn new(args: &Args, recording: Option<&str>) -> App {
        let view = View::new(args);
        let pilot = match (args.stroll, recording) {
            (Some(seconds), _) => {
                let (right, forward) = args.heading.to_radians().sin_cos();
                let held = Keys { held: [forward, right, 0.0, args.turn / TURN, 0.0, 0.0, 0.0], ..Keys::default() };
                Pilot::Stroll { held, left: ((seconds / FRAME) as usize).max(1), started: false }
            }
            (None, Some(text)) => Pilot::Replay(Replay::read(text)),
            (None, None) => Pilot::Still,
        };
        let length = match &pilot {
            Pilot::Still => 0.0,
            Pilot::Stroll { left, .. } => *left as f64 * FRAME,
            Pilot::Replay(replay) => replay.frames.last().map_or(0.0, |f| f.0),
        };
        // Its first line the world and threads, which --replay takes.
        let recording = args.record.as_ref().map(|path| {
            let mut file = std::io::BufWriter::new(std::fs::File::create(path).expect("--record file"));
            writeln!(file, "# {} --threads {}", view.world_args, view.world.threads).expect("--record file");
            (file, Instant::now())
        });
        App {
            view,
            pilot: Some(pilot),
            steered: 0.0,
            shown: false,
            stats: String::new(),
            times: Times::default(),
            framing: 0.0,
            picture: None,
            rect: egui::Rect::NOTHING,
            probed: None,
            samples: Vec::new(),
            sample: None,
            by_work: args.by_work,
            headless: args.headless,
            shot: args.shot.clone(),
            shots: args.shots.clone().map(|dir| Shots { dir, every: length / KEPT as f64, kept: Vec::new() }),
            recording,
        }
    }

    /**
     * A frame: the camera steered as the pilot says (as `keys` say, for
     * `dt` seconds, if none does), at `size` if given, and the engine's
     * picture of its view kept to be drawn; the world asked for what that
     * wants. Whether more of it is to come, still: tiles, or what the
     * engine makes of them.
     */
    fn frame(&mut self, keys: Keys, dt: f64, size: Option<[f64; 2]>) -> bool {
        let view = &mut self.view;
        let steered = match &mut self.pilot {
            Some(Pilot::Stroll { held, left, started }) if *started || self.shown => {
                (*started, *left) = (true, *left - 1);
                self.steered += FRAME;
                view.steer(*held, FRAME);
                true
            }
            Some(Pilot::Replay(replay)) => {
                let (t, args) = replay.next();
                self.steered = t;
                view.apply(&args);
                true
            }
            _ => {
                view.steer(keys, dt);
                false
            }
        };
        if let Some(size) = size {
            view.look.size = size;
        }
        view.light_look();
        let start = Instant::now();
        let frame = view.engine.frame(&view.look, &view.world);
        let framed = Instant::now();
        self.framing = ms(start, framed);
        let camera = view.look.camera;
        // Drawn from the next frame on.
        let wants = with_ground(frame.wants, view.walker.is_some(), &view.world, camera);
        let worked = self.by_work.then(|| view.world.worked());
        let arrived = view.world.update(camera, &wants);
        if let Some(worked) = worked {
            worked_a_frame(&view.world, &worked);
        }
        self.shown = !frame.busy && !view.world.lacks(&wants);
        if steered {
            let [tiles, bricks] = view.world.owed();
            let (owed, still) = ([wants.asked.len(), tiles, bricks], !self.shown);
            self.sample =
                Some(Sample { ms: [self.framing, ms(framed, Instant::now()), 0.0, 0.0, 0.0], t: self.steered, camera, owed, still });
        }
        let more = arrived || frame.busy || view.world.making() > 0;
        (self.picture, self.stats) = (Some(frame.picture), frame.stats);
        let options = format!("{} {}", view.world_args, view.options());
        if let Some((file, start)) = &mut self.recording {
            // Line by line, all there should the window panic.
            let line = format!("{:.4}\t{options}\t{}", start.elapsed().as_secs_f64(), self.stats.replace('\n', ", "));
            writeln!(file, "{line}").and_then(|()| file.flush()).expect("--record file");
        }
        *LAST_FRAME.lock().unwrap_or_else(|e| e.into_inner()) = format!("{options}\n{}", self.stats);
        more
    }

    /** The frame drawn, where its time went: its sample's rest, and its picture kept, should --shots want it. */
    fn drawn(&mut self, times: Times) {
        self.times = times;
        let Some(mut sample) = self.sample.take() else { return };
        let [engine, world, ..] = sample.ms;
        sample.ms[2..].copy_from_slice(&[times.upload, times.waiting, engine + world + times.upload + times.waiting]);
        if let Some(shots) = &mut self.shots
            && sample.still
            && shots.kept.len() < KEPT
            && shots.kept.last().is_none_or(|k| sample.t - k.0 >= shots.every)
            && let Some(picture) = self.picture.take()
        {
            shots.kept.push((sample.t, self.view.look, picture));
        }
        self.samples.push(sample);
    }

    /** Whether the pilot has no frames more to steer, and `end` is due. */
    fn ended(&self) -> bool {
        self.pilot.as_ref().is_some_and(Pilot::ended)
    }

    /**
     * What the steering's frames cost, said; the view drawn complete, said
     * and saved; the pictures --shots kept, saved beside their views
     * complete. The keys steer from now on.
     */
    fn end(&mut self, easel: &mut Easel) {
        if let Some(Pilot::Replay(Replay { start: Some(start), frames, .. })) = &self.pilot {
            println!("ended {:.2} s late", start.elapsed().as_secs_f64() - frames.last().map_or(0.0, |f| f.0));
        }
        self.pilot = None;
        report(&self.samples);
        // A window, which goes on, waits for the view complete only if asked.
        let View { engine, world, look, .. } = &mut self.view;
        if self.headless || self.shot.is_some() || !self.samples.is_empty() {
            let (start, before) = (Instant::now(), world.worked());
            let frame = drawn_complete(&mut **engine, world, look, easel);
            let work: Vec<f64> = world.worked().iter().zip(&before).map(|(a, b)| a - b).collect();
            println!(
                "the view complete in {:.2} s on {} threads, their work {:.2} s, the busiest's {:.2} s: {}{}",
                start.elapsed().as_secs_f64(),
                world.threads,
                work.iter().sum::<f64>(),
                work.iter().copied().fold(0.0, f64::max),
                frame.stats.replace('\n', ", "),
                easel.describe()
            );
            if let Some(path) = &self.shot {
                easel.save(&*frame.picture, look.size.map(|v| v as u32), path);
                println!("saved {path}");
            }
        }
        if let Some(Shots { dir, kept, .. }) = &self.shots {
            for (t, look, picture) in kept {
                let pixels = look.size.map(|v| v as u32);
                easel.save(&**picture, pixels, &format!("{dir}/{t:08.3}-shown.png"));
                let whole = drawn_complete(&mut **engine, world, look, easel);
                easel.save(&*whole.picture, pixels, &format!("{dir}/{t:08.3}-complete.png"));
            }
        }
    }
}

/** ms from `a` to `b`. */
fn ms(a: Instant, b: Instant) -> f64 {
    (b - a).as_secs_f64() * 1000.0
}

/**
 * Says what the frames a stroll or a replay steered cost and where the
 * slowest were; what they asked for (where rays took a coarser level)
 * and the world owed them, on average and at most.
 */
fn report(samples: &[Sample]) {
    if samples.is_empty() {
        return;
    }
    let short = samples.iter().filter(|s| s.still).count();
    println!("{} frames, {short} with what they show still coming; ms at the median, 90th and 99th percentile, and most:", samples.len());
    for (k, name) in ["engine", "world", "upload", "GPU", "frame"].iter().enumerate() {
        let mut t: Vec<f64> = samples.iter().map(|s| s.ms[k]).collect();
        t.sort_by(f64::total_cmp);
        let at = |q: f64| t[((t.len() - 1) as f64 * q) as usize];
        println!("  {name:>6}: {:6.1} {:6.1} {:6.1} {:6.1}", at(0.5), at(0.9), at(0.99), at(1.0));
    }
    for (k, name) in ["tiles and bricks asked for", "tiles owed", "bricks owed"].iter().enumerate() {
        let (sum, most) = samples.iter().fold((0, 0), |(sum, most), s| (sum + s.owed[k], most.max(s.owed[k])));
        println!("  {name}: {:.1} on average, {most} at most", sum as f64 / samples.len() as f64);
    }
    let mut slowest: Vec<&Sample> = samples.iter().collect();
    slowest.sort_by(|a, b| b.ms[4].total_cmp(&a.ms[4]));
    for s in slowest.iter().take(5) {
        let ([x, y, z], [e, w, u, g, f]) = (s.camera, s.ms);
        println!("  at {:.3} s: {f:.1} ms (engine {e:.1}, world {w:.1}, upload {u:.1}, GPU {g:.1}) at --camera {x},{y},{z}", s.t);
    }
}

/** --headless: as a window draws, offscreen, the GPU a frame behind, a stroll's frames a 60th of a second apart at least, till the steering ends. */
fn headless(mut app: App) {
    let mut easel = Easel::offscreen(None);
    let mut last = None;
    while !app.ended() {
        let start = Instant::now();
        app.frame(Keys::default(), FRAME, None);
        let framed = Instant::now();
        let next = easel.draw(&**app.picture.as_ref().expect("a frame's picture"), app.view.look.size.map(|v| v as u32));
        let uploaded = Instant::now();
        if let Some(last) = last.replace(next) {
            easel.wait_for(last);
        }
        let end = Instant::now();
        app.drawn(Times {
            ui: ms(start, framed),
            upload: ms(framed, uploaded),
            waiting: ms(uploaded, end),
            rest: 0.0,
            since: ms(start, end),
        });
        // A stroll steers a frame's time each: no sooner than a window would draw the next. (A replay's come at their own.)
        if matches!(app.pilot, Some(Pilot::Stroll { .. }))
            && let Some(early) = std::time::Duration::from_secs_f64(FRAME).checked_sub(start.elapsed())
        {
            std::thread::sleep(early);
        }
    }
    app.end(&mut easel);
}

/**
 * What the world is to make for `wants`; walking, first the ground's level
 * a tile around the body at `camera`, which it asks for whether seen or not.
 */
fn with_ground(mut wants: Wants, walking: bool, world: &World, camera: Vec3) -> Wants {
    if walking {
        let level = COLLISION_LEVEL as usize;
        wants.reach[level] = wants.reach[level].or(Some(TILE as f64));
        let ground = world.lacking(level, TILE as f64, camera).into_iter().map(|id| (level, Piece::Tile(id)));
        wants.asked = Arc::new(ground.chain(wants.asked.iter().copied()).collect());
    }
    wants
}

/** The last frame, as the options that show it, and its stats: for a report, should the window panic or lose its GPU device. */
static LAST_FRAME: std::sync::Mutex<String> = std::sync::Mutex::new(String::new());

fn last_frame() -> String {
    LAST_FRAME.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

/**
 * Frames of `look` until all it shows is in: every tile it lacks made,
 * and all the engine makes of them.
 */
fn complete(engine: &mut dyn Engine, world: &mut World, look: &Look) -> Frame {
    loop {
        let frame = engine.frame(look, world);
        if !world.lacks(&frame.wants) && !frame.busy {
            return frame;
        }
        if world.update(look.camera, &frame.wants) || engine.wait() {
            continue;
        }
        // Nothing came and nothing is to come: the engine is to be drawn, to ask for more (drawn_complete).
        if world.making() == 0 {
            return frame;
        }
        world.wait();
    }
}

/**
 * The view of `look` once complete, as `complete` makes it, drawn on
 * `easel` till settled, and made complete and drawn again while its
 * painter asks the engine for what it left out (the chunks' probes of
 * cells hidden that show): so that all it shows is in.
 */
fn drawn_complete(engine: &mut dyn Engine, world: &mut World, look: &Look, easel: &mut Easel) -> Frame {
    loop {
        let frame = complete(engine, world, look);
        easel.settle(&*frame.picture, look.size.map(|v| v as u32));
        if !easel.painter.as_ref().is_some_and(|p| p.asked()) {
            return frame;
        }
    }
}

/** A frame's time, as the window draws them, and a stroll steps. */
const FRAME: f64 = 1.0 / 60.0;

/** Degrees a second the keys turn the view at. */
const TURN: f64 = 90.0;

/**
 * How long the world's threads took from `before` to `after`
 * (World::worked), had they the cores to themselves: as long as the
 * busiest was on one.
 */
fn passed(before: &[f64], after: &[f64]) -> f64 {
    after.iter().zip(before).map(|(a, b)| a - b).fold(0.0, f64::max)
}

/**
 * Waits till the world's threads have had a frame's time on their cores
 * since `before` (World::worked), or work no more, with nothing left to
 * do: what they would do in a frame, did drawing it take none of their
 * cores. Idle is a guess: a millisecond in which none worked a twentieth
 * of it, which threads starved of their cores would pass for too.
 */
fn worked_a_frame(world: &World, before: &[f64]) {
    let mut last = 0.0;
    loop {
        std::thread::sleep(std::time::Duration::from_millis(1));
        let passed = passed(before, &world.worked());
        if passed >= FRAME || passed - last < 5e-5 {
            return;
        }
        last = passed;
    }
}

/** The view it starts with, at the size it draws headless unless --size; the window's size unless --size. */
const FOV: f64 = 90.0;
const RENDER_DISTANCE: f64 = 8000.0;
const SHOT: [f64; 2] = [1280.0, 800.0];
const WINDOW: [f64; 2] = [1580.0, 800.0];

/** Units a second it flies at unless --speed. */
const SPEED: f64 = 40.0;

fn main() {
    // Tiles come and go, in blocks of MBs. glibc maps blocks that large apart, to give back once freed, but raises that
    // threshold past each it frees: later ones would stay in its heap.
    // SAFETY: mallopt only sets a parameter, before any thread starts.
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    unsafe {
        libc::mallopt(libc::M_MMAP_THRESHOLD, 128 << 10);
    }
    let panic = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let last = last_frame();
        if !last.is_empty() {
            eprintln!("The last frame:\n{last}");
        }
        // A browser shows nothing of stderr.
        #[cfg(target_arch = "wasm32")]
        web_sys::console::error_1(&format!("{info}\nThe last frame:\n{last}").into());
        panic(info);
    }));
    #[cfg(not(target_arch = "wasm32"))]
    let argv: Vec<String> = std::env::args().collect();
    // In a browser, the page's query gives the options, each key=value as --key value.
    #[cfg(target_arch = "wasm32")]
    let argv: Vec<String> = {
        let query = web_sys::window().and_then(|w| w.location().search().ok()).unwrap_or_default();
        let pairs = query.trim_start_matches('?').split('&').filter(|p| !p.is_empty());
        // URLSearchParams writes commas as %2C.
        let options = pairs.flat_map(|p| match p.split_once('=') {
            Some((key, value)) => vec![format!("--{key}"), value.replace("%2C", ",").replace("%2c", ",")],
            None => vec![format!("--{p}")],
        });
        std::iter::once(String::new()).chain(options).collect()
    };
    let mut args = parse(&argv);
    // A recording's world, seed and threads, its first line, unless given.
    let recording = args.replay.as_ref().map(|path| std::fs::read_to_string(path).expect("--replay file"));
    if let Some(text) = &recording {
        let header = text.lines().next().and_then(|l| l.strip_prefix("# ")).expect("a recording");
        let (program, given) = argv.split_first().expect("the program");
        args = parse(
            &[program.clone()].into_iter().chain(header.split_whitespace().map(str::to_string)).chain(given.to_vec()).collect::<Vec<_>>(),
        );
    }
    let app = App::new(&args, recording.as_deref());
    if args.headless {
        headless(app);
    } else {
        window::run("Parallax", args.size.unwrap_or(WINDOW), app);
    }
}

#[cfg(test)]
mod tests;
