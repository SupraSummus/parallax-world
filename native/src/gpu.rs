//! Where engines' pictures are drawn: what a device must allow, and an
//! Easel, a device with the painter of the engine that drew on it last,
//! in the window or offscreen, where it also times a picture or saves it.

use std::any::Any;
use std::time::Instant;

use egui_wgpu::wgpu;

use crate::engine::{Painter, Picture};

/**
 * The backend engines draw with: on DX12, wgpu's indirect draws counted on
 * the GPU leave their first instance out of the instance index, which
 * carries the tile, and their first vertex out of the vertex index, which
 * carries a splat's chunk's slot (chunks' compositor.rs).
 */
pub const BACKENDS: wgpu::Backends = wgpu::Backends::VULKAN;

/**
 * What the chunks' compositor needs of a device: draws listed on the GPU,
 * a tile or a cluster's splats each; depth tested before tiles' pixels
 * march, though they write it (compositor.wgsl); see-through splats
 * added up in 32-bit floats; and flat pictures cut to their faces' rays.
 */
pub const FEATURES: wgpu::Features = wgpu::Features::INDIRECT_FIRST_INSTANCE
    .union(wgpu::Features::MULTI_DRAW_INDIRECT_COUNT)
    .union(wgpu::Features::SHADER_EARLY_DEPTH_TEST)
    .union(wgpu::Features::FLOAT32_BLENDABLE)
    .union(wgpu::Features::CLIP_DISTANCES);

/** What engines need of a device: buffers as large as the adapter allows, for what is in view. */
pub fn limits(adapter: &wgpu::Adapter) -> wgpu::Limits {
    let adapter = adapter.limits();
    wgpu::Limits {
        max_buffer_size: adapter.max_buffer_size,
        max_storage_buffer_binding_size: adapter.max_storage_buffer_binding_size,
        // egui's, for surfaces up to 8K.
        max_texture_dimension_2d: 8192,
        ..Default::default()
    }
}

/** egui's target format, so that a shot looks as the window does. */
const OFFSCREEN: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/** A device to draw pictures on, the format they are shown in, and the painter of the engine whose picture it drew last. */
pub struct Easel {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub format: wgpu::TextureFormat,
    pub painter: Option<Box<dyn Painter>>,
}

impl Easel {
    pub fn new(device: wgpu::Device, queue: wgpu::Queue, format: wgpu::TextureFormat) -> Easel {
        Easel { device, queue, format, painter: None }
    }

