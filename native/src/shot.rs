//! A frame drawn offscreen and saved as a PNG, or timed, without a window.

use std::time::Instant;

use eframe::egui_wgpu::wgpu;

use crate::compositor::{self, Compositor, View};
use crate::renderer::Frame;

/** egui's target format, so a shot looks as the window does. */
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/** A device and a compositor drawing offscreen. */
pub struct Offscreen {
    device: wgpu::Device,
    queue: wgpu::Queue,
    compositor: Compositor,
}

fn offscreen(frame: &Frame, view: &View) -> Offscreen {
    let mut o = Offscreen::new();
    o.prepare(frame, view);
    o
}

impl Offscreen {
    pub fn new() -> Offscreen {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: compositor::BACKENDS,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())).expect("adapter");
        let descriptor = wgpu::DeviceDescriptor {
            required_limits: compositor::limits(&adapter),
            required_features: compositor::FEATURES,
            ..Default::default()
        };
        let (device, queue) = pollster::block_on(adapter.request_device(&descriptor)).expect("device");
        let compositor = Compositor::new(&device, FORMAT);
        Offscreen { device, queue, compositor }
    }

    /** Uploads what `frame` needs (Compositor::prepare). */
    pub fn prepare(&mut self, frame: &Frame, view: &View) {
        let mut encoder = self.device.create_command_encoder(&Default::default());
        self.compositor.prepare(&self.device, &self.queue, &mut encoder, frame, view);
        self.queue.submit([encoder.finish()]);
    }

    /** Draws the frame prepared last. */
    pub fn render(&self) -> wgpu::SubmissionIndex {
        let mut encoder = self.device.create_command_encoder(&Default::default());
        self.compositor.render(&mut encoder);
        self.queue.submit([encoder.finish()])
    }

    pub fn wait(&self) {
        self.device.poll(wgpu::PollType::wait_indefinitely()).expect("poll");
    }

    /** Waits until the GPU is done with `submission`. */
    pub fn wait_for(&self, submission: wgpu::SubmissionIndex) {
        self.device.poll(wgpu::PollType::Wait { submission_index: Some(submission), timeout: None }).expect("poll");
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

    /** The first `size` bytes of `buffer`. */
    fn read_buffer(&self, buffer: &wgpu::Buffer, size: u64) -> Vec<u8> {
        self.read(size, |encoder, to| encoder.copy_buffer_to_buffer(buffer, 0, to, 0, size))
    }
}

/**
 * The time the GPU takes to draw `frame` again and again, in ms, as the
 * window does when moving (frames back to back, no vsync), once the
 * culling has settled; and how many splats the last drew in each phase.
 */
pub fn time(frame: &Frame, view: &View, runs: usize) -> (f64, [u32; 2]) {
    let o = offscreen(frame, view);
    for _ in 0..3 {
        o.render();
    }
    o.wait();
    let start = Instant::now();
    for _ in 0..runs {
        o.render();
    }
    o.wait();
    let ms = start.elapsed().as_secs_f64() * 1000.0 / runs as f64;
    let (draws, counts) = o.compositor.draws();
    let drawn = std::array::from_fn(|phase| {
        let n = u32::from_le_bytes(o.read_buffer(&counts[phase], 4).try_into().expect("count")) as u64;
        let draws = o.read_buffer(&draws[phase], (n * 16).max(16));
        draws.as_chunks::<16>().0[..n as usize].iter().map(|d| u32::from_le_bytes(d[4..8].try_into().expect("draw"))).sum()
    });
    (ms, drawn)
}

/** Draws `frame` as the window would after a few frames still, and saves it. */
pub fn save(frame: &Frame, view: &View, path: &str) {
    let o = offscreen(frame, view);
    for _ in 0..3 {
        o.render();
    }
    let picture = o.compositor.picture();
    let wgpu::Extent3d { width, height, .. } = picture.size();
    // Rows of the copy are padded to 256 bytes.
    let row = (width * 4).div_ceil(256) * 256;
    let mapped = o.read((row * height) as u64, |encoder, buffer| {
        let layout = wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: None };
        encoder.copy_texture_to_buffer(picture.as_image_copy(), wgpu::TexelCopyBufferInfo { buffer, layout }, picture.size())
    });
    let pixels: Vec<u8> = mapped.chunks(row as usize).flat_map(|r| &r[..(width * 4) as usize]).copied().collect();

    let mut encoder = png::Encoder::new(std::io::BufWriter::new(std::fs::File::create(path).expect("shot file")), width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header().expect("png").write_image_data(&pixels).expect("png");
}
