//! The chunks engine, as the web draws: the renderer picks the cells of the
//! lattice in view (renderer.rs), worker threads bake them into chunks
//! (bakery.rs), their whole voxels reliefs or splats each, and the
//! compositor draws them (compositor.rs).
//! See docs/architecture.md, sections 1 to 5.

mod bakery;
// The slabs draw with its globals.
pub(super) mod compositor;
mod renderer;

use bake::lod::Lod;
use egui_wgpu::wgpu;

use compositor::{Compositor, View};
use renderer::Renderer;

use super::{Colours, Engine, Frame, Look, Picture};
use crate::gpu::Easel;
use crate::world::{Lit, Wants, World};

pub struct Chunks {
    renderer: Renderer,
    /** Whether chunks in view were baking at the last frame. */
    baking: bool,
}

impl Chunks {
    /** Draws `world`, its whole voxels reliefs or splats each (Renderer's `relief`), reliefs flat if `flat`. */
    pub fn new(world: &World, relief: bool, flat: bool) -> Chunks {
        Chunks { renderer: Renderer::new(world, relief, flat), baking: false }
    }
}

impl Engine for Chunks {
    fn frame(&mut self, look: &Look, world: &World) -> Frame {
        let (frame, stats) = self.renderer.render(look, world);
        self.baking = !stats.wanted.is_empty();
        let lit = if look.sky.is_some() { Lit::Reading } else { Lit::Not };
        let lod = Lod::new(look.shift, look.detail);
        Frame {
            stats: format!("{} chunks\n{} baking, {} tiles wanted", frame.chunks.len(), stats.wanted.len(), stats.tiles.len()),
            picture: Box::new(Composed { frame, view: View::new(look) }),
            // Whichever way the eye turns.
            wants: Wants { reach: std::array::from_fn(|g| lod.grid_reach(g, look.far)), lit, tiles: stats.tiles },
            busy: self.baking,
        }
    }

    fn wait(&mut self) -> bool {
        if self.baking {
            self.renderer.wait();
        }
        self.baking
    }

    fn panel(&mut self, ui: &mut egui::Ui, look: &mut Look) {
        look.detail_panel(ui);
        ui.checkbox(&mut look.seams, "Show Face Seams");
        ui.horizontal(|ui| {
            ui.label("Colour by");
            for (colours, name) in Colours::ALL {
                ui.radio_value(&mut look.colours, colours, name);
            }
        });
    }
}

/** A frame's chunks, and how they are seen. */
struct Composed {
    frame: renderer::Frame,
    view: View,
}

impl Picture for Composed {
    fn draw(&self, easel: &mut Easel, encoder: &mut wgpu::CommandEncoder, pixels: [u32; 2]) {
        let (compositor, device, queue) = easel.painter(|device, format| Compositor::new(device, format, true));
        compositor.prepare(device, queue, encoder, &self.frame, &View { pixels, ..self.view });
        compositor.render(encoder);
    }
}

#[cfg(test)]
mod tests {
    use std::any::Any;
    use std::collections::HashSet;

    use super::*;
    use crate::view::view_basis;

    /**
     * World `name` of seed 1 from its spawn, looking `look`, at `detail`,
     * once the view is complete, its whole voxels reliefs (flat if `flat`) or splats.
     */
    fn complete_view(name: &str, look: [f64; 2], detail: f64, relief: bool, flat: bool) -> Composed {
        let make = worlds::by_name(name).expect("world").make;
        let mut world = World::new(make, 1, None, 4);
        let look = Look { detail, ..Look::new(make(1).spawn.map(|v| v as f64), look, 90.0, 1000.0, [320.0, 200.0]) };
        complete(&mut Chunks::new(&world, relief, flat), &mut world, &look)
    }

    /** What `chunks` draws of `world` as `look` sees it, once complete. */
    fn complete(chunks: &mut Chunks, world: &mut World, look: &Look) -> Composed {
        let picture: Box<dyn Any> = crate::complete(chunks, world, look, false).picture;
        *picture.downcast().expect("chunks")
    }

