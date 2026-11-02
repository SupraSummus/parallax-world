//! Draws baked chunks with wgpu, as compositor.ts does with WebGL 2, inside
//! egui's render pass (which has the depth buffer): every splat of the
//! frame in one instanced draw, then the sky behind them.

use eframe::egui_wgpu::{self, wgpu};
use wgpu::util::DeviceExt;

use crate::view::{Vec3, to_local, view_basis};

/** A chunk as the shader reads it (Chunk in compositor.wgsl). */
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GpuChunk {
    pub origin: [f32; 2],
    pub extent: [f32; 2],
    pub centre: f32,
    pub face: u32,
    pub depth: f32,
    pub pad: f32,
}

/** A splat instance: SPLAT numbers (chunk-baker.ts), then its chunk's index, the top bit set if grown. */
pub type GpuSplat = [u32; 6];

pub struct Compositor {
    splats: wgpu::RenderPipeline,
    sky: wgpu::RenderPipeline,
    globals: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    instances: wgpu::Buffer,
    count: u32,
}

const SKY_TOP: [f32; 3] = [0x3f as f32 / 255.0, 0x73 as f32 / 255.0, 0xb8 as f32 / 255.0];
const SKY_HORIZON: [f32; 3] = [0xb9 as f32 / 255.0, 0xd3 as f32 / 255.0, 0xe8 as f32 / 255.0];
/** f32 in Globals (compositor.wgsl). */
const GLOBALS: usize = 224;

impl Compositor {
    pub fn new(render_state: &egui_wgpu::RenderState, chunks: &[GpuChunk], splats: &[GpuSplat]) -> Compositor {
        let device = &render_state.device;
        let shader = device.create_shader_module(wgpu::include_wgsl!("compositor.wgsl"));
        let globals = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("globals"),
            size: (GLOBALS * 4) as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        // A storage buffer may not be empty.
        let chunk_bytes: &[u8] = if chunks.is_empty() { &[0; 32] } else { bytemuck::cast_slice(chunks) };
        let chunk_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("chunks"),
            contents: chunk_bytes,
            usage: wgpu::BufferUsages::STORAGE,
        });
        let splat_bytes: &[u8] = if splats.is_empty() { &[0; 24] } else { bytemuck::cast_slice(splats) };
        let instances = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("splats"),
            contents: splat_bytes,
            usage: wgpu::BufferUsages::VERTEX,
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: globals.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: chunk_buffer.as_entire_binding() },
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
                    targets: &[Some(render_state.target_format.into())],
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
            array_stride: 24,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &wgpu::vertex_attr_array![0 => Float32x4, 1 => Unorm8x4, 2 => Uint32],
        };
        Compositor {
            splats: pipeline("splat_vertex", "splat_fragment", &[Some(splat_layout)]),
            sky: pipeline("sky_vertex", "sky_fragment", &[]),
            globals,
            bind_group,
            instances,
            count: splats.len() as u32,
        }
    }
}

/** What a frame is drawn from: the camera, the faces' axes and the layers' count. */
#[derive(Clone)]
pub struct View {
    pub camera: Vec3,
    pub yaw: f64,
    pub pitch: f64,
    pub focal: f64,
    pub fog_distance: f64,
    pub sky_fog: f64,
    pub layers: usize,
    pub faces: [[Vec3; 3]; 12],
    /** The view's rectangle, in points. */
    pub rect: eframe::egui::Rect,
}

impl View {
    fn globals(&self, pixels_per_point: f32) -> [f32; GLOBALS] {
        let mut g = [0.0f32; GLOBALS];
        let size = self.rect.size();
        g[0..2].copy_from_slice(&[size.x, size.y]);
        g[2..4].copy_from_slice(&[size.x * pixels_per_point, size.y * pixels_per_point]);
        g[4..6].copy_from_slice(&[self.rect.min.x * pixels_per_point, self.rect.min.y * pixels_per_point]);
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
        // Layer i at (i + ½) / (L + 1.5), its grown splats half a layer behind, the sky behind all.
        let layers = self.layers as f32 + 1.5;
        g[28] = (self.layers as f32 + 0.5) / layers;
        g[29] = 0.5 / layers;
        g[30] = self.sky_fog as f32;
        for (f, face) in self.faces.iter().enumerate() {
            let at = 32 + f * 16;
            columns(&mut g[at..at + 12], axes.map(|axis| to_local(face, axis)));
            let camera = to_local(face, self.camera);
            g[at + 12..at + 15].copy_from_slice(&camera.map(|v| v as f32));
        }
        g
    }
}

impl egui_wgpu::CallbackTrait for View {
    fn prepare(
        &self,
        _device: &wgpu::Device,
        queue: &wgpu::Queue,
        screen: &egui_wgpu::ScreenDescriptor,
        _encoder: &mut wgpu::CommandEncoder,
        resources: &mut egui_wgpu::CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        let compositor: &Compositor = resources.get().expect("compositor");
        queue.write_buffer(&compositor.globals, 0, bytemuck::cast_slice(&self.globals(screen.pixels_per_point)));
        Vec::new()
    }

    fn paint(&self, _info: egui::PaintCallbackInfo, pass: &mut wgpu::RenderPass<'static>, resources: &egui_wgpu::CallbackResources) {
        let compositor: &Compositor = resources.get().expect("compositor");
        pass.set_bind_group(0, &compositor.bind_group, &[]);
        pass.set_pipeline(&compositor.splats);
        pass.set_vertex_buffer(0, compositor.instances.slice(..));
        pass.draw(0..4, 0..compositor.count);
        pass.set_pipeline(&compositor.sky);
        pass.draw(0..4, 0..1);
    }
}

use eframe::egui;
