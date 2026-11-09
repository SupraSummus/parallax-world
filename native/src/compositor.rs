//! Draws a frame's chunks with wgpu, as compositor.ts does with WebGL 2,
//! into a picture of its own, which egui's render pass (or a shot) shows:
//! only the clusters of splats that may show, culled on the GPU against
//! the depth of what the last frame showed (compositor.wgsl), then the sky
//! behind them.
//!
//! A chunk's splats are uploaded when it comes into view, into one buffer
//! for all, in clusters of splats near each other on its plane, and dropped
//! when it leaves; each frame rewrites only a table of the chunks'
//! placements, relative to the camera, so precision does not fall off far
//! from the origin, and the list of their clusters.

use std::collections::{HashMap, HashSet};
use std::ops::Range;

use bake::light::{linear_to_srgb, srgb_to_linear};
use bake::slicing::half_layer;
use egui_wgpu::wgpu;

use crate::renderer::{Chunk, Frame};
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
    /** RGBA to paint its splats, shaded as their voxels are (Colours); 0 for their own colours. */
    tint: u32,
    /** How far a splat is past the plane per step of its alpha past 128. */
    half_layer: f32,
    _pad: f32,
}

/** A splat (splat_vertex's instance in compositor.wgsl): its SPLAT numbers (chunk-baker.ts), its height in place of its run, and its order in its chunk (depth_of). */
pub type GpuSplat = [u32; 6];

/** A cluster (Cluster in compositor.wgsl). */
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GpuCluster {
    min: [f32; 2],
    max: [f32; 2],
    first: u32,
    slot_count: u32,
    order: f32,
    visible: u32,
    near: f32,
    far: f32,
    wide: f32,
    _pad: u32,
}

/**
 * Splats at most in a cluster (its count is a byte), and points across the
 * blocks of a chunk's plane its clusters gather splats from, where the chunk
 * is when it comes into view: the fastest in the forest, between fewer
 * splats drawn and fewer clusters to cull and draw.
 */
const CLUSTER: usize = 16;
const BLOCK: f64 = 12.0;

/** The size of the blocks that cluster() gathers splats from, for a chunk on a plane `distance` ahead seen at `focal`. */
pub fn block(distance: f64, focal: f64) -> f64 {
    BLOCK * distance / focal
}

/**
 * A chunk's splats (far to near) as the GPU takes them, each with a run as
 * a row of splats (see rows), in clusters of the
 * splats in each block of size `block` on its plane (row by row, nearest
 * first in each, so that a block's first clusters are its front), and the
 * clusters, nearest first, so that splats are drawn near to far; their
 * `first` counts from the chunk's first splat, and their chunk's slot is
 * not in them yet. A cluster's box holds its splats' outlines along the
 * axis, from `near` to `far` past the plane, `half_layer` per step of their
 * alpha past 128, and how wide the widest is: the GPU finds where they are
 * drawn for its camera. In the bake workers, so that a chunk's upload is a
 * copy.
 */
pub fn cluster(numbers: &[f32], half_layer: f64, block: f64) -> (Vec<GpuSplat>, Vec<GpuCluster>) {
    let numbers = &rows(numbers.as_chunks::<5>().0)[..];
    let n = numbers.len();
    // Nearer the later painted.
    let orders: Vec<f32> = (0..n).map(|k| (n - k) as f32 / (n + 1) as f32).collect();
    let block_of = |s: usize| [1, 0].map(|d| (numbers[s][d] as f64 / block).floor() as i64);
    let mut sorted: Vec<usize> = (0..n).rev().collect();
    sorted.sort_by_key(|&s| block_of(s));
    let mut clusters = Vec::new();
    let mut start = 0;
    for run in sorted.chunk_by(|&a, &b| block_of(a) == block_of(b)) {
        // Split evenly.
        let parts = run.len().div_ceil(CLUSTER);
        for p in 0..parts {
            let part = &run[run.len() * p / parts..run.len() * (p + 1) / parts];
            let first = start + run.len() * p / parts;
            let mut cluster = GpuCluster {
                min: [f32::INFINITY; 2],
                max: [f32::NEG_INFINITY; 2],
                first: first as u32,
                slot_count: part.len() as u32,
                order: orders[part[0]],
                visible: 0,
                near: f32::INFINITY,
                far: f32::NEG_INFINITY,
                wide: 0.0,
                _pad: 0,
            };
            for &s in part {
                let [x, y, w, _, paint] = numbers[s];
                let h = w * std::f32::consts::SQRT_2;
                let off = (((paint.to_bits() >> 24) as f64 - 128.0) * half_layer) as f32;
                cluster.min = [cluster.min[0].min(x - w), cluster.min[1].min(y - h)];
                cluster.max = [cluster.max[0].max(x + w), cluster.max[1].max(y + h)];
                (cluster.near, cluster.far, cluster.wide) = (cluster.near.min(off), cluster.far.max(off), cluster.wide.max(w));
            }
            clusters.push(cluster);
        }
        start += run.len();
    }
    clusters.sort_by(|a, b| a.order.total_cmp(&b.order));
    // Its height in place of its run, drawn by rows if at all.
    let splats = sorted.iter().map(|&s| {
        let [x, y, w, _, paint] = numbers[s];
        [x, y, w, w * std::f32::consts::SQRT_2, paint, orders[s]].map(f32::to_bits)
    });
    (splats.collect(), clusters)
}

