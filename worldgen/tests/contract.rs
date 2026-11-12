//! The engine on small test worlds: they keep the contract
//! (worldgen::contract), and the framework refuses what breaks it.

use std::rc::Rc;

use worldgen::World;
use worldgen::contract::{Measure, REGION, Report, check, heights_hold, margins, regions, things_hold, whole_equals_parts};
use worldgen::generator::{
    ANY_SIZE, Anchors, Child, Children, Field, FieldValues, Generator, KindDef, Layer, Node, Op, Population, SOLID, Shapes, Span, ball,
    cuboid, generate, materials_of, things_in,
};
use worldgen::generator::{Cell, Fill, Point};
use worldgen::grid::Box3;
use worldgen::plan::Plan;
use worldgen::survey::survey;

static KINDS: &[KindDef] = &[
    KindDef { name: "fog", kind_type: 8, colors: &["#cccccc"], walk_through: false, glows: false, medium: true },
    KindDef { name: "rock", kind_type: 3, colors: &["#777777"], walk_through: false, glows: false, medium: false },
    KindDef { name: "wall", kind_type: 10, colors: &["#eeeeee"], walk_through: false, glows: false, medium: false },
    KindDef { name: "wood", kind_type: 4, colors: &["#664422"], walk_through: false, glows: false, medium: false },
    KindDef { name: "grass", kind_type: 1, colors: &["#44aa44"], walk_through: false, glows: false, medium: false },
    KindDef { name: "smoke", kind_type: 8, colors: &["#333333"], walk_through: false, glows: false, medium: true },
    KindDef { name: "glowing fog", kind_type: 8, colors: &["#cccccc"], walk_through: false, glows: true, medium: true },
];
const FOG: u8 = 0;
const ROCK: u8 = 1;
const WALL: u8 = 2;
const WOOD: u8 = 3;
const GRASS: u8 = 4;
const SMOKE: u8 = 5;
const GLOWING_FOG: u8 = 6;

/** One population of one thing, near the origin: cells reach 1024 + `reach` from their centres. */
fn single(make: impl Fn(Point, &Cell) -> Option<Node> + 'static, reach: i64) -> Layer {
    Layer::Population(Population {
        name: "single",
        order: 1.0,
        cell: 1024,
        reach: [reach; 3],
        size: ANY_SIZE,
        anchor: Anchors::Cells(Box::new(|cell| if cell.i == 0 && cell.j == 0 && cell.k == 0 { Some([64.0, 64.0, 64.0]) } else { None })),
        make: Box::new(make),
        far: None,
        on: None,
    })
}

fn spans(name: &'static str, from: i64, to: i64, kind: u8) -> Layer {
    Layer::Field(Field {
        name,
        order: 0.0,
        op: Op::Paint,
        values: FieldValues::Spans { axis: 1, spans: Box::new(move |_, _, _| vec![Span { from, to, kind, extinction: None }]) },
        about: None,
    })
}

/** A ball of fog, each site kept with chance 0.3. */
fn fog() -> Generator {
    Generator {
        kinds: KINDS.to_vec(),
        seed: 1,
        layers: vec![single(|_, _| Some(Node::of(vec![ball([64.0; 3], 56.0, FOG, Fill::Const(0.3), Op::Fill)])), 64)],
    }
}

/** Rock with winding tunnels carved out of it. */
fn caves() -> Generator {
    let tunnels = Field {
        name: "tunnels",
        order: 1.0,
        op: Op::Carve,
        values: FieldValues::At(Box::new(|x, y, z, _| {
            let (x, y, z) = (x as f64, y as f64, z as f64);
            let v = (x / 23.0).sin() + (y / 17.0).sin() + (z / 29.0).sin() + ((x + z) / 31.0).sin();
            if v > 1.4 { Some((ROCK, None)) } else { None }
        })),
        about: None,
    };
    Generator { kinds: KINDS.to_vec(), seed: 2, layers: vec![spans("rock", -256, 256, ROCK), Layer::Field(tunnels)] }
}

/**
 * Houses on a grass plain: thin walls with a door carved through, rooms
 * of tables inside. On y = 0, a plane of sites at every level, where the
 * ground's top and the walls' feet would sink half a site but for how
 * sites on faces are split ("Levels" in docs/generators.md).
 */
