//! The slabs engine (a prototype): the view as rays walking each face's
//! slabs (slabs.wgsl), not as splats. Per face in view and grid level,
//! the band of depths the level draws (as Lod picks levels for cells) is cut into slabs of
//! THICKNESS layers along the face's axis, and each slab is a picture of
//! its columns (bake's bake_slab): the first voxel of each and where its
//! run ends. Slabs are baked around a camera, with a margin, and drawn
//! from wherever the eye is: moving a little needs no new ones. They are
//! baked on a thread of their own, again when the eye leaves their margin,
//! the view their faces, or tiles come; the slabs baked last are drawn
//! meanwhile. Till the eye first moves or turns, they are baked for the
//! view alone, with no margin: a third of the memory, and sooner.
//!
//! A ray is traced for every FACTOR × FACTOR pixels; each pixel then takes
//! the nearest of the runs its four nearest samples met that its own ray
//! meets, so voxels keep sharp edges at full resolution; one no sample
//! covers takes its nearest sample's.

use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::Instant;

use bake::Baker;
use bake::slicing::LAYER;
use egui_wgpu::wgpu;
use fcc::Tiles;
use fcc::{GRID_LEVELS, GRID_REACH};

use super::chunks::compositor::{GLOBALS, View};
use super::{Engine, Frame, Look, Painter, Picture};
use crate::gpu::Easel;
use crate::view::{self, Vec3, focal_length};
use crate::world::{Lit, Wants, World};

/** Layers of grid 0 around the eye slabs are baked with, once it moves. */
const MARGIN: f64 = 64.0;

/** What slabs to bake: around which eye, of which faces, from which tiles, how. */
type Bake = (Vec3, Vec<(usize, [f64; 4])>, Vec<Arc<Tiles>>, Params);

/**
 * The slabs engine: the slabs baked last and their version, the tiles and
 * how they were asked for last, whether those are baking, and whether
 * slabs are baked with room to move and turn.
 */
pub struct Slabs {
    slabs: Option<Arc<Baked>>,
    version: u64,
    asked: Option<(Vec<Arc<Tiles>>, Params)>,
    baking: bool,
    roomy: bool,
    ask: Sender<Bake>,
    done: Receiver<Baked>,
    /** Received by wait(), not taken yet. */
    early: Option<Baked>,
}

impl Slabs {
    /** Bakes `world`'s tiles on a thread of its own, with half as many bakers as the world has threads. */
    pub fn new(world: &World) -> Slabs {
        let (ask, asked) = channel::<Bake>();
        let (send, done) = channel();
        let (palette, threads) = (world.palette.clone(), (world.threads / 2).max(1));
        std::thread::spawn(move || {
            while let Ok(mut bake) = asked.recv() {
                // Only the latest.
                while let Ok(later) = asked.try_recv() {
                    bake = later;
                }
                let (camera, faces, levels, params) = bake;
                if send.send(self::bake(&palette, &levels, camera, &faces, &params, threads)).is_err() {
                    return;
                }
            }
        });
        Slabs { slabs: None, version: 0, asked: None, baking: false, roomy: false, ask, done, early: None }
    }
}

