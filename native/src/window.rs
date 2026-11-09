//! The window: egui on winit and wgpu, put together here rather than by
//! eframe, which makes its GPU device once. A device can be lost (a hang
//! the driver resets, memory running out): then all that lives on it is
//! made again, and the window goes on from where it was, the world and
//! its baked chunks kept.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use egui_wgpu::wgpu;
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::{Window, WindowId};

use crate::compositor::{self, Compositor};
use crate::renderer::Frame;

/** What the window shows. */
pub trait Content {
    /** egui's ui for a frame. */
    fn ui(&mut self, ui: &mut egui::Ui);
    /** The frame the ui last asked to draw (the picture compositor::Shown shows), its view, and where it shows. */
    fn picture(&mut self) -> Option<(Arc<Frame>, compositor::View, egui::Rect)>;
    /** What it showed last, for a report. */
    fn describe(&self) -> String;
}

/** Shows `content` in a window until it is closed. */
pub fn run(title: &str, size: [f64; 2], content: impl Content) {
    let event_loop = EventLoop::new().expect("an event loop");
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: compositor::BACKENDS,
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    let mut windowed = Windowed { title: title.to_string(), size, content, instance, open: None, next: None };
    event_loop.run_app(&mut windowed).expect("the event loop");
}

struct Windowed<C> {
    title: String,
    size: [f64; 2],
    content: C,
    instance: wgpu::Instance,
    open: Option<Open>,
    /** When egui wants the next frame. */
    next: Option<Instant>,
}

/** A window open: it, egui's state for it, and its GPU. */
struct Open {
    window: Arc<Window>,
    egui: egui_winit::State,
    gpu: Gpu,
}

/** What lives on a GPU device: the device, the window's surface on it, egui's renderer and the compositor (in the renderer's callback resources). */
struct Gpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    renderer: egui_wgpu::Renderer,
    /** Set, with what wgpu said, once the device is lost. */
    lost: Arc<(AtomicBool, Mutex<String>)>,
}

/** The device is lost, wgpu says why. */
struct Lost(String);

impl Gpu {
    fn new(instance: &wgpu::Instance, window: &Arc<Window>) -> Gpu {
        let surface = instance.create_surface(window.clone()).expect("a surface");
        let options = wgpu::RequestAdapterOptions { compatible_surface: Some(&surface), ..Default::default() };
        let adapter = pollster::block_on(instance.request_adapter(&options)).expect("an adapter");
        let descriptor = wgpu::DeviceDescriptor {
            required_limits: compositor::limits(&adapter),
            required_features: compositor::FEATURES,
            ..Default::default()
        };
        let (device, queue) = pollster::block_on(adapter.request_device(&descriptor)).expect("a device");
        let lost = Arc::new((AtomicBool::new(false), Mutex::new(String::new())));
        let on_lost = lost.clone();
        device.set_device_lost_callback(move |reason, message| {
            if reason != wgpu::DeviceLostReason::Destroyed {
                *on_lost.1.lock().unwrap_or_else(|e| e.into_inner()) = message;
                on_lost.0.store(true, Ordering::SeqCst);
            }
        });
        let format = egui_wgpu::preferred_framebuffer_format(&surface.get_capabilities(&adapter).formats).expect("a surface format");
        let size = window.inner_size();
        let config = wgpu::SurfaceConfiguration {
            format,
            view_formats: vec![format],
            present_mode: wgpu::PresentMode::AutoNoVsync,
            ..surface.get_default_config(&adapter, size.width.max(1), size.height.max(1)).expect("the surface on this adapter")
        };
        surface.configure(&device, &config);
        let mut renderer = egui_wgpu::Renderer::new(&device, format, Default::default());
        renderer.callback_resources.insert(Compositor::new(&device, format, true));
        Gpu { device, queue, surface, config, renderer, lost }
    }

    fn check(&self) -> Result<(), Lost> {
        match self.lost.0.load(Ordering::SeqCst) {
            true => Err(Lost(self.lost.1.lock().unwrap_or_else(|e| e.into_inner()).clone())),
            false => Ok(()),
        }
    }

    fn resize(&mut self, width: u32, height: u32) {
        if width > 0 && height > 0 {
            (self.config.width, self.config.height) = (width, height);
            self.surface.configure(&self.device, &self.config);
        }
    }

