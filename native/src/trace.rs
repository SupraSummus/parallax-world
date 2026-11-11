//! The slabs engine (a prototype): the view as rays walking each face's
//! slabs (trace.wgsl), not as splats. Per face in view and grid level, the band of depths the
//! level draws (as Lod picks levels for cells) is cut into slabs of
//! THICKNESS layers along the face's axis, and each slab is a picture of
//! its columns (bake's bake_slab): the first voxel of each and where its
//! run ends. Slabs are baked around a camera, with a margin, and drawn
//! from wherever the eye is: moving a little needs no new ones.
//!
//! A ray is traced for every FACTOR × FACTOR pixels; each pixel then takes
//! the nearest of the runs its four nearest samples met that its own ray
//! meets, so voxels keep sharp edges at full resolution; one no sample
//! covers takes its nearest sample's.

use std::sync::Arc;
use std::time::Instant;

use bake::slicing::{GRID_LEVELS, LAYER};
use bake::{Baker, Tiles};
use egui_wgpu::wgpu;

use crate::view::{self, Vec3};

/** Layers of its grid a slab is thick: thicker are slower, crossing as many columns in more texels. */
const THICKNESS: i32 = 16;
/** Pixels each way per ray traced. */
const FACTOR: u32 = 4;

/** A band (Band in trace.wgsl). */
#[repr(C)]
#[derive(Clone, Copy, Default, bytemuck::Pod, bytemuck::Zeroable)]
struct GpuBand {
    eye: [f32; 4],
    start: f32,
    end: f32,
    thickness: f32,
    /** Where its blocks begin: whether each block of BLOCK × BLOCK columns holds a voxel in any slab, then in each slab. */
    blocks: u32,
    columns: u32,
    rows: u32,
    slabs: u32,
    offset: u32,
}

/** Columns across a block (BLOCK in trace.wgsl). */
const BLOCK: usize = 8;

/**
 * Of a band's texels, `columns` × `rows` a slab, whether each block of
 * BLOCK × BLOCK columns and those a column around it holds a voxel: in any
 * slab, then in each.
 */
fn blocks(texels: &[[u32; 2]], [columns, rows]: [usize; 2], slabs: usize) -> Vec<u32> {
    let [across, up] = [columns.div_ceil(BLOCK), rows.div_ceil(BLOCK)];
    let mut blocks = vec![0; across * up * (slabs + 1)];
    for s in 0..slabs {
        for j in 0..rows {
            for i in 0..columns {
                if texels[(s * rows + j) * columns + i][1] == u32::MAX {
                    continue;
                }
                // The blocks it is in, or next to.
                for bj in (j.saturating_sub(1) / BLOCK)..=((j + 1) / BLOCK).min(up - 1) {
                    for bi in (i.saturating_sub(1) / BLOCK)..=((i + 1) / BLOCK).min(across - 1) {
                        blocks[bj * across + bi] = 1;
                        blocks[((s + 1) * up + bj) * across + bi] = 1;
                    }
                }
            }
        }
    }
    blocks
}

/**
 * How slabs are made: grid ℓ draws from 2Q · 2^(ℓ − 1 + shift) to
 * 2Q · 2^(ℓ + shift) layers ahead (grid 0 from the eye), as Lod's cells of
 * level ℓ + shift do, to `far`; and `margin` layers of grid 0 more around
 * the eye, behind it and to the sides, so that it may move that far.
 */
#[derive(Clone, Copy)]
pub struct Params {
    pub shift: i32,
    pub q: i32,
    pub far: f64,
    pub margin: f64,
}

impl Params {
    /** The layers of grid 0 ahead from which, and to which, grid `level` draws; None if it draws nothing. */
    fn band(&self, level: usize) -> Option<(f64, f64)> {
        let reach = |l: i32| (2 * self.q) as f64 * 2f64.powi(l + self.shift);
        let from = if level == 0 { -self.margin } else { reach(level as i32 - 1) };
        if from * LAYER > self.far {
            return None;
        }
        let n = (1 << level) as f64;
        let to = if level + 1 == GRID_LEVELS { self.far / LAYER } else { reach(level as i32).min(self.far / LAYER + 2.0 * n) };
        Some((from, to))
    }