fn village() -> Generator {
    let houses = Population {
        name: "houses",
        order: 1.0,
        cell: 64,
        reach: [0; 3],
        size: ANY_SIZE,
        // On the plain: the cells centred on y = 0, every other one of a square grid.
        anchor: Anchors::Cells(Box::new(|cell| if cell.j == 0 { Some(cell.centre.map(|v| v as f64)) } else { None })),
        make: Box::new(|[x, _, z], cell| {
            let (w, d, h) = (16.0 + (cell.random("size").next() * 12.0).floor(), 16.0 + (cell.random("size").next() * 12.0).floor(), 20.0);
            let (x0, z0, x1, z1, y) = (x - w, z - d, x + w, z + d, 0.0);
            let tables = move |clip: &Box3| -> Vec<Child> {
                (0..4)
                    .map(|n| {
                        let (tx, tz) = (x0 + 6.0 + n as f64 * 6.0, z0 + 6.0);
                        Child {
                            name: format!("table {n}"),
                            size: 2,
                            node: Node::of(vec![cuboid([tx, y, tz], [tx + 4.0, y + 3.0, tz + 3.0], WOOD, SOLID, Op::Fill)]),
                        }
                    })
                    .filter(|t| t.node.shapes.get()[0].bounds.min[0] < clip.max[0])
                    .collect()
            };
            let wall = |min: Point, max: Point| cuboid(min, max, WALL, SOLID, Op::Fill);
            Some(Node {
                bounds: Some(b(x0 as i64, y as i64, z0 as i64, x1 as i64, (y + h) as i64 + 1, z1 as i64)),
                shapes: Shapes::Now(vec![
                    wall([x0, y, z0], [x1, y + h, z0 + 1.0]),
                    wall([x0, y, z1 - 1.0], [x1, y + h, z1]),
                    wall([x0, y, z0], [x0 + 1.0, y + h, z1]),
                    wall([x1 - 1.0, y, z0], [x1, y + h, z1]),
                    wall([x0, y + h, z0], [x1, y + h + 1.0, z1]),
                    cuboid([x - 3.0, y, z0], [x + 3.0, y + 10.0, z0 + 1.0], WALL, SOLID, Op::Carve),
                ]),
                children: Some(Children::Reaching(Box::new(tables))),
                far: None,
            })
        }),
        far: None,
        on: None,
    };
    Generator { kinds: KINDS.to_vec(), seed: 3, layers: vec![spans("ground", -64, 0, GRASS), Layer::Population(houses)] }
}

fn b(min_x: i64, min_y: i64, min_z: i64, max_x: i64, max_y: i64, max_z: i64) -> Box3 {
    Box3 { min: [min_x, min_y, min_z], max: [max_x, max_y, max_z] }
}

/** A world of `generator`, which has nothing outside `heights`. */
fn world_of(generator: Generator, heights: (i64, i64)) -> World {
    World { generator, spawn: [0; 3], heights, points: Vec::new() }
}

/** The face of a wall of rock facing +x, at x = wall(y, z): a ground along x. */
fn wall(y: i64, z: i64) -> i64 {
    (3 * y + 2 * z).div_euclid(4)
}

/** The wall, and `knobs` on its face: a unit of wood at each anchor. */
fn knobbly_wall(knobs: Population) -> Generator {
    let rock = Field::spans("rock", 0.0, Op::Fill, 0, |y, z, _| {
        vec![Span { from: wall(y, z) - 64, to: wall(y, z), kind: ROCK, extinction: None }]
    });
    let knobs = knobs.reach(2).make(|p, _| Some(Node::of(vec![cuboid(p, p.map(|v| v + 1.0), WOOD, SOLID, Op::Fill)])));
    Generator { kinds: KINDS.to_vec(), seed: 4, layers: vec![Layer::Field(rock), Layer::Population(knobs)] }
}

