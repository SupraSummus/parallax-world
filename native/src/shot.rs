//! A frame drawn offscreen and saved as a PNG, without a window.

use eframe::egui_wgpu::wgpu;

use crate::compositor::{Compositor, GpuChunk, GpuSplat, View};

/** egui's target format, so a shot looks as the window does. */
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

pub fn save(view: &View, chunks: &[GpuChunk], splats: &[GpuSplat], [width, height]: [u32; 2], path: &str) {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())).expect("adapter");
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).expect("device");
    let size = wgpu::Extent3d { width, height, depth_or_array_layers: 1 };
    let texture = |format, usage| {
        device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage,
            view_formats: &[],
        })
    };
    let target = texture(FORMAT, wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC);
    let depth = texture(wgpu::TextureFormat::Depth32Float, wgpu::TextureUsages::RENDER_ATTACHMENT);
    let compositor = Compositor::new(&device, FORMAT, chunks, splats);
    compositor.write(&queue, view, 1.0);

    // Rows of the copy are padded to 256 bytes.
    let row = (width * 4).div_ceil(256) * 256;
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: (row * height) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    {
        let target_view = target.create_view(&Default::default());
        let depth_view = depth.create_view(&Default::default());
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: None,
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &target_view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &depth_view,
                depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Discard }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        compositor.draw(&mut pass);
    }
    encoder.copy_texture_to_buffer(
        target.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: None },
        },
        size,
    );
    queue.submit([encoder.finish()]);
    buffer.slice(..).map_async(wgpu::MapMode::Read, |r| r.expect("map"));
    device.poll(wgpu::PollType::wait_indefinitely()).expect("poll");
    let mapped = buffer.slice(..).get_mapped_range().expect("mapped");
    let pixels: Vec<u8> = mapped.chunks(row as usize).flat_map(|r| &r[..(width * 4) as usize]).copied().collect();

    let mut encoder = png::Encoder::new(std::io::BufWriter::new(std::fs::File::create(path).expect("shot file")), width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header().expect("png").write_image_data(&pixels).expect("png");
}
