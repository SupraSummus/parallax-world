//! The city: a district of a few blocks for checking flat walls, thin
//! frames and openings. A 3 × 3 grid of blocks, each with a building of
//! 1 to 3 storeys whose rooms form a maze joined by doorways, some round a
//! courtyard; windows, balconies, furniture, pavements with kerbs, street
//! lamps and trees, zebra crossings at the junctions; lawn beyond.
//! Everything is boxes on a plan of quarter metres (plan cells, two
//! lattice units), drawn over what is there, as one would paint voxels.
//! From afar each building is its outline (INSIDE).

use std::rc::Rc;

use worldgen::World;
use worldgen::column::Column;
use worldgen::generator::{ANY_SIZE, Child, Children, Far, Field, Generator, Layer, Node, Op, Population, Shape, Shapes};
use worldgen::grid::Box3;
use worldgen::kinds;
use worldgen::noise::{Rng, fx, hash, hash64, stream};
use worldgen::plan::Plan;
use worldgen::units::m;

kinds! {
    base: 0;
    asphalt: 12, ["#3d3f42", "#414346", "#393b3e"];
    roadMark: 12, ["#e6e2d6"];
    paving0: 12, ["#aba8a1"];
    paving1: 12, ["#b4b1aa"];
    paving2: 12, ["#a29f98"];
    kerb: 12, ["#d0cdc5"];
    lawn: 1, ["#5f8f3a", "#679a40", "#588536"];
    hedge: 1, ["#2f5e2a", "#356a2f", "#2a5426"];
    soil: 2, ["#4a3423", "#54392a"];
    bark: 4, ["#4b3621", "#57402a"];
    canopy: 1, ["#4c7f2a", "#5a8f30", "#447324", "#679a38"], walk_through;
    pole: 3, ["#2e3236"];
    light: 6, ["#fff3c4"], glows;
    concrete: 12, ["#9a9690", "#a19d97"];
    plinth: 3, ["#5e5a55", "#57534e"];
    gravel: 9, ["#6d6a66", "#75716c", "#686561"];
    glass: 11, ["#5b7f97"];
    ceiling: 10, ["#f4f2ec"];
    skirting: 4, ["#6b5a48"];
    brass: 3, ["#c9a640"];
    railing: 3, ["#2b2d30"];
    roofUnit: 3, ["#9aa0a6", "#a3a9ae"];
    water: 13, ["#3f7fa8"], walk_through;
    courtLight: 12, ["#c8c0b0"];
    courtDark: 12, ["#bdb5a5"];
    table: 4, ["#6e4a2c"];
    chair: 4, ["#4a3424"];
    shelf: 4, ["#5c3e26"];
    bookRed: 6, ["#8c2f2f"];
    bookBlue: 6, ["#2f5a8c"];
    bookYellow: 6, ["#d8b04a"];
    bookGreen: 6, ["#3f7a4a"];
    bookPurple: 6, ["#5a3a6e"];
    bookWhite: 6, ["#e0dccf"];
    pot: 2, ["#b5603c"];
    plant: 1, ["#3f7f35", "#4a8f3d", "#357030"], walk_through;
    facadeSand: 10, ["#c9a27e"];
    facadeRust: 10, ["#b5654a"];
    facadeCream: 10, ["#d8cfb8"];
    facadeSlate: 10, ["#8a9aa6"];
    facadeSage: 10, ["#a8b08a"];
    facadeTerracotta: 10, ["#c47d5a"];
    facadeMoss: 10, ["#7d8a7a"];
    facadeMustard: 10, ["#d6b85a"];
    trimWhite: 10, ["#f2efe6"];
    trimCream: 10, ["#e6dfcc"];
    trimDark: 10, ["#4a4a4a"];
    doorGreen: 4, ["#2f4f3a"];
    doorRed: 4, ["#7a2b2b"];
    doorBlue: 4, ["#2b3a5a"];
    doorBlack: 4, ["#3a3a3a"];
    interiorLinen: 10, ["#ece6d8"];
    interiorGrey: 10, ["#e3e8ea"];
    interiorPeach: 10, ["#efe3d3"];
    interiorMint: 10, ["#dfe6d6"];
    interiorWhite: 10, ["#f3f0ea"];
    roomDoorOak: 4, ["#8a5a32"];
    roomDoorWalnut: 4, ["#6e4a2c"];
    roomDoorWhite: 4, ["#ece8de"];
    checkerLight: 12, ["#e8e4dc"];
    checkerDark: 12, ["#2c2c2e"];
    tileLight: 12, ["#d9d4c8"];
    tileDark: 12, ["#c9c3b5"];
    parquet0: 4, ["#a9743f"];
    parquet1: 4, ["#b8834a"];
    parquet2: 4, ["#9a6634"];
    parquet3: 4, ["#c0904f"];
    carpetRed: 6, ["#7a2e35"];
    carpetBlue: 6, ["#2e4a6b"];
    carpetGreen: 6, ["#3e5e3a"];
    carpetMauve: 6, ["#6b5a7a"];
    carpetGold: 6, ["#c9a85a"];
    carpetBeige: 6, ["#d8d0bc"];
}

/** Lattice units per plan cell: plan cell (x, y, z) holds the sites with x in [2x − 1, 2x] (y, z likewise). */
const PLAN: i64 = 2;
/** Plan cells in `metres`. */
const fn vox(metres: f64) -> i64 {
    m(metres) / PLAN
}

// Buildings stand on a grid of square rooms; walls stand on the grid lines.
const CELL: i64 = vox(4.0);
/** Centre of a wall segment, measured from the grid line it starts at. */
const MID: i64 = CELL / 2;
// A storey: floor layer (row 0), wall rows 1..WALL_TOP, ceiling layer (CEILING).
const STOREY: i64 = vox(3.5);
const WALL_TOP: i64 = STOREY - 2;
const CEILING: i64 = STOREY - 1;
/** Floor layer of the ground storey, one step above the pavement (y = 0). */
const FLOOR0: i64 = 1;
const MAX_STOREYS: i64 = 3;

// Streets: a 3 × 3 grid of blocks, each a lot ringed by pavement, with roads between and around.
const ROAD: i64 = vox(8.0);
const SIDEWALK: i64 = vox(3.0);
const LOT: i64 = vox(22.0);
const PLOT: i64 = LOT + 2 * SIDEWALK;
const PERIOD: i64 = ROAD + PLOT;
const BLOCKS: i64 = 3;
const SIZE: i64 = BLOCKS * PERIOD + ROAD;
const MIN_X: i64 = -SIZE / 2;
const MIN_Z: i64 = 0;
/** The roads' surface; the pavements' and lawns' is a layer above. */
const MIN_Y: i64 = -1;
const MAX_Y: i64 = FLOOR0 + MAX_STOREYS * STOREY + vox(3.5);

