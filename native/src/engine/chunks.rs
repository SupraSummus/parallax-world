//! The chunks engine, as the web draws: the renderer picks the cells of the
//! lattice in view (renderer.rs), on a thread of its own, worker threads
//! bake them into chunks (bakery.rs), their whole voxels reliefs or splats
//! each, and the compositor draws them (compositor.rs).
//! See docs/architecture.md, sections 1 to 5.

mod bakery;
mod compositor;
mod renderer;

use std::sync::{Arc, Condvar, Mutex};

use bake::lod::Lod;
use egui_wgpu::wgpu;
use fcc::GRID_LEVELS;

use compositor::{Compositor, View};
use renderer::Renderer;

use super::{Colours, Engine, Frame, Look, Picture};
use crate::gpu::Easel;
use crate::world::{Level, Lit, Wants, World};

/** What passes between the renderer's thread and the one that draws. */
#[derive(Default)]
struct Mailbox {
    /** The look to render next, with the world's levels then, and how many looks were handed over. */
    next: Option<(Look, Vec<Arc<Level>>)>,
    asked: u64,
    /** The last frame rendered, and for how many looks; how many frames were. */
    rendered: Option<(renderer::Frame, renderer::Stats, u64)>,
    published: u64,
    /** Whether a chunk was baked since the last render began. */
    baked: bool,
    /** Whether the renderer's thread is to stop, or has. */
    stop: bool,
}

type Shared = Arc<(Mutex<Mailbox>, Condvar)>;

/**
 * Chunks, rendered on a thread of its own (picking a face's cells anew
 * takes many milliseconds at once): a frame draws the last rendered, which
 * the GPU places from where the eye is now (compositor.wgsl's `place`).
 */
pub struct Chunks {
    shared: Shared,
    /** The look and levels the last frame handed over. */
    asked: Option<(Look, Vec<Arc<Level>>)>,
    /** Whether what the last frame showed was still coming, and the renderer's frames then (Mailbox's `published`). */
    busy: bool,
    published: u64,
}

impl Chunks {
    /** Draws `world`, its whole voxels reliefs or splats each (Renderer's `relief`), reliefs flat if `flat`. */
    pub fn new(world: &World, relief: bool, flat: bool) -> Chunks {
        let shared: Shared = Arc::default();
        let baked = {
            let shared = shared.clone();
            move || {
                shared.0.lock().expect("mailbox").baked = true;
                shared.1.notify_all();
            }
        };
        let renderer = Renderer::new(world, relief, flat, baked);
        let mailbox = shared.clone();
        std::thread::spawn(move || render_on(renderer, &mailbox));
        Chunks { shared, asked: None, busy: false, published: 0 }
    }
}

/** The renderer's thread: it renders the latest look handed over, and the last again as chunks are baked, till told to stop. */
fn render_on(mut renderer: Renderer, shared: &(Mutex<Mailbox>, Condvar)) {
    // Stopped, even by a panic: the thread that draws waits for nothing more.
    struct Stopped<'a>(&'a (Mutex<Mailbox>, Condvar));
    impl Drop for Stopped<'_> {
        fn drop(&mut self) {
            self.0.0.lock().unwrap_or_else(|e| e.into_inner()).stop = true;
            self.0.1.notify_all();
        }
    }
    let _stopped = Stopped(shared);
    let mut last: Option<(Look, Vec<Arc<Level>>, u64)> = None;
    loop {
        let (look, levels, asked) = {
            let mut m = shared.0.lock().expect("mailbox");
            let job = loop {
                if m.stop {
                    return;
                }
                if let Some((look, levels)) = m.next.take() {
                    break (look, levels, m.asked);
                }
                if m.baked
                    && let Some(last) = &last
                {
                    break last.clone();
                }
                m = shared.1.wait(m).expect("mailbox");
            };
            m.baked = false;
            job
        };
        let (frame, stats) = renderer.render(&look, &levels);
        let rendered = (frame, stats.clone(), asked);
        {
            let mut m = shared.0.lock().expect("mailbox");
            m.rendered = Some(rendered);
            m.published += 1;
        }
        shared.1.notify_all();
        last = Some((look, levels, asked));
    }
}

impl Drop for Chunks {
    fn drop(&mut self) {
        self.shared.0.lock().expect("mailbox").stop = true;
        self.shared.1.notify_all();
    }
}