    /** The grid levels slabs read. */
    pub fn levels(&self) -> [bool; GRID_LEVELS] {
        std::array::from_fn(|level| self.band(level).is_some())
    }
}

/** The faces a view shows, each with the bounds of x / z and y / z of its rays it shows (view::View::face_tangents). */
pub fn faces_in(view: &view::View) -> Vec<(usize, [f64; 4])> {
    view::faces().iter().enumerate().filter_map(|(f, face)| view.face_tangents(face).map(|t| (f, t))).collect()
}

/** The slabs of the faces around a camera: their bands, by face the first and how many, and all their texels. */
pub struct Slabs {
    bands: Vec<GpuBand>,
    /** Per band, its face, its grid level and its first column and slab, (a, b, c) in its grid's units. */
    origins: Vec<(usize, usize, [i32; 3])>,
    face_bands: [[u32; 2]; 12],
    texels: Vec<[u32; 2]>,
    blocks: Vec<u32>,
    /** Where they were baked around, and the faces and rays of each they hold. */
    pub camera: Vec3,
    pub faces: Vec<(usize, [f64; 4])>,
}

impl Slabs {
    /** Whether they hold all a view shows, `faces`. */
    pub fn hold(&self, faces: &[(usize, [f64; 4])]) -> bool {
        faces.iter().all(|(f, t)| self.faces.iter().any(|(g, h)| g == f && t[0] >= h[0] && t[1] <= h[1] && t[2] >= h[2] && t[3] <= h[3]))
    }

    /** Bytes they take. */
    pub fn bytes(&self) -> usize {
        self.texels.len() * 8 + self.blocks.len() * 4
    }

    /** The bands as the GPU takes them for an eye at `camera`. */
    fn bands_at(&self, camera: Vec3) -> Vec<GpuBand> {
        let faces = view::faces();
        self.bands
            .iter()
            .zip(&self.origins)
            .map(|(band, &(face, level, origin))| {
                let lattice = faces[face].lattice;
                let along = |v: [i32; 3]| (0..3).map(|d| camera[d] * v[d] as f64).sum::<f64>();
                let n = (1 << level) as f64;
                let eye = [along(lattice.r), along(lattice.u), along(lattice.f)].map(|v| v / n);
                GpuBand {
                    eye: [0, 1, 2]
                        .map(|d| (eye[d] - origin[d] as f64) as f32)
                        .into_iter()
                        .chain([n as f32])
                        .collect::<Vec<_>>()
                        .try_into()
                        .expect("4"),
                    ..*band
                }
            })
            .collect()
    }
}

/** What a band is, before it is baked. */
struct Plan {
    face: usize,
    level: usize,
    origin: [i32; 3],
    size: [usize; 2],
    slabs: usize,
    layers: [i32; 2],
    band: GpuBand,
}