impl Engine for Slabs {
    fn frame(&mut self, look: &Look, world: &World) -> Frame {
        if let Some(slabs) = self.early.take().or_else(|| self.done.try_recv().ok()) {
            (self.slabs, self.version, self.baking) = (Some(Arc::new(slabs)), self.version + 1, false);
        }
        let view = look.view();
        let margin = |roomy| if roomy { MARGIN } else { 0.0 };
        let stale = self.slabs.as_ref().is_none_or(|s| {
            let moved = (0..3).map(|d| (look.camera[d] - s.camera[d]).powi(2)).sum::<f64>().sqrt();
            moved > margin(self.roomy) * LAYER / 2.0 || !s.hold(&faces_in(&view))
        });
        self.roomy |= stale && self.slabs.is_some();
        let params = Params::of(look, margin(self.roomy));
        let levels: Vec<_> = (0..GRID_LEVELS).map(|l| world.level(l).grids.clone()).collect();
        let changed =
            self.asked.as_ref().is_none_or(|(asked, p)| *p != params || asked.iter().zip(&levels).any(|(a, b)| !Arc::ptr_eq(a, b)));
        if !self.baking && (changed || stale) {
            // Room to turn.
            let wide = view::View { focal: focal_length((look.fov + 40.0).min(170.0), view.width, view.height), ..view };
            let faces = faces_in(if self.roomy { &wide } else { &view });
            if self.ask.send((look.camera, faces, levels.clone(), params)).is_ok() {
                (self.baking, self.asked) = (true, Some((levels, params)));
            }
        }
        let size = self.slabs.as_ref().map_or(0.0, |s| s.bytes() as f64 / 1e6);
        // The slabs show no light.
        let view = View { sky: 1.0, ..View::new(look) };
        let reach = params.levels().map(|read| read.then_some(GRID_REACH));
        Frame {
            picture: Box::new(Traced {
                slabs: self.slabs.clone(),
                version: self.version,
                eye: look.camera,
                view,
                // What of the sky the fog hides, as of something at the far end.
                sky_fog: 1.0 - (-1.0 / look.fog).exp(),
            }),
            // Slabs are baked all around the eye.
            wants: Wants { reach, lit: Lit::Not, tiles: world.lacking(&reach, look.camera, Lit::Not) },
            busy: self.baking,
            stats: format!("slabs v{} ({size:.0} MB){}", self.version, if self.baking { ", baking" } else { "" }),
        }
    }

    fn wait(&mut self) -> bool {
        if self.baking && self.early.is_none() {
            self.early = self.done.recv().ok();
        }
        self.baking
    }

    fn panel(&mut self, ui: &mut egui::Ui, look: &mut Look) {
        look.detail_panel(ui);
    }
}

/** The slabs baked last, if any, and their version, as seen from `eye`, and how much of the sky the fog hides. */
struct Traced {
    slabs: Option<Arc<Baked>>,
    version: u64,
    eye: Vec3,
    view: View,
    sky_fog: f64,
}

impl Picture for Traced {
    fn draw(&self, easel: &mut Easel, encoder: &mut wgpu::CommandEncoder, pixels: [u32; 2]) {
        let (tracer, device, queue) = easel.painter(Tracer::new);
        if let Some(slabs) = &self.slabs {
            tracer.draw(device, queue, encoder, slabs, self.version, self.eye, &self.view.globals(pixels, self.sky_fog), pixels);
        }
    }
}

/** Layers of its grid a slab is thick: thicker are slower, crossing as many columns in more texels. */
const THICKNESS: i32 = 16;
/** Pixels each way per ray traced. */
const FACTOR: u32 = 4;

/** A band (Band in slabs.wgsl). */
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

/** Columns across a block (BLOCK in slabs.wgsl). */
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
#[derive(Clone, Copy, PartialEq)]
struct Params {
    pub shift: i32,
    pub q: i32,
    pub far: f64,
    pub margin: f64,
}

impl Params {
    /** As `look`'s detail, shift and render distance say, with `margin` layers around the eye. */
    fn of(look: &Look, margin: f64) -> Params {
        let q = ((2.0 * look.detail).ceil() as i32).max(4);
        Params { shift: look.shift, q, far: look.far, margin }
    }

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
    fn levels(&self) -> [bool; GRID_LEVELS] {
        std::array::from_fn(|level| self.band(level).is_some())
    }
}

/** The faces a view shows, each with the bounds of x / z and y / z of its rays it shows (view::View::face_tangents). */
fn faces_in(view: &view::View) -> Vec<(usize, [f64; 4])> {
    view::faces().iter().enumerate().filter_map(|(f, face)| view.face_tangents(face).map(|t| (f, t))).collect()
}

