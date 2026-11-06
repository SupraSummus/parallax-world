//! Real places: towns and villages from Overture Maps (OpenStreetMap's
//! buildings, roads, land use and water), drawn at a metre a cell in
//! towns.rs's labels by scripts/overture.py into out/real/. Each place is
//! a square around its centre, so it holds the town or village and what
//! surrounds it: fields, forest, rivers, and the edges between them; and
//! its lines, the main roads and water, for plans (plan.rs).

use crate::experiment::OUT;
use crate::image::Image;
use crate::plan::NONE;

/** A real place: its map, and its lines (main roads and water as the map shows them, NONE elsewhere). */
pub struct Place {
    pub map: Image,
    pub lines: Image,
}

/** The places drawn in out/real/ (none if the script hasn't run), in the order places.tsv lists them. */
pub fn places() -> Vec<Place> {
    let dir = format!("{OUT}/real");
    let Ok(list) = std::fs::read_to_string(format!("{dir}/places.tsv")) else { return Vec::new() };
    let read = |path: String, none: Option<u16>| -> Option<Image> {
        let bytes = std::fs::read(path).ok()?;
        let size = |i: usize| bytes[i..i + 4].try_into().ok().map(|b| u32::from_le_bytes(b) as usize);
        let (w, h) = (size(0)?, size(4)?);
        let label = |b: u8| if b == u8::MAX { none.unwrap_or(b as u16) } else { b as u16 };
        Some(Image::new(w, h, bytes[8..8 + w * h].iter().map(|b| label(*b)).collect()))
    };
    list.lines()
        .skip(1)
        .filter_map(|line| {
            let id = line.split('\t').next()?;
            Some(Place { map: read(format!("{dir}/{id}.bin"), None)?, lines: read(format!("{dir}/{id}.lines.bin"), Some(NONE))? })
        })
        .collect()
}

/** Whether a place is held out: one in five, by its place in the list. */
pub fn held_out(i: usize) -> bool {
    i % 5 == 4
}

/** Crops `side` wide on a grid of `per` × `per` over each map, its corners' crops at the map's. */
pub fn crops(maps: &[&Image], side: usize, per: usize) -> Vec<Image> {
    let step = |n: usize| if per > 1 { (n - side) / (per - 1) } else { 0 };
    maps.iter().flat_map(|m| (0..per * per).map(move |i| m.crop((i % per) * step(m.w), (i / per) * step(m.h), side, side))).collect()
}