/** The slabs of `faces` (each with the bounds of its rays to hold) around a camera at `camera`, on `threads` threads. */
pub fn bake(colours: &[u32], levels: &[Arc<Tiles>], camera: Vec3, faces: &[(usize, [f64; 4])], params: &Params, threads: usize) -> Slabs {
    let start = Instant::now();
    let all = view::faces();
    let mut plans = Vec::new();
    for &(f, [x0, x1, y0, y1]) in faces {
        let lattice = all[f].lattice;
        let along = |v: [i32; 3]| (0..3).map(|d| camera[d] * v[d] as f64).sum::<f64>();
        let eye0 = [along(lattice.r), along(lattice.u), along(lattice.f)];
        for level in 0..GRID_LEVELS {
            let Some((from, to)) = params.band(level) else { break };
            let n = (1 << level) as f64;
            let eye = eye0.map(|v| v / n);
            let (cs, ce) = ((eye[2] + from / n).floor() as i32, (eye[2] + to / n).ceil() as i32);
            let first = cs.div_euclid(THICKNESS);
            let slabs = (ce - first * THICKNESS + THICKNESS - 1).div_euclid(THICKNESS) as usize;
            let (depth, margin) = (ce as f64 - eye[2], 3.0 + params.margin / n);
            let even = |v: f64| (v / 2.0).floor() as i32 * 2;
            let a0 = even(eye[0] + x0.min(0.0) * depth - margin);
            let a1 = eye[0] + x1.max(0.0) * depth + margin;
            let b0 = even(eye[1] + y0.min(0.0) * LAYER * depth - margin);
            let b1 = eye[1] + y1.max(0.0) * LAYER * depth + margin;
            let size = [((a1 - a0 as f64) / 2.0).ceil() as usize + 1, (b1 - b0 as f64).ceil() as usize + 1];
            let c0 = first * THICKNESS;
            plans.push(Plan {
                face: f,
                level,
                origin: [a0, b0, c0],
                size,
                slabs,
                layers: [cs, ce],
                band: GpuBand {
                    start: (cs - c0) as f32,
                    end: (ce - c0) as f32,
                    thickness: THICKNESS as f32,
                    columns: size[0] as u32,
                    rows: size[1] as u32,
                    slabs: slabs as u32,
                    ..Default::default()
                },
            });
        }
    }
    // Each band's texels, baked on threads.
    let next = std::sync::atomic::AtomicUsize::new(0);
    let mut baked: Vec<(usize, Vec<[u32; 2]>)> = std::thread::scope(|scope| {
        let workers: Vec<_> = (0..threads.max(1))
            .map(|_| {
                scope.spawn(|| {
                    let mut baker = Baker::new(colours);
                    for (level, tiles) in levels.iter().enumerate() {
                        baker.share_tiles(level, tiles.clone());
                    }
                    let mut done = Vec::new();
                    loop {
                        let k = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        let Some(p) = plans.get(k) else { break };
                        let mut texels = Vec::with_capacity(p.size[0] * p.size[1] * p.slabs);
                        for s in 0..p.slabs as i32 {
                            let c0 = p.origin[2] + s * THICKNESS;
                            let layers = c0.max(p.layers[0])..(c0 + THICKNESS).min(p.layers[1]);
                            texels.extend(baker.bake_slab(p.face, p.level, [p.origin[0], p.origin[1], c0], p.size, layers));
                        }
                        done.push((k, texels));
                    }
                    done
                })
            })
            .collect();
        workers.into_iter().flat_map(|w| w.join().expect("baker")).collect()
    });
    baked.sort_by_key(|b| b.0);
    let mut slabs = Slabs {
        bands: Vec::new(),
        origins: Vec::new(),
        face_bands: [[0, 0]; 12],
        texels: Vec::new(),
        blocks: Vec::new(),
        camera,
        faces: faces.to_vec(),
    };
    for (p, (_, texels)) in plans.iter().zip(baked) {
        if slabs.face_bands[p.face][1] == 0 {
            slabs.face_bands[p.face][0] = slabs.bands.len() as u32;
        }
        slabs.face_bands[p.face][1] += 1;
        slabs.bands.push(GpuBand { offset: slabs.texels.len() as u32, blocks: slabs.blocks.len() as u32, ..p.band });
        slabs.origins.push((p.face, p.level, p.origin));
        slabs.blocks.extend(blocks(&texels, p.size, p.slabs));
        slabs.texels.extend(texels);
    }
    let full = slabs.texels.iter().filter(|t| t[1] != u32::MAX).count();
    println!(
        "slabs baked in {:.2} s: {} bands, {} texels ({:.1} MB), {:.1} % holding a voxel",
        start.elapsed().as_secs_f64(),
        slabs.bands.len(),
        slabs.texels.len(),
        slabs.bytes() as f64 / 1e6,
        100.0 * full as f64 / slabs.texels.len().max(1) as f64
    );
    slabs
}

