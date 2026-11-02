//! The generator contract (docs/generators.md, "Checks"), for every world:
//! a box generated in parts, alone and in any order, equals it generated
//! whole; and each level shows what the level two finer shows.

use worldgen::forest::forest;
use worldgen::generator::{Cell, Fill, Point};
use worldgen::generator::{
    Child, Children, Field, FieldValues, Generator, KindDef, Layer, Node, Op, Population, SOLID, Shapes, Span, ball, cuboid, generate,
    materials_of,
};
use worldgen::grid::{Box3, VoxelGrid};
use worldgen::noise::Rng;

static KINDS: &[KindDef] = &[
    KindDef { kind_type: 8, colors: &["#cccccc"] },
    KindDef { kind_type: 3, colors: &["#777777"] },
    KindDef { kind_type: 10, colors: &["#eeeeee"] },
    KindDef { kind_type: 4, colors: &["#664422"] },
    KindDef { kind_type: 1, colors: &["#44aa44"] },
];
const FOG: u8 = 0;
const ROCK: u8 = 1;
const WALL: u8 = 2;
const WOOD: u8 = 3;
const GRASS: u8 = 4;

/** One population of one thing, near the origin: cells reach 1024 + `reach` from their centres. */
fn single(make: impl Fn(Point, &Cell) -> Option<Node> + 'static, reach: f64) -> Layer {
    Layer::Population(Population {
        name: "single",
        order: 1.0,
        cell: 1024.0,
        reach: [reach; 3],
        size: f64::INFINITY,
        anchor: Box::new(|cell| if cell.i == 0.0 && cell.j == 0.0 && cell.k == 0.0 { Some([64.0, 64.0, 64.0]) } else { None }),
        make: Box::new(make),
        far: None,
    })
}

fn spans(name: &'static str, from: f64, to: f64, kind: u8) -> Layer {
    Layer::Field(Field {
        name,
        order: 0.0,
        op: Op::Paint,
        values: FieldValues::Spans { axis: 1, spans: Box::new(move |_, _, _| vec![Span { from, to, kind, extinction: None }]) },
    })
}

/** A ball of fog, each site kept with chance 0.3. */
fn fog() -> Generator {
    Generator {
        kinds: KINDS,
        seed: 1.0,
        layers: vec![single(|_, _| Some(Node::of(vec![ball([64.0; 3], 56.0, FOG, Fill::Const(0.3), Op::Fill)])), 64.0)],
    }
}

/** Rock with winding tunnels carved out of it. */
fn caves() -> Generator {
    let tunnels = Field {
        name: "tunnels",
        order: 1.0,
        op: Op::Carve,
        values: FieldValues::At(Box::new(|x, y, z, _| {
            let v = (x / 23.0).sin() + (y / 17.0).sin() + (z / 29.0).sin() + ((x + z) / 31.0).sin();
            if v > 1.4 { Some((ROCK, None)) } else { None }
        })),
    };
    Generator { kinds: KINDS, seed: 2.0, layers: vec![spans("rock", -1000.0, 1000.0, ROCK), Layer::Field(tunnels)] }
}

