//! What draws the world: engines, each its own way, from the same tiles
//! (world.rs), behind one interface. On the main thread an engine makes a
//! Frame of what a Look sees: a Picture, and what it wants of the world.
//! On a GPU, the engine's Painter draws its pictures into one of its own,
//! which the window shows or a shot saves (gpu.rs).
//!
//! - chunks: cells of the lattice baked into chunks, as the web draws them,
//!   their whole voxels reliefs or splats (Kind's Reliefs, Splats), or
//!   reliefs drawn flat, a picture a cell (Flat, an experiment);
//! - voxels: rays through the tiles' voxels, nothing baked.

mod chunks;
// The world packs its tiles for the voxels' GPU as it makes them.
pub mod voxels;

use std::any::Any;

use egui_wgpu::wgpu;
use worldgen::light::Brightness;

use crate::gpu::Easel;
use crate::view::{self, Vec3, focal_length};
use crate::world::{Wants, World};

/** The engines, as --engine and the panel name them. */
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Kind {
    Reliefs,
    Flat,
    Splats,
    Voxels,
}

impl Kind {
    pub const ALL: [(Kind, &str); 4] =
        [(Kind::Reliefs, "reliefs"), (Kind::Flat, "flat"), (Kind::Splats, "splats"), (Kind::Voxels, "voxels")];

    /** An engine of this kind, drawing `world`. */
    pub fn make(self, world: &World) -> Box<dyn Engine> {
        match self {
            Kind::Reliefs => Box::new(chunks::Chunks::new(world, true, false)),
            Kind::Flat => Box::new(chunks::Chunks::new(world, true, true)),
            Kind::Splats => Box::new(chunks::Chunks::new(world, false, false)),
            Kind::Voxels => Box::new(voxels::Voxels),
        }
    }
}

/** What draws the world, a frame at a time, on the main thread. */
pub trait Engine {
    /** A frame of the world's tiles as `look` sees them. */
    fn frame(&mut self, look: &Look, world: &World) -> Frame;

    /** Waits for what it makes on threads of its own for frames to come, if it is making any; whether it was. */
    fn wait(&mut self) -> bool {
        false
    }

    /** Its own part of the panel: the settings of `look` it reads. */
    fn panel(&mut self, _ui: &mut egui::Ui, _look: &mut Look) {}
}

/** An engine's frame: what it draws, what it wants of the world, and what it says of itself. */
pub struct Frame {
    pub picture: Box<dyn Picture>,
    pub wants: Wants,
    /** Whether what it shows is still coming, tiles or what it makes of them: frames to come show more, the eye still. */
    pub busy: bool,
    pub stats: String,
}

/** What a frame draws, all its engine's painter needs, on any GPU. */
pub trait Picture: Any {
    /** Draws it into a picture of `pixels` with its engine's painter on `easel` (Easel::painter). */
    fn draw(&self, easel: &mut Easel, encoder: &mut wgpu::CommandEncoder, pixels: [u32; 2]);
}

/** An engine's own on a GPU: what it keeps there from frame to frame, and the picture it drew last. */
pub trait Painter: Any {
    fn picture(&self) -> &wgpu::Texture;

    /** Shows that picture on `pass`'s viewport. */
    fn show(&self, pass: &mut wgpu::RenderPass<'_>);

    /** What a bench says of the frame it drew last, beside its time. */
    fn describe(&self, _easel: &Easel) -> String {
        String::new()
    }

    /** Whether the frame it drew last showed all it was given (the chunks' compositor spreads changes over frames). */
    fn settled(&self) -> bool {
        true
    }
}

/**
 * How the world is seen: from where, which way, how wide (degrees across
 * the longer side) and how far, at what size (points), lit or not; and
 * what engines that draw cells, or chunks, read besides.
 */
#[derive(Clone, Copy, PartialEq)]
pub struct Look {
    pub camera: Vec3,
    pub yaw: f64,
    pub pitch: f64,
    pub fov: f64,
    /** The render distance, and the fog's, as a share of it. */
    pub far: f64,
    pub fog: f64,
    pub size: [f64; 2],
    /** How bright the light is, if voxels are lit by light worked out across the world (worldgen's Light); None, shaded. */
    pub light: Option<Brightness>,
    /** Viewing distance over a cell's thickness, and how many levels finer than its cell its voxels are (bake's Lod). */
    pub detail: f64,
    pub shift: i32,
    /** Whether to draw the seams between faces over the picture, and what to paint voxels in. */
    pub seams: bool,
    pub colours: Colours,
}

impl Look {
    /** From `camera`, looking `yaw` and `pitch` across `fov`, as far as `far`, at `size`; as the web's page starts. */
    pub fn new(camera: Vec3, [yaw, pitch]: [f64; 2], fov: f64, far: f64, size: [f64; 2]) -> Look {
        Look {
            camera,
            yaw,
            pitch,
            fov,
            far,
            // renderer.ts's FOG_PER_RENDER_DISTANCE and DEFAULT_DETAIL.
            fog: 0.7,
            size,
            light: None,
            detail: 2f64.powf(3.25),
            shift: 3,
            seams: false,
            colours: Colours::Voxels,
        }
    }

    /** Its orientation, size and focal length. */
    pub fn view(&self) -> view::View {
        let [width, height] = self.size;
        view::View { yaw: self.yaw, pitch: self.pitch, width, height, focal: focal_length(self.fov, width, height) }
    }

    /** The panel's sliders of its detail and shift, for engines that draw cells. */
    pub fn detail_panel(&mut self, ui: &mut egui::Ui) {
        // In quarter octaves, as the web's: dragging picks cells anew only at each step.
        let mut octaves = self.detail.log2();
        let detail = egui::Slider::new(&mut octaves, 0.0..=6.0).step_by(0.25).custom_formatter(|v, _| format!("{:.1}", 2f64.powf(v)));
        if ui.add(detail.text("Detail")).changed() {
            self.detail = 2f64.powf(octaves);
        }
        ui.add(egui::Slider::new(&mut self.shift, 1..=5).text("Voxels Finer than Cells (levels)"));
    }
}

/** What voxels are painted in: their own colours, or to see how the picture is cut up, a colour per layer or per chunk. */
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Colours {
    #[default]
    Voxels,
    Layers,
    Chunks,
}

impl Colours {
    pub const ALL: [(Colours, &str); 3] = [(Colours::Voxels, "Voxels"), (Colours::Layers, "Layers"), (Colours::Chunks, "Chunks")];
}
