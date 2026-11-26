//! The engine on small test worlds: they keep the contract
//! (worldgen::checks::contract), and the framework refuses what breaks it.

use std::collections::BTreeSet;

use worldgen::World;
use worldgen::checks::contract::{
    Accuracy, Held, REGION, Region, Report, SHOWN, STOPPED, TOLERANCE, breaks, check, heights_hold, hold_to_baseline, in_regions, regions,
    things_hold,
};
use worldgen::checks::survey::{HELD, survey};
use worldgen::generator::{
    ANY_SIZE, Child, Children, Far, Field, FieldValues, Generator, KindDef, Layer, Node, Op, Population, SOLID, Shapes, Span, ball, cuboid,
    generate, kind_of,
};
use worldgen::generator::{Anchor, Fill, Point, lattice};
use worldgen::grid::{ALPHA, Box3};
use worldgen::noise::fx;
use worldgen::sites::spacing;

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

/** One population of one thing, anchored at (64, 64, 64), reaching `reach` from there. */
fn single(make: impl Fn(&Anchor) -> Option<Node> + Send + Sync + 'static, reach: i64) -> Layer {
    Layer::Population(
        Population::new("single", 1.0, 1024).reach(reach).size(ANY_SIZE).at(|_| vec![([64.0; 3], ())], move |_, anchor| make(anchor)),
    )
}

/** The sites of N·FCC on y = 0 over box `b`: every other one of a square grid. */
fn plain(n: i64, b: &Box3) -> Vec<([f64; 3], ())> {
    lattice(n, &Box3 { min: [b.min[0], 0, b.min[2]], max: [b.max[0], 1, b.max[2]] })
        .into_iter()
        .map(|p| (p.map(|v| v as f64), ()))
        .collect()
}

fn spans(name: &'static str, from: i64, to: i64, kind: u8) -> Layer {
    Layer::Field(Field {
        name,
        order: 0.0,
        op: Op::Paint,
        values: FieldValues::Spans { axis: 1, spans: Box::new(move |_, _| vec![Span::solid(from, to, kind)]) },
        meets: None,
    })
}

/** A ball of fog, each site kept with chance 0.3. */
fn fog() -> Generator {
    Generator {
        kinds: KINDS.to_vec(),
        seed: 1,
        layers: vec![single(|_| Some(Node::of(vec![ball([64.0; 3], 56.0, FOG, Fill::Const(0.3), Op::Fill)])), 64)],
    }
}