/** Houses on a grass plain: thin walls with a door carved through, rooms of tables inside. */
fn village() -> Generator {
    let houses = Population {
        name: "houses",
        order: 1.0,
        cell: 64.0,
        reach: [0.0; 3],
        size: f64::INFINITY,
        // On the plane y = 0: the cells centred on it, every other one of a square grid.
        anchor: Box::new(|cell| if cell.j == 0.0 { Some(cell.centre) } else { None }),
        make: Box::new(|[x, _, z], cell| {
            let (w, d, h) = (16.0 + (cell.random("size").next() * 12.0).floor(), 16.0 + (cell.random("size").next() * 12.0).floor(), 20.0);
            let (x0, z0, x1, z1) = (x - w, z - d, x + w, z + d);
            let tables = move |clip: &Box3| -> Vec<Child> {
                (0..4)
                    .map(|n| {
                        let (tx, tz) = (x0 + 6.0 + n as f64 * 6.0, z0 + 6.0);
                        Child {
                            name: format!("table {n}"),
                            size: 2.0,
                            node: Node::of(vec![cuboid([tx, 0.0, tz], [tx + 4.0, 3.0, tz + 3.0], WOOD, SOLID, Op::Fill)]),
                        }
                    })
                    .filter(|t| t.node.shapes.get()[0].bounds.min_x < clip.max_x)
                    .collect()
            };
            let wall = |min: Point, max: Point| cuboid(min, max, WALL, SOLID, Op::Fill);
            Some(Node {
                bounds: Some(Box3 { min_x: x0, min_y: 0.0, min_z: z0, max_x: x1, max_y: h + 1.0, max_z: z1 }),
                shapes: Shapes::Now(vec![
                    wall([x0, 0.0, z0], [x1, h, z0 + 1.0]),
                    wall([x0, 0.0, z1 - 1.0], [x1, h, z1]),
                    wall([x0, 0.0, z0], [x0 + 1.0, h, z1]),
                    wall([x1 - 1.0, 0.0, z0], [x1, h, z1]),
                    wall([x0, h, z0], [x1, h + 1.0, z1]),
                    cuboid([x - 3.0, 0.0, z0], [x + 3.0, 10.0, z0 + 1.0], WALL, SOLID, Op::Carve),
                ]),
                children: Some(Children::Reaching(Box::new(tables))),
                far: None,
            })
        }),
        far: None,
    };
    Generator { kinds: KINDS, seed: 3.0, layers: vec![spans("ground", -64.0, 0.0, GRASS), Layer::Population(houses)] }
}

fn b(min_x: f64, min_y: f64, min_z: f64, max_x: f64, max_y: f64, max_z: f64) -> Box3 {
    Box3 { min_x, min_y, min_z, max_x, max_y, max_z }
}

/** The box of `level`'s sites covering `region` and a margin of two sites. */
fn box_at(r: &Box3, level: u32) -> Box3 {
    let s = (1u64 << level) as f64;
    b(
        (r.min_x / s).floor() - 2.0,
        (r.min_y / s).floor() - 2.0,
        (r.min_z / s).floor() - 2.0,
        (r.max_x / s).ceil() + 2.0,
        (r.max_y / s).ceil() + 2.0,
        (r.max_z / s).ceil() + 2.0,
    )
}

fn sites(b: &Box3) -> impl Iterator<Item = (i64, i64, i64)> + '_ {
    (b.min_z as i64..b.max_z as i64).flat_map(move |z| {
        (b.min_y as i64..b.max_y as i64).flat_map(move |y| {
            let from = b.min_x as i64 + ((b.min_x as i64 + y + z) & 1);
            (from..b.max_x as i64).step_by(2).map(move |x| (x, y, z))
        })
    })
}

fn nearest_site(p: [f64; 3]) -> [i64; 3] {
    let mut r = p.map(|v| (v + 0.5).floor());
    if (r[0] as i64 + r[1] as i64 + r[2] as i64) & 1 != 0 {
        // Off by one in the coordinate rounded the furthest.
        let errors: Vec<f64> = (0..3).map(|i| (p[i] - r[i]).abs()).collect();
        let k = (0..3).fold(0, |m, i| if errors[i] > errors[m] { i } else { m });
        r[k] += if p[k] > r[k] { 1.0 } else { -1.0 };
    }
    r.map(|v| v as i64)
}

struct Ray {
    origin: [f64; 3],
    direction: [f64; 3],
}

/** How far each ray goes before entering a voxel of `grid` (at `level`), and its material type; or till it leaves `region`, type 0. */
fn first_hits(grid: &VoxelGrid, types: &[u8], level: u32, rays: &[Ray], region: &Box3) -> Vec<(f64, u8)> {
    let s = (1u64 << level) as f64;
    let inside = |p: [f64; 3]| {
        p[0] >= region.min_x
            && p[0] < region.max_x
            && p[1] >= region.min_y
            && p[1] < region.max_y
            && p[2] >= region.min_z
            && p[2] < region.max_z
    };
    rays.iter()
        .map(|Ray { origin, direction }| {
            let mut t = 0.0;
            loop {
                let p = [0, 1, 2].map(|i| origin[i] + t * direction[i]);
                if !inside(p) {
                    return (t, 0);
                }
                let [x, y, z] = nearest_site(p.map(|v| v / s));
                let id = grid.get(x, y, z);
                if id != 0 {
                    return (t, types[id as usize]);
                }
                t += 0.1 * s.min(4.0);
            }
        })
        .collect()
}

