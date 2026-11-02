//! The forest (forest.ts): an autumn forest valley, endless, on the
//! generator framework. Ported from forest.ts step for step; its numbers
//! are moving to integers (docs/generators.md).

use std::cell::RefCell;
use std::rc::Rc;

use crate::generator::{
    ANY_SIZE, Cell, Child, Children, Far, Field, FieldValues, Fill, Generator, Kind, KindDef, Layer, Node, Op, Point, Population, SOLID,
    Shape, Shapes, Span, area_profile, ball, bounds_of_shapes, capsule, cuboid, ellipsoid, lathe,
};
use crate::grid::Box3;
use crate::js::{self, round};
use crate::noise::{Rng, fbm, hash, hash64, smoothstep, stream, unit, value_noise};

/** Metres per lattice unit (UNIT_SIZE in forest.ts). */
pub const UNIT_SIZE: f64 = 0.125;
fn vox(metres: f64) -> f64 {
    round(metres / UNIT_SIZE)
}
const BRANCH_RADIUS: f64 = 1.2;
fn log_radius() -> f64 {
    vox(0.45)
}
const FINE: f64 = 1.0;
const PI: f64 = std::f64::consts::PI;

/** How far around the camera the grid of level 0 must hold the forest, in units. */
pub const REACH: f64 = 352.0;

/** The heights it has anything at, in units of `level`. */
pub fn heights(level: u32) -> (i64, i64) {
    let s = 1i64 << level;
    ((vox(-36.0) as i64).div_euclid(s), -(-(vox(80.0) as i64)).div_euclid(s))
}

macro_rules! kinds {
    ($($name:ident: $type:expr, [$($color:expr),*];)*) => {
        #[allow(non_camel_case_types, clippy::upper_case_acronyms)]
        #[derive(Clone, Copy, PartialEq, Eq)]
        #[repr(u8)]
        enum K { $($name),* }
        static KINDS: &[KindDef] = &[$(KindDef { kind_type: $type, colors: &[$($color),*] }),*];
    };
}

// In the order of KINDS in forest.ts: a kind's materials follow it.
kinds! {
    moss: 1, ["#4f7a2a", "#5a8a30", "#466e26", "#628f37"];
    grass: 1, ["#74a83c", "#7fb544", "#6a9c36"];
    tallGrass: 1, ["#8fbf4d", "#9cc95a", "#a3b84a"];
    litter: 2, ["#8a5a2b", "#9c6b30", "#7a4e24", "#a8763a"];
    needles: 2, ["#6b4a2e", "#5e4029", "#76533a"];
    path: 2, ["#a08262", "#957656", "#ab8c6b"];
    soil: 2, ["#4a3423", "#3f2c1e", "#54392a"];
    stone: 3, ["#7d7d78", "#6e6e6a", "#8a8a84"];
    mossyStone: 3, ["#5f7348", "#687c4f", "#56693f"];
    bark: 4, ["#4b3621", "#3f2d1c", "#57402a"];
    spruceBark: 4, ["#4a3426", "#5a4030"];
    birchBark: 4, ["#e6e2d6", "#dcd7c9", "#efece4"];
    birchMark: 4, ["#2e2b28", "#3b3733"];
    deadwood: 4, ["#6a5640", "#5c4a36", "#776149"];
    spruce: 5, ["#1f4a2c", "#24553a", "#1a3f26", "#2b5e3c"];
    oakGreen: 5, ["#4c7f2a", "#5a8f30", "#447324", "#679a38"];
    oakOrange: 5, ["#c46a1c", "#d68526", "#b0521a", "#9c4a1c"];
    oakGold: 5, ["#d9a521", "#c8941c", "#e3b53a", "#b88a1e"];
    birchGreen: 5, ["#7fae3c", "#8cbc45", "#9ac24f"];
    birchGold: 5, ["#d8cf4a", "#e6d65a", "#c9be36", "#efe07a"];
    fern: 5, ["#3f7a2c", "#4a8a34", "#56953a"];
    bracken: 5, ["#a8642a", "#b97a35", "#8f5524"];
    bush: 5, ["#3d6b28", "#477a2e", "#355f22"];
    flowerYellow: 6, ["#e8d34a", "#f0dc58"];
    flowerViolet: 6, ["#c9b1ff", "#b89cf0"];
    flowerWhite: 6, ["#f2f2f2", "#e8e8e0"];
    flowerRed: 6, ["#e86a6a", "#d95858"];
    barkFurrow: 4, ["#2e2115", "#352717", "#281c12"];
    spruceScale: 4, ["#33241a", "#2b1e15", "#3d2b1f"];
    twig: 4, ["#5a4430", "#4e3a28", "#634b35"];
    hay: 1, ["#b9a15a", "#c8b06a", "#a89050"];
    fallenLeaf: 2, ["#c4581c", "#d9861f", "#b8401a", "#e0b030", "#9a3a16", "#8c5a22"];
    cone: 4, ["#6b4423", "#7a5030", "#5c3a1e"];
    amanita: 6, ["#c8281e", "#d63a2a", "#b52018"];
    amanitaSpot: 6, ["#f4efe6", "#ebe4d6"];
    mushroomStem: 6, ["#e9e2d0", "#ddd4bf"];
    bolete: 6, ["#7a4a26", "#8a5530", "#6c4020"];
    berry: 6, ["#c01f2a", "#a81824", "#d43a30"];
}

fn k(kind: K) -> Kind {
    kind as Kind
}