impl Engine for Chunks {
    fn frame(&mut self, look: &Look, world: &World) -> Frame {
        let levels: Vec<Arc<Level>> = (0..GRID_LEVELS).map(|l| world.level(l).clone()).collect();
        let (frame, stats, answered, asked) = {
            let mut m = self.shared.0.lock().expect("mailbox");
            // Handed over only if it changed, or the renderer would be ever behind.
            let same = self.asked.as_ref().is_some_and(|(l, ls)| l == look && ls.iter().zip(&levels).all(|(a, b)| Arc::ptr_eq(a, b)));
            if !same {
                m.next = Some((*look, levels.clone()));
                m.asked += 1;
                self.asked = Some((*look, levels));
                self.shared.1.notify_all();
            }
            // The first frame waits for something to draw.
            while !m.stop && m.rendered.is_none() {
                m = self.shared.1.wait(m).expect("mailbox");
            }
            assert!(!m.stop, "the chunks' renderer stopped");
            self.published = m.published;
            let (frame, stats, answered) = m.rendered.clone().expect("rendered");
            (frame, stats, answered, m.asked)
        };
        // Until rendered for this look and these tiles, what it wants is not known: while the eye moves, never quite.
        self.busy = stats.baking > 0 || answered < asked;
        let lit = if look.light.is_some() { Lit::Reading } else { Lit::Not };
        let whole = Lod::new(look.shift, look.detail);
        // As far as either Lod's chunks read each grid.
        let reach = |g| [whole, whole.for_medium()].iter().filter_map(|lod| lod.grid_reach(g, look.far)).reduce(f64::max);
        Frame {
            stats: format!("{} chunks\n{} baking, {} tiles wanted", frame.chunks.len(), stats.baking, stats.tiles.len()),
            picture: Box::new(Composed { frame, view: View::new(look) }),
            // Whichever way the eye turns.
            wants: Wants { reach: std::array::from_fn(reach), lit, tiles: stats.tiles },
            busy: self.busy,
        }
    }

    /** Waits, if what it shows was still coming at the last frame, until the renderer renders again. */
    fn wait(&mut self) -> bool {
        if self.busy {
            let mut m = self.shared.0.lock().expect("mailbox");
            while m.published == self.published && !m.stop {
                m = self.shared.1.wait(m).expect("mailbox");
            }
        }
        self.busy
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
    use std::sync::Arc;

    use super::*;
    use crate::view::view_basis;

    /**
     * World `name` of seed 1 from its spawn, looking `look`, at `detail`,
     * once the view is complete, its whole voxels reliefs (flat if `flat`) or splats.
     */
    fn complete_view(name: &str, look: [f64; 2], detail: f64, relief: bool, flat: bool) -> Composed {
        let (mut world, look) = world_and_look(name, look, detail);
        complete(&mut Chunks::new(&world, relief, flat), &mut world, &look)
    }

    /** World `name` of seed 1, and the look from its spawn, looking `look`, at `detail`. */
    fn world_and_look(name: &str, look: [f64; 2], detail: f64) -> (World, Look) {
        let make = worlds::by_name(name).expect("world").make;
        let look = Look { detail, ..Look::new(make(1).spawn.map(|v| v as f64), look, 90.0, 1000.0, [320.0, 200.0]) };
        (World::new(make, 1, None, 4), look)
    }

    /** What `chunks` draws of `world` as `look` sees it, once complete. */
    fn complete(chunks: &mut Chunks, world: &mut World, look: &Look) -> Composed {
        let picture: Box<dyn Any> = crate::complete(chunks, world, look, false).picture;
        *picture.downcast().expect("chunks")
    }

    const PIXELS: [u32; 2] = [320, 200];

    /** How two pictures differ: in how many pixels, by how much at most, and where the first is. */
    fn differ(a: &[u8], b: &[u8]) -> String {
        let pixels = a.chunks(4).zip(b.chunks(4)).enumerate().filter(|(_, (a, b))| a != b);
        let (mut count, mut most, mut first) = (0, 0, None);
        for (i, (a, b)) in pixels {
            count += 1;
            most = most.max(a.iter().zip(b).map(|(a, b)| a.abs_diff(*b)).max().unwrap_or(0));
            first = first.or(Some((i as u32 % PIXELS[0], i as u32 / PIXELS[0], [a, b])));
        }
        format!("{count} pixels differ, by {most} at most, the first {first:?}")
    }

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
        // Level 0's cells, which change most, are many of the frame's in the moor's heather.
        assert!(kept * 10 > after.len() * 6, "{kept} of {} kept", after.len());
    }