/** Rays from random points of `region`, in random directions, seeded so failures reproduce. */
fn random_rays(r: &Box3, count: usize) -> Vec<Ray> {
    let mut random = Rng::new(11.0);
    (0..count)
        .map(|_| {
            let origin = [
                r.min_x + random.next() * (r.max_x - r.min_x),
                r.min_y + random.next() * (r.max_y - r.min_y),
                r.min_z + random.next() * (r.max_z - r.min_z),
            ];
            let z = 2.0 * random.next() - 1.0;
            let a = 2.0 * std::f64::consts::PI * random.next();
            Ray { origin, direction: [(1.0 - z * z).sqrt() * a.cos(), (1.0 - z * z).sqrt() * a.sin(), z] }
        })
        .collect()
}

/**
 * How far a level may stray from the one two finer, in the share of rays
 * through and of materials hit (see generator.test.ts).
 */
const TOLERANCE: f64 = 0.1;

fn whole_equals_parts(generator: &Generator, region: &Box3, level: u32) {
    let whole = box_at(region, level);
    let grid = generate(generator, &whole, level);
    assert!(grid.cells.iter().any(|id| *id != 0), "level {level} is empty");
    // Eight parts, split at random points (seeded, so failures reproduce).
    let mut random = Rng::new(7.0);
    let mut split = |lo: f64, hi: f64, even: bool| {
        let at = lo + (hi - lo) * (0.2 + 0.6 * random.next());
        if even { 2.0 * (at / 2.0 + 0.5).floor() } else { (at + 0.5).floor() }
    };
    let (x, y, z) = (split(whole.min_x, whole.max_x, true), split(whole.min_y, whole.max_y, false), split(whole.min_z, whole.max_z, false));
    let mut parts = Vec::new();
    for (min_x, max_x) in [(whole.min_x, x), (x, whole.max_x)] {
        for (min_y, max_y) in [(whole.min_y, y), (y, whole.max_y)] {
            for (min_z, max_z) in [(whole.min_z, z), (z, whole.max_z)] {
                parts.push(b(min_x, min_y, min_z, max_x, max_y, max_z));
            }
        }
    }
    for part in parts.iter().rev() {
        let alone = generate(generator, part, level);
        for (x, y, z) in sites(part) {
            assert_eq!(alone.get(x, y, z), grid.get(x, y, z), "level {level}: part {part:?} differs at ({x}, {y}, {z})");
        }
    }
}