#[test]
fn anchors_along_an_axis_are_one_per_square_across_it() {
    // On a slanting wall, so the lines of cells along x cross it in any of their cells: a spot picked evenly in each line's square
    // across x, owned by the cell it is in, is one per square 2N wide, wherever in the lattice's period.
    const N: i64 = 8;
    const LINES: i64 = 128;
    let generator = knobbly_wall(Population::new("knobs", 1.0, N).anchor_along(0, |line| Some(line.pick(&mut line.random("spot"), wall))));
    let Layer::Population(knobs) = &generator.layers[1] else { unreachable!() };
    // The cells of LINES² lines along x, as far along as the wall over them goes.
    let along = (wall(-N, -N).div_euclid(N) - 1, wall(LINES * N, LINES * N).div_euclid(N) + 1);
    let things = things_in(&generator, knobs, [along, (0, LINES - 1), (0, LINES - 1)]);
    let expected = (LINES * LINES) as f64 / 4.0;
    assert!((things.len() as f64 / expected - 1.0).abs() < 0.05, "{} knobs, not about {expected}", things.len());
    // Where they are across x within the lattice's period, 2N, in squares N / 2 wide.
    let mut squares = [0; 16];
    for (_, bounds, _) in &things {
        let [y, z] = [1, 2].map(|i| (bounds.min[i].rem_euclid(2 * N) / (N / 2)) as usize);
        squares[4 * y + z] += 1;
    }
    let each = expected / 16.0;
    assert!(squares.iter().all(|n| (*n as f64 / each - 1.0).abs() < 0.25), "{squares:?} knobs by where in 2N, not about {each}");
    for level in [0, 2] {
        whole_equals_parts(&generator, &b(0, 0, 0, 128, 128, 128), level);
    }
}

/** `world` keeps the contract up to `levels`, seen from above to level 3. */
fn keeps_the_contract(name: &str, world: &World, levels: u32) {
    check(name, world, levels, 3, true, &[]).assert_kept(&[]);
}

#[test]
fn fog_keeps_the_contract() {
    keeps_the_contract("fog", &world_of(fog(), (0, 128)), 4);
}

#[test]
fn caves_keep_the_contract() {
    keeps_the_contract("caves", &world_of(caves(), (-256, 256)), 3);
}

#[test]
fn village_keeps_the_contract() {
    keeps_the_contract("village", &world_of(village(), (-64, 64)), 4);
}

/**
 * Grass with a stripe of rock a unit wide every 8 along x, from a plan
 * of scale 1, dithered or not: the share of rock among the sites drawn at
 * `level`, and the grid.
 */
fn stripes(dithered: bool, level: u32) -> (f64, Vec<u8>) {
    let make = |x: i64, _| if x.rem_euclid(8) == 0 { ROCK } else { GRASS };
    let plan = Rc::new(if dithered { Plan::dithered("stripes", 1, make) } else { Plan::new("stripes", 1, make) });
    let ground =
        Field::columns("ground", 0.0, Op::Fill, move |x, z, _| vec![Span { from: -8, to: 0, kind: plan.at(x, z), extinction: None }]);
    let grid = generate(&world(Layer::Field(ground)), &Box3 { min: [0, -8 >> level, 0], max: [4096 >> level, 0, 64] }, level);
    let count = |kind: u8| grid.cells.iter().filter(|id| **id == kind + 1).count() as f64;
    (count(ROCK) / (count(ROCK) + count(GRASS)), grid.cells)
}

#[test]
fn fields_read_a_dithered_plan_as_often_as_it_covers() {
    // Lines of sites 4 apart read where they are: every other one is on a stripe. Read at a point of the 4 × 4
    // columns each stands for, one in eight.
    let (point, dithered) = (stripes(false, 2).0, stripes(true, 2).0);
    assert!((point - 0.5).abs() < 0.01, "read where each line is: {point}");
    assert!((dithered - 0.125).abs() < 0.01, "dithered: {dithered}");
    assert_eq!(stripes(false, 0).1, stripes(true, 0).1, "level 0 reads where each line is");
}

fn near() -> Box3 {
    b(0, 0, 0, 128, 128, 128)
}

fn world(layer: Layer) -> Generator {
    Generator { kinds: KINDS.to_vec(), seed: 1, layers: vec![layer] }
}

#[test]
#[should_panic(expected = "reaches past its population's reach")]
fn refuses_a_thing_reaching_past_its_populations_reach() {
    generate(&world(single(|_, _| Some(Node::of(vec![ball([64.0; 3], 1200.0, ROCK, SOLID, Op::Fill)])), 64)), &near(), 0);
}

