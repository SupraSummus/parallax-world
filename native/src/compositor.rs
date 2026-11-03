//! Draws a frame's chunks with wgpu, as compositor.ts does with WebGL 2,
//! inside egui's render pass (which has the depth buffer) or one of its own
//! for a shot: each chunk's splats instanced, then the sky behind them.
//!
//! A chunk's splats are uploaded when it comes into view, into one buffer
//! for all, and dropped when it leaves; each frame rewrites only a table of
//! the chunks' placements, relative to the camera, so precision does not
//! fall off far from the origin.

use std::collections::HashMap;
use std::ops::Range;

use eframe::egui_wgpu::{self, wgpu};

use crate::renderer::Frame;
use crate::view::{Vec3, to_local, view_basis};

/** A chunk as the shader reads it (Chunk in compositor.wgsl). */
#[repr(C)]
#[derive(Clone, Copy, Default, bytemuck::Pod, bytemuck::Zeroable)]
struct GpuChunk {
    /** Top-left corner of what it draws, relative to the camera, face coordinates. */
    origin: [f32; 2],
    extent: [f32; 2],
    distance: f32,
    face: u32,
    depth: f32,
    pad: f32,
}

/** A splat instance: its SPLAT numbers (chunk-baker.ts), then its chunk's slot, the top bit set if grown. */
type GpuSplat = [u32; 6];
const SPLAT_BYTES: u64 = std::mem::size_of::<GpuSplat>() as u64;

/** A chunk's splats on the GPU: where in the splat buffer, its slot in the table, and the frame it was last drawn in. */
struct Resident {
    splats: Range<u32>,
    slot: u32,
    drawn: u64,
}

pub struct Compositor {
    splats: wgpu::RenderPipeline,
    sky: wgpu::RenderPipeline,
    globals: wgpu::Buffer,
    layout: wgpu::BindGroupLayout,
    bind_group: wgpu::BindGroup,
    /** The chunk table, by slot. */
    table: wgpu::Buffer,
    slots: u32,
    free_slots: Vec<u32>,
    instances: wgpu::Buffer,
    /** Splats the buffer holds, the end of those in use, and free ranges below it. */
    capacity: u32,
    top: u32,
    free: Vec<Range<u32>>,
    resident: HashMap<u32, Resident>,
    /** Whether the buffer grew this frame: its copy runs after the frame's writes, so free ranges below it wait. */
    grew: bool,
    frames: u64,
    draws: Vec<Range<u32>>,
}

const SKY_TOP: [f32; 3] = [0x3f as f32 / 255.0, 0x73 as f32 / 255.0, 0xb8 as f32 / 255.0];
const SKY_HORIZON: [f32; 3] = [0xb9 as f32 / 255.0, 0xd3 as f32 / 255.0, 0xe8 as f32 / 255.0];
/** f32 in Globals (compositor.wgsl). */
const GLOBALS: usize = 224;

fn buffer(device: &wgpu::Device, label: &str, size: u64, usage: wgpu::BufferUsages) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor { label: Some(label), size, usage, mapped_at_creation: false })
}

fn table_buffer(device: &wgpu::Device, slots: u32) -> wgpu::Buffer {
    buffer(device, "chunks", slots as u64 * 32, wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST)
}

fn splat_buffer(device: &wgpu::Device, capacity: u32) -> wgpu::Buffer {
    buffer(
        device,
        "splats",
        capacity as u64 * SPLAT_BYTES,
        wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::COPY_SRC,
    )
}

fn bind(device: &wgpu::Device, layout: &wgpu::BindGroupLayout, globals: &wgpu::Buffer, table: &wgpu::Buffer) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout,
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: globals.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 1, resource: table.as_entire_binding() },
        ],
    })
}