fn level_shows_what_finer_shows(generator: &Generator, region: &Box3, level: u32) {
    let (s, finer) = ((1u64 << level) as f64, level.saturating_sub(2));
    let types: Vec<u8> = materials_of(generator).iter().map(|m| m.material_type).collect();
    // At least 16 sites of the level across, around the region's centre.
    let grow = |lo: f64, hi: f64| 0f64.max(8.0 * s - (hi - lo) / 2.0);
    let r = region;
    let (gx, gy, gz) = (grow(r.min_x, r.max_x), grow(r.min_y, r.max_y), grow(r.min_z, r.max_z));
    let area = b(r.min_x - gx, r.min_y - gy, r.min_z - gz, r.max_x + gx, r.max_y + gy, r.max_z + gz);
    let reference = generate(generator, &box_at(&area, finer), finer);
    // From where a camera could be: empty sites of the finer level.
    let fs = (1u64 << finer) as f64;
    let rays: Vec<Ray> = random_rays(&area, 4000)
        .into_iter()
        .filter(|ray| {
            let [x, y, z] = nearest_site(ray.origin.map(|v| v / fs));
            reference.get(x, y, z) == 0
        })
        .take(2000)
        .collect();
    let fine = first_hits(&reference, &types, finer, &rays, &area);
    let coarse = first_hits(&generate(generator, &box_at(&area, level), level), &types, level, &rays, &area);
    // A level is seen from afar: of the rays that got 2 of its sites far, how many get 4 more.
    let (from, d) = (2.0 * s, 4.0 * s);
    let through = |hits: &[(f64, u8)]| {
        let seen: Vec<_> = hits.iter().filter(|(t, ty)| *t > from && !(*ty == 0 && *t <= from + d)).collect();
        seen.iter().filter(|(t, _)| *t > from + d).count() as f64 / seen.len() as f64
    };
    let shares = |hits: &[(f64, u8)]| {
        let far: Vec<_> = hits.iter().filter(|(t, _)| *t > from).collect();
        let mut counts = std::collections::BTreeMap::<u8, f64>::new();
        for (_, ty) in &far {
            *counts.entry(*ty).or_insert(0.0) += 1.0 / far.len() as f64;
        }
        counts
    };
    let (a, c) = (shares(&fine), shares(&coarse));
    let keys: std::collections::BTreeSet<u8> = a.keys().chain(c.keys()).copied().collect();
    let tv = keys.iter().map(|t| (a.get(t).unwrap_or(&0.0) - c.get(t).unwrap_or(&0.0)).abs()).sum::<f64>() / 2.0;
    let gap = (through(&coarse) - through(&fine)).abs();
    assert!(gap < TOLERANCE, "level {level}: {gap} more or fewer rays get through than at level {finer}");
    assert!(tv < TOLERANCE, "level {level}: the materials hit differ by {tv} from level {finer}");
}

/** Every generator with a region with a bit of everything in it, and the level up to which it keeps what a camera sees. */
fn check(generator: Generator, region: Box3, levels: u32) {
    // Up to one level past the checked ones: the forest's trees as a medium.
    for level in [0, 2, levels + 1] {
        whole_equals_parts(&generator, &region, level);
    }
    for level in 1..=levels {
        level_shows_what_finer_shows(&generator, &region, level);
    }
}

#[test]
fn forest_keeps_the_contract() {
    // Near the spawn point: path, trees, undergrowth, from under the ground to above the crowns.
    check(forest(42.0).0, b(-128.0, -64.0, -384.0, 128.0, 384.0, -128.0), 6);
}

#[test]
fn fog_keeps_the_contract() {
    check(fog(), b(32.0, 32.0, 32.0, 96.0, 96.0, 96.0), 4);
}

#[test]
fn caves_keep_the_contract() {
    check(caves(), b(0.0, 0.0, 0.0, 128.0, 128.0, 128.0), 3);
}

#[test]
fn village_keeps_the_contract() {
    check(village(), b(-64.0, -16.0, -64.0, 64.0, 48.0, 64.0), 5);
}

fn near() -> Box3 {
    b(0.0, 0.0, 0.0, 128.0, 128.0, 128.0)
}

fn world(layer: Layer) -> Generator {
    Generator { kinds: KINDS, seed: 1.0, layers: vec![layer] }
}

#[test]
#[should_panic(expected = "reaches past its population's reach")]
fn refuses_a_thing_reaching_past_its_populations_reach() {
    generate(&world(single(|_, _| Some(Node::of(vec![ball([64.0; 3], 1200.0, ROCK, SOLID, Op::Fill)])), 64.0)), &near(), 0);
}

fn parent(children: fn() -> Vec<Child>) -> Generator {
    world(single(
        move |_, _| {
            Some(Node {
                shapes: Shapes::Now(Vec::new()),
                bounds: Some(b(60.0, 60.0, 60.0, 70.0, 70.0, 70.0)),
                children: Some(Children::Fixed(children())),
                far: None,
            })
        },
        64.0,
    ))
}

fn child(name: &str, radius: f64) -> Child {
    Child { name: name.into(), size: f64::INFINITY, node: Node::of(vec![ball([64.0; 3], radius, ROCK, SOLID, Op::Fill)]) }
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