    /** Draws the content's picture, then egui's `output` over it, and shows them. */
    fn draw(&mut self, window: &Window, content: &mut impl Content, output: egui::FullOutput, ctx: &egui::Context) -> Result<(), Lost> {
        let pixels_per_point = output.pixels_per_point;
        let screen = egui_wgpu::ScreenDescriptor { size_in_pixels: [self.config.width, self.config.height], pixels_per_point };
        self.check()?;
        if let Some((frame, view, rect)) = content.picture() {
            let viewport = egui::epaint::ViewportInPixels::from_points(&rect, pixels_per_point, screen.size_in_pixels);
            let view = compositor::View { pixels: [viewport.width_px as u32, viewport.height_px as u32], ..view };
            let compositor: &mut Compositor = self.renderer.callback_resources.get_mut().expect("compositor");
            let mut encoder = self.device.create_command_encoder(&Default::default());
            compositor.prepare(&self.device, &self.queue, &mut encoder, &frame, &view);
            compositor.render(&mut encoder);
            self.queue.submit([encoder.finish()]);
        }
        // A lost device shows when used: here, before egui's renderer uses it (it panics on a lost one).
        self.check()?;
        for (id, images) in &output.textures_delta.set {
            for image in images {
                self.renderer.update_texture(&self.device, &self.queue, *id, image);
            }
        }
        let jobs = ctx.tessellate(output.shapes, pixels_per_point);
        let mut encoder = self.device.create_command_encoder(&Default::default());
        let commands = self.renderer.update_buffers(&self.device, &self.queue, &mut encoder, &jobs, &screen);
        let texture = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(texture) | wgpu::CurrentSurfaceTexture::Suboptimal(texture) => Some(texture),
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                self.surface.configure(&self.device, &self.config);
                None
            }
            _ => None,
        };
        if let Some(texture) = &texture {
            let target = texture.texture.create_view(&Default::default());
            let mut pass = encoder
                .begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: None,
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &target,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                })
                .forget_lifetime();
            self.renderer.render(&mut pass, &jobs, &screen);
        }
        self.queue.submit(commands.into_iter().chain([encoder.finish()]));
        if let Some(texture) = texture {
            window.pre_present_notify();
            self.queue.present(texture);
        }
        for id in &output.textures_delta.free {
            self.renderer.free_texture(id);
        }
        self.check()
    }
}

impl<C: Content> Windowed<C> {
    /** The window's GPU, made anew (as egui's state, so that it uploads its textures again). */
    fn make(&self, window: Arc<Window>) -> Open {
        let gpu = Gpu::new(&self.instance, &window);
        let most = gpu.device.limits().max_texture_dimension_2d as usize;
        let egui = egui_winit::State::new(egui::Context::default(), egui::ViewportId::ROOT, &window, None, None, Some(most));
        Open { window, egui, gpu }
    }

    fn redraw(&mut self) {
        let Some(open) = &mut self.open else { return };
        let input = open.egui.take_egui_input(&open.window);
        let ctx = open.egui.egui_ctx().clone();
        let content = &mut self.content;
        let mut output = ctx.run_ui(input, |ui| content.ui(ui));
        open.egui.handle_platform_output(&open.window, std::mem::take(&mut output.platform_output));
        self.next = output.viewport_output.get(&egui::ViewportId::ROOT).and_then(|v| Instant::now().checked_add(v.repaint_delay));
        if let Err(Lost(message)) = open.gpu.draw(&open.window, &mut self.content, output, &ctx) {
            eprintln!("\nThe GPU device was lost: {message}");
            eprintln!("The driver may say why: journalctl -k (on Intel, a GPU HANG and its state in /sys/class/drm/card*/error).");
            eprintln!("The last frame:\n{}\nMaking the device again.\n", self.content.describe());
            let window = open.window.clone();
            // The old surface goes before the new one is made for the same window.
            self.open = None;
            self.open = Some(self.make(window));
            self.next = Some(Instant::now());
        }
    }
}

impl<C: Content> ApplicationHandler for Windowed<C> {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.open.is_none() {
            let [width, height] = self.size;
            let attributes =
                Window::default_attributes().with_title(&self.title).with_inner_size(winit::dpi::LogicalSize::new(width, height));
            let window = Arc::new(event_loop.create_window(attributes).expect("a window"));
            self.open = Some(self.make(window));
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        let Some(open) = &mut self.open else { return };
        let response = open.egui.on_window_event(&open.window, &event);
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => open.gpu.resize(size.width, size.height),
            // egui asks for a repaint after one too: when, it says in its output (redraw).
            WindowEvent::RedrawRequested => return self.redraw(),
            _ => {}
        }
        if response.repaint {
            self.next = Some(Instant::now());
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        match self.next {
            Some(next) if next <= Instant::now() => {
                if let Some(open) = &self.open {
                    open.window.request_redraw();
                }
                self.next = None;
                event_loop.set_control_flow(ControlFlow::Wait);
            }
            Some(next) => event_loop.set_control_flow(ControlFlow::WaitUntil(next)),
            None => event_loop.set_control_flow(ControlFlow::Wait),
        }
    }
}