/** Half layers between a column's voxels. */
const VOXEL: i32 = 4;

/**
 * A chunk's splats (far to near) and, for each with a run (a
 * pillar along the axis, see chunk_splats in bake/src/splats.rs), a splat
 * at each voxel of it, the same but deeper; still far to near. A voxel apart, a row's
 * hexagons meet from every ray of the face, so a slanted ray meets the
 * pillar wherever its voxels are; farther apart, gaps open between them.
 */
fn rows(numbers: &[[f32; 5]]) -> Vec<[f32; 5]> {
    let mut all: Vec<(i32, [f32; 5])> = Vec::with_capacity(numbers.len());
    for &[x, y, w, run, paint] in numbers {
        let paint = paint.to_bits();
        let depth = (paint >> 24) as i32 - 128;
        // Its depth past the plane in the alpha byte, as the splat's.
        let at = |d: i32| f32::from_bits(paint & 0x00ff_ffff | ((128 + depth + d) as u32) << 24);
        all.extend((0..=run as i32).step_by(VOXEL as usize).map(|d| (depth + d, [x, y, w, 0.0, at(d)])));
    }
    // Far first; stable, so the chunk's order where depths tie.
    all.sort_by_key(|&(depth, _)| std::cmp::Reverse(depth));
    all.into_iter().map(|(_, s)| s).collect()
}

/** A chunk's splats and clusters on the GPU, and its slot in the table. */
struct Resident {
    splats: Range<u32>,
    clusters: Range<u32>,
    slot: u32,
}

/** A buffer of items `stride` bytes long, ranges of them allocated and freed. */
struct Arena {
    label: &'static str,
    stride: u64,
    usage: wgpu::BufferUsages,
    buffer: wgpu::Buffer,
    /** Items the buffer holds, the end of those in use, and free ranges below it. */
    capacity: u32,
    top: u32,
    free: Vec<Range<u32>>,
    /** Whether the buffer grew this frame: its copy runs after the frame's writes, so free ranges below it wait. */
    grew: bool,
}

impl Arena {
    fn new(device: &wgpu::Device, label: &'static str, stride: u64, usage: wgpu::BufferUsages, capacity: u32) -> Arena {
        let usage = usage | wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::COPY_SRC;
        let buffer = buffer(device, label, capacity as u64 * stride, usage);
        Arena { label, stride, usage, buffer, capacity, top: 0, free: Vec::new(), grew: false }
    }

    /** Room for frames of `items` as the camera moves: three times that when it has less than twice, so that it seldom grows. */
    fn room(&mut self, device: &wgpu::Device, encoder: &mut wgpu::CommandEncoder, items: u32) {
        if 2 * items > self.capacity {
            self.grow(device, encoder, 3 * items);
        }
    }

    /** Grown to `capacity` items, as far as the device allows, what it holds copied over. */
    fn grow(&mut self, device: &wgpu::Device, encoder: &mut wgpu::CommandEncoder, capacity: u32) {
        let limits = device.limits();
        let mut most = limits.max_buffer_size;
        if self.usage.contains(wgpu::BufferUsages::STORAGE) {
            most = most.min(limits.max_storage_buffer_binding_size);
        }
        let most = most / self.stride;
        let capacity = capacity.min(most.min(u32::MAX as u64) as u32);
        if capacity > self.capacity {
            let grown = buffer(device, self.label, capacity as u64 * self.stride, self.usage);
            encoder.copy_buffer_to_buffer(&self.buffer, 0, &grown, 0, self.top as u64 * self.stride);
            (self.buffer, self.capacity, self.grew) = (grown, capacity, true);
        }
    }