// Openings, centred on the segment (u = MID). Walls are `thick` layers, w = 0 .. thick − 1.
const DOOR_TOP: i64 = vox(2.25);
const SILL: i64 = vox(0.75);
const WINDOW_TOP: i64 = vox(2.75);
const TRANSOM: i64 = WINDOW_TOP - 2;

/** The heights the city has anything at (lattice units). */
const HEIGHTS: (i64, i64) = (m(-16.0), m(16.0));

// Per building: one of each.
const FACADES: [K; 8] =
    [K::facadeSand, K::facadeRust, K::facadeCream, K::facadeSlate, K::facadeSage, K::facadeTerracotta, K::facadeMoss, K::facadeMustard];
const TRIMS: [K; 3] = [K::trimWhite, K::trimCream, K::trimDark];
const ENTRANCE_DOORS: [K; 4] = [K::doorGreen, K::doorRed, K::doorBlue, K::doorBlack];
const INTERIORS: [K; 5] = [K::interiorLinen, K::interiorGrey, K::interiorPeach, K::interiorMint, K::interiorWhite];
const ROOM_DOORS: [K; 3] = [K::roomDoorOak, K::roomDoorWalnut, K::roomDoorWhite];
const BOOKS: [K; 6] = [K::bookRed, K::bookBlue, K::bookYellow, K::bookGreen, K::bookPurple, K::bookWhite];
const PARQUET: [K; 4] = [K::parquet0, K::parquet1, K::parquet2, K::parquet3];
const CARPETS: [K; 4] = [K::carpetRed, K::carpetBlue, K::carpetGreen, K::carpetMauve];
const CARPET_BORDERS: [K; 2] = [K::carpetGold, K::carpetBeige];
const PAVING: [K; 3] = [K::paving0, K::paving1, K::paving2];

/** One of `list`, picked by `t` in [0, 1). */
fn pick<T: Copy>(list: &[T], t: f64) -> T {
    list[(t * list.len() as f64) as usize]
}

/** A thing's shapes, painted plan cell by plan cell: each box over what the thing has there. */
#[derive(Default)]
struct Canvas {
    shapes: Vec<Shape>,
}

impl Canvas {
    /** The plan cells with corners a and b (inclusive, either order) of `kind`, over what is there; None cuts them out of the thing. */
    fn cells(&mut self, a: [i64; 3], b: [i64; 3], kind: Option<K>) {
        let min = [0, 1, 2].map(|i| (PLAN * a[i].min(b[i]) - 1) as f64);
        let max = [0, 1, 2].map(|i| (PLAN * a[i].max(b[i]) + 1) as f64);
        self.shapes.push(match kind {
            Some(kind) => Shape::cuboid(min, max, kind).paint(),
            None => Shape::cuboid(min, max, 0).carve(),
        });
    }

    fn put(&mut self, p: [i64; 3], kind: K) {
        self.cells(p, p, Some(kind));
    }
}

/**
 * A wall segment in its own coordinates: u along the wall from a grid
 * intersection, row up from the storey's floor layer, w across the wall
 * (w = 0 on the grid line, growing `out`wards: outdoors for facades).
 */
#[derive(Clone, Copy)]
struct Wall {
    x: i64,
    y: i64,
    z: i64,
    along_x: bool,
    out: i64,
}

impl Wall {
    fn at(&self, u: i64, row: i64, w: i64) -> [i64; 3] {
        if self.along_x { [self.x + u, self.y + row, self.z + self.out * w] } else { [self.x + self.out * w, self.y + row, self.z + u] }
    }

    /** Fills u0..u1 × row0..row1 × w0..w1 (inclusive; u and w either order, nothing if row1 < row0). */
    #[allow(clippy::too_many_arguments)]
    fn fill(&self, c: &mut Canvas, u0: i64, u1: i64, row0: i64, row1: i64, w0: i64, w1: i64, kind: Option<K>) {
        if row0 <= row1 {
            c.cells(self.at(u0, row0, w0), self.at(u1, row1, w1), kind);
        }
    }

    fn put(&self, c: &mut Canvas, u: i64, row: i64, w: i64, kind: K) {
        c.put(self.at(u, row, w), kind);
    }