#[test]
fn finds_a_far_thing_reaching_past_its_populations_reach() {
    // Only cells from 30 along x make things, too big for their reach: no box near spawn draws them.
    let far = Layer::Population(Population {
        name: "far",
        order: 1.0,
        cell: 64,
        reach: [16; 3],
        size: ANY_SIZE,
        anchor: Anchors::Cells(Box::new(|cell| (cell.i >= 30 && cell.j == 0).then_some(cell.centre.map(|v| v as f64)))),
        make: Box::new(|[x, y, z], _| Some(Node::of(vec![ball([x, y, z], 100.0, ROCK, SOLID, Op::Fill)]))),
        far: None,
        on: None,
    });
    let world = world_of(world(far), (-256, 256));
    let broken = things_hold(&world, &survey(&world, REGION, |_| true));
    assert_eq!(broken.len(), 1, "{broken:?}");
    assert!(
        broken[0].contains("far surveyed reach past their population's reach [16, 16, 16]; it needs at least [37, 37, 37]"),
        "{}",
        broken[0]
    );
}

#[test]
fn a_rare_thing_far_from_spawn_gets_a_region() {
    // A wooden post in every cell on y = 0, but one of rock 400 m away: a stratum of its own.
    let posts = Population::new("posts", 1.0, 64).anchor(|cell| (cell.j == 0).then_some(cell.centre.map(|v| v as f64))).make(|p, cell| {
        let kind = if (cell.i, cell.k) == (50, 50) { ROCK } else { WOOD };
        Some(Node::of(vec![cuboid(p, p.map(|v| v + 8.0), kind, SOLID, Op::Fill)]))
    });
    let (_, regions) = regions(&world_of(world(Layer::Population(posts)), (-64, 64)), &[]);
    let names: Vec<String> = regions.iter().flat_map(|(_, strata)| strata.clone()).collect();
    assert_eq!(names, ["posts rock", "posts wood"]);
    assert!(regions[0].0.holds([50 * 64, 0, 50 * 64]), "{regions:?}");
}

#[test]
fn finds_voxels_outside_the_heights() {
    let broken = heights_hold(&world_of(world(spans("rock", -256, 256, ROCK)), (-64, 64)), &[near()], 1);
    assert!(broken.iter().any(|b| b.contains("above the heights (-64, 64)") && b.contains("such as rock")), "{broken:?}");
    assert!(broken.iter().any(|b| b.contains("below the heights")), "{broken:?}");
}

/** Fog in a layer from y = `from` to `to`, its kind and its extinction per lattice unit given the sites' spacing. */
fn fog_layer(from: i64, to: i64, kind: fn(i64) -> u8, extinction: fn(i64) -> f64) -> Generator {
    world(Layer::Field(Field {
        name: "fog",
        order: 0.0,
        op: Op::Fill,
        values: FieldValues::At(Box::new(move |_, y, _, spacing| {
            (from..to).contains(&y).then(|| (kind(spacing), Some(extinction(spacing))))
        })),
        about: None,
    }))
}

/** The broken margins of fog of `kind` and `extinction` (as `fog_layer`) at levels 1 and 2. */
fn fog_breaks(kind: fn(i64) -> u8, extinction: fn(i64) -> f64) -> Report {
    margins("fog", &world_of(fog_layer(-64, 64, kind, extinction), (-256, 256)), &[b(0, -32, 0, 128, 32, 128)], 2, 2).broken()
}

#[test]
fn finds_fog_drawn_thicker_at_coarser_levels() {
    // A medium's extinction is per unit of a ray's path at every level; read per site, it thickens a level up.
    let fine = fog_breaks(|_| FOG, |_| 0.01);
    assert!(fine.margins.is_empty(), "{fine}");
    let thick = fog_breaks(|_| FOG, |spacing| 0.01 * spacing as f64);
    assert!(thick.margins.iter().any(|m| m.measure == Measure::Through), "{thick}");
}