    /** `count` items' room in the buffer, grown if need be; none when the device allows no more. */
    fn allocate(&mut self, device: &wgpu::Device, encoder: &mut wgpu::CommandEncoder, count: u32) -> Option<Range<u32>> {
        if let Some(k) = self.free.iter().position(|r| r.len() >= count as usize).filter(|_| !self.grew) {
            let r = self.free[k].clone();
            if r.len() == count as usize {
                self.free.remove(k);
            } else {
                self.free[k].start += count;
            }
            return Some(r.start..r.start + count);
        }
        if self.top + count > self.capacity {
            self.grow(device, encoder, (self.capacity * 2).max(self.top + count));
        }
        (self.top + count <= self.capacity).then(|| {
            self.top += count;
            self.top - count..self.top
        })
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
}

/** What the compositor draws into, the size of the view in pixels. */
struct Targets {
    size: [u32; 2],
    picture: wgpu::Texture,
    depth: wgpu::Texture,
    /** The pyramid of farthest depths, and per level a bind group to make it and its size. */
    farthest: wgpu::Texture,
    levels: Vec<(wgpu::BindGroup, [u32; 2])>,
    show: wgpu::BindGroup,
}

/** The bind group layouts and pipelines (compositor.wgsl). */
struct Pipelines {
    draw: wgpu::BindGroupLayout,
    cull: wgpu::BindGroupLayout,
    reduce_depth: wgpu::BindGroupLayout,
    reduce: wgpu::BindGroupLayout,
    show: wgpu::BindGroupLayout,
    splats: wgpu::RenderPipeline,
    sky: wgpu::RenderPipeline,
    seams: wgpu::RenderPipeline,
    show_picture: wgpu::RenderPipeline,
    cull_drawn: wgpu::ComputePipeline,
    cull_rest: wgpu::ComputePipeline,
    reduce_depth_level: wgpu::ComputePipeline,
    reduce_level: wgpu::ComputePipeline,
}

pub struct Compositor {
    format: wgpu::TextureFormat,
    pipelines: Pipelines,
    globals: wgpu::Buffer,
    /** The chunk table, by slot. */
    table: wgpu::Buffer,
    slots: u32,
    free_slots: Vec<u32>,
    splats: Arena,
    clusters: Arena,
    resident: HashMap<u32, Resident>,
    /** The frame's chunks' clusters, [first, first + count) each. */
    candidates: wgpu::Buffer,
    chunks: u32,
    /** The clusters in view. */
    in_view: u32,
    /** By phase, the draws of clusters the culling lists, and how many. */
    draws: [wgpu::Buffer; 2],
    drawn: [wgpu::Buffer; 2],
    targets: Option<Targets>,
    seams: bool,
    /** What draws, and by phase what culls. */
    draw_group: Option<wgpu::BindGroup>,
    cull_groups: Vec<wgpu::BindGroup>,
}

const SKY_TOP: [f32; 3] = [0x3f as f32 / 255.0, 0x73 as f32 / 255.0, 0xb8 as f32 / 255.0];
const SKY_HORIZON: [f32; 3] = [0xb9 as f32 / 255.0, 0xd3 as f32 / 255.0, 0xe8 as f32 / 255.0];
/** f32 in Globals (compositor.wgsl). */
const GLOBALS: usize = 220;
const DEPTH: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
const FARTHEST: wgpu::TextureFormat = wgpu::TextureFormat::R32Float;

fn buffer(device: &wgpu::Device, label: &str, size: u64, usage: wgpu::BufferUsages) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor { label: Some(label), size, usage, mapped_at_creation: false })
}

fn storage(device: &wgpu::Device, label: &str, size: u64) -> wgpu::Buffer {
    buffer(device, label, size, wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST)
}

/** Replaces `buffer` if it holds fewer than `size` bytes, by one of the next power of two. */
fn at_least(device: &wgpu::Device, buffer: &mut wgpu::Buffer, size: u64) {
    if buffer.size() < size {
        *buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: size.next_power_of_two(),
            usage: buffer.usage(),
            mapped_at_creation: false,
        });
    }
}

/**
 * The backend the compositor draws with: on DX12, wgpu's indirect draws
 * counted on the GPU leave their first vertex out of the vertex index,
 * which carries the chunk's slot.
 */
pub const BACKENDS: wgpu::Backends = wgpu::Backends::VULKAN;