impl Compositor {
    /** For a target of `format`, with a Depth32Float depth buffer. */
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Compositor {
        let shader = device.create_shader_module(wgpu::include_wgsl!("compositor.wgsl"));
        let globals = buffer(device, "globals", (GLOBALS * 4) as u64, wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST);
        let entry = |binding, ty| wgpu::BindGroupLayoutEntry { binding, visibility: wgpu::ShaderStages::VERTEX_FRAGMENT, ty, count: None };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &[
                entry(
                    0,
                    wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                ),
                entry(
                    1,
                    wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                ),
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = |vertex: &str, fragment: &str, buffers: &[Option<wgpu::VertexBufferLayout>]| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(vertex),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState { module: &shader, entry_point: Some(vertex), compilation_options: Default::default(), buffers },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(fragment),
                    compilation_options: Default::default(),
                    targets: &[Some(format.into())],
                }),
                primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleStrip, ..Default::default() },
                // Within a chunk, a later splat covers an earlier one at the same depth.
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: wgpu::TextureFormat::Depth32Float,
                    depth_write_enabled: Some(true),
                    depth_compare: Some(wgpu::CompareFunction::LessEqual),
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: Default::default(),
                multiview_mask: None,
                cache: None,
            })
        };
        let splat_layout = wgpu::VertexBufferLayout {
            array_stride: SPLAT_BYTES,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &wgpu::vertex_attr_array![0 => Float32x4, 1 => Unorm8x4, 2 => Uint32],
        };
        let (slots, capacity) = (1024, 1 << 20);
        let table = table_buffer(device, slots);
        Compositor {
            splats: pipeline("splat_vertex", "splat_fragment", &[Some(splat_layout)]),
            sky: pipeline("sky_vertex", "sky_fragment", &[]),
            bind_group: bind(device, &layout, &globals, &table),
            globals,
            layout,
            table,
            slots,
            free_slots: Vec::new(),
            instances: splat_buffer(device, capacity),
            capacity,
            top: 0,
            free: Vec::new(),
            resident: HashMap::new(),
            grew: false,
            frames: 0,
            draws: Vec::new(),
        }
    }

    /** `count` splats' room in the buffer, grown if need be. */
    fn allocate(&mut self, device: &wgpu::Device, encoder: &mut wgpu::CommandEncoder, count: u32) -> Range<u32> {
        if let Some(k) = self.free.iter().position(|r| r.len() >= count as usize).filter(|_| !self.grew) {
            let r = self.free[k].clone();
            if r.len() == count as usize {
                self.free.remove(k);
            } else {
                self.free[k].start += count;
            }
            return r.start..r.start + count;
        }
        if self.top + count > self.capacity {
            let capacity = (self.capacity * 2).max(self.top + count);
            let grown = splat_buffer(device, capacity);
            encoder.copy_buffer_to_buffer(&self.instances, 0, &grown, 0, self.top as u64 * SPLAT_BYTES);
            (self.instances, self.capacity, self.grew) = (grown, capacity, true);
        }
        self.top += count;
        self.top - count..self.top
    }

    /** Frees a range, merging it with free neighbours. */
    fn release(&mut self, r: Range<u32>) {
        let at = self.free.partition_point(|f| f.start < r.start);
        self.free.insert(at, r);
        if at + 1 < self.free.len() && self.free[at].end == self.free[at + 1].start {
            self.free[at].end = self.free.remove(at + 1).end;
        }
        if at > 0 && self.free[at - 1].end == self.free[at].start {
            self.free[at - 1].end = self.free.remove(at).end;
        }
        if let Some(last) = self.free.pop_if(|f| f.end == self.top) {
            self.top = last.start;
        }
    }

    /** Uploads what `frame` draws that is not on the GPU yet, drops what it no longer draws, and writes the chunk table and globals. */
    pub fn prepare(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, encoder: &mut wgpu::CommandEncoder, frame: &Frame, view: &View) {
        self.frames += 1;
        self.grew = false;
        let layers = frame.layers.len() as f32 + 1.5;
        let mut table = Vec::new();
        self.draws.clear();
        for (k, layer) in frame.layers.iter().enumerate() {
            // Layer k at (k + ½) / (L + 1.5), its grown splats half a layer behind, the sky behind all.
            let depth = (k as f32 + 0.5) / layers;
            for p in &layer.placements {
                let chunk = &p.chunk;
                if !self.resident.contains_key(&chunk.id) {
                    let count = (chunk.baked.numbers.len() / 5) as u32;
                    let splats = self.allocate(device, encoder, count);
                    let slot = self.free_slots.pop().unwrap_or_else(|| {
                        self.slots += 1;
                        self.slots - 1
                    });
                    let data: Vec<GpuSplat> = chunk
                        .baked
                        .numbers
                        .as_chunks::<5>()
                        .0
                        .iter()
                        .enumerate()
                        .map(|(s, n)| {
                            let grown = if s < chunk.baked.painted { 0 } else { 1 << 31 };
                            [n[0].to_bits(), n[1].to_bits(), n[2].to_bits(), n[3].to_bits(), n[4].to_bits(), slot | grown]
                        })
                        .collect();
                    queue.write_buffer(&self.instances, splats.start as u64 * SPLAT_BYTES, bytemuck::cast_slice(&data));
                    self.resident.insert(chunk.id, Resident { splats, slot, drawn: 0 });
                }
                let resident = self.resident.get_mut(&chunk.id).expect("resident");
                resident.drawn = self.frames;
                let slot = resident.slot as usize;
                if table.len() <= slot {
                    table.resize(slot + 1, GpuChunk::default());
                }
                let [width, height] = chunk.baked.extent;
                table[slot] = GpuChunk {
                    origin: [p.x as f32, p.y as f32],
                    extent: [width as f32, height as f32],
                    distance: layer.distance as f32,
                    face: layer.face as u32,
                    depth,
                    pad: 0.0,
                };
                self.draws.push(resident.splats.clone());
            }
        }
        // Neighbouring chunks in the splat buffer draw in one call.
        self.draws.sort_unstable_by_key(|r| r.start);
        self.draws.dedup_by(|next, last| {
            let joins = next.start <= last.end;
            if joins {
                last.end = last.end.max(next.end);
            }
            joins
        });
        let frames = self.frames;
        let gone: Vec<u32> = self.resident.iter().filter(|(_, r)| r.drawn < frames).map(|(id, _)| *id).collect();
        for id in gone {
            let r = self.resident.remove(&id).expect("resident");
            self.free_slots.push(r.slot);
            self.release(r.splats);
        }
        if self.slots as u64 * 32 > self.table.size() {
            self.table = table_buffer(device, self.slots.next_power_of_two());
            self.bind_group = bind(device, &self.layout, &self.globals, &self.table);
        }
        if !table.is_empty() {
            queue.write_buffer(&self.table, 0, bytemuck::cast_slice(&table));
        }
        let sky_fog = 1.0 - (-frame.farthest / view.fog_distance).exp();
        queue.write_buffer(&self.globals, 0, bytemuck::cast_slice(&view.globals(1.0 / layers, sky_fog)));
    }

    pub fn draw(&self, pass: &mut wgpu::RenderPass<'_>) {
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.set_pipeline(&self.splats);
        pass.set_vertex_buffer(0, self.instances.slice(..));
        for r in &self.draws {
            pass.draw(0..4, r.clone());
        }
        pass.set_pipeline(&self.sky);
        pass.draw(0..4, 0..1);
    }
}

