//! What the ray through the pointer's pixel meets, told of in the window:
//! as the voxels engine marched it (voxels.rs's Probe), and the looks its
//! voxels' tiles give them. At an edge, the picture, made of fewer rays,
//! may show the voxel beside the one told of.

use fcc::{ALPHA, FACE_KINDS, in_part};
use worldgen::units::UNIT_SIZE;

use crate::engine::voxels::Probe;
use crate::world::{TILE, World};

/** A line told of: its name, what it says, and the colours it shows (sRGB, alpha last). */
pub type Line = (&'static str, String, Vec<[u8; 4]>);

/** What `probe` says, as lines, with the looks `world`'s tiles give its voxels; the light as light (`lit`) or a shade. */
pub fn lines(probe: &Probe, world: &World, lit: bool) -> Vec<Line> {
    let grid = |level: u32, site: [i32; 3]| {
        world.level(level as usize).tiles.get(site.map(|v| i64::from(v).div_euclid(TILE) as i32)).map(|t| &t.grid)
    };
    let mut out: Vec<Line> = Vec::new();
    if probe.stopped() {
        let scale = f64::from(1u32 << probe.level);
        let [mx, my, mz] = probe.site.map(|v| f64::from(v) * scale * UNIT_SIZE);
        out.push(("voxel", place(probe.level, probe.site), vec![]));
        out.push(("at", format!("{mx:.2} {my:.2} {mz:.2} m, {} m across", scale * UNIT_SIZE), vec![]));
        out.push(("distance", length(probe.distance), vec![]));
        if let Some(grid) = grid(probe.level, probe.site) {
            let [x, y, z] = probe.site;
            let own = grid.get(x, y, z);
            out.push(("look", described(own), vec![own.to_le_bytes()]));
            let faces: Vec<_> = (0..FACE_KINDS).map(|k| grid.face(x, y, z, k)).collect();
            if faces.iter().any(|&f| f != own) {
                out.push(("faces", "unlike through each".into(), faces.iter().map(|f| f.to_le_bytes()).collect()));
            }
        }
        if lit {
            out.push(("light on", three(probe.on), vec![]));
            out.push(("glow", three(probe.glow), vec![]));
        } else {
            out.push(("shade", format!("{:.3}", probe.on[0]), vec![]));
        }
    } else {
        out.push(("voxel", "none".into(), vec![]));
    }
    if probe.crossed > 0 {
        let text = format!("{} see-through voxels, across {}, letting through {}", probe.crossed, length(probe.path), three(probe.through));
        out.push(("before", text, vec![]));
    }
    for crossed in probe.media() {
        let text =
            format!("{}, across {}, letting through {}", place(crossed.level, crossed.site), length(crossed.path), three(crossed.through));
        let [x, y, z] = crossed.site;
        let colour = grid(crossed.level, crossed.site).map(|grid| grid.get(x, y, z).to_le_bytes());
        out.push(("", text, colour.into_iter().collect()));
    }
    if probe.crossed as usize > probe.media().len() {
        out.push(("", format!("and {} more", probe.crossed as usize - probe.media().len()), vec![]));
    }
    out
}

fn place(level: u32, [x, y, z]: [i32; 3]) -> String {
    format!("{x} {y} {z} of level {level}")
}

/** World units, and metres. */
fn length(units: f32) -> String {
    format!("{units:.2} units, {:.2} m", f64::from(units) * UNIT_SIZE)
}

fn three([r, g, b]: [f32; 3]) -> String {
    format!("{r:.3} {g:.3} {b:.3}")
}

/** A look: its colour, and what it is of (fcc's Look). */
fn described(look: u32) -> String {
    let [r, g, b, alpha] = look.to_le_bytes();
    let of = match alpha {
        ALPHA => "opaque".into(),
        _ if in_part(look) => format!("pieces held in part, stopping {alpha}/255 of rays"),
        _ => format!("a medium, stopping {alpha}/255 of rays"),
    };
    format!("#{r:02x}{g:02x}{b:02x}, {of}")
}

/** Shows `lines` in `ui`, each colour as a swatch. */
pub fn show(ui: &mut egui::Ui, lines: &[Line]) {
    egui::Grid::new("voxel").num_columns(2).show(ui, |ui| {
        for (name, text, colours) in lines {
            ui.label(*name);
            ui.horizontal(|ui| {
                for &[r, g, b, _] in colours {
                    let (rect, _) = ui.allocate_exact_size(egui::vec2(12.0, 12.0), egui::Sense::hover());
                    ui.painter().rect_filled(rect, 2.0, egui::Color32::from_rgb(r, g, b));
                }
                ui.label(text);
            });
            ui.end_row();
        }
    });
}