    /** One of its own, offscreen, with buffers as large as its device allows, or `most` bytes. */
    pub fn offscreen(most: Option<u64>) -> Easel {
        let instance =
            wgpu::Instance::new(wgpu::InstanceDescriptor { backends: BACKENDS, ..wgpu::InstanceDescriptor::new_without_display_handle() });
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())).expect("adapter");
        let mut limits = limits(&adapter);
        if let Some(most) = most {
            limits.max_buffer_size = limits.max_buffer_size.min(most);
        }
        let descriptor = wgpu::DeviceDescriptor { required_limits: limits, required_features: FEATURES, ..Default::default() };
        let (device, queue) = pollster::block_on(adapter.request_device(&descriptor)).expect("device");
        Easel::new(device, queue, OFFSCREEN)
    }

    /** Its painter, if a `P`. */
    pub fn held<P: Painter>(&self) -> Option<&P> {
        self.painter.as_deref().and_then(|p| (p as &dyn Any).downcast_ref())
    }

    /** Its painter, if a `P`, else one `make` makes there for its device and format; and its device and queue. */
    pub fn painter<P: Painter>(
        &mut self,
        make: impl FnOnce(&wgpu::Device, wgpu::TextureFormat) -> P,
    ) -> (&mut P, &wgpu::Device, &wgpu::Queue) {
        if self.held::<P>().is_none() {
            self.painter = Some(Box::new(make(&self.device, self.format)));
        }
        let Easel { device, queue, painter, .. } = self;
        let painter = painter.as_deref_mut().expect("a painter") as &mut dyn Any;
        (painter.downcast_mut().expect("held"), device, queue)
    }

    /** Draws `picture` into a picture of `pixels`, without waiting for the GPU. */
    pub fn draw(&mut self, picture: &dyn Picture, pixels: [u32; 2]) -> wgpu::SubmissionIndex {
        let mut encoder = self.device.create_command_encoder(&Default::default());
        picture.draw(self, &mut encoder, pixels);
        self.queue.submit([encoder.finish()])
    }

    /** Shows the picture drawn last on `pass`'s viewport. */
    pub fn show(&self, pass: &mut wgpu::RenderPass<'_>) {
        if let Some(painter) = &self.painter {
            painter.show(pass);
        }
    }

    /** What a bench says of the frame drawn last (Painter::describe). */
    pub fn describe(&self) -> String {
        self.painter.as_ref().map_or_else(String::new, |p| p.describe(self))
    }

    pub fn wait(&self) {
        self.device.poll(wgpu::PollType::wait_indefinitely()).expect("poll");
    }

    /** Waits until the GPU is done with `submission`. */
    pub fn wait_for(&self, submission: wgpu::SubmissionIndex) {
        self.device.poll(wgpu::PollType::Wait { submission_index: Some(submission), timeout: None }).expect("poll");
    }

    /**
     * The time `picture` takes to draw again and again, in ms, uploads and
     * all, as the window draws it when still (frames back to back, no
     * vsync), once what it draws has settled (the chunks' culling).
     */
    pub fn time(&mut self, picture: &dyn Picture, pixels: [u32; 2], runs: usize) -> f64 {
        for _ in 0..3 {
            self.draw(picture, pixels);
        }
        self.wait();
        let start = Instant::now();
        for _ in 0..runs {
            self.draw(picture, pixels);
        }
        self.wait();
        start.elapsed().as_secs_f64() * 1000.0 / runs as f64
    }

    /** `picture` drawn as the window shows it after a few frames still: its width, height and RGBA pixels, row by row from the top. */
    pub fn pixels(&mut self, picture: &dyn Picture, pixels: [u32; 2]) -> (u32, u32, Vec<u8>) {
        for _ in 0..3 {
            self.draw(picture, pixels);
        }
        let texture = self.painter.as_ref().expect("a painter").picture();
        let wgpu::Extent3d { width, height, .. } = texture.size();
        // Rows of the copy are padded to 256 bytes.
        let row = (width * 4).div_ceil(256) * 256;
        let mapped = self.read((row * height) as u64, |encoder, buffer| {
            let layout = wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: None };
            encoder.copy_texture_to_buffer(texture.as_image_copy(), wgpu::TexelCopyBufferInfo { buffer, layout }, texture.size())
        });
        (width, height, mapped.chunks(row as usize).flat_map(|r| &r[..(width * 4) as usize]).copied().collect())
    }

    /** Draws `picture` as `pixels` does, and saves it as a PNG at `path`. */
    pub fn save(&mut self, picture: &dyn Picture, pixels: [u32; 2], path: &str) {
        let (width, height, pixels) = self.pixels(picture, pixels);
        let mut png = png::Encoder::new(std::io::BufWriter::new(std::fs::File::create(path).expect("shot file")), width, height);
        png.set_color(png::ColorType::Rgba);
        png.set_depth(png::BitDepth::Eight);
        png.write_header().expect("png").write_image_data(&pixels).expect("png");
    }

    /** `size` bytes that `copy` copies into a buffer, once the GPU is done. */
    fn read(&self, size: u64, copy: impl FnOnce(&mut wgpu::CommandEncoder, &wgpu::Buffer)) -> Vec<u8> {
        let usage = wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ;
        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor { label: None, size, usage, mapped_at_creation: false });
        let mut encoder = self.device.create_command_encoder(&Default::default());
        copy(&mut encoder, &buffer);
        self.queue.submit([encoder.finish()]);
        buffer.slice(..).map_async(wgpu::MapMode::Read, |r| r.expect("map"));
        self.wait();
        buffer.slice(..).get_mapped_range().expect("mapped").to_vec()
    }

    /** The first `size` bytes of `buffer`, once the GPU is done. */
    pub fn read_buffer(&self, buffer: &wgpu::Buffer, size: u64) -> Vec<u8> {
        self.read(size, |encoder, to| encoder.copy_buffer_to_buffer(buffer, 0, to, 0, size))
    }
}