/** What the compositor needs of a device: draws listed on the GPU, a cluster's splats each. */
pub const FEATURES: wgpu::Features = wgpu::Features::INDIRECT_FIRST_INSTANCE.union(wgpu::Features::MULTI_DRAW_INDIRECT_COUNT);

/** What the compositor needs of a device: buffers as large as the adapter allows, for the splats in view. */
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

fn layout(device: &wgpu::Device, entries: &[(u32, wgpu::ShaderStages, wgpu::BindingType)]) -> wgpu::BindGroupLayout {
    let entries: Vec<_> =
        entries.iter().map(|&(binding, visibility, ty)| wgpu::BindGroupLayoutEntry { binding, visibility, ty, count: None }).collect();
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: None, entries: &entries })
}

fn bind(device: &wgpu::Device, layout: &wgpu::BindGroupLayout, entries: &[(u32, wgpu::BindingResource)]) -> wgpu::BindGroup {
    let entries: Vec<_> =
        entries.iter().map(|(binding, resource)| wgpu::BindGroupEntry { binding: *binding, resource: resource.clone() }).collect();
    device.create_bind_group(&wgpu::BindGroupDescriptor { label: None, layout, entries: &entries })
}

impl Pipelines {
    fn new(device: &wgpu::Device, format: wgpu::TextureFormat, culls: bool) -> Pipelines {
        use wgpu::ShaderStages as S;
        let shader = device.create_shader_module(wgpu::include_wgsl!("compositor.wgsl"));
        let buffer = |ty| wgpu::BindingType::Buffer { ty, has_dynamic_offset: false, min_binding_size: None };
        let uniform = buffer(wgpu::BufferBindingType::Uniform);
        let read = buffer(wgpu::BufferBindingType::Storage { read_only: true });
        let write = buffer(wgpu::BufferBindingType::Storage { read_only: false });
        let texture =
            |sample_type| wgpu::BindingType::Texture { sample_type, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false };
        let float = texture(wgpu::TextureSampleType::Float { filterable: false });
        let level_out = wgpu::BindingType::StorageTexture {
            access: wgpu::StorageTextureAccess::WriteOnly,
            format: FARTHEST,
            view_dimension: wgpu::TextureViewDimension::D2,
        };
        let draw = layout(device, &[(0, S::VERTEX_FRAGMENT, uniform), (1, S::VERTEX, read)]);
        let cull = layout(
            device,
            &[
                (0, S::COMPUTE, uniform),
                (1, S::COMPUTE, read),
                (4, S::COMPUTE, write),
                (5, S::COMPUTE, read),
                (6, S::COMPUTE, write),
                (7, S::COMPUTE, write),
                (8, S::COMPUTE, float),
            ],
        );
        let reduce_depth = layout(device, &[(10, S::COMPUTE, texture(wgpu::TextureSampleType::Depth)), (12, S::COMPUTE, level_out)]);
        let reduce = layout(device, &[(11, S::COMPUTE, float), (12, S::COMPUTE, level_out)]);
        let show = layout(device, &[(13, S::FRAGMENT, float)]);
        let pipeline_layout = |layout: &wgpu::BindGroupLayout| {
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: None,
                bind_group_layouts: &[Some(layout)],
                immediate_size: 0,
            })
        };
        let splat = wgpu::VertexBufferLayout {
            array_stride: size_of::<GpuSplat>() as u64,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &wgpu::vertex_attr_array![0 => Float32x4, 1 => Unorm8x4, 2 => Float32],
        };
        // Ties (the sky, cleared to 1) are drawn.
        let depth = |compare, write| wgpu::DepthStencilState {
            format: DEPTH,
            depth_write_enabled: Some(write),
            depth_compare: Some(compare),
            stencil: Default::default(),
            bias: Default::default(),
        };
        let (tested, over) = (Some(depth(wgpu::CompareFunction::LessEqual, true)), Some(depth(wgpu::CompareFunction::Always, false)));
        let render = |layout: &wgpu::BindGroupLayout,
                      vertex: &str,
                      fragment: &str,
                      buffers: &[Option<wgpu::VertexBufferLayout>],
                      depth_stencil: Option<wgpu::DepthStencilState>,
                      blend: Option<wgpu::BlendState>| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(fragment),
                layout: Some(&pipeline_layout(layout)),
                vertex: wgpu::VertexState { module: &shader, entry_point: Some(vertex), compilation_options: Default::default(), buffers },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(fragment),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState { format, blend, write_mask: wgpu::ColorWrites::ALL })],
                }),
                primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleStrip, ..Default::default() },
                depth_stencil,
                multisample: Default::default(),
                multiview_mask: None,
                cache: None,
            })
        };
        let compute = |layout: &wgpu::BindGroupLayout, entry: &str, constants: &[(&str, f64)]| {
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(entry),
                layout: Some(&pipeline_layout(layout)),
                module: &shader,
                entry_point: Some(entry),
                compilation_options: wgpu::PipelineCompilationOptions { constants, ..Default::default() },
                cache: None,
            })
        };
        Pipelines {
            splats: render(&draw, "splat_vertex", "splat_fragment", &[Some(splat)], tested.clone(), None),
            sky: render(&draw, "sky_vertex", "sky_fragment", &[], tested, None),
            seams: render(&draw, "sky_vertex", "seams_fragment", &[], over, Some(wgpu::BlendState::ALPHA_BLENDING)),
            show_picture: render(&show, "show_vertex", "show_fragment", &[], None, None),
            cull_drawn: compute(&cull, "cull_drawn", &[]),
            cull_rest: compute(&cull, "cull_rest", &[("culls", f64::from(culls))]),
            reduce_depth_level: compute(&reduce_depth, "reduce_depth", &[]),
            reduce_level: compute(&reduce, "reduce", &[]),
            draw,
            cull,
            reduce_depth,
            reduce,
            show,
        }
    }
}

