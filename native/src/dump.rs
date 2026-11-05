//! A view dumped by the spike's TS script: the camera, the faces'
//! lattices and the grids as chunk-baker.ts hands them over, and the
//! chunks the renderer wants.

use std::sync::Arc;

use bake::{Grid, Lattice, VIEW_FACES};

pub struct Request {
    pub face: usize,
    pub level: i32,
    pub layer: i32,
    pub cutoff: f64,
    pub i: i32,
    pub j: i32,
}

pub struct Dump {
    pub camera: [f64; 3],
    pub yaw: f64,
    pub pitch: f64,
    pub fov: f64,
    /** The view the chunks were picked for, in pixels. */
    pub width: f64,
    pub height: f64,
    pub render_distance: f64,
    pub lattices: [Lattice; VIEW_FACES],
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
    assert_eq!(b.u32(), 0x32445850, "not a dump");
    let camera = [b.f64(), b.f64(), b.f64()];
    let (yaw, pitch, fov) = (b.f64(), b.f64(), b.f64());
    let (width, height) = (b.f64(), b.f64());
    let render_distance = b.f64();
    // The faces' axes: view.rs makes its own.
    for _ in 0..VIEW_FACES * 9 {
        b.f64();
    }
    let l = b.array(Bytes::i32);
    let lattices = std::array::from_fn(|f| {
        let l = &l[f * 9..(f + 1) * 9];
        Lattice { r: [l[0], l[1], l[2]], u: [l[3], l[4], l[5]], f: [l[6], l[7], l[8]] }
    });
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
            Some(Arc::new(Grid { min, n, bricks, cells, faces: Vec::new() }))
        })
        .collect();
    let requests = b.array(|b| {
        let r = Request { face: b.u32() as usize, level: b.i32(), layer: b.i32(), cutoff: b.f64(), i: b.i32(), j: b.i32() };
        // The slice's centre plane: bake's slicing finds it.
        b.f64();
        r
    });
    Dump { camera, yaw, pitch, fov, width, height, render_distance, lattices, grids, requests }
}