// Shapes with forest.ts's defaults: ellipsoids, balls, lathes and boxes fill, capsules paint.
fn cub(min: Point, max: Point, kind: K) -> Shape {
    cuboid(min, max, k(kind), SOLID, Op::Fill)
}
fn cub_op(min: Point, max: Point, kind: K, op: Op) -> Shape {
    cuboid(min, max, k(kind), SOLID, op)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Species {
    Spruce,
    Birch,
    Oak,
}
const SPECIES: [Species; 3] = [Species::Spruce, Species::Birch, Species::Oak];

type GroundAt<'a> = &'a dyn Fn(i64, i64) -> i64;

fn make_tree(species: Species, x: f64, ground: f64, z: f64, random: &mut Rng, ground_at: GroundAt) -> Node {
    match species {
        Species::Spruce => spruce_tree(x, ground, z, random, ground_at),
        Species::Birch => birch_tree(x, ground, z, random),
        Species::Oak => oak_tree(x, ground, z, random, ground_at),
    }
}

/** The colour of each species' crowns from far away. */
fn foliage(species: Species) -> K {
    match species {
        Species::Spruce => K::spruce,
        Species::Birch => K::birchGold,
        Species::Oak => K::oakOrange,
    }
}

#[derive(Clone, Copy)]
struct Column {
    height: i64,
    surface: K,
    conifer: f64,
    density: f64,
    on_path: bool,
}

/** The forest's state shared by its layers: what it knows about any column, from the seed alone. */
struct Forest {
    seed: u64,
    path_phase: f64,
    spawn: [i64; 3],
    /** Recent ground heights, by a hash of (x, z): x, z, height. */
    kept: RefCell<(Vec<i64>, Vec<i64>, Vec<i64>)>,
}

impl Forest {
    fn s(&self, k: u64) -> u64 {
        stream(self.seed, k)
    }

    /** Path centre line, in metres. */
    fn path_x(&self, zm: f64) -> f64 {
        9.0 * js::sin(zm / 31.0 + self.path_phase) + 5.0 * js::sin(zm / 13.0 + self.path_phase * 2.0)
    }

    fn ground_metres(&self, xm: f64, zm: f64) -> f64 {
        let from_path = (xm - self.path_x(zm)).abs();
        let hills = (fbm(self.s(2), xm / 45.0, zm / 45.0, 4) - 0.5) * 10.0 + (value_noise(self.s(3), xm / 6.0, zm / 6.0) - 0.5) * 0.8;
        let valley = smoothstep(8.0, 42.0, from_path) * 18.0;
        let far = (fbm(self.s(9), xm / 900.0, zm / 900.0, 4) - 0.5) * 50.0;
        let path_dip = (1.0 - smoothstep(1.0, 2.2, from_path)) * 0.3;
        hills + valley + far - path_dip
    }

    /** Ground height in units. */
    fn ground(&self, x: i64, z: i64) -> i64 {
        let key = (((x as i32).wrapping_mul(0x9e3779b1u32 as i32) ^ (z as i32).wrapping_mul(0x85ebca77u32 as i32)) as u32 >> 18) as usize;
        let mut kept = self.kept.borrow_mut();
        if kept.0[key] != x || kept.1[key] != z {
            kept.0[key] = x;
            kept.1[key] = z;
            kept.2[key] = vox(self.ground_metres(x as f64 * UNIT_SIZE, z as f64 * UNIT_SIZE)) as i64;
        }
        kept.2[key]
    }

    /** Column (x, z), given the ground's height there and at its four neighbours. */
    fn column_of(&self, x: i64, z: i64, h: i64, neighbours: [i64; 4]) -> Column {
        let xm = x as f64 * UNIT_SIZE;
        let zm = z as f64 * UNIT_SIZE;
        let slope = neighbours.map(|n| (n - h).abs()).into_iter().max().expect("four");
        let conifer =
            smoothstep(0.4, 0.6, fbm(self.s(5), xm / 40.0, zm / 40.0, 4)) * 0.75 + smoothstep(6.0, 16.0, h as f64 * UNIT_SIZE) * 0.25;
        let density = smoothstep(0.3, 0.48, fbm(self.s(6), xm / 22.0, zm / 22.0, 4));
        let on_path = (xm - self.path_x(zm)).abs() < 1.1 + value_noise(self.s(7), zm / 3.0, 0.0) * 0.5;
        let floor = value_noise(self.s(8), xm / 2.5, zm / 2.5);
        let surface = if on_path {
            K::path
        } else if slope >= 3 {
            if floor < 0.5 { K::stone } else { K::mossyStone }
        } else if density < 0.2 {
            if floor < 0.75 { K::grass } else { K::moss }
        } else if conifer > 0.5 {
            if floor < 0.55 { K::needles } else { K::moss }
        } else if floor < 0.6 {
            K::litter
        } else {
            K::moss
        };
        Column { height: h, surface, conifer, density, on_path }
    }

    fn column(&self, x: i64, z: i64) -> Column {
        self.column_of(
            x,
            z,
            self.ground(x, z),
            [self.ground(x - 1, z), self.ground(x + 1, z), self.ground(x, z - 1), self.ground(x, z + 1)],
        )
    }

    /** A random spot on the ground under the square a cell reaches over (2N wide). */
    fn spot(&self, cell: &Cell, n: i64) -> Point {
        let [cx, _, cz] = cell.centre;
        let mut pick = Rng::new(hash64(self.s(19), n, cell.i, cell.k));
        let x = cx - n + pick.below(2 * n as u32) as i64;
        let z = cz - n + pick.below(2 * n as u32) as i64;
        [x as f64, self.ground(x, z) as f64, z as f64]
    }

    /** The chance a cell of trees around column (x, z) has one. */
    fn tree_chance(&self, x: i64, z: i64, c: &Column) -> f64 {
        if c.surface == K::stone || (x as f64 * UNIT_SIZE - self.path_x(z as f64 * UNIT_SIZE)).abs() < 2.8 {
            return 0.0;
        }
        // Keep the view from the spawn point clear of low branches.
        if js::hypot(&[(x - self.spawn[0]) as f64, (z - self.spawn[2]) as f64]) < vox(7.0) {
            return 0.0;
        }
        0.15 + c.density * 0.75
    }

    fn species_chances(c: &Column) -> [f64; 3] {
        [c.conifer, (1.0 - c.conifer) * 0.45, (1.0 - c.conifer) * 0.55]
    }
}

/** SPECIES.find(sp => (pick -= chances[sp]) < 0) ?? 'oak' */
fn pick_species(mut pick: f64, chances: &[f64; 3]) -> Species {
    for (i, species) in SPECIES.iter().enumerate() {
        pick -= chances[i];
        if pick < 0.0 {
            return *species;
        }
    }
    Species::Oak
}

/** The forest's generator for `seed`, and where the camera starts (forest() in forest.ts). */
pub fn forest(seed: u64) -> (Generator, [i64; 3]) {
    let mut f =
        Forest { seed, path_phase: 0.0, spawn: [0; 3], kept: RefCell::new((vec![i64::MIN; 1 << 14], vec![0; 1 << 14], vec![0; 1 << 14])) };
    f.path_phase = unit(hash(f.s(1), 0, 0, 0)) * 100.0;
    // Start on the path at the near end, at about the height of a tall shrub.
    let spawn_z = vox(-31.0) as i64;
    let spawn_x = round(f.path_x(spawn_z as f64 * UNIT_SIZE) / UNIT_SIZE) as i64;
    let spawn_y = f.ground(spawn_x, spawn_z) + vox(3.0) as i64;
    f.spawn = [spawn_x, spawn_y + ((spawn_x + spawn_y + spawn_z) & 1), spawn_z];
    let f = Rc::new(f);

    // The ground: along each column, soil or stone under a layer of its surface.
    let terrain = {
        let f = f.clone();
        Field {
            name: "terrain",
            order: 0.0,
            op: Op::Paint,
            values: FieldValues::Spans {
                axis: 1,
                spans: Box::new(move |x, z, spacing| {
                    let h = f.ground(x, z);
                    let around = [f.ground(x - spacing, z), f.ground(x + spacing, z), f.ground(x, z - spacing), f.ground(x, z + spacing)];
                    let surface = if spacing == 1 { f.column_of(x, z, h, around) } else { f.column(x, z) }.surface;
                    let bottom = around.into_iter().fold(h - vox(1.5) as i64, i64::min);
                    let top = h + 1 - (vox(1.0) as i64).max(2 * spacing);
                    let below = if surface == K::stone || surface == K::mossyStone { K::stone } else { K::soil };
                    vec![
                        Span { from: bottom, to: top, kind: k(below), extinction: None },
                        Span { from: top, to: h + 1, kind: k(surface), extinction: None },
                    ]
                }),
            },
        }
    };

    // Trees, each in the cell its foot is in.
    let tree_cell = vox(2.0) as i64;
    let tree_top = vox(26.0) as i64;
    let below: Rc<[Vec<f64>; 3]> = Rc::new(SPECIES.map(|species| {
        let samples: Vec<Node> =
            (0..64).map(|i| make_tree(species, 0.0, 0.0, 0.0, &mut Rng::new(hash64(f.s(17), i, 0, 0)), &|_, _| 0)).collect();
        let profile = area_profile(&samples, 1);
        let mut sums = vec![0.0; tree_top as usize + 1];
        for h in 0..tree_top as usize {
            sums[h + 1] = sums[h] + profile(h as i64);
        }
        sums
    }));
    let trees = Population {
        name: "trees",
        order: 1.0,
        cell: tree_cell,
        reach: [vox(9.0) as i64, tree_top, vox(9.0) as i64],
        size: vox(1.0) as i64,
        anchor: {
            let f = f.clone();
            Box::new(move |cell| Some(f.spot(cell, tree_cell)))
        },
        make: {
            let f = f.clone();
            Box::new(move |[x, y, z], cell| {
                let mut random = cell.random("tree");
                let (xi, zi) = (x as i64, z as i64);
                let c = f.column(xi, zi);
                if random.next() > f.tree_chance(xi, zi, &c) {
                    return None;
                }
                let species = pick_species(random.next(), &Forest::species_chances(&c));
                Some(make_tree(species, x, y, z, &mut random, &|x, z| f.ground(x, z)))
            })
        },
        // A medium as dense as the trees' area per volume over each column.
        far: Some({
            let f = f.clone();
            Field {
                name: "canopy",
                order: 1.0,
                op: Op::Fill,
                values: FieldValues::Spans {
                    axis: 1,
                    spans: Box::new(move |x, z, spacing| {
                        let c = f.column(x, z);
                        let per_area = f.tree_chance(x, z, &c) / ((2 * tree_cell) * (2 * tree_cell)) as f64;
                        if per_area == 0.0 {
                            return Vec::new();
                        }
                        let chances = Forest::species_chances(&c);
                        let kind = k(foliage(pick_species(unit(hash(f.s(18), x, z, 0)), &chances)));
                        let mut spans = Vec::new();
                        let mut from = 0;
                        while from < tree_top {
                            let to = (from + spacing).min(tree_top);
                            let area = (0..3).fold(0.0, |sum, sp| sum + chances[sp] * (below[sp][to as usize] - below[sp][from as usize]));
                            if area > 0.0 {
                                spans.push(Span {
                                    from: c.height + from,
                                    to: c.height + from + spacing,
                                    kind,
                                    extinction: Some((per_area * area) / spacing as f64 / 4.0),
                                });
                            }
                            from += spacing;
                        }
                        spans
                    }),
                },
            }
        }),
    };

    // Boulders and fallen logs.
    let rock_cell = vox(4.0) as i64;
    let boulders = Population {
        name: "boulders",
        order: 2.0,
        cell: rock_cell,
        reach: [vox(3.0) as i64; 3],
        size: vox(2.0) as i64,
        anchor: {
            let f = f.clone();
            Box::new(move |cell| Some(f.spot(cell, rock_cell)))
        },
        make: Box::new(|[x, y, z], cell| {
            let mut random = cell.random("boulder");
            if random.next() < 0.15 { Some(Node::of(boulder(x, y, z, &mut random))) } else { None }
        }),
        far: None,
    };
    let logs = Population {
        name: "logs",
        order: 2.0,
        cell: rock_cell,
        reach: [vox(10.0) as i64; 3],
        size: vox(2.0) as i64,
        anchor: {
            let f = f.clone();
            Box::new(move |cell| Some(f.spot(cell, rock_cell)))
        },
        make: {
            let f = f.clone();
            Box::new(move |[x, _, z], cell| {
                let mut random = cell.random("log");
                if random.next() < 0.077 && !f.column(x as i64, z as i64).on_path {
                    Some(Node::of(fallen_log(x, z, &|x, z| f.ground(x, z), &mut random)))
                } else {
                    None
                }
            })
        },
        far: None,
    };

    // Undergrowth, per column of (0.25 m)², in square patches N wide around each cell's centre.
    const PLANTS: i64 = 16;
    let column_width = vox(0.25) as i64;
    let plants = Population {
        name: "plants",
        order: 3.0,
        cell: PLANTS,
        reach: [0, vox(10.0) as i64, 0],
        size: column_width,
        anchor: {
            let f = f.clone();
            Box::new(move |cell| {
                let [cx, _, cz] = cell.centre;
                Some([cx as f64, f.ground(cx, cz) as f64 + 0.5, cz as f64])
            })
        },
        make: {
            let f = f.clone();
            Box::new(move |_, cell| {
                let [cx, _, cz] = cell.centre;
                let mut shapes = Vec::new();
                let mut xi = cx - PLANTS / 2;
                while xi < cx + PLANTS / 2 {
                    let mut zi = cz - PLANTS / 2;
                    while zi < cz + PLANTS / 2 {
                        let c = f.column(xi, zi);
                        let (x, y, z) = (xi as f64, (c.height + 1) as f64, zi as f64);
                        let r = unit(hash(f.s(15), xi, zi, 0));
                        let random = &mut Rng::new(hash64(f.s(16), xi, zi, 0));
                        if c.on_path {
                            if r < 0.05 {
                                shapes.push(cub([x, y, z], [x + 2.0, y + 1.0, z + 1.0], K::stone));
                            }
                        } else if c.surface == K::grass {
                            if r < 0.02 {
                                let count = 3.0 + random.below(3) as f64;
                                let kind = if random.next() < 0.3 { K::hay } else { K::tallGrass };
                                shapes.extend(tuft(x, y, z, count, vox(0.3), kind, random));
                            } else if r < 0.0275 {
                                shapes.extend(flower(x, y, z, random));
                            } else if r < 0.2 {
                                let count = 1.0 + random.below(2) as f64;
                                shapes.extend(tuft(x, y, z, count, vox(0.12), K::grass, random));
                            }
                        } else if c.surface != K::stone && c.surface != K::mossyStone {
                            if r < 0.00625 {
                                shapes.extend(fern(x, y, z, if c.conifer > 0.5 { K::fern } else { K::bracken }, random));
                            } else if r < 0.008 {
                                shapes.extend(bush(x, y, z, random));
                            } else if r < 0.0085 {
                                shapes.extend(flower(x, y, z, random));
                            } else if r < 0.011 && c.density > 0.3 {
                                shapes.extend(mushrooms(x, y, z, if c.conifer > 0.5 { K::amanita } else { K::bolete }, random));
                            } else if c.surface == K::needles && r < 0.04 {
                                shapes.push(cub([x, y, z], [x + 2.0, y + 2.0, z + 1.0], K::cone));
                            } else if c.surface == K::litter && r < 0.4 {
                                shapes.push(cub_op([x - 1.0, y - 1.0, z - 1.0], [x + 1.0, y, z + 1.0], K::fallenLeaf, Op::Paint));
                            }
                        }
                        zi += column_width;
                    }
                    xi += column_width;
                }
                if shapes.is_empty() { None } else { Some(Node::of(shapes)) }
            })
        },
        far: None,
    };

    let spawn = f.spawn;
    let generator = Generator {
        kinds: KINDS,
        seed: f.s(4),
        layers: vec![
            Layer::Field(terrain),
            Layer::Population(trees),
            Layer::Population(boulders),
            Layer::Population(logs),
            Layer::Population(plants),
        ],
    };
    (generator, spawn)
}

// ---------------------------------------------------------------------------
// Things. `random` picks their shapes.

struct Crown {
    trunk: Shape,
    clusters: Vec<Child>,
    fill: f64,
}

/** A tree of `shapes`, with the parts its crown hides as a child (tree() in forest.ts). */
fn tree(shapes: Vec<Shape>, hidden: Vec<Shape>, fine: Vec<Child>, crown: Option<Crown>) -> Node {
    let hidden_bounds = bounds_of_shapes(hidden.iter().map(|s| &s.bounds));
    let mut children = vec![Child { name: "hidden".into(), size: vox(0.5) as i64, node: Node::of(hidden) }];
    children.extend(fine);
    let far = crown.map(|Crown { trunk, clusters, fill }| {
        let far_shapes: Vec<&Shape> = clusters.iter().flat_map(|c| c.node.far.iter().flat_map(|f| &f.shapes)).collect();
        // An ellipsoid of the clusters' outline, shrunk to their volume and solid.
        let b = bounds_of_shapes(far_shapes.iter().map(|s| &s.bounds));
        let half = [0, 1, 2].map(|i| (b.max[i] - b.min[i]) as f64 / 2.0);
        let volume = (4.0 / 3.0) * PI * half[0] * half[1] * half[2];
        let filled = far_shapes.iter().fold(0.0, |sum, s| {
            let [a, c, d] = s.extents;
            sum + (PI / 6.0) * a * c * d * fill
        });
        let shrink = js::pow(1.0 - js::exp(-filled / volume), 1.0 / 3.0);
        let outline = ellipsoid(
            [0, 1, 2].map(|i| (b.min[i] + b.max[i]) as f64 / 2.0),
            [half[0] * shrink, half[1] * shrink, half[2] * shrink],
            far_shapes[0].kind,
            SOLID,
            Op::Fill,
        );
        let detail = js::min(far_shapes.iter().map(|s| js::min(s.extents)));
        let far = Far { detail, shapes: vec![trunk, outline] };
        children.extend(clusters);
        far
    });
    let child_bounds: Vec<Box3> = children
        .iter()
        .skip(1)
        .map(|c| c.node.bounds.unwrap_or_else(|| bounds_of_shapes(c.node.shapes.get().iter().map(|s| &s.bounds))))
        .collect();
    let bounds = bounds_of_shapes(shapes.iter().map(|s| &s.bounds).chain([&hidden_bounds]).chain(child_bounds.iter()));
    Node { shapes: Shapes::Now(shapes), bounds: Some(bounds), children: Some(Children::Fixed(children)), far }
}

/** A vertical trunk, `width` units square, from just below `ground` up to `top`. */
fn trunk(x: f64, ground: f64, z: f64, top: f64, width: f64, kind: K) -> Shape {
    cub_op([x, ground - 1.0, z], [x + width, top + 1.0, z + width], kind, Op::Paint)
}

/** A random direction, evenly over the sphere. */
fn direction(random: &mut Rng) -> Point {
    let y = 2.0 * random.next() - 1.0;
    let a = 2.0 * PI * random.next();
    let r = (1.0 - y * y).sqrt();
    [r * js::cos(a), y, r * js::sin(a)]
}

/** A cluster of leaves around `centre`: sprays of leaves on twigs at level 0, further the ball they fill. */
#[allow(clippy::too_many_arguments)]
fn leaf_cluster(name: String, centre: Point, radius: f64, kind: K, fill: f64, random: &mut Rng, droop: f64, turning: K) -> Child {
    let seed = random.next_u64();
    let sprays = Shapes::lazy(seed, move |random| {
        let spray = vox(0.45).min(radius * 0.4);
        let count = 4f64.max(round(0.95 * js::pow(radius / spray, 2.0)));
        let (mut twigs, mut leaves) = (Vec::new(), Vec::new());
        let at = |d: Point, r: f64| [centre[0] + d[0] * r, centre[1] + d[1] * r, centre[2] + d[2] * r];
        for _ in 0..count as usize {
            let [dx, dy, dz] = direction(random);
            let length = js::hypot(&[dx, dy - droop, dz]);
            let d = [dx / length, (dy - droop) / length, dz / length];
            let tip = radius - spray;
            twigs.push(capsule(at(d, radius * 0.1), at(d, tip - spray / 2.0), FINE, k(K::twig), SOLID, Op::Fill));
            let leaf = if random.next() < 0.25 { turning } else { kind };
            leaves.push(capsule(at(d, radius * 0.3), at(d, tip), spray, k(leaf), Fill::Const(0.75), Op::Fill));
        }
        twigs.push(ball(centre, radius * 0.55, k(kind), Fill::Const(fill), Op::Fill));
        twigs.extend(leaves);
        twigs
    });
    let far = ball(centre, radius, k(kind), Fill::Const(fill), Op::Fill);
    Child {
        name,
        size: ANY_SIZE,
        node: Node { shapes: sprays, bounds: Some(far.bounds), children: None, far: Some(Far { detail: FINE, shapes: vec![far] }) },
    }
}

/** A child of a thing within `bounds`, drawn only at level 0, its shapes made only then. */
fn fine_child(name: &str, bounds: Box3, random: &mut Rng, make: impl Fn(&mut Rng) -> Vec<Shape> + 'static) -> Child {
    let seed = random.next_u64();
    Child {
        name: name.into(),
        size: FINE as i64,
        node: Node { shapes: Shapes::lazy(seed, make), bounds: Some(bounds), children: None, far: None },
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum BarkStyle {
    Furrows,
    Dashes,
}

/** Bark on the four faces of a trunk [x, x + width) × [from, to) × [z, z + width). */
#[allow(clippy::too_many_arguments)]
fn bark(x: f64, z: f64, width: f64, from: f64, to: f64, style: BarkStyle, kind: K, random: &mut Rng) -> Vec<Shape> {
    let mut shapes = Vec::new();
    let mark = |f: usize, u: f64, length: f64, y0: f64, y1: f64| {
        let layer = if f.is_multiple_of(2) {
            if f < 2 { z } else { x }
        } else if f < 2 {
            z + width - 1.0
        } else {
            x + width - 1.0
        };
        if f < 2 {
            cub_op([u, y0, layer], [u + length, y1, layer + 1.0], kind, Op::Paint)
        } else {
            cub_op([layer, y0, u], [layer + 1.0, y1, u + length], kind, Op::Paint)
        }
    };
    let start = |f: usize| if f < 2 { x } else { z };
    for f in 0..4 {
        if style == BarkStyle::Furrows {
            let mut n = 1f64.max(round(width / 3.0));
            while n > 0.0 {
                let mut u = start(f) + random.below(width as u32 - 1) as f64;
                let mut y = from;
                while y < to {
                    let length = vox(0.4 + random.next() * 1.2);
                    shapes.push(mark(f, u, 2.0, y, (y + length).min(to)));
                    y += length + (random.next() * vox(0.3)).floor();
                    u = start(f).max((start(f) + width - 2.0).min(u + random.below(3) as f64 - 1.0));
                }
                n -= 1.0;
            }
        } else {
            let mut y = from;
            while y < to {
                if random.next() < 0.5 {
                    let u = start(f) + random.below(width as u32 - 1) as f64;
                    let length = 2.0 + random.below(2) as f64;
                    shapes.push(mark(f, u, length, y, y + 1.0));
                }
                y += vox(0.25);
            }
        }
    }
    shapes
}

/** Roots spreading from a trunk's foot around (x, z) into the ground, `half` its half-width. */
fn roots(x: f64, ground: f64, z: f64, half: f64, kind: K, random: &mut Rng, ground_at: GroundAt) -> Vec<Shape> {
    let count = 4.0 + random.below(3) as f64;
    (0..count as usize)
        .map(|i| {
            let angle = (i as f64 / count) * PI * 2.0 + random.next() * 0.6;
            let (c, s) = (js::cos(angle), js::sin(angle));
            let out = half + vox(0.5 + random.next() * 0.7);
            let (ex, ez) = (x + c * out, z + s * out);
            let p = [x + c * half * 0.6, ground + vox(0.35), z + s * half * 0.6];
            let q = [ex, (ground_at(round(ex) as i64, round(ez) as i64) - 1) as f64, ez];
            capsule(p, q, vox(0.15 + random.next() * 0.08), k(kind), SOLID, Op::Fill)
        })
        .collect()
}

/** Conical spruce with drooping whorls of branches. */
fn spruce_tree(x: f64, ground: f64, z: f64, random: &mut Rng, ground_at: GroundAt) -> Node {
    let height = vox(14.0 + random.next() * 10.0);
    let width = if height >= vox(20.0) { vox(1.0) } else { vox(0.5) };
    let top = ground + height;
    let base = ground + round(height * (0.15 + random.next() * 0.15));
    let bole = trunk(x, ground, z, base, width, K::spruceBark);
    let hidden = vec![cub_op([x, base + 1.0, z], [x + width, top, z + width], K::spruceBark, Op::Paint)];
    let radius = height * (0.17 + random.next() * 0.05);
    let whorl = vox(1.5 + random.next() * 0.6);
    let (cx, cz) = (x + (width - 1.0) / 2.0, z + (width - 1.0) / 2.0);
    let crown_top = top + vox(0.5);
    let r = move |y: f64| {
        // Each whorl is widest at its bottom, giving the layered silhouette.
        let phase = (((top - y) % whorl) + whorl) % whorl / whorl;
        radius * (1.0 - (y - base) / (crown_top - base)) * (0.5 + 0.5 * phase) + vox(0.35)
    };
    let (from, to) = (base - 0.5, crown_top + 0.5);
    let density = move |distance: f64, py: f64| 0.95 - 0.35 * js::pow(distance / r(py), 3.0);
    let spokes = 6.0 + random.below(3) as f64;
    let sector = (2.0 * PI) / spokes;
    let twist = random.next() * sector;
    let half_width = |distance: f64| vox(0.15) + 0.07 * distance;
    let covered = move |distance: f64| 1f64.min((2.0 * half_width(distance)) / (distance * sector));
    let fans = move |px: f64, py: f64, pz: f64| {
        let distance = js::hypot(&[px - cx, pz - cz]);
        if distance < vox(0.5) {
            return density(distance, py);
        }
        let a = js::atan2(pz - cz, px - cx) - twist - ((top - py) / whorl).floor() * 2.39996;
        let off = ((a % sector) + sector) % sector;
        if off.min(sector - off) * distance < half_width(distance) { density(distance, py) } else { 0.0 }
    };
    let crown = |fill: Fill| lathe([cx, from, cz], [cx, to, cz], move |t| r(round(from + t * (to - from))), k(K::spruce), fill, Op::Fill);
    let foot = roots(cx, ground, cz, width / 2.0, K::spruceBark, random, ground_at);
    let fine = vec![fine_child("bark", bole.bounds, random, move |random| {
        bark(x, z, width, ground, base, BarkStyle::Furrows, K::spruceScale, random)
    })];
    let mut shapes = vec![bole.clone()];
    shapes.extend(foot.iter().cloned());
    shapes.push(crown(Fill::At(Rc::new(fans))));
    let mut near = tree(shapes, hidden, fine, None);
    // Its hidden parts are not drawn beyond level 0: inside a crown this dense they don't show.
    let mut far = vec![bole];
    far.extend(foot);
    far.push(crown(Fill::At(Rc::new(move |px, py, pz| {
        let distance = js::hypot(&[px - cx, pz - cz]);
        density(distance, py) * covered(vox(0.5).max(distance))
    }))));
    near.far = Some(Far { detail: FINE, shapes: far });
    near
}

/** Slender white birch with a wavy trunk and a loose, clumpy crown of drooping twigs. */
fn birch_tree(x: f64, ground: f64, z: f64, random: &mut Rng) -> Node {
    let height = vox(12.0 + random.next() * 7.0);
    let width = vox(0.375);
    let (mut shapes, mut marks) = (Vec::new(), Vec::new());
    let mut segments: Vec<[f64; 4]> = Vec::new();
    // The trunk shifts by a unit now and then.
    let (mut tx, mut tz, mut from) = (x, z, ground - 1.0);
    let mut y = ground - 1.0;
    while y <= ground + height + 1.0 {
        let end = y == ground + height + 1.0;
        if end || ((y - ground) % vox(3.0) == 0.0 && y - ground > vox(4.0) && random.next() < 0.35) {
            shapes.push(cub_op([tx, from, tz], [tx + width, y, tz + width], K::birchBark, Op::Paint));
            segments.push([tx, tz, from.max(ground), y]);
            if end {
                break;
            }
            if random.next() < 0.5 {
                tx += if random.next() < 0.5 { -1.0 } else { 1.0 };
            } else {
                tz += if random.next() < 0.5 { -1.0 } else { 1.0 };
            }
            from = y;
        }
        // Black marks in bands half a metre tall.
        if (y - ground) % vox(0.5) == 0.0 && random.next() < 0.18 {
            marks.push(cub_op([tx, y, tz], [tx + width, y + vox(0.5), tz + width], K::birchMark, Op::Paint));
        }
        y += 1.0;
    }
    shapes.extend(marks);
    let foliage = if random.next() < 0.65 { K::birchGold } else { K::birchGreen };
    let turning = if foliage == K::birchGold { K::birchGreen } else { K::birchGold };
    let crown_bottom = ground + height * 0.45;
    let bole = cub_op([x, ground - 1.0, z], [x + width, crown_bottom, z + width], K::birchBark, Op::Paint);
    let crown_top = ground + height + vox(1.0);
    let cy = (crown_bottom + crown_top) / 2.0;
    let ry = (crown_top - crown_bottom) / 2.0;
    let rx = vox(2.0 + random.next() * 1.2);
    let count = 12.0 + random.below(6) as f64;
    let mid = (width - 1.0) / 2.0;
    let (mut branches, mut clusters) = (Vec::new(), Vec::new());
    for i in 0..count as usize {
        // Uniform point in the crown ellipsoid.
        let (mut ox, mut oy, mut oz);
        loop {
            ox = random.next() * 2.0 - 1.0;
            oy = random.next() * 2.0 - 1.0;
            oz = random.next() * 2.0 - 1.0;
            if ox * ox + oy * oy + oz * oz <= 1.0 {
                break;
            }
        }
        let p = [tx + ox * rx, cy + oy * ry, tz + oz * rx];
        branches.push(capsule([tx + mid, p[1] - vox(0.5), tz + mid], p, BRANCH_RADIUS, k(K::birchBark), SOLID, Op::Paint));
        let radius = vox(0.8 + random.next() * 0.6);
        clusters.push(leaf_cluster(format!("leaves {i}"), p, radius, foliage, 0.7, random, 0.6, turning));
    }
    let dashes = fine_child("bark", bounds_of_shapes(shapes.iter().map(|s| &s.bounds)), random, move |random| {
        segments.iter().flat_map(|&[sx, sz, y0, y1]| bark(sx, sz, width, y0, y1, BarkStyle::Dashes, K::birchMark, random)).collect()
    });
    tree(shapes, branches, vec![dashes], Some(Crown { trunk: bole, clusters, fill: 0.7 }))
}

/** Broad oak: thick trunk, a few branches each ending in a leaf cluster. */
fn oak_tree(x: f64, ground: f64, z: f64, random: &mut Rng, ground_at: GroundAt) -> Node {
    let height = vox(12.0 + random.next() * 8.0);
    let trunk_top = ground + round(height * (0.45 + random.next() * 0.1));
    let width = vox(1.0);
    let mut shapes = vec![trunk(x, ground, z, trunk_top, width, K::bark)];
    let roll = random.next();
    let foliage = if roll < 0.45 {
        K::oakGreen
    } else if roll < 0.75 {
        K::oakOrange
    } else {
        K::oakGold
    };
    let turning = if foliage == K::oakGold { K::oakOrange } else { K::oakGold };
    let cx = x + (width - 1.0) / 2.0;
    let cz = z + (width - 1.0) / 2.0;
    shapes.extend(roots(cx, ground, cz, width / 2.0, K::bark, random, ground_at));
    let branches = 4.0 + random.below(3) as f64;
    let mut ends: Vec<Point> = vec![[cx, ground + height - vox(2.0), cz]];
    let mut hidden = Vec::new();
    for i in 0..branches as usize {
        let angle = (i as f64 / branches) * PI * 2.0 + random.next() * 0.8;
        let reach = vox(2.5 + random.next() * 2.5);
        let start_y = trunk_top - (random.next() * vox(2.0)).floor();
        let end_x = cx + js::cos(angle) * reach;
        let end_y = (start_y + vox(2.0 + random.next() * 3.0)).min(ground + height - vox(1.5));
        let end = [end_x, end_y, cz + js::sin(angle) * reach];
        hidden.push(capsule([cx, start_y, cz], end, BRANCH_RADIUS * 1.5, k(K::bark), SOLID, Op::Paint));
        ends.push(end);
    }
    let clusters = ends
        .iter()
        .enumerate()
        .map(|(i, end)| {
            let radius = vox(1.8 + random.next() * 1.2);
            leaf_cluster(format!("leaves {i}"), *end, radius, foliage, 0.8, random, 0.0, turning)
        })
        .collect();
    let fine = vec![fine_child("bark", shapes[0].bounds, random, move |random| {
        bark(x, z, width, ground, trunk_top, BarkStyle::Furrows, K::barkFurrow, random)
    })];
    let trunk_shape = shapes[0].clone();
    tree(shapes, hidden, fine, Some(Crown { trunk: trunk_shape, clusters, fill: 0.8 }))
}

// Undergrowth on the ground at (x, y, z): blades, stems and leaves a unit or two thick.

/** A tuft of `count` blades about `tall` high. */
fn tuft(x: f64, y: f64, z: f64, count: f64, tall: f64, kind: K, random: &mut Rng) -> Vec<Shape> {
    (0..count as usize)
        .map(|_| {
            let dx = random.below(3) as f64 - 1.0;
            let dz = random.below(3) as f64 - 1.0;
            let blade = 1f64.max(round(tall * (0.6 + random.next() * 0.6)));
            cub([x + dx, y, z + dz], [x + dx + 2.0, y + blade, z + dz + 1.0], kind)
        })
        .collect()
}

const FLOWERS: [K; 4] = [K::flowerYellow, K::flowerViolet, K::flowerWhite, K::flowerRed];

/** A flower: a stem and a head two sites wide. */
fn flower(x: f64, y: f64, z: f64, random: &mut Rng) -> Vec<Shape> {
    let stem = 2.0 + random.below(2) as f64;
    let colour = FLOWERS[random.below(FLOWERS.len() as u32) as usize];
    vec![
        cub([x, y, z], [x + 2.0, y + stem, z + 1.0], K::tallGrass),
        cub([x - 1.0, y + stem, z - 1.0], [x + 1.0, y + stem + 1.0, z + 1.0], colour),
    ]
}

/** A fern or bracken clump about a metre across. */
fn fern(x: f64, y: f64, z: f64, kind: K, random: &mut Rng) -> Vec<Shape> {
    let fronds = 5.0 + random.below(4) as f64;
    let mut shapes = Vec::new();
    for i in 0..fronds as usize {
        let angle = (i as f64 / fronds) * PI * 2.0 + random.next() * 0.5;
        let (c, s) = (js::cos(angle), js::sin(angle));
        let length = vox(0.45 + random.next() * 0.35);
        let rise = vox(0.25 + random.next() * 0.2);
        let bend = [x + c * length * 0.5, y + rise, z + s * length * 0.5];
        shapes.push(capsule([x, y, z], bend, 1.1, k(kind), Fill::Const(0.85), Op::Fill));
        shapes.push(capsule(bend, [x + c * length, y + rise * 0.3, z + s * length], 1.0, k(kind), Fill::Const(0.7), Op::Fill));
    }
    shapes
}

/** A bush, half the time with red berries on its upper side. */
fn bush(x: f64, y: f64, z: f64, random: &mut Rng) -> Vec<Shape> {
    let radius = vox(0.5 + random.next() * 0.5);
    let mut shapes = vec![ball([x, y, z], radius, k(K::bush), Fill::Const(0.8), Op::Fill)];
    if random.next() < 0.5 {
        let mut n = 4.0 + random.below(6) as f64;
        while n > 0.0 {
            let [dx, dy, dz] = direction(random);
            let (px, py, pz) = (round(x + dx * radius * 0.9), round(y + dy.abs() * radius * 0.9), round(z + dz * radius * 0.9));
            // On a site.
            let at = [px, py + ((px as i64 + py as i64 + pz as i64) & 1) as f64, pz];
            shapes.push(cub_op(at, [at[0] + 1.0, at[1] + 1.0, at[2] + 1.0], K::berry, Op::Paint));
            n -= 1.0;
        }
    }
    shapes
}

/** One to three mushrooms: fly agarics (white-spotted red caps) or boletes. */
fn mushrooms(x: f64, y: f64, z: f64, kind: K, random: &mut Rng) -> Vec<Shape> {
    let mut shapes = Vec::new();
    let mut n = 1.0 + random.below(3) as f64;
    while n > 0.0 {
        let mx = x + random.below(5) as f64 - 2.0;
        let mz = z + random.below(5) as f64 - 2.0;
        let stem = 1.0 + random.below(2) as f64;
        let cap = [mx + 0.5, y + stem, mz];
        let rx = 1.4 + random.next() * 0.4;
        let radii = [rx, 1.0, 1.4 + random.next() * 0.4];
        shapes.push(cub([mx, y, mz], [mx + 2.0, y + stem, mz + 1.0], K::mushroomStem));
        shapes.push(ellipsoid(cap, radii, k(kind), SOLID, Op::Fill));
        if kind == K::amanita {
            let top = cap[1];
            shapes.push(ellipsoid(
                cap,
                radii,
                k(K::amanitaSpot),
                Fill::At(Rc::new(move |_, py, _| if py > top { 0.35 } else { 0.0 })),
                Op::Paint,
            ));
        }
        n -= 1.0;
    }
    shapes
}

/** A half-buried rock with moss on its upper side. */
fn boulder(x: f64, ground: f64, z: f64, random: &mut Rng) -> Vec<Shape> {
    let rx = vox(0.8 + random.next() * 1.4);
    let ry = rx * (0.6 + random.next() * 0.3);
    let rz = vox(0.8 + random.next() * 1.4);
    vec![
        ellipsoid([x, ground, z], [rx, ry, rz], k(K::stone), SOLID, Op::Paint),
        ellipsoid(
            [x, ground, z],
            [rx, ry, rz],
            k(K::mossyStone),
            Fill::At(Rc::new(move |_, y, _| if y - ground > ry * 0.4 { 0.7 } else { 0.0 })),
            Op::Paint,
        ),
    ]
}

/** A fallen trunk lying across the ground, with moss on top. */
fn fallen_log(x: f64, z: f64, ground: GroundAt, random: &mut Rng) -> Vec<Shape> {
    let length = vox(4.0 + random.next() * 5.0);
    let angle = random.next() * PI;
    let dx = js::cos(angle);
    let dz = js::sin(angle);
    let (ex, ez) = (x + dx * length, z + dz * length);
    let lift = log_radius();
    let at = |x: f64, z: f64| ground(x as i64, z as i64) as f64;
    let mut shapes =
        vec![capsule([x, at(x, z) + lift, z], [ex, at(round(ex), round(ez)) + lift, ez], log_radius(), k(K::deadwood), SOLID, Op::Paint)];
    // Moss in patches on top.
    let mut i = 0.0;
    while i <= length {
        let px = round(x + dx * i);
        let pz = round(z + dz * i);
        if random.next() < 0.5 {
            shapes.push(ball([px, at(px, pz) + lift * 2.0 + vox(0.25), pz], vox(0.3), k(K::moss), SOLID, Op::Fill));
        }
        i += vox(0.5);
    }
    shapes
}