impl Targets {
    fn new(device: &wgpu::Device, pipelines: &Pipelines, format: wgpu::TextureFormat, [width, height]: [u32; 2]) -> Targets {
        let texture = |format, usage, mip_level_count, width, height| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: None,
                size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
                mip_level_count,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage,
                view_formats: &[],
            })
        };
        use wgpu::TextureUsages as U;
        let picture = texture(format, U::RENDER_ATTACHMENT | U::TEXTURE_BINDING | U::COPY_SRC, 1, width, height);
        let depth = texture(DEPTH, U::RENDER_ATTACHMENT | U::TEXTURE_BINDING, 1, width, height);
        // Level 0 covers the view at 2 × 2 pixels a texel, in powers of two (so each next level halves the last), down to a texel.
        let mut sizes = vec![[width.div_ceil(2).next_power_of_two(), height.div_ceil(2).next_power_of_two()]];
        while let Some(&[w, h]) = sizes.last().filter(|s| s[0] > 1 || s[1] > 1) {
            sizes.push([(w / 2).max(1), (h / 2).max(1)]);
        }
        let farthest = texture(FARTHEST, U::STORAGE_BINDING | U::TEXTURE_BINDING, sizes.len() as u32, sizes[0][0], sizes[0][1]);
        let level = |k: usize| {
            farthest.create_view(&wgpu::TextureViewDescriptor { base_mip_level: k as u32, mip_level_count: Some(1), ..Default::default() })
        };
        let levels = sizes
            .iter()
            .enumerate()
            .map(|(k, &size)| {
                let group = if k == 0 {
                    let depth = depth.create_view(&Default::default());
                    bind(
                        device,
                        &pipelines.reduce_depth,
                        &[(10, wgpu::BindingResource::TextureView(&depth)), (12, wgpu::BindingResource::TextureView(&level(0)))],
                    )
                } else {
                    bind(
                        device,
                        &pipelines.reduce,
                        &[(11, wgpu::BindingResource::TextureView(&level(k - 1))), (12, wgpu::BindingResource::TextureView(&level(k)))],
                    )
                };
                (group, size)
            })
            .collect();
        let show = bind(device, &pipelines.show, &[(13, wgpu::BindingResource::TextureView(&picture.create_view(&Default::default())))]);
        Targets { size: [width, height], picture, depth, farthest, levels, show }
    }
}