/** Slabs on a GPU, and what draws them into a target of one format. */
pub struct TraceGpu {
    /** Sampling, then resolving. */
    sample: wgpu::RenderPipeline,
    resolve: wgpu::RenderPipeline,
    globals: wgpu::Buffer,
    /** The picture drawn last, and what shows it in egui (Shown). */
    show: wgpu::RenderPipeline,
    picture: Option<(wgpu::Texture, wgpu::BindGroup)>,
    /**
     * The slabs drawn, by version, and their bands, texels, faces' bands and
     * blocks on the GPU; and those being uploaded, UPLOAD bytes of texels a
     * frame (how many so far), drawn once all are in, so that no frame waits
     * for them all: wgpu uploads under a lock the window's thread needs too,
     * even from a thread of its own.
     */
    data: Option<(u64, Arc<Slabs>, [wgpu::Buffer; 4])>,
    uploading: Option<(u64, Arc<Slabs>, [wgpu::Buffer; 4], usize)>,
    /** For a target's size: the samples' textures and the passes' bind groups. */
    passes: Option<([u32; 2], Vec<wgpu::Texture>, Vec<wgpu::BindGroup>)>,
}

impl TraceGpu {
    /** Shows its picture on targets of `format`. */
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> TraceGpu {
        let shader = device.create_shader_module(wgpu::include_wgsl!("trace.wgsl"));
        let constants = [("factor", f64::from(FACTOR))];
        let pipeline = |entry: &str, formats: &[wgpu::TextureFormat]| {
            let targets: Vec<_> = formats.iter().map(|&f| Some(f.into())).collect();
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(entry),
                layout: None,
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("screen_vertex"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(entry),
                    compilation_options: wgpu::PipelineCompilationOptions { constants: &constants, ..Default::default() },
                    targets: &targets,
                }),
                primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleStrip, ..Default::default() },
                depth_stencil: None,
                multisample: Default::default(),
                multiview_mask: None,
                cache: None,
            })
        };
        let globals = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("trace globals"),
            size: (crate::compositor::GLOBALS * 4) as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        TraceGpu {
            sample: pipeline("sample_fragment", &[SAMPLE_RUNS, SAMPLE_HITS]),
            resolve: pipeline("resolve_fragment", &[FORMAT]),
            globals,
            show: pipeline("show_fragment", &[format]),
            picture: None,
            data: None,
            uploading: None,
            passes: None,
        }
    }

    /** The picture drawn last. */
    pub fn picture(&self) -> &wgpu::Texture {
        &self.picture.as_ref().expect("a picture").0
    }

    /** Draws the picture into egui's pass. */
    pub fn show(&self, pass: &mut wgpu::RenderPass<'static>) {
        if let Some((_, group)) = &self.picture {
            pass.set_pipeline(&self.show);
            pass.set_bind_group(0, group, &[]);
            pass.draw(0..4, 0..1);
        }
    }

    /** Draws `slabs` (uploaded anew when `version` changes) seen as `globals` says from `camera` into its picture of `pixels`. */
    #[allow(clippy::too_many_arguments)]
    pub fn draw(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        slabs: &Arc<Slabs>,
        version: u64,
        camera: Vec3,
        globals: &[f32],
        pixels: [u32; 2],
    ) {
        if self.picture.as_ref().is_none_or(|p| [p.0.width(), p.0.height()] != pixels) {
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("picture"),
                size: wgpu::Extent3d { width: pixels[0].max(1), height: pixels[1].max(1), depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: FORMAT,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });
            let view = texture.create_view(&Default::default());
            let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: None,
                layout: &self.show.get_bind_group_layout(0),
                entries: &[wgpu::BindGroupEntry { binding: 9, resource: wgpu::BindingResource::TextureView(&view) }],
            });
            self.picture = Some((texture, group));
        }
        let target = self.picture.as_ref().expect("a picture").0.clone();
        let target = &target;
        if self.data.as_ref().is_none_or(|d| d.0 != version) && self.uploading.as_ref().is_none_or(|u| u.0 != version) {
            self.uploading = Some((version, slabs.clone(), buffers(device, queue, slabs), 0));
        }
        if let Some((_, uploading, buffers, done)) = &mut self.uploading {
            let texels: &[u8] = bytemuck::cast_slice(&uploading.texels);
            // With nothing to draw yet, all at once.
            let end = if self.data.is_none() { texels.len() } else { (*done + UPLOAD).min(texels.len()) };
            queue.write_buffer(&buffers[1], *done as u64, &texels[*done..end]);
            *done = end;
            if end == texels.len() {
                let (version, uploaded, buffers, _) = self.uploading.take().expect("uploading");
                self.data = Some((version, uploaded, buffers));
                self.passes = None;
            }
        }
        let Some((_, drawn, _)) = &self.data else { return };
        let drawn = drawn.clone();
        let [width, height] = [target.width(), target.height()];
        if self.passes.as_ref().is_none_or(|p| p.0 != [width, height]) {
            let size = [width, height].map(|v| v.div_ceil(FACTOR));
            let samples: Vec<_> = [SAMPLE_RUNS, SAMPLE_HITS]
                .map(|format| {
                    device.create_texture(&wgpu::TextureDescriptor {
                        label: Some("samples"),
                        size: wgpu::Extent3d { width: size[0], height: size[1], depth_or_array_layers: 1 },
                        mip_level_count: 1,
                        sample_count: 1,
                        dimension: wgpu::TextureDimension::D2,
                        format,
                        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
                        view_formats: &[],
                    })
                })
                .to_vec();
            let buffers = &self.data.as_ref().expect("uploaded").2;
            // The samples' pass walks the slabs; the resolving one reads the texels the samples met, and the samples.
            let views: Vec<_> = samples.iter().map(|t| t.create_view(&Default::default())).collect();
            let all = [&self.globals, &buffers[0], &buffers[1], &buffers[2], &buffers[3]];
            let buffer = |k: usize| wgpu::BindGroupEntry { binding: k as u32, resource: all[k].as_entire_binding() };
            let sampled =
                (0..2).map(|k| wgpu::BindGroupEntry { binding: 5 + k, resource: wgpu::BindingResource::TextureView(&views[k as usize]) });
            let group = |pipeline: &wgpu::RenderPipeline, entries: Vec<wgpu::BindGroupEntry>| {
                device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: None,
                    layout: &pipeline.get_bind_group_layout(0),
                    entries: &entries,
                })
            };
            let groups = vec![
                group(&self.sample, (0..5).map(buffer).collect()),
                group(&self.resolve, [0, 2, 3].map(buffer).into_iter().chain(sampled).collect()),
            ];
            self.passes = Some(([width, height], samples, groups));
        }
        queue.write_buffer(&self.globals, 0, bytemuck::cast_slice(globals));
        let buffers = &self.data.as_ref().expect("uploaded").2;
        queue.write_buffer(&buffers[0], 0, bytemuck::cast_slice(&drawn.bands_at(camera)));
        let (_, samples, groups) = self.passes.as_ref().expect("made");
        let passes = [(&self.sample, &groups[0], samples.iter().collect::<Vec<_>>()), (&self.resolve, &groups[1], vec![target])];
        for (pipeline, group, targets) in passes {
            let views: Vec<_> = targets.iter().map(|t| t.create_view(&Default::default())).collect();
            let attachments: Vec<_> = views
                .iter()
                .map(|view| {
                    Some(wgpu::RenderPassColorAttachment {
                        view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store },
                    })
                })
                .collect();
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &attachments,
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, group, &[]);
            pass.draw(0..4, 0..1);
        }
    }
}