    /** A frame one voxel proud of the wall on layer w: posts at u0 and u1 up to `top`, lintel at `top`. */
    fn architrave(&self, c: &mut Canvas, u0: i64, u1: i64, top: i64, w: i64, kind: K) {
        self.fill(c, u0, u0, 1, top, w, w, Some(kind));
        self.fill(c, u1, u1, 1, top, w, w, Some(kind));
        self.fill(c, u0, u1, top, top, w, w, Some(kind));
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Leaf {
    OpenIn,
    OpenOut,
    Closed,
    None,
}

/** A 1 m doorway with a frame on both sides and a leaf: swung open against the wall, closed, or none. */
fn door(c: &mut Canvas, wall: &Wall, thick: i64, leaf: Leaf, leaf_kind: K, trim: K, glazed: bool) {
    let m = MID;
    wall.fill(c, m - 2, m + 1, 1, DOOR_TOP, 0, thick - 1, None);
    wall.architrave(c, m - 3, m + 2, DOOR_TOP + 1, -1, trim);
    wall.architrave(c, m - 3, m + 2, DOOR_TOP + 1, thick, trim);
    match leaf {
        Leaf::Closed => {
            wall.fill(c, m - 2, m + 1, 1, DOOR_TOP, 0, 0, Some(leaf_kind));
            if glazed {
                wall.fill(c, m - 1, m, 5, 7, 0, 0, Some(K::glass));
            }
            wall.put(c, m + 1, 4, -1, K::brass);
            wall.put(c, m + 1, 4, 1, K::brass);
        }
        Leaf::OpenIn | Leaf::OpenOut => {
            // Hinged at u = m − 2, standing perpendicular to the wall, 4 deep.
            let d = if leaf == Leaf::OpenIn { -1 } else { 1 };
            let w0 = if d < 0 { -1 } else { thick };
            wall.fill(c, m - 2, m - 2, 1, DOOR_TOP, w0, w0 + 3 * d, Some(leaf_kind));
            if glazed {
                wall.fill(c, m - 2, m - 2, 5, 7, w0 + d, w0 + 2 * d, Some(K::glass));
            }
            wall.put(c, m - 3, 4, w0 + 3 * d, K::brass);
            wall.put(c, m - 1, 4, w0 + 3 * d, K::brass);
        }
        Leaf::None => {}
    }
}

/** A 2.5 m wide opening between rooms, framed on both sides. */
fn arch(c: &mut Canvas, wall: &Wall, trim: K) {
    wall.fill(c, 3, CELL - 4, 1, DOOR_TOP + 1, 0, 0, None);
    wall.architrave(c, 2, CELL - 3, DOOR_TOP + 2, -1, trim);
    wall.architrave(c, 2, CELL - 3, DOOR_TOP + 2, 1, trim);
}

/**
 * A 1.75 m window: glass on the inner layer with a mullion and a transom,
 * the outer layers cut back into a reveal, a sill outside and a board inside.
 * A French window (`to_floor`) reaches down to the floor.
 */
fn window(c: &mut Canvas, wall: &Wall, thick: i64, trim: K, to_floor: bool) {
    let m = MID;
    let bottom = if to_floor { 1 } else { SILL + 1 };
    if thick > 1 {
        wall.fill(c, m - 3, m + 3, bottom, WINDOW_TOP, 1, thick - 1, None);
    }
    wall.fill(c, m - 3, m + 3, bottom, WINDOW_TOP, 0, 0, Some(K::glass));
    wall.fill(c, m - 3, m + 3, TRANSOM, TRANSOM, 0, 0, Some(trim));
    wall.fill(c, m, m, bottom, TRANSOM - 1, 0, 0, Some(trim));
    if thick > 1 {
        wall.fill(c, m - 4, m + 4, WINDOW_TOP + 1, WINDOW_TOP + 1, thick - 1, thick - 1, Some(trim));
    }
    if !to_floor {
        wall.fill(c, m - 4, m + 4, SILL, SILL, thick, thick, Some(trim));
        wall.fill(c, m - 3, m + 3, SILL, SILL, -1, -1, Some(trim));
    }
}

/** A French window onto a 1 m deep balcony with a railing. */
fn balcony(c: &mut Canvas, wall: &Wall, thick: i64, trim: K) {
    window(c, wall, thick, trim, true);
    let m = MID;
    let depth = vox(1.0);
    let edge = thick + depth - 1;
    wall.fill(c, m - 4, m + 4, 0, 0, thick, edge, Some(K::concrete));
    // The rail on its three open sides, balusters every other cell under it.
    wall.fill(c, m - 4, m - 4, 4, 4, thick, edge, Some(K::railing));
    wall.fill(c, m + 4, m + 4, 4, 4, thick, edge, Some(K::railing));
    wall.fill(c, m - 4, m + 4, 4, 4, edge, edge, Some(K::railing));
    for u in m - 4..=m + 4 {
        for w in thick..=edge {
            if (u == m - 4 || u == m + 4 || w == edge) && (u + w) % 2 == 0 {
                wall.fill(c, u, u, 1, 3, w, w, Some(K::railing));
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Buildings

#[derive(Clone, Copy, PartialEq)]
enum EdgeKind {
    Wall,
    Door,
    Arch,
    Open,
    Window,
    Balcony,
    Entrance,
}

#[derive(Clone, Copy)]
struct Edge {
    kind: EdgeKind,
    /** Two layers thick with the outer one in facade colours: outside walls and walls onto the courtyard. */
    facade: bool,
    /** On the building's outside (not the courtyard). */
    exterior: bool,
    out: i64,
    leaf: Leaf,
}

#[derive(Clone, Copy)]
struct Building {
    /** Grid line x = bx + i · CELL, z = bz + j · CELL. */
    bx: i64,
    bz: i64,
    w: i64,
    d: i64,
    storeys: i64,
    court: Option<(i64, i64)>,
    /** Cell with the front (−z) entrance. */
    entrance: i64,
    /** Cell with a back door, if any. */
    back_door: Option<i64>,
    facade: K,
    trim: K,
    interior: K,
    entrance_door: K,
}

impl Building {
    fn is_court(&self, i: i64, j: i64) -> bool {
        self.court == Some((i, j))
    }

    fn inside(&self, i: i64, j: i64) -> bool {
        i >= 0 && i < self.w && j >= 0 && j < self.d
    }
}

#[derive(Clone, Copy)]
enum Side {
    Left,
    Right,
    Front,
    Back,
}

const SIDES: [Side; 4] = [Side::Left, Side::Right, Side::Front, Side::Back];

/** Edges of one storey: on the lines x = const (between cells (i − 1, j) and (i, j)) and z = const. */
struct StoreyPlan {
    w: i64,
    d: i64,
    x_edges: Vec<Edge>,
    z_edges: Vec<Edge>,
}

impl StoreyPlan {
    fn x_edge(&self, i: i64, j: i64) -> &Edge {
        &self.x_edges[(i * self.d + j) as usize]
    }

    fn z_edge(&self, i: i64, j: i64) -> &Edge {
        &self.z_edges[(j * self.w + i) as usize]
    }

    fn side(&self, i: i64, j: i64, side: Side) -> &Edge {
        match side {
            Side::Left => self.x_edge(i, j),
            Side::Right => self.x_edge(i + 1, j),
            Side::Front => self.z_edge(i, j),
            Side::Back => self.z_edge(i, j + 1),
        }
    }

    fn side_mut(&mut self, i: i64, j: i64, side: Side) -> &mut Edge {
        let (w, d) = (self.w, self.d);
        match side {
            Side::Left => &mut self.x_edges[(i * d + j) as usize],
            Side::Right => &mut self.x_edges[((i + 1) * d + j) as usize],
            Side::Front => &mut self.z_edges[(j * w + i) as usize],
            Side::Back => &mut self.z_edges[((j + 1) * w + i) as usize],
        }
    }
}

/**
 * Lays out one storey as a maze: a random spanning tree of doorways joins
 * all rooms, plus a few extra openings so there are loops. Outside walls
 * get windows (and on the ground floor the entrances), upper storeys
 * sometimes balconies.
 */
fn plan_storey(b: &Building, storey: i64, random: &mut Rng) -> StoreyPlan {
    // Room-to-room walls: which list (x or z edges), index, and the rooms either side.
    let mut between: Vec<(bool, usize, i64, i64)> = Vec::new();
    // The edge between cell a and its +x or +z neighbour c; at most one of them is outside.
    let mut edge = |ai: i64, aj: i64, ci: i64, cj: i64, along_x: bool, index: usize| -> Edge {
        let a_in = b.inside(ai, aj);
        if !a_in || !b.inside(ci, cj) {
            let roll = random.next();
            let kind = if storey > 0 && roll < 0.2 {
                EdgeKind::Balcony
            } else if roll > 0.92 {
                EdgeKind::Wall
            } else {
                EdgeKind::Window
            };
            return Edge { kind, facade: true, exterior: true, out: if a_in { 1 } else { -1 }, leaf: Leaf::None };
        }
        let c_court = b.is_court(ci, cj);
        if c_court || b.is_court(ai, aj) {
            return Edge { kind: EdgeKind::Window, facade: true, exterior: false, out: if c_court { 1 } else { -1 }, leaf: Leaf::None };
        }
        let out = if random.next() < 0.5 { 1 } else { -1 };
        between.push((along_x, index, aj * b.w + ai, cj * b.w + ci));
        Edge { kind: EdgeKind::Wall, facade: false, exterior: false, out, leaf: Leaf::None }
    };
    let x_edges: Vec<Edge> = (0..(b.w + 1) * b.d).map(|k| edge(k / b.d - 1, k % b.d, k / b.d, k % b.d, false, k as usize)).collect();
    let z_edges: Vec<Edge> = (0..b.w * (b.d + 1)).map(|k| edge(k % b.w, k / b.w - 1, k % b.w, k / b.w, true, k as usize)).collect();
    let mut plan = StoreyPlan { w: b.w, d: b.d, x_edges, z_edges };

    // Kruskal over the room-to-room walls in random order.
    let mut parent: Vec<usize> = (0..(b.w * b.d) as usize).collect();
    fn root(parent: &mut [usize], k: usize) -> usize {
        if parent[k] != k {
            parent[k] = root(parent, parent[k]);
        }
        parent[k]
    }
    for k in (1..between.len()).rev() {
        let r = (random.next() * (k + 1) as f64) as usize;
        between.swap(k, r);
    }
    for (along_x, index, a, c) in between {
        let (ra, rc) = (root(&mut parent, a as usize), root(&mut parent, c as usize));
        if ra != rc {
            parent[ra] = rc;
        } else if random.next() > 0.15 {
            continue;
        }
        let e = if along_x { &mut plan.z_edges[index] } else { &mut plan.x_edges[index] };
        let roll = random.next();
        e.kind = if roll < 0.72 {
            EdgeKind::Door
        } else if roll < 0.87 {
            EdgeKind::Arch
        } else {
            EdgeKind::Open
        };
        let l = random.next();
        e.leaf = if l < 0.3 {
            Leaf::OpenIn
        } else if l < 0.6 {
            Leaf::OpenOut
        } else if l < 0.75 {
            Leaf::Closed
        } else {
            Leaf::None
        };
    }

    if storey == 0 {
        let front = plan.side_mut(b.entrance, 0, Side::Front);
        front.kind = EdgeKind::Entrance;
        front.leaf = Leaf::OpenIn;
        if let Some(i) = b.back_door {
            let back = plan.side_mut(i, b.d - 1, Side::Back);
            back.kind = EdgeKind::Entrance;
            back.leaf = Leaf::Closed;
        }
        // One door from a neighbouring room out into the courtyard.
        if let Some((i, j)) = b.court {
            let e = plan.side_mut(i, j, pick(&SIDES, random.next()));
            e.kind = EdgeKind::Door;
            e.leaf = Leaf::OpenOut;
        }
    }
    plan
}

#[derive(Clone, Copy, PartialEq)]
enum Floor {
    Checker,
    Tiles,
    Parquet,
    Carpet,
    Concrete,
}

const FLOORS: [Floor; 5] = [Floor::Checker, Floor::Tiles, Floor::Parquet, Floor::Carpet, Floor::Concrete];

/**
 * A building's shapes, in plan cells over the world's lattice: as it is,
 * and its outline, solid but for its courtyard's well, with its facades
 * and what stands on its roof as they are.
 */
fn draw_building(b: &Building, random: &mut Rng) -> (Vec<Shape>, Vec<Shape>) {
    let c = &mut Canvas::default();
    let far = &mut Canvas::default();
    let x1 = b.bx + b.w * CELL;
    let z1 = b.bz + b.d * CELL;
    let top = FLOOR0 + b.storeys * STOREY;
    // The courtyard's open cells, between its walls, at height y.
    let court = |c: &mut Canvas, y: i64, kind: Option<K>| {
        if let Some((i, j)) = b.court {
            let (x, z) = (b.bx + i * CELL, b.bz + j * CELL);
            c.cells([x + 1, y, z + 1], [x + CELL - 1, y, z + CELL - 1], kind);
        }
    };

    c.cells([b.bx, 0, b.bz], [x1, 0, z1], Some(K::concrete));
    far.cells([b.bx - 1, 0, b.bz - 1], [x1 + 1, top - 1, z1 + 1], Some(b.facade));
    far.cells([b.bx - 1, 0, b.bz - 1], [x1 + 1, FLOOR0 + 1, z1 + 1], Some(K::plinth));

    for s in 0..b.storeys {
        let y0 = FLOOR0 + s * STOREY;
        let plan = plan_storey(b, s, random);

        // Floor and ceiling layers, open over the courtyard; its paving, a checker of half metres, on the ground floor.
        c.cells([b.bx, y0, b.bz], [x1, y0, z1], Some(K::concrete));
        c.cells([b.bx, y0 + CEILING, b.bz], [x1, y0 + CEILING, z1], Some(K::ceiling));
        court(c, y0 + CEILING, None);
        if s == 0 {
            court(c, y0, Some(K::courtLight));
            if let Some((i, j)) = b.court {
                let (x, z) = (b.bx + i * CELL, b.bz + j * CELL);
                for sx in (x + 1).div_euclid(2)..=(x + CELL - 1).div_euclid(2) {
                    for sz in (z + 1).div_euclid(2)..=(z + CELL - 1).div_euclid(2) {
                        if (sx + sz).rem_euclid(2) == 1 {
                            c.cells(
                                [(2 * sx).max(x + 1), y0, (2 * sz).max(z + 1)],
                                [(2 * sx + 1).min(x + CELL - 1), y0, (2 * sz + 1).min(z + CELL - 1)],
                                Some(K::courtDark),
                            );
                        }
                    }
                }
            }
        } else {
            court(c, y0, None);
        }

        // Walls.
        let segment = |i: i64, j: i64, along_x: bool, e: &Edge| Wall { x: b.bx + i * CELL, y: y0, z: b.bz + j * CELL, along_x, out: e.out };
        let edges: Vec<(Wall, Edge)> = (0..=b.w)
            .flat_map(|i| (0..b.d).map(move |j| (i, j)))
            .map(|(i, j)| (segment(i, j, false, plan.x_edge(i, j)), *plan.x_edge(i, j)))
            .chain(
                (0..=b.d)
                    .flat_map(|j| (0..b.w).map(move |i| (i, j)))
                    .map(|(i, j)| (segment(i, j, true, plan.z_edge(i, j)), *plan.z_edge(i, j))),
            )
            .collect();
        for (wall, e) in &edges {
            if e.kind == EdgeKind::Open {
                continue;
            }
            wall.fill(c, 1, CELL - 1, 2, WALL_TOP, 0, 0, Some(b.interior));
            wall.fill(c, 1, CELL - 1, 1, 1, 0, 0, Some(K::skirting));
            if !e.facade {
                continue;
            }
            // The outer layer wraps the building's corners; on the courtyard it stays inside the court.
            let (u0, u1) = if e.exterior { (-1, CELL + 1) } else { (1, CELL - 1) };
            let r0 = if e.exterior && s == 0 { -1 } else { 0 };
            wall.fill(c, u0, u1, r0, CEILING, 1, 1, Some(b.facade));
            if s == 0 {
                wall.fill(c, u0, u1, r0, 1, 1, 1, Some(K::plinth));
            }
            if e.exterior && s < b.storeys - 1 {
                for c in [&mut *c, &mut *far] {
                    wall.fill(c, u0 - 1, u1 + 1, CEILING, CEILING, 2, 2, Some(b.trim));
                }
            }
        }
        for i in 0..=b.w {
            for j in 0..=b.d {
                let (x, z) = (b.bx + i * CELL, b.bz + j * CELL);
                c.cells([x, y0 + 2, z], [x, y0 + WALL_TOP, z], Some(b.interior));
                c.put([x, y0 + 1, z], K::skirting);
            }
        }

        // Openings; the facades' on the outline too, as they are.
        for (wall, e) in &edges {
            let thick = if e.facade { 2 } else { 1 };
            let leaf = if e.kind == EdgeKind::Door { pick(&ROOM_DOORS, random.next()) } else { b.entrance_door };
            for c in [&mut *c, &mut *far].into_iter().take(if e.facade { 2 } else { 1 }) {
                match e.kind {
                    EdgeKind::Door => door(c, wall, thick, e.leaf, leaf, b.trim, false),
                    EdgeKind::Arch => arch(c, wall, b.trim),
                    EdgeKind::Window => window(c, wall, thick, b.trim, false),
                    EdgeKind::Balcony => balcony(c, wall, thick, b.trim),
                    EdgeKind::Entrance => {
                        door(c, wall, thick, e.leaf, leaf, b.trim, true);
                        // Landing, step down to the pavement, and a canopy.
                        wall.fill(c, MID - 3, MID + 2, 0, 0, 2, 3, Some(K::plinth));
                        wall.fill(c, MID - 4, MID + 3, DOOR_TOP + 2, DOOR_TOP + 2, 2, 4, Some(b.trim));
                    }
                    EdgeKind::Wall | EdgeKind::Open => {}
                }
            }
        }

        // Rooms: floor pattern, ceiling lamp, furniture.
        for i in 0..b.w {
            for j in 0..b.d {
                if b.is_court(i, j) {
                    if s == 0 {
                        courtyard(c, b.bx + i * CELL, y0, b.bz + j * CELL, random);
                    }
                    continue;
                }
                let front = s == 0 && i == b.entrance && j == 0;
                let floor = if front { Floor::Checker } else { pick(&FLOORS, random.next()) };
                room(c, b, &plan, i, j, y0, floor, random);
            }
        }
    }

    // Roof: gravel, a parapet with coping, a stair housing and a few machines.
    c.cells([b.bx, top, b.bz], [x1, top, z1], Some(K::gravel));
    court(c, top, None);
    far.cells([b.bx, top, b.bz], [x1, top, z1], Some(K::gravel));
    if let Some((i, j)) = b.court {
        // The well inside the courtyard's facades, paved at its foot.
        let (x, z) = (b.bx + i * CELL, b.bz + j * CELL);
        far.cells([x + 2, FLOOR0 + 1, z + 2], [x + CELL - 2, top, z + CELL - 2], None);
        far.cells([x + 2, FLOOR0, z + 2], [x + CELL - 2, FLOOR0, z + CELL - 2], Some(K::courtLight));
    }
    let parapet = |c: &mut Canvas, along_x: bool, x: i64, z: i64, out: i64, length: i64| {
        let wall = Wall { x, y: top, z, along_x, out };
        wall.fill(c, -1, length + 1, 0, 2, 0, 1, Some(b.facade));
        wall.fill(c, -1, length + 1, 3, 3, 0, 1, Some(b.trim));
    };
    for c in [&mut *c, &mut *far] {
        parapet(c, true, b.bx, b.bz, -1, b.w * CELL);
        parapet(c, true, b.bx, z1, 1, b.w * CELL);
        parapet(c, false, b.bx, b.bz, -1, b.d * CELL);
        parapet(c, false, x1, b.bz, 1, b.d * CELL);
    }

    let rooms: Vec<(i64, i64)> = (0..b.w).flat_map(|i| (0..b.d).map(move |j| (i, j))).filter(|&(i, j)| !b.is_court(i, j)).collect();
    let pick_room = |random: &mut Rng| pick(&rooms, random.next());
    {
        let (i, j) = pick_room(random);
        let (x, z) = (b.bx + i * CELL, b.bz + j * CELL);
        let housing = top + vox(2.5);
        for c in [&mut *c, &mut *far] {
            c.cells([x + 4, top + 1, z + 4], [x + CELL - 4, housing, z + CELL - 4], Some(b.facade));
            c.cells([x + 3, housing + 1, z + 3], [x + CELL - 3, housing + 1, z + CELL - 3], Some(b.trim));
            c.cells([x + MID - 2, top + 1, z + 4], [x + MID + 1, top + DOOR_TOP, z + 4], Some(K::pole));
        }
    }
    let units = 1 + (random.next() * 3.0) as i64;
    for _ in 0..units {
        let (i, j) = pick_room(random);
        let x = b.bx + i * CELL + 2 + (random.next() * 8.0) as i64;
        let z = b.bz + j * CELL + 2 + (random.next() * 8.0) as i64;
        let (dx, dy, dz) = ((random.next() * 3.0) as i64, (random.next() * 3.0) as i64, (random.next() * 2.0) as i64);
        for c in [&mut *c, &mut *far] {
            c.cells([x, top + 1, z], [x + 2 + dx, top + 2 + dy, z + 2 + dz], Some(K::roofUnit));
        }
    }
    (std::mem::take(&mut c.shapes), std::mem::take(&mut far.shapes))
}

/** Interior of room (i, j) on the storey whose floor layer is y0. */
#[allow(clippy::too_many_arguments)]
fn room(c: &mut Canvas, b: &Building, plan: &StoreyPlan, i: i64, j: i64, y0: i64, floor: Floor, random: &mut Rng) {
    let x0 = b.bx + i * CELL;
    let z0 = b.bz + j * CELL;
    let room_seed = random.next_u64();
    let shade = random.next();
    // Cells (u, v) of the room's floor, from its corner on the grid lines: 1 to CELL − 1, between the walls.
    let area = |c: &mut Canvas, u0: i64, u1: i64, v0: i64, v1: i64, kind: K| {
        c.cells([x0 + u0.max(1), y0, z0 + v0.max(1)], [x0 + u1.min(CELL - 1), y0, z0 + v1.min(CELL - 1)], Some(kind));
    };
    // Squares `n` cells wide on a checker of `pale` and `dark`.
    let checker = |c: &mut Canvas, n: i64, pale: K, dark: K| {
        area(c, 1, CELL - 1, 1, CELL - 1, pale);
        for su in 0..CELL / n {
            for sv in (0..CELL / n).filter(|sv| (su + sv) % 2 == 1) {
                area(c, n * su, n * su + n - 1, n * sv, n * sv + n - 1, dark);
            }
        }
    };
    match floor {
        Floor::Checker => checker(c, 2, K::checkerLight, K::checkerDark),
        Floor::Tiles => checker(c, 4, K::tileLight, K::tileDark),
        Floor::Parquet => {
            // Planks 1 m long, staggered row by row, each in one shade.
            for v in 1..CELL {
                let offset = (v % 2) * 2;
                for plank in 0..=(CELL - 1 + offset) / 4 {
                    let u0 = 4 * plank - offset;
                    let shade = ((hash(room_seed, plank, v, 0) as u64 * PARQUET.len() as u64) >> 32) as usize;
                    area(c, u0, u0 + 3, v, v, PARQUET[shade]);
                }
            }
        }
        Floor::Carpet => {
            area(c, 1, CELL - 1, 1, CELL - 1, K::concrete);
            area(c, 2, CELL - 2, 2, CELL - 2, pick(&CARPET_BORDERS, shade));
            area(c, 3, CELL - 3, 3, CELL - 3, pick(&CARPETS, shade));
        }
        Floor::Concrete => area(c, 1, CELL - 1, 1, CELL - 1, K::concrete),
    }
    c.cells([x0 + MID - 1, y0 + WALL_TOP, z0 + MID - 1], [x0 + MID, y0 + WALL_TOP, z0 + MID], Some(K::light));

    // Furniture, in the room's (u, v) and rows up from its floor.
    let at = |c: &mut Canvas, u: [i64; 2], v: [i64; 2], row: [i64; 2], kind: K| {
        c.cells([x0 + u[0], y0 + row[0], z0 + v[0]], [x0 + u[1], y0 + row[1], z0 + v[1]], Some(kind));
    };
    // Against a side: t along it, d from it.
    let along = |side: Side, t: i64, d: i64| match side {
        Side::Left => (d, t),
        Side::Right => (CELL - d, t),
        Side::Front => (t, d),
        Side::Back => (t, CELL - d),
    };

    if random.next() < 0.5 {
        // Table 1.25 m × 0.75 m with a chair on each long side.
        at(c, [MID - 2, MID + 2], [MID - 1, MID + 1], [3, 3], K::table);
        for (u, v) in [(MID - 2, MID - 1), (MID + 2, MID - 1), (MID - 2, MID + 1), (MID + 2, MID + 1)] {
            at(c, [u, u], [v, v], [1, 2], K::table);
        }
        for (v, back) in [(MID - 3, MID - 4), (MID + 3, MID + 4)] {
            at(c, [MID, MID], [v, v], [1, 2], K::chair);
            at(c, [MID, MID], [back, back], [1, 4], K::chair);
        }
    }
    for side in SIDES {
        let e = plan.side(i, j, side);
        if e.kind != EdgeKind::Wall || e.facade || random.next() > 0.4 {
            continue;
        }
        // Bookcase: shelves every 0.75 m and its ends, books in between with gaps.
        let (first, last) = (3, CELL - 4);
        let shelf = |c: &mut Canvas, t0: i64, t1: i64, row0: i64, row1: i64| {
            let ((u0, v0), (u1, v1)) = (along(side, t0, 1), along(side, t1, 1));
            at(c, [u0, u1], [v0, v1], [row0, row1], K::shelf);
        };
        for row in [1, 4, 7] {
            shelf(c, first, last, row, row);
        }
        shelf(c, first, first, 1, 8);
        shelf(c, last, last, 1, 8);
        for t in first + 1..last {
            for row in (1..=8).filter(|row| row % 3 != 1) {
                if random.next() < 0.8 {
                    let (u, v) = along(side, t, 1);
                    at(c, [u, u], [v, v], [row, row], pick(&BOOKS, random.next()));
                }
            }
        }
    }
    if random.next() < 0.4 {
        // Potted plant in a corner.
        let (u, v) = pick(&[(1, 1), (CELL - 1, 1), (1, CELL - 1), (CELL - 1, CELL - 1)], random.next());
        at(c, [u, u], [v, v], [1, 2], K::pot);
        at(c, [u, u], [v, v], [3, 6], K::plant);
        let (du, dv) = (if u == 1 { 1 } else { -1 }, if v == 1 { 1 } else { -1 });
        for (a, b) in [(du, 0), (0, dv), (du, dv)] {
            at(c, [u + a, u + a], [v + b, v + b], [5, 6], K::plant);
        }
    }
}

/** The courtyard floor's centrepiece: a tree in a planter, or a fountain. */
fn courtyard(c: &mut Canvas, x0: i64, y0: i64, z0: i64, random: &mut Rng) {
    let (cx, cz) = (x0 + MID, z0 + MID);
    if random.next() < 0.5 {
        c.cells([cx - 2, y0 + 1, cz - 2], [cx + 2, y0 + 2, cz + 2], Some(K::plinth));
        c.cells([cx - 1, y0 + 2, cz - 1], [cx + 1, y0 + 2, cz + 1], Some(K::soil));
        c.cells([cx, y0 + 3, cz], [cx, y0 + vox(2.5), cz], Some(K::bark));
        tree_crown(c, cx, y0 + vox(2.5), cz);
    } else {
        c.cells([cx - 3, y0 + 1, cz - 3], [cx + 3, y0 + 2, cz + 3], Some(K::plinth));
        c.cells([cx - 2, y0 + 1, cz - 2], [cx + 2, y0 + 2, cz + 2], Some(K::water));
        c.cells([cx, y0 + 1, cz], [cx, y0 + 4, cz], Some(K::plinth));
        c.put([cx, y0 + 5, cz], K::water);
    }
}

/** A boxy crown, 1.25 m across with its edges trimmed, sitting on `y`: a core and a cross through its middle. */
fn tree_crown(c: &mut Canvas, x: i64, y: i64, z: i64) {
    c.cells([x - 1, y, z - 1], [x + 1, y + 4, z + 1], Some(K::canopy));
    c.cells([x - 2, y + 1, z - 1], [x + 2, y + 3, z + 1], Some(K::canopy));
    c.cells([x - 1, y + 1, z - 2], [x + 1, y + 3, z + 2], Some(K::canopy));
}

// ---------------------------------------------------------------------------
// Streets

/** What a column of the city is: road (marked or not), kerb, pavement (slabs in three shades), lawn. */
#[derive(Clone, Copy, PartialEq, Default)]
enum Spot {
    #[default]
    Lawn,
    Asphalt,
    Mark,
    Kerb,
    Paving(u8),
}

/** Where t (plan cells from the city's edge) falls: on a road or in a plot, and how far into it. */
fn strip(t: i64) -> (bool, i64) {
    let offset = t % PERIOD;
    if offset < ROAD { (true, offset) } else { (false, offset - ROAD) }
}

/** Road markings: a dashed centre line, and zebra crossings next to the junctions. */
fn road_paint(across: i64, along: i64) -> Spot {
    let zebra = (2..2 + vox(3.0)).contains(&along) || (PLOT - 2 - vox(3.0)..PLOT - 2).contains(&along);
    let mark =
        if zebra { (2..ROAD - 2).contains(&across) && (across >> 1) % 2 == 0 } else { across == ROAD / 2 && along % vox(6.0) < vox(3.0) };
    if mark { Spot::Mark } else { Spot::Asphalt }
}

/** What plan cell (x, z) of the ground is; the pavements' slabs, a metre square, shaded by `seed`. */
fn spot(seed: u64, x: i64, z: i64) -> Spot {
    let (tx, tz) = (x - MIN_X, z - MIN_Z);
    if !(0..SIZE).contains(&tx) || !(0..SIZE).contains(&tz) {
        return Spot::Lawn;
    }
    let ((x_road, x_off), (z_road, z_off)) = (strip(tx), strip(tz));
    match (x_road, z_road) {
        (true, true) => Spot::Asphalt,
        (true, false) => road_paint(x_off, z_off),
        (false, true) => road_paint(z_off, x_off),
        (false, false) => {
            let ring = x_off.min(z_off).min(PLOT - 1 - x_off).min(PLOT - 1 - z_off);
            if ring == 0 {
                Spot::Kerb
            } else if ring < SIDEWALK {
                Spot::Paving(((hash(seed, x.div_euclid(4), z.div_euclid(4), 0) as u64 * PAVING.len() as u64) >> 32) as u8)
            } else {
                Spot::Lawn
            }
        }
    }
}

/** The plan cell holding lattice coordinate `v`. */
fn cell_of(v: i64) -> i64 {
    (v + 1).div_euclid(PLAN)
}

/** The ground: roads a step below pavements and lawns, all on soil down to the world's bottom. */
fn ground(seed: u64) -> Field {
    // A plan of categories finer than far sites (markings, kerbs, slabs): dithered, so each shows as often as it covers.
    let spots = Plan::dithered("ground", PLAN, move |x, z| spot(seed, cell_of(x), cell_of(z)));
    Field::columns("ground", 0.0, Op::Paint, move |x, z, spacing| {
        let (surface, road, paved) = match spots.at(x, z) {
            Spot::Asphalt => (K::asphalt, true, false),
            Spot::Mark => (K::roadMark, true, false),
            Spot::Kerb => (K::kerb, false, true),
            Spot::Paving(shade) => (PAVING[shade as usize], false, true),
            Spot::Lawn => (K::lawn, false, false),
        };
        // The top plan cell: MIN_Y for roads, a cell higher elsewhere. Only where sites are as close as plan cells:
        // further apart, roads a step lower than the sites are fine would be hidden from above by the kerbs beside them.
        let top = if road && spacing <= PLAN { MIN_Y } else { MIN_Y + 1 };
        let column = Column::new(PLAN * top, spacing).surface(surface, PLAN);
        // Pavements on a cell of concrete, where sites are as fine.
        let column = if paved { column.fill_below_exactly(K::concrete, PLAN * MIN_Y - 1) } else { column };
        column.fill_below_exactly(K::soil, HEIGHTS.0)
    })
}

/** A street lamp on the pavement at (x, z), its arm reaching over the road along (dx, dz). */
fn street_lamp(c: &mut Canvas, x: i64, z: i64, dx: i64, dz: i64) {
    let height = vox(4.0);
    c.cells([x, 1, z], [x, height, z], Some(K::pole));
    c.cells([x + dx, height, z + dz], [x + 3 * dx, height, z + 3 * dz], Some(K::pole));
    c.put([x + 3 * dx, height - 1, z + 3 * dz], K::light);
}

fn street_tree(c: &mut Canvas, x: i64, z: i64) {
    c.cells([x - 1, 0, z - 1], [x + 1, 0, z + 1], Some(K::soil));
    c.cells([x, 1, z], [x, vox(2.25), z], Some(K::bark));
    tree_crown(c, x, vox(2.25), z);
}

/** A block of the district: its plot's corner, in plan cells, and its building. */
#[derive(Clone, Copy)]
struct Block {
    px: i64,
    pz: i64,
    building: Building,
    seed: u64,
}

impl Block {
    /** Its plot, pavements included, in lattice units, from the pavement's foot to above any roof. */
    fn bounds(&self) -> Box3 {
        let cells = |c0: i64, c1: i64| (PLAN * c0 - 1, PLAN * c1 + 1);
        let ((x0, x1), (y0, y1), (z0, z1)) = (cells(self.px, self.px + PLOT - 1), cells(MIN_Y, MAX_Y), cells(self.pz, self.pz + PLOT - 1));
        Box3 { min: [x0, y0, z0], max: [x1, y1, z1] }
    }

    /** Lamps at the corners and trees between them along every side; hedges round the back and sides of the lot, a gap behind the back door. */
    fn street(&self) -> Vec<Shape> {
        let c = &mut Canvas::default();
        let (px, pz, b) = (self.px, self.pz, &self.building);
        for t in [vox(2.0), PLOT - 1 - vox(2.0)] {
            street_lamp(c, px + t, pz + 1, 0, -1);
            street_lamp(c, px + t, pz + PLOT - 2, 0, 1);
            street_lamp(c, px + 1, pz + t, -1, 0);
            street_lamp(c, px + PLOT - 2, pz + t, 1, 0);
        }
        for t in [vox(8.0), PLOT - 1 - vox(8.0)] {
            street_tree(c, px + t, pz + vox(1.5));
            street_tree(c, px + t, pz + PLOT - 1 - vox(1.5));
            street_tree(c, px + vox(1.5), pz + t);
            street_tree(c, px + PLOT - 1 - vox(1.5), pz + t);
        }
        let (lx, lz) = (px + SIDEWALK, pz + SIDEWALK);
        let hedge = |c: &mut Canvas, x0: i64, x1: i64, z0: i64, z1: i64| {
            if x0 <= x1 && z0 <= z1 {
                c.cells([x0, 1, z0], [x1, 3, z1], Some(K::hedge));
            }
        };
        let back = lz + LOT - 1;
        match b.back_door.map(|i| b.bx + i * CELL + MID) {
            Some(gate) => {
                hedge(c, lx, gate - 4, back, back);
                hedge(c, gate + 4, lx + LOT - 1, back, back);
            }
            None => hedge(c, lx, lx + LOT - 1, back, back),
        }
        let from = (b.bz + b.d * CELL + 3).max(lz);
        hedge(c, lx, lx, from, back);
        hedge(c, lx + LOT - 1, lx + LOT - 1, from, back);
        std::mem::take(&mut c.shapes)
    }
}

/** The district's blocks: each building's size, storeys, courtyard, doors and colours, from `seed`. */
fn blocks(seed: u64) -> Vec<Block> {
    let mut blocks = Vec::new();
    for bi in 0..BLOCKS {
        for bj in 0..BLOCKS {
            let mut random = Rng::new(hash64(seed, bi, bj, 3));
            let (px, pz) = (MIN_X + bi * PERIOD + ROAD, MIN_Z + bj * PERIOD + ROAD);
            let front = bi == 1 && bj == 0;
            let w = 3 + (random.next() * 3.0) as i64;
            let d = 3 + (random.next() * 3.0) as i64;
            let (lx, lz) = (px + SIDEWALK, pz + SIDEWALK);
            let storeys = if front { 2 + (random.next() * 2.0) as i64 } else { 1 + (random.next() * MAX_STOREYS as f64) as i64 };
            let court =
                (random.next() < 0.6).then(|| (1 + (random.next() * (w - 2) as f64) as i64, 1 + (random.next() * (d - 2) as f64) as i64));
            let back_door = (random.next() < 0.5).then(|| (random.next() * w as f64) as i64);
            let building = Building {
                bx: lx + (LOT - w * CELL) / 2,
                bz: lz + 2,
                w,
                d,
                storeys,
                court,
                entrance: w / 2,
                back_door,
                facade: pick(&FACADES, random.next()),
                trim: pick(&TRIMS, random.next()),
                interior: pick(&INTERIORS, random.next()),
                entrance_door: pick(&ENTRANCE_DOORS, random.next()),
            };
            blocks.push(Block { px, pz, building, seed: hash64(seed, bi, bj, 4) });
        }
    }
    blocks
}

/**
 * A building is its outline, and while sites are at most INSIDE apart
 * (a wall a plan cell thick is then a sheet of sites), a child that cuts
 * the outline away and draws the building as it is. Beyond OUTLINE the
 * outline is measured: measured itself, a hollow building would keep
 * only the share of its cells its walls fill (Far::measured).
 */
const INSIDE: i64 = 2 * PLAN;
const OUTLINE: i64 = 2 * INSIDE;

/** The district: each block's building and its street, made only for the boxes they reach. */
fn district(blocks: Rc<Vec<Block>>) -> Node {
    let cells = |c0: i64, c1: i64| (PLAN * c0 - 1, PLAN * c1 + 1);
    let ((x0, x1), (y0, y1), (z0, z1)) = (cells(MIN_X, MIN_X + SIZE - 1), cells(MIN_Y, MAX_Y), cells(MIN_Z, MIN_Z + SIZE - 1));
    Node {
        shapes: Shapes::Now(Vec::new()),
        bounds: Some(Box3 { min: [x0, y0, z0], max: [x1, y1, z1] }),
        children: Some(Children::Reaching(Box::new(move |units: &Box3| {
            let mut children = Vec::new();
            for (n, block) in blocks.iter().enumerate() {
                let bounds = block.bounds();
                if !bounds.overlaps(units) {
                    continue;
                }
                let (shapes, outline) = draw_building(&block.building, &mut Rng::new(block.seed));
                let mut inside = vec![Shape::cuboid(bounds.min.map(|v| v as f64), bounds.max.map(|v| v as f64), 0).carve()];
                inside.extend(shapes);
                let building = Node::of(outline).with_children(vec![Child::new("inside", INSIDE, Node::of(inside))]);
                children.push(Child::new(format!("building {n}"), ANY_SIZE, building.with_far(Far::measured(fx(OUTLINE as f64)))));
                let block = *block;
                children.push(Child::new(format!("street {n}"), ANY_SIZE, Node::lazy(bounds, 0, move |_| block.street())));
            }
            children
        }))),
        far: None,
    }
}

/** The city of `seed`; the camera starts on the road in front of the middle front building's open entrance, at eye height. */
pub fn city(seed: u64) -> World {
    let blocks = Rc::new(blocks(seed));
    let front = blocks.iter().find(|b| b.px == MIN_X + PERIOD + ROAD && b.pz == MIN_Z + ROAD).expect("the middle front block").building;
    // In plan cells; their upper sites are sites of the lattice.
    let spawn = [front.bx + front.entrance * CELL + MID - 1, MIN_Y + vox(1.75), MIN_Z + vox(2.0)].map(|v| PLAN * v);
    // One thing, the district, in the cell centred on the origin: it reaches nowhere past the cell.
    let population = Population::new("district", 1.0, m(256.0))
        .anchor(|cell| (cell.centre == [0, 0, 0]).then_some([0.0; 3]))
        .make(move |_, _| Some(district(blocks.clone())));
    let layers = vec![Layer::Field(ground(stream(seed, 2))), Layer::Population(population)];
    World { generator: Generator { kinds: KINDS.to_vec(), seed: stream(seed, 6), layers }, spawn, heights: HEIGHTS, points: Vec::new() }
}