impl Compositor {
    /** For a target of `format`; `culls` off, it draws every cluster in view (compositor.wgsl). */
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat, culls: bool) -> Compositor {
        use wgpu::BufferUsages as U;
        let draws = |label| buffer(device, label, 1 << 20, U::STORAGE | U::INDIRECT | U::COPY_SRC);
        let drawn = |label| buffer(device, label, 4, U::STORAGE | U::INDIRECT | U::COPY_DST | U::COPY_SRC);
        Compositor {
            format,
            pipelines: Pipelines::new(device, format, culls),
            globals: buffer(device, "globals", (GLOBALS * 4) as u64, wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST),
            table: storage(device, "chunks", 1024 * size_of::<GpuChunk>() as u64),
            slots: 0,
            free_slots: Vec::new(),
            splats: Arena::new(device, "splats", size_of::<GpuSplat>() as u64, wgpu::BufferUsages::VERTEX, 1 << 20),
            clusters: Arena::new(device, "clusters", size_of::<GpuCluster>() as u64, wgpu::BufferUsages::STORAGE, 1 << 15),
            resident: HashMap::new(),
            candidates: storage(device, "candidates", 1024 * 8),
            chunks: 0,
            in_view: 0,
            draws: [draws("draws first"), draws("draws next")],
            drawn: [drawn("drawn first"), drawn("drawn next")],
            targets: None,
            seams: false,
            draw_group: None,
            cull_groups: Vec::new(),
        }
    }

    /** The frame last drawn. */
    pub fn picture(&self) -> &wgpu::Texture {
        &self.targets.as_ref().expect("a frame").picture
    }

    /** By phase, the draws of clusters (how many splats each draws is its second word) and how many. */
    pub fn draws(&self) -> (&[wgpu::Buffer; 2], &[wgpu::Buffer; 2]) {
        (&self.draws, &self.drawn)
    }

    /** Uploads `chunk`'s splats and clusters (see cluster), unless the buffers can hold no more. */
    fn upload(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, encoder: &mut wgpu::CommandEncoder, chunk: &Chunk) -> bool {
        let baked = &chunk.baked;
        let Some(splats) = self.splats.allocate(device, encoder, baked.splats.len() as u32) else { return false };
        let Some(range) = self.clusters.allocate(device, encoder, baked.clusters.len() as u32) else {
            self.splats.release(splats);
            return false;
        };
        let slot = self.free_slots.pop().unwrap_or_else(|| {
            self.slots += 1;
            self.slots - 1
        });
        queue.write_buffer(&self.splats.buffer, splats.start as u64 * self.splats.stride, bytemuck::cast_slice(&baked.splats));
        let clusters: Vec<GpuCluster> = baked
            .clusters
            .iter()
            .map(|c| GpuCluster { first: splats.start + c.first, slot_count: slot << 8 | c.slot_count, ..*c })
            .collect();
        queue.write_buffer(&self.clusters.buffer, range.start as u64 * self.clusters.stride, bytemuck::cast_slice(&clusters));
        self.resident.insert(chunk.id, Resident { splats, clusters: range, slot });
        true
    }

    /** Uploads what `frame` draws that is not on the GPU yet, drops what it no longer draws, and writes the chunk table, the clusters to cull and globals. */
    pub fn prepare(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, encoder: &mut wgpu::CommandEncoder, frame: &Frame, view: &View) {
        (self.splats.grew, self.clusters.grew) = (false, false);
        let size = view.pixels.map(|v| v.max(1));
        self.seams = view.seams;
        if self.targets.as_ref().is_none_or(|t| t.size != size) {
            self.targets = Some(Targets::new(device, &self.pipelines, self.format, size));
        }
        let scale = depth_scale(frame.layers.len());
        let mut table = Vec::new();
        // What the frame no longer draws makes room first.
        let wanted: HashSet<u32> = frame.layers.iter().flat_map(|l| &l.placements).map(|p| p.chunk.id).collect();
        let gone: Vec<u32> = self.resident.keys().filter(|id| !wanted.contains(id)).copied().collect();
        for id in gone {
            let r = self.resident.remove(&id).expect("resident");
            self.free_slots.push(r.slot);
            self.splats.release(r.splats);
            self.clusters.release(r.clusters);
        }
        // Growing copies all a buffer holds, so grow seldom.
        let chunks = || frame.layers.iter().flat_map(|l| &l.placements).map(|p| &p.chunk.baked);
        self.splats.room(device, encoder, chunks().map(|b| b.splats.len() as u32).sum());
        self.clusters.room(device, encoder, chunks().map(|b| b.clusters.len() as u32).sum());
        let mut candidates: Vec<[u32; 2]> = Vec::new();
        let mut full = false;
        for (k, layer) in frame.layers.iter().enumerate() {
            // A hue per layer (Colours), neighbours in a stack far apart.
            let s = &layer.slice;
            let hue = hash(&[layer.face as u64, s.level as u64, s.cutoff.to_bits()]) + 0.618 * s.layer as f64;
            let depth = ((k as f64 - frame.layers.len() as f64) * scale as f64).exp2() as f32;
            for p in &layer.placements {
                let chunk = &p.chunk;
                // Layers go near to far: once the buffers are full, nothing farther is uploaded.
                if !self.resident.contains_key(&chunk.id) && (full || !self.upload(device, queue, encoder, chunk)) {
                    full = true;
                    continue;
                }
                let resident = &self.resident[&chunk.id];
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
                    tint: match view.colours {
                        Colours::Voxels => 0,
                        Colours::Layers => tint(hue),
                        Colours::Chunks => tint(hash(&[hue.to_bits(), chunk.i as u64, chunk.j as u64])),
                    },
                    half_layer: half_layer(s.atoms) as f32,
                    _pad: 0.0,
                };
                candidates.push([resident.clusters.start, resident.clusters.len() as u32]);
            }
        }
        at_least(device, &mut self.table, self.slots as u64 * size_of::<GpuChunk>() as u64);
        if !table.is_empty() {
            queue.write_buffer(&self.table, 0, bytemuck::cast_slice(&table));
        }
        at_least(device, &mut self.candidates, candidates.len().max(1) as u64 * 8);
        queue.write_buffer(&self.candidates, 0, bytemuck::cast_slice(&candidates));
        self.chunks = candidates.len() as u32;
        self.in_view = candidates.iter().map(|c| c[1]).sum();
        for draws in &mut self.draws {
            at_least(device, draws, self.in_view.max(1) as u64 * 16);
        }
        let sky_fog = 1.0 - (-frame.farthest / view.fog_distance).exp();
        queue.write_buffer(&self.globals, 0, bytemuck::cast_slice(&view.globals(size, scale, sky_fog)));
        self.bind_groups(device);
    }

    fn bind_groups(&mut self, device: &wgpu::Device) {
        let t = self.targets.as_ref().expect("targets");
        let farthest = t.farthest.create_view(&Default::default());
        use wgpu::BindingResource as R;
        self.draw_group =
            Some(bind(device, &self.pipelines.draw, &[(0, self.globals.as_entire_binding()), (1, self.table.as_entire_binding())]));
        self.cull_groups = (0..2)
            .map(|k| {
                let entries = [
                    (0, self.globals.as_entire_binding()),
                    (1, self.table.as_entire_binding()),
                    (4, self.clusters.buffer.as_entire_binding()),
                    (5, self.candidates.as_entire_binding()),
                    (6, self.draws[k].as_entire_binding()),
                    (7, self.drawn[k].as_entire_binding()),
                    (8, R::TextureView(&farthest)),
                ];
                bind(device, &self.pipelines.cull, &entries)
            })
            .collect();
    }

    /**
     * Draws the frame prepared last into the picture: the clusters that
     * showed last frame, then those that show now that did not, culled
     * against the farthest depths of the first.
     */
    pub fn render(&self, encoder: &mut wgpu::CommandEncoder) {
        let t = self.targets.as_ref().expect("prepared");
        let p = &self.pipelines;
        for drawn in &self.drawn {
            encoder.clear_buffer(drawn, 0, None);
        }
        let cull = |encoder: &mut wgpu::CommandEncoder, phase: usize| {
            let mut pass = encoder.begin_compute_pass(&Default::default());
            if phase == 1 {
                // The pyramid of what the first phase drew.
                for (k, (group, [width, height])) in t.levels.iter().enumerate() {
                    pass.set_pipeline(if k == 0 { &p.reduce_depth_level } else { &p.reduce_level });
                    pass.set_bind_group(0, group, &[]);
                    pass.dispatch_workgroups(width.div_ceil(8), height.div_ceil(8), 1);
                }
            }
            if self.chunks > 0 {
                pass.set_pipeline(if phase == 0 { &p.cull_drawn } else { &p.cull_rest });
                pass.set_bind_group(0, &self.cull_groups[phase], &[]);
                pass.dispatch_workgroups(self.chunks, 1, 1);
            }
        };
        let draw = |encoder: &mut wgpu::CommandEncoder, phase: usize| {
            let (colour, depth) = if phase == 0 {
                (wgpu::LoadOp::Clear(wgpu::Color::BLACK), wgpu::LoadOp::Clear(1.0))
            } else {
                (wgpu::LoadOp::Load, wgpu::LoadOp::Load)
            };
            let picture = t.picture.create_view(&Default::default());
            let depth_view = t.depth.create_view(&Default::default());
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &picture,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations { load: colour, store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &depth_view,
                    depth_ops: Some(wgpu::Operations { load: depth, store: wgpu::StoreOp::Store }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_bind_group(0, self.draw_group.as_ref(), &[]);
            pass.set_pipeline(&p.splats);
            pass.set_vertex_buffer(0, self.splats.buffer.slice(..));
            pass.multi_draw_indirect_count(&self.draws[phase], 0, &self.drawn[phase], 0, self.in_view);
            if phase == 1 {
                pass.set_pipeline(&p.sky);
                pass.draw(0..4, 0..1);
                if self.seams {
                    pass.set_pipeline(&p.seams);
                    pass.draw(0..4, 0..1);
                }
            }
        };
        cull(encoder, 0);
        draw(encoder, 0);
        cull(encoder, 1);
        draw(encoder, 1);
    }

    /** Shows the picture on the pass's viewport. */
    pub fn show(&self, pass: &mut wgpu::RenderPass<'_>) {
        let t = self.targets.as_ref().expect("prepared");
        pass.set_pipeline(&self.pipelines.show_picture);
        pass.set_bind_group(0, &t.show, &[]);
        pass.draw(0..4, 0..1);
    }
}

/** depth_scale (depth_of in compositor.wgsl) for `layers` layers: depths from 2^-120, far above f32's least normal, at most half an octave a layer. */
fn depth_scale(layers: usize) -> f32 {
    (120.0 / layers.max(1) as f32).min(0.5)
}

/** How a frame is seen: orientation, focal length, fog, the faces' axes, and the view's size in points and in pixels (the picture's). */
#[derive(Clone, Copy)]
pub struct View {
    pub yaw: f64,
    pub pitch: f64,
    pub focal: f64,
    pub fog_distance: f64,
    pub faces: [[Vec3; 3]; 12],
    pub size: [f32; 2],
    pub pixels: [u32; 2],
    /** Whether to draw the seams between faces over the picture. */
    pub seams: bool,
    pub colours: Colours,
    /** How bright the sky is: 1 by day, less as --light dims it. */
    pub sky: f32,
}

/** What splats are painted in: their voxels' colours, or to see how the picture is cut up, a colour per layer or per chunk. */
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

/** A number in [0, 1) from `numbers`, as if at random. */
fn hash(numbers: &[u64]) -> f64 {
    let h = numbers.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, &n| (h ^ n).wrapping_mul(0x0100_0000_01b3).rotate_left(29));
    (h.wrapping_mul(0x9e37_79b9_7f4a_7c15) >> 11) as f64 / (1u64 << 53) as f64
}