/** Bytes of texels uploaded a frame. */
const UPLOAD: usize = 16 << 20;

/** Buffers for slabs' bands, texels, faces' bands and blocks on the GPU, all but the texels written. */
fn buffers(device: &wgpu::Device, queue: &wgpu::Queue, slabs: &Slabs) -> [wgpu::Buffer; 4] {
    let contents: [&[u8]; 4] = [
        bytemuck::cast_slice(&slabs.bands),
        bytemuck::cast_slice(&slabs.texels),
        bytemuck::cast_slice(&slabs.face_bands),
        bytemuck::cast_slice(&slabs.blocks),
    ];
    let labels = ["bands", "texels", "face bands", "blocks"];
    std::array::from_fn(|k| {
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(labels[k]),
            size: (contents[k].len() as u64).max(4),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        if k != 1 {
            queue.write_buffer(&buffer, 0, contents[k]);
        }
        buffer
    })
}

/** What samples keep (Sample in trace.wgsl): their runs, and what they met. */
const SAMPLE_RUNS: wgpu::TextureFormat = wgpu::TextureFormat::Rgba32Float;
const SAMPLE_HITS: wgpu::TextureFormat = wgpu::TextureFormat::Rgba32Uint;

const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/** A device drawing slabs offscreen, for --shot and --bench. */
pub struct Tracer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    gpu: TraceGpu,
    slabs: Arc<Slabs>,
    globals: Vec<f32>,
    pixels: [u32; 2],
}