    /** World, look and detail as a power of two. The thickest cells too: in relief, their columns move most with the camera. */
    const VIEWS: [(&str, [f64; 2], f64); 3] =
        [("forest", [0.0, 0.0], 3.25), ("forest", [20.0, -25.0], 2.25), ("moor", [-30.0, 10.0], 3.25)];

    const PIXELS: [u32; 2] = [320, 200];

    #[test]
    fn moving_keeps_most_chunks() {
        let make = worlds::by_name("moor").expect("moor").make;
        let mut world = World::new(make, 1, None, 4);
        let mut chunks = Chunks::new(&world, true, false);
        let look = Look::new(make(1).spawn.map(|v| v as f64), [30.0, -10.0], 90.0, 400.0, [320.0, 200.0]);
        let ids = |c: &Composed| c.frame.chunks.iter().map(|p| p.chunk.id).collect::<HashSet<_>>();
        let before = ids(&complete(&mut chunks, &mut world, &look));
        // A few units along the view: each face's levels claim cells a little farther on.
        let forward = view_basis(look.yaw, look.pitch)[2];
        let look = Look { camera: std::array::from_fn(|i| look.camera[i] + forward[i] * 7.0), ..look };
        let after = ids(&complete(&mut chunks, &mut world, &look));
        let kept = after.intersection(&before).count();
        // Level 0's cells, which change most, are over half the frame's in the moor's heather and mist, seen through.
        assert!(kept * 10 > after.len() * 6, "{kept} of {} kept", after.len());
    }

    #[test]
    #[ignore = "draws on the GPU: needs Vulkan, as Mesa's software one"]
    fn culling_on_the_gpu_leaves_the_picture_unchanged() {
        for (name, look, octaves) in VIEWS {
            for (relief, flat) in [(true, false), (true, true), (false, false)] {
                let composed = complete_view(name, look, 2f64.powf(octaves), relief, flat);
                let [culled, all] = [true, false].map(|culls| {
                    let mut easel = Easel::offscreen(None);
                    easel.painter = Some(Box::new(Compositor::new(&easel.device, easel.format, culls)));
                    let pixels = easel.pixels(&composed, PIXELS).2;
                    (pixels, easel.held::<Compositor>().expect("a compositor").drawn(&easel).iter().sum::<u32>())
                });
                let drew = if flat {
                    "flat tiles"
                } else if relief {
                    "tiles"
                } else {
                    "splats"
                };
                assert!(culled.0 == all.0, "{name} {look:?} {drew}");
                assert!(culled.1 < all.1, "{name} {look:?}: culling drew {} {drew} of {}", culled.1, all.1);
            }
        }
    }

    #[test]
    #[ignore = "draws on the GPU: needs Vulkan, as Mesa's software one"]
    fn what_the_buffers_cannot_hold_is_left_out_farthest_first() {
        let mut composed = complete_view("moor", [-30.0, 10.0], 2f64.powf(3.25), true, false);
        let all = Easel::offscreen(None).pixels(&composed, PIXELS).2;
        // Room for about half of the frame's columns: the chunks that come first, as many as fit, of columns, splats and clusters.
        let bytes =
            |b: &bakery::Baked| [size_of_val(&b.columns[..]), size_of_val(&b.splats[..]), size_of_val(&b.clusters[..])].map(|n| n as u64);
        let chunks = &composed.frame.chunks;
        let room = chunks.iter().map(|p| bytes(&p.chunk.baked)[0]).sum::<u64>() / 2;
        let some = Easel::offscreen(Some(room)).pixels(&composed, PIXELS).2;
        let sums = chunks.iter().scan([0; 3], |sum, p| {
            sum.iter_mut().zip(bytes(&p.chunk.baked)).for_each(|(s, b)| *s += b);
            Some(*sum)
        });
        let fit = sums.take_while(|sum| sum.iter().all(|&s| s <= room)).count();
        assert!(fit > 0 && fit < chunks.len(), "{fit} of {} chunks fit", chunks.len());
        composed.frame.chunks.truncate(fit);
        let first = Easel::offscreen(None).pixels(&composed, PIXELS).2;
        assert!(some != all, "some left out");
        assert!(some == first, "what is drawn shows as it would alone");
    }
}