/** The slabs of the faces around a camera: their bands, by face the first and how many, and all their texels. */
struct Baked {
    bands: Vec<GpuBand>,
    /** Per band, its face, its grid level and its first column and slab, (a, b, c) in its grid's units. */
    origins: Vec<(usize, usize, [i32; 3])>,
    face_bands: [[u32; 2]; 12],
    texels: Vec<[u32; 2]>,
    blocks: Vec<u32>,
    /** Where they were baked around, and the faces and rays of each they hold. */
    camera: Vec3,
    faces: Vec<(usize, [f64; 4])>,
}

impl Baked {
    /** Whether they hold all a view shows, `faces`: as far as rounding tells, where both reach a face's edge. */
    fn hold(&self, faces: &[(usize, [f64; 4])]) -> bool {
        let e = 1e-9;
        faces.iter().all(|(f, t)| {
            self.faces.iter().any(|(g, h)| g == f && t[0] >= h[0] - e && t[1] <= h[1] + e && t[2] >= h[2] - e && t[3] <= h[3] + e)
        })
    }

    /** Bytes they take. */
    fn bytes(&self) -> usize {
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
fn bake(colours: &[u32], levels: &[Arc<Tiles>], camera: Vec3, faces: &[(usize, [f64; 4])], params: &Params, threads: usize) -> Baked {
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
    let mut slabs = Baked {
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

/** Slabs on a GPU, and what draws them into a picture, shown on targets of one format. */
struct Tracer {
    /** Sampling, then resolving. */
    sample: wgpu::RenderPipeline,
    resolve: wgpu::RenderPipeline,
    globals: wgpu::Buffer,
    /** The picture drawn last, and what shows it. */
    show: wgpu::RenderPipeline,
    picture: Option<(wgpu::Texture, wgpu::BindGroup)>,
    /**
     * The slabs drawn, by version, and their bands, texels, faces' bands and
     * blocks on the GPU; and those being uploaded, UPLOAD bytes of texels a
     * frame (how many so far), drawn once all are in, so that no frame waits
     * for them all: wgpu uploads under a lock the window's thread needs too,
     * even from a thread of its own.
     */
    data: Option<(u64, Arc<Baked>, [wgpu::Buffer; 4])>,
    uploading: Option<(u64, Arc<Baked>, [wgpu::Buffer; 4], usize)>,
    /** For a target's size: the samples' textures and the passes' bind groups. */
    passes: Option<([u32; 2], Vec<wgpu::Texture>, Vec<wgpu::BindGroup>)>,
}

impl Tracer {
    /** Shows its picture on targets of `format`. */
    fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Tracer {
        let shader = device.create_shader_module(wgpu::include_wgsl!("slabs.wgsl"));
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
            size: (GLOBALS * 4) as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Tracer {
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

    /** Draws `slabs` (uploaded anew when `version` changes) seen as `globals` says from `camera` into its picture of `pixels`. */
    #[allow(clippy::too_many_arguments)]
    fn draw(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        slabs: &Arc<Baked>,
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
fn buffers(device: &wgpu::Device, queue: &wgpu::Queue, slabs: &Baked) -> [wgpu::Buffer; 4] {
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

/** What samples keep (Sample in slabs.wgsl): their runs, and what they met. */
const SAMPLE_RUNS: wgpu::TextureFormat = wgpu::TextureFormat::Rgba32Float;
const SAMPLE_HITS: wgpu::TextureFormat = wgpu::TextureFormat::Rgba32Uint;

/** The picture's, as egui's target. */
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

impl Painter for Tracer {
    fn picture(&self) -> &wgpu::Texture {
        &self.picture.as_ref().expect("a picture").0
    }

    fn show(&self, pass: &mut wgpu::RenderPass<'_>) {
        if let Some((_, group)) = &self.picture {
            pass.set_pipeline(&self.show);
            pass.set_bind_group(0, group, &[]);
            pass.draw(0..4, 0..1);
        }
    }
}