/** A bright colour of `hue` (in turns), as RGBA. */
fn tint(hue: f64) -> u32 {
    let channel = |n: f64| {
        let k = (n + hue.rem_euclid(1.0) * 6.0) % 6.0;
        (255.0 * (1.0 - 0.75 * k.min(4.0 - k).clamp(0.0, 1.0))) as u32
    };
    0xff00_0000 | channel(1.0) << 16 | channel(3.0) << 8 | channel(5.0)
}

impl View {
    fn globals(&self, pixels: [u32; 2], depth_scale: f32, sky_fog: f64) -> [f32; GLOBALS] {
        let mut g = [0.0f32; GLOBALS];
        g[0..2].copy_from_slice(&self.size);
        g[2..4].copy_from_slice(&pixels.map(|v| v as f32));
        g[4] = self.focal as f32;
        g[5] = self.fog_distance as f32;
        g[6] = depth_scale;
        g[7] = sky_fog as f32;
        // Dimmed in linear terms, as light adds up.
        let dim = |colour: [f32; 3]| colour.map(|c| linear_to_srgb(srgb_to_linear(c) * self.sky));
        g[8..11].copy_from_slice(&dim(SKY_TOP));
        g[12..15].copy_from_slice(&dim(SKY_HORIZON));
        let axes = view_basis(self.yaw, self.pitch);
        let columns = |g: &mut [f32], axes: [Vec3; 3]| {
            for (c, axis) in axes.iter().enumerate() {
                for d in 0..3 {
                    g[c * 4 + d] = axis[d] as f32;
                }
            }
        };
        columns(&mut g[16..28], axes);
        for (f, face) in self.faces.iter().enumerate() {
            let at = 28 + f * 16;
            columns(&mut g[at..at + 12], axes.map(|axis| to_local(face, axis)));
        }
        g
    }
}

/** egui's paint callback for where the window shows the picture drawn last. */
pub struct Shown;

impl egui_wgpu::CallbackTrait for Shown {
    fn paint(&self, _info: egui::PaintCallbackInfo, pass: &mut wgpu::RenderPass<'static>, resources: &egui_wgpu::CallbackResources) {
        let compositor: &Compositor = resources.get().expect("compositor");
        compositor.show(pass);
    }
}