/** Rock with winding tunnels carved out of it. */
fn caves() -> Generator {
    let tunnels = Field {
        name: "tunnels",
        order: 1.0,
        op: Op::Carve,
        values: FieldValues::At(Box::new(|x, y, z| {
            let (x, y, z) = (x as f64, y as f64, z as f64);
            let v = (x / 23.0).sin() + (y / 17.0).sin() + (z / 29.0).sin() + ((x + z) / 31.0).sin();
            if v > 1.4 { Some((ROCK, None)) } else { None }
        })),
        meets: None,
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
    /** The seed of the houses' sizes. */
    const SIZES: u64 = 7;
    let houses = Population::new("houses", 1.0, 64).reach(64).size(ANY_SIZE).at(
        |b| plain(64, b),
        |_, cell| {
            let [x, _, z] = cell.at;
            let (w, d, h) = (16.0 + (cell.random(SIZES).next() * 12.0).floor(), 16.0 + (cell.random(SIZES).next() * 12.0).floor(), 20.0);
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
        },
    );
    Generator { kinds: KINDS.to_vec(), seed: 3, layers: vec![spans("ground", -64, 0, GRASS), Layer::Population(houses)] }
}

fn b(min_x: i64, min_y: i64, min_z: i64, max_x: i64, max_y: i64, max_z: i64) -> Box3 {
    Box3 { min: [min_x, min_y, min_z], max: [max_x, max_y, max_z] }
}

/** A world of `generator`, which has nothing outside `heights`. */
fn world_of(generator: Generator, heights: (i64, i64)) -> World {
    World { generator, spawn: [0; 3], heights: Some(heights), points: Vec::new() }
}

/**
 * `world` keeps the contract up to `levels`, but where its baseline,
 * data/contract/`name`.txt, says it breaks it, and by how much
 * (CONTRACT_BLESS=1 writes it).
 */
fn keeps_the_contract(name: &str, world: &World, levels: u32) {
    let checked = check(&[(name, world)], levels, &[]);
    hold_to_baseline(&breaks(&checked, &[]), &format!("{}/data/contract/{name}.txt", env!("CARGO_MANIFEST_DIR")), true);
}

#[test]
fn a_declared_accuracy_covers_its_parts() {
    let (loose, looser) = (Accuracy { apart: 0.1, spread: 0.2 }, Accuracy { apart: 0.2, spread: 0.3 });
    let declared = [("towns", loose), ("towns/block/house", looser)];
    assert_eq!(Accuracy::of(&declared, "towns/block/house wall"), looser);
    assert_eq!(Accuracy::of(&declared, "towns/church stone"), loose);
    assert_eq!(Accuracy::of(&declared, "townsfolk grass"), Accuracy::DEFAULT);
}

#[test]
fn an_accuracy_declared_for_nothing_breaks() {
    let checked = check(&[("fog", &world_of(fog(), (0, 128)))], 1, &[]);
    let breaks = breaks(&checked, &[("fgo", Accuracy::DEFAULT)]);
    assert!(breaks.iter().any(|b| b.of.is_none() && b.message.contains("fgo")), "a declaration for no stratum passed");
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

#[test]
fn a_stratum_measures_the_same_in_any_region_holding_it() {
    // Its cells are drawn by where they are, and none is cut off at its region's edge;
    // a cell is the first region's to sample it, so regions that meet take none twice.
    let world = world_of(village(), (-64, 64));
    let house = b(-32, -8, -32, 32, 24, 32);
    let region = |area, share| Region { area, strata: vec![Held { name: "houses".into(), places: vec![house], share }] };
    let measured = |regions: &[Region]| {
        let (report, _) = in_regions("village", &world, regions, 2);
        report.margins.iter().map(|m| (m.level, m.looks, m.spread)).collect::<Vec<_>>()
    };
    assert_eq!(measured(&[region(house, [0, 1])]), measured(&[region(b(-96, -8, -96, 96, 24, 96), [0, 1])]));
    assert_eq!(measured(&[region(house, [0, 2]), region(house, [1, 2])]), measured(&[region(house, [0, 2])]));
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
    generate(&world(single(|_| Some(Node::of(vec![ball([64.0; 3], 1200.0, ROCK, SOLID, Op::Fill)])), 64)), &near(), 0);
}

#[test]
fn finds_a_far_thing_reaching_past_its_populations_reach() {
    // Only cells from 30 along x make things, too big for their reach: no box near spawn draws them.
    let far = Layer::Population(Population::new("far", 1.0, 64).reach(16).size(ANY_SIZE).at(
        |b| plain(64, b).into_iter().filter(|(p, _)| p[0] >= 30.0 * 64.0).collect(),
        |_, anchor| Some(Node::of(vec![ball(anchor.at, 100.0, ROCK, SOLID, Op::Fill)])),
    ));
    let world = world_of(world(far), (-256, 256));
    let broken = things_hold(&world, &survey(&world, 1, REGION, &[]));
    assert_eq!(broken.len(), 1, "{broken:?}");
    assert!(
        broken[0].contains("far surveyed reach past their population's reach [16, 16, 16]; it needs at least [100, 100, 100]"),
        "{}",
        broken[0]
    );
}

#[test]
fn a_rare_thing_gets_a_region_first_and_seeds_checked_together_share_regions() {
    // A wooden post in every cell on y = 0, but one of rock 400 m away: a stratum of its own.
    let posts = Population::new("posts", 1.0, 64).at(
        |b| plain(64, b),
        |_, cell| {
            let p = cell.at;
            let kind = if cell.key == [50 * 64, 0, 50 * 64] { ROCK } else { WOOD };
            Some(Node::of(vec![cuboid(p, p.map(|v| v + 8.0), kind, SOLID, Op::Fill)]))
        },
    );
    let world = world_of(world(Layer::Population(posts)), (-64, 64));
    // As two seeds: the rock has a place at each, the wood places everywhere.
    let (_, regions) = regions(&[&world, &world], &[]);
    assert!(regions[0][0].area.holds([50 * 64, 0, 50 * 64]), "{regions:?}");
    let holding = |name: &str| regions.iter().flatten().filter(|r| r.strata.iter().any(|s| s.name == name)).count();
    assert_eq!([holding("posts rock"), holding("posts wood")], [2.min(HELD), HELD], "{regions:?}");
}

#[test]
fn finds_voxels_over_the_heights() {
    let broken = heights_hold(&world_of(world(spans("rock", -256, 256, ROCK)), (-64, 64)), 1);
    assert!(broken.iter().any(|b| b.contains("above the heights (-64, 64)") && b.contains("such as rock")), "{broken:?}");
    // Under them, the ground goes on.
    assert!(!broken.iter().any(|b| b.contains("below the heights")), "{broken:?}");
}

/** Fog in a layer from y = `from` to `to`, its kind and its extinction per lattice unit given the sites' spacing. */
fn fog_layer(from: i64, to: i64, kind: fn(i64) -> u8, extinction: fn(i64) -> f64) -> Generator {
    world(Layer::Field(Field {
        name: "fog",
        order: 0.0,
        op: Op::Fill,
        values: FieldValues::At(Box::new(move |_, y, _| {
            let spacing = spacing();
            (from..to).contains(&y).then(|| (kind(spacing), Some(extinction(spacing))))
        })),
        meets: None,
    }))
}

/** The broken margins of fog of `kind` and `extinction` (as `fog_layer`) at levels 1 and 2. */
fn fog_breaks(kind: fn(i64) -> u8, extinction: fn(i64) -> f64) -> Report {
    let fog = world_of(fog_layer(-64, 64, kind, extinction), (-256, 256));
    in_regions("fog", &fog, &[Region::whole("fog", b(0, -32, 0, 128, 32, 128))], 2).0.broken(Accuracy::DEFAULT)
}

#[test]
fn finds_fog_drawn_thicker_at_coarser_levels() {
    // A medium's extinction is per unit of a ray's path at every level; read per site, it thickens a level up.
    let fine = fog_breaks(|_| FOG, |_| 0.01);
    assert!(fine.margins.is_empty(), "{fine}");
    let thick = fog_breaks(|_| FOG, |spacing| 0.01 * spacing as f64);
    assert!(thick.margins.iter().any(|m| m.apart(STOPPED) >= TOLERANCE), "{thick}");
}

#[test]
fn finds_fog_of_another_colour_at_coarser_levels() {
    // Of one type, as the materials' types would have it, but darker.
    let smoke = fog_breaks(|spacing| if spacing == 1 { FOG } else { SMOKE }, |_| 0.01);
    assert!(smoke.margins.iter().all(|m| m.apart(STOPPED) < TOLERANCE), "{smoke}");
    assert!(smoke.margins.iter().any(|m| m.apart(SHOWN) >= TOLERANCE), "{smoke}");
}

#[test]
fn finds_fog_that_glows_only_near() {
    // Of one colour, but it glows only at level 0: what rays see is brighter there.
    let glow = fog_breaks(|spacing| if spacing == 1 { GLOWING_FOG } else { FOG }, |_| 0.01);
    assert!(!glow.margins.is_empty(), "{glow}");
}

#[test]
fn finds_a_speck_too_small_to_show_outside_the_heights() {
    // A speck stops a few of a site's rays: it shows no voxel, but light and the composited rays see it.
    let speck = world(single(|_| Some(Node::of(vec![ball([64.5, 70.5, 64.5], 0.2, ROCK, SOLID, Op::Fill)])), 64));
    let grid = generate(&speck, &near(), 0);
    assert!(grid.pieces.alpha.iter().all(|&a| a < ALPHA) && grid.pieces.alpha.iter().any(|&a| a > 0), "the speck shows");
    let broken = heights_hold(&world_of(speck, (-64, 64)), 0);
    assert!(
        broken.iter().any(|b| b.contains("1 sites stopping light above the heights (-64, 64)") && b.contains("such as rock")),
        "{broken:?}"
    );
}

fn parent(children: fn() -> Vec<Child>) -> Generator {
    world(single(
        move |_| {
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
#[should_panic(expected = "draws past its bounds")]
fn refuses_a_node_drawing_past_its_bounds() {
    let past = |_: &Anchor| {
        let shapes = Shapes::Now(vec![ball([64.0; 3], 20.0, ROCK, SOLID, Op::Fill)]);
        Some(Node { shapes, bounds: Some(b(60, 60, 60, 70, 70, 70)), children: None, far: None })
    };
    generate(&world(single(past, 64)), &near(), 0);
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
                |_| {
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
    let kind = kind_of(&generator);
    let at = |x, y, z| kind(grid.get(x, y, z)).map(|k| k.name);
    assert_eq!(
        (at(64, 70, 64), at(64, 44, 64), at(50, 70, 50)),
        (None, Some("rock"), Some("wood")),
        "carved, rock under it, wood beside it"
    );
}

/** The kinds of the sites `generator` holds anything in, in `b` at `level`, by name; a site of several, none. */
fn kinds_drawn(generator: &Generator, b: &Box3, level: u32) -> BTreeSet<Option<&'static str>> {
    let grid = generate(generator, b, level);
    let kind = kind_of(generator);
    let sites = (b.min[2]..b.max[2]).flat_map(|z| (b.min[1]..b.max[1]).flat_map(move |y| (b.min[0]..b.max[0]).map(move |x| [x, y, z])));
    sites.filter(|&[x, y, z]| grid.inside(x, y, z) != 0).map(|[x, y, z]| kind(grid.inside(x, y, z)).map(|k| k.name)).collect()
}

#[test]
fn a_component_alone_draws_only_itself() {
    // The tables in the village's houses, not their walls, nor the ground; at level 2 too, where the houses draw none.
    let near_house = |level: u32| b(-64 >> level, -8 >> level, -64 >> level, 64 >> level, 32 >> level, 64 >> level);
    let tables = village().alone("houses/table");
    assert_eq!(kinds_drawn(&tables, &near_house(0), 0), BTreeSet::from([Some("wood")]));
    assert!(kinds_drawn(&tables, &near_house(2), 2).contains(&Some("wood")), "the tables are drawn at any level");
    assert!(!kinds_drawn(&village(), &near_house(2), 2).contains(&Some("wood")), "the village draws no tables at level 2");
    assert_eq!(kinds_drawn(&village().alone("ground"), &near_house(0), 0), BTreeSet::from([Some("grass")]));
}

#[test]
fn a_component_alone_stands_for_its_parents_far_form() {
    // A ball of rock in a thing drawn far away as a box of wall: alone, past the far form's detail, it is the ball measured.
    let thing = || {
        world(single(
            |_| {
                let box_of_wall = vec![cuboid([56.0; 3], [72.0; 3], WALL, SOLID, Op::Fill)];
                let ball = Child { name: "ball".into(), size: ANY_SIZE, node: Node::of(vec![ball([64.0; 3], 6.0, ROCK, SOLID, Op::Fill)]) };
                Some(Node::group(vec![ball]).with_far(Far::of(fx(4.0), box_of_wall)))
            },
            64,
        ))
    };
    let around = b(4, 4, 4, 12, 12, 12);
    assert_eq!(kinds_drawn(&thing(), &around, 3), BTreeSet::from([Some("wall")]));
    let alone = kinds_drawn(&thing().alone("single/ball"), &around, 3);
    assert!(alone.contains(&Some("rock")) && !alone.contains(&Some("wall")), "{alone:?}");
    // Cut again, it is the same.
    let drawn = |generator: Generator| generate(&generator, &around, 3).pieces.alpha;
    assert_eq!(drawn(thing().alone("single/ball").alone("single/ball")), drawn(thing().alone("single/ball")));
}

#[test]
fn a_component_is_checked_in_regions_of_its_own() {
    // A house fits a region, so its tables are a stratum only as the name goes down to them.
    let checked = check(&[("tables", &world_of(village(), (-64, 64)))], 1, &["houses/table"]);
    let held: Vec<Vec<&str>> = checked[0].regions.iter().map(|r| r.strata.iter().map(|s| s.name.as_str()).collect()).collect();
    assert_eq!(held, vec![vec!["houses/table wood"]; HELD]);
    assert!(checked[0].report.margins.iter().all(|m| m.looks[1][0] > 0.0), "cells near the tables in each region");
}