#[test]
fn finds_fog_of_another_colour_at_coarser_levels() {
    // Of one type, as the materials' types would have it, but darker.
    let smoke = fog_breaks(|spacing| if spacing == 1 { FOG } else { SMOKE }, |_| 0.01);
    assert!(smoke.margins.iter().all(|m| m.measure != Measure::Through), "{smoke}");
    assert!(smoke.margins.iter().any(|m| m.measure == Measure::Colour), "{smoke}");
}

#[test]
fn finds_fog_that_glows_only_near() {
    // Of one colour, but it glows only at level 0: what rays see is brighter there.
    let glow = fog_breaks(|spacing| if spacing == 1 { GLOWING_FOG } else { FOG }, |_| 0.01);
    assert!(glow.margins.iter().any(|m| m.measure == Measure::FromAbove), "{glow}");
}

#[test]
fn finds_a_speck_too_small_to_show_outside_the_heights() {
    // A speck stops a few of a site's rays: it shows no voxel, but light and the composited rays see it.
    let speck = world(single(|_, _| Some(Node::of(vec![ball([64.0, 70.0, 64.0], 0.1, ROCK, SOLID, Op::Fill)])), 64));
    let grid = generate(&speck, &near(), 0);
    assert!(grid.cells.iter().all(|id| *id == 0) && grid.pieces.alpha.iter().any(|a| *a > 0), "the speck shows");
    let broken = heights_hold(&world_of(speck, (-64, 64)), &[near()], 0);
    assert!(
        broken.iter().any(|b| b.contains("1 sites stopping light above the heights (-64, 64)") && b.contains("such as rock")),
        "{broken:?}"
    );
}

#[test]
fn margins_break_down_by_type_and_kind() {
    let report = margins("fog", &world_of(fog(), (0, 128)), &[b(32, 32, 32, 96, 96, 96)], 2, 2);
    let all = format!("{report:#}");
    assert!(all.starts_with("fog in x 32..96, y 32..96, z 32..96:\n  level 1 against 0, rays through: "), "{all}");
    assert!(all.contains("type   8: 1.000 -> 1.000 (+0.000): fog 1.000 -> 1.000"), "{all}");
    assert!(report.broken().margins.is_empty());
}

fn parent(children: fn() -> Vec<Child>) -> Generator {
    world(single(
        move |_, _| {
            Some(Node {
                shapes: Shapes::Now(Vec::new()),
                bounds: Some(b(60, 60, 60, 70, 70, 70)),
                children: Some(Children::Fixed(children())),
                far: None,
            })
        },
        64,
    ))
}

fn child(name: &str, radius: f64) -> Child {
    Child { name: name.into(), size: ANY_SIZE, node: Node::of(vec![ball([64.0; 3], radius, ROCK, SOLID, Op::Fill)]) }
}

#[test]
#[should_panic(expected = "reaches past its parent")]
fn refuses_a_child_reaching_past_its_parent() {
    generate(&parent(|| vec![child("a", 20.0)]), &near(), 0);
}

#[test]
#[should_panic(expected = "two children named a")]
fn refuses_two_children_of_one_name() {
    generate(&parent(|| vec![child("a", 2.0), child("a", 2.0)]), &near(), 0);
}

#[test]
fn a_things_carve_cuts_only_the_thing() {
    // A box of wood with a hole carved through it, standing in rock: the hole reaches into the rock but leaves it whole.
    let generator = Generator {
        kinds: KINDS.to_vec(),
        seed: 1,
        layers: vec![
            spans("rock", 0, 64, ROCK),
            single(
                |_, _| {
                    Some(Node::of(vec![
                        cuboid([48.0, 48.0, 48.0], [80.0, 80.0, 80.0], WOOD, SOLID, Op::Paint),
                        cuboid([60.0, 40.0, 60.0], [68.0, 90.0, 68.0], WOOD, SOLID, Op::Carve),
                    ]))
                },
                64,
            ),
        ],
    };
    let grid = generate(&generator, &b(40, 40, 40, 90, 90, 90), 0);
    let types: Vec<u8> = materials_of(&generator).iter().map(|m| m.material_type).collect();
    let at = |x, y, z| types[grid.get(x, y, z) as usize];
    assert_eq!((at(64, 70, 64), at(64, 44, 64), at(50, 70, 50)), (0, 3, 4), "carved, rock under it, wood beside it");
}