impl Tracer {
    pub fn new(slabs: Slabs, view: &crate::compositor::View, sky_fog: f64) -> Tracer {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: crate::compositor::BACKENDS,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())).expect("adapter");
        let descriptor = wgpu::DeviceDescriptor { required_limits: crate::compositor::limits(&adapter), ..Default::default() };
        let (device, queue) = pollster::block_on(adapter.request_device(&descriptor)).expect("device");
        let gpu = TraceGpu::new(&device, FORMAT);
        Tracer { gpu, slabs: Arc::new(slabs), globals: view.globals(view.pixels, sky_fog).to_vec(), pixels: view.pixels, device, queue }
    }

    fn render(&mut self) {
        let mut encoder = self.device.create_command_encoder(&Default::default());
        let camera = self.slabs.camera;
        self.gpu.draw(&self.device, &self.queue, &mut encoder, &self.slabs, 0, camera, &self.globals, self.pixels);
        self.queue.submit([encoder.finish()]);
    }

    fn wait(&self) {
        self.device.poll(wgpu::PollType::wait_indefinitely()).expect("poll");
    }

    /** The time the GPU takes to draw the frame again and again, in ms. */
    pub fn time(&mut self, runs: usize) -> f64 {
        for _ in 0..3 {
            self.render();
        }
        self.wait();
        let start = Instant::now();
        for _ in 0..runs {
            self.render();
        }
        self.wait();
        start.elapsed().as_secs_f64() * 1000.0 / runs as f64
    }

    /** Draws the frame and saves it as a PNG. */
    pub fn save(&mut self, path: &str) {
        self.render();
        let picture = self.gpu.picture();
        let wgpu::Extent3d { width, height, .. } = picture.size();
        let row = (width * 4).div_ceil(256) * 256;
        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: (row * height) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = self.device.create_command_encoder(&Default::default());
        let layout = wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: None };
        encoder.copy_texture_to_buffer(picture.as_image_copy(), wgpu::TexelCopyBufferInfo { buffer: &buffer, layout }, picture.size());
        self.queue.submit([encoder.finish()]);
        buffer.slice(..).map_async(wgpu::MapMode::Read, |r| r.expect("map"));
        self.wait();
        let mapped = buffer.slice(..).get_mapped_range().expect("mapped").to_vec();
        let pixels: Vec<u8> = mapped.chunks(row as usize).flat_map(|r| &r[..(width * 4) as usize]).copied().collect();
        let mut png = png::Encoder::new(std::io::BufWriter::new(std::fs::File::create(path).expect("shot file")), width, height);
        png.set_color(png::ColorType::Rgba);
        png.set_depth(png::BitDepth::Eight);
        png.write_header().expect("png").write_image_data(&pixels).expect("png");
    }
}

/** Shows the slabs' picture in egui. */
pub struct Shown;

impl egui_wgpu::CallbackTrait for Shown {
    fn paint(&self, _info: egui::PaintCallbackInfo, pass: &mut wgpu::RenderPass<'static>, resources: &egui_wgpu::CallbackResources) {
        let gpu: &TraceGpu = resources.get().expect("slabs");
        gpu.show(pass);
    }
}
