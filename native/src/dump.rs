//! A view dumped by the spike's TS script: the camera, the faces, the
//! baker's tables and grids as chunk-baker.ts hands them over, and the
//! chunks the renderer wants.

use std::sync::Arc;

use bake::{Geometry, Grid, Lattice, VIEW_FACES};

pub struct Request {
    pub face: usize,
    pub level: i32,
    pub layer: i32,
    pub cutoff: f64,
    pub i: i32,
    pub j: i32,
    pub eye: [i32; 3],
    pub fill: bool,
    /** The slice's centre plane, along the face's axis (sliceCentre). */
    pub centre: f64,
}

pub struct Dump {
    pub camera: [f64; 3],
    pub yaw: f64,
    pub pitch: f64,
    pub fov: f64,
    pub render_distance: f64,
    /** Per face: right, up, forward in world coordinates. */
    pub faces: [[[f64; 3]; 3]; VIEW_FACES],
    pub geometry: Vec<f64>,
    pub lattices: [Lattice; VIEW_FACES],
    pub palettes: Vec<u32>,
    pub grids: Vec<Option<Arc<Grid>>>,
    pub requests: Vec<Request>,
}

struct Bytes<'a>(&'a [u8]);

impl Bytes<'_> {
    fn take<const N: usize>(&mut self) -> [u8; N] {
        let (head, rest) = self.0.split_at(N);
        self.0 = rest;
        head.try_into().expect("N bytes")
    }
    fn u32(&mut self) -> u32 {
        u32::from_le_bytes(self.take())
    }
    fn i32(&mut self) -> i32 {
        i32::from_le_bytes(self.take())
    }
    fn f64(&mut self) -> f64 {
        f64::from_le_bytes(self.take())
    }
    fn array<T>(&mut self, read: impl Fn(&mut Self) -> T) -> Vec<T> {
        let n = self.u32() as usize;
        (0..n).map(|_| read(self)).collect()
    }
}

pub fn read(bytes: &[u8]) -> Dump {
    let mut b = Bytes(bytes);
    assert_eq!(b.u32(), 0x31445850, "not a dump");
    let camera = [b.f64(), b.f64(), b.f64()];
    let (yaw, pitch, fov) = (b.f64(), b.f64(), b.f64());
    let (_width, _height) = (b.f64(), b.f64());
    let render_distance = b.f64();
    let faces = std::array::from_fn(|_| std::array::from_fn(|_| [b.f64(), b.f64(), b.f64()]));
    let geometry = b.array(Bytes::f64);
    let l = b.array(Bytes::i32);
    let lattices = std::array::from_fn(|f| {
        let l = &l[f * 9..(f + 1) * 9];
        Lattice { r: [l[0], l[1], l[2]], u: [l[3], l[4], l[5]], f: [l[6], l[7], l[8]] }
    });
    let palettes = b.array(Bytes::u32);
    let levels = b.u32();
    let grids = (0..levels)
        .map(|_| {
            if b.u32() == 0 {
                return None;
            }
            let min = [b.i32(), b.i32(), b.i32()];
            let n = [b.i32(), b.i32(), b.i32()];
            let bricks = b.array(Bytes::i32);
            let cells = b.array(|b| b.take::<1>()[0]);
            Some(Arc::new(Grid { min, n, bricks, cells }))
        })
        .collect();
    let requests = b.array(|b| Request {
        face: b.u32() as usize,
        level: b.i32(),
        layer: b.i32(),
        cutoff: b.f64(),
        i: b.i32(),
        j: b.i32(),
        eye: [b.i32(), b.i32(), b.i32()],
        fill: b.i32() == 1,
        centre: b.f64(),
    });
    Dump { camera, yaw, pitch, fov, render_distance, faces, geometry, lattices, palettes, grids, requests }
}

impl Dump {
    pub fn geometry(&self) -> Geometry {
        Geometry::from_table(&self.geometry)
    }
}