    #[test]
    #[ignore = "draws on the GPU: needs Vulkan, as Mesa's software one"]
    fn a_compositor_following_changes_draws_as_one_taking_the_frame_whole() {
        let make = worlds::by_name("moor").expect("moor").make;
        let mut world = World::new(make, 1, None, 4);
        let look = Look::new(make(1).spawn.map(|v| v as f64), [30.0, -10.0], 90.0, 400.0, [320.0, 200.0]);
        let forward = view_basis(look.yaw, look.pitch)[2];
        let moved = Look { camera: std::array::from_fn(|i| look.camera[i] + forward[i] * 7.0), ..look };
        // Not flat: following changes lays the clusters out in another order than taking the frame whole, and ties
        // fall by it (docs/todo.md).
        for relief in [true, false] {
            let mut chunks = Chunks::new(&world, relief, false);
            let first = complete(&mut chunks, &mut world, &look);
            let second = complete(&mut chunks, &mut world, &moved);
            assert!(second.frame.since(first.frame.number).is_some(), "the changes since the first frame kept");
            let mut easel = Easel::offscreen(None);
            easel.pixels(&first, PIXELS);
            let followed = easel.pixels(&second, PIXELS).2;
            let whole = Easel::offscreen(None).pixels(&second, PIXELS).2;
            assert!(followed == whole, "relief {relief}: {}", differ(&followed, &whole));
        }
    }

    /**
     * Culling on the GPU leaves world `name` from its spawn, looking `angles`, at detail 2^`octaves`, unchanged, drawn
     * every way, and draws less. A test per view, side by side.
     */
    fn culling_leaves_the_picture_unchanged(name: &str, angles: [f64; 2], octaves: f64) {
        let (mut world, look) = world_and_look(name, angles, 2f64.powf(octaves));
        for (relief, flat) in [(true, false), (true, true), (false, false)] {
            let composed = complete(&mut Chunks::new(&world, relief, flat), &mut world, &look);
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
            assert!(culled.0 == all.0, "{name} {angles:?} {drew}: {}", differ(&culled.0, &all.0));
            assert!(culled.1 < all.1, "{name} {angles:?}: culling drew {} {drew} of {}", culled.1, all.1);
        }
    }

    /** The thickest cells too: in relief, their columns move most with the camera. */
    mod culling_on_the_gpu_leaves_the_picture_unchanged {
        use super::culling_leaves_the_picture_unchanged as unchanged;

        #[test]
        #[ignore = "draws on the GPU: needs Vulkan, as Mesa's software one"]
        fn forest() {
            unchanged("forest", [0.0, 0.0], 3.25);
        }

        #[test]
        #[ignore = "draws on the GPU: needs Vulkan, as Mesa's software one"]
        fn forest_turned() {
            unchanged("forest", [20.0, -25.0], 2.25);
        }

        #[test]
        #[ignore = "draws on the GPU: needs Vulkan, as Mesa's software one"]
        fn moor() {
            unchanged("moor", [-30.0, 10.0], 3.25);
        }
    }

    #[test]
    #[ignore = "draws on the GPU: needs Vulkan, as Mesa's software one"]
    fn what_the_buffers_cannot_hold_is_left_out_farthest_first() {
        let mut composed = complete_view("moor", [-30.0, 10.0], 2f64.powf(3.25), true, false);
        let all = Easel::offscreen(None).pixels(&composed, PIXELS).2;
        // Room for about half of the frame's columns: the chunks that come first, as many as fit, of columns, splats and
        // clusters, and records.
        let bytes = |b: &bakery::Baked| {
            [size_of_val(&b.columns[..]) as u64, size_of_val(&b.splats[..]) as u64, size_of_val(&b.clusters[..]) as u64, compositor::RECORD]
        };
        let chunks = &composed.frame.chunks;
        let room = chunks.iter().map(|p| bytes(&p.chunk.baked)[0]).sum::<u64>() / 2;
        let some = Easel::offscreen(Some(room)).pixels(&composed, PIXELS).2;
        let sums = chunks.iter().scan([0; 4], |sum, p| {
            sum.iter_mut().zip(bytes(&p.chunk.baked)).for_each(|(s, b)| *s += b);
            Some(*sum)
        });
        let fit = sums.take_while(|sum| sum.iter().all(|&s| s <= room)).count();
        assert!(fit > 0 && fit < chunks.len(), "{fit} of {} chunks fit", chunks.len());
        Arc::make_mut(&mut composed.frame.chunks).truncate(fit);
        let first = Easel::offscreen(None).pixels(&composed, PIXELS).2;
        assert!(some != all, "some left out");
        assert!(some == first, "what is drawn shows as it would alone: {}", differ(&some, &first));
    }
}