/** How a frame is seen: orientation, focal length, fog, the faces' axes, and the view's rectangle in pixels of the target. */
#[derive(Clone, Copy)]
pub struct View {
    pub yaw: f64,
    pub pitch: f64,
    pub focal: f64,
    pub fog_distance: f64,
    pub faces: [[Vec3; 3]; 12],
    /** Left, top, width and height in points, and pixels per point. */
    pub rect: [f32; 4],
    pub pixels_per_point: f32,
}

impl View {
    /** `step`: the depth from one layer to the next. */
    fn globals(&self, step: f32, sky_fog: f64) -> [f32; GLOBALS] {
        let mut g = [0.0f32; GLOBALS];
        let [left, top, width, height] = self.rect;
        let ppp = self.pixels_per_point;
        g[0..2].copy_from_slice(&[width, height]);
        g[2..4].copy_from_slice(&[width * ppp, height * ppp]);
        g[4..6].copy_from_slice(&[left * ppp, top * ppp]);
        g[6] = self.focal as f32;
        g[7] = self.fog_distance as f32;
        g[8..11].copy_from_slice(&SKY_TOP);
        g[12..15].copy_from_slice(&SKY_HORIZON);
        let axes = view_basis(self.yaw, self.pitch);
        let columns = |g: &mut [f32], axes: [Vec3; 3]| {
            for (c, axis) in axes.iter().enumerate() {
                for d in 0..3 {
                    g[c * 4 + d] = axis[d] as f32;
                }
            }
        };
        columns(&mut g[16..28], axes);
        g[28] = 1.0 - step;
        g[29] = step / 2.0;
        g[30] = sky_fog as f32;
        for (f, face) in self.faces.iter().enumerate() {
            let at = 32 + f * 16;
            columns(&mut g[at..at + 12], axes.map(|axis| to_local(face, axis)));
        }
        g
    }
}

/** A frame for egui's render pass. */
pub struct Callback {
    pub frame: std::sync::Arc<Frame>,
    pub view: View,
}

impl egui_wgpu::CallbackTrait for Callback {
    fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        screen: &egui_wgpu::ScreenDescriptor,
        encoder: &mut wgpu::CommandEncoder,
        resources: &mut egui_wgpu::CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        let compositor: &mut Compositor = resources.get_mut().expect("compositor");
        let view = View { pixels_per_point: screen.pixels_per_point, ..self.view };
        compositor.prepare(device, queue, encoder, &self.frame, &view);
        Vec::new()
    }

    fn paint(&self, _info: egui::PaintCallbackInfo, pass: &mut wgpu::RenderPass<'static>, resources: &egui_wgpu::CallbackResources) {
        let compositor: &Compositor = resources.get().expect("compositor");
        compositor.draw(pass);
    }
}

use eframe::egui;
