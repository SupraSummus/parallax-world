use super::*;
use lattice::{HOLDS_ALL, Holds, opposite};

const UNIT: f64 = 0.125;

/**
 * A lattice of level 0: the ground (`ground`, holding on across
 * `ground_holds`), and the sites of cells asleep; across which faces
 * each site holds on, as woken or as it fell asleep.
 */
struct World<'a> {
    ground: &'a dyn Fn([i64; 3]) -> bool,
    ground_holds: Holds,
    holds: FastMap<[i64; 3], Holds>,
    asleep: FastSet<[i64; 3]>,
}

impl Lattice for World<'_> {
    fn asleep(&mut self, s: [i64; 3]) -> Option<Material> {
        (self.asleep.contains(&s) || (self.ground)(s)).then_some(WOOD)
    }

    fn joined(&mut self, s: [i64; 3], f: usize) -> f64 {
        let holds = |s: [i64; 3]| self.holds.get(&s).copied().unwrap_or(if (self.ground)(s) { self.ground_holds } else { 0 });
        let t = faces(s).nth(f).expect("a face");
        f64::from(u8::from(holds(s) & 1 << f != 0 && holds(t) & 1 << opposite(f) != 0))
    }
}

/** Cells of `material` at `sites`, holding on across every face. */
fn of(material: Material, sites: impl Iterator<Item = [i64; 3]>) -> Vec<Cell<()>> {
    let mass = material.rho * UNIT * UNIT * UNIT;
    sites.map(|site| Cell { site, mass, material, fill: 1.0, holds: HOLDS_ALL, what: () }).collect()
}

/** A block of wood: sites from `lo` to `hi`, not that. */
fn block(lo: [i64; 3], hi: [i64; 3]) -> Vec<Cell<()>> {
    of(WOOD, (lo[0]..hi[0]).flat_map(move |x| (lo[1]..hi[1]).flat_map(move |y| (lo[2]..hi[2]).map(move |z| [x, y, z]))))
}

/** A post 4 sites square and 40 high, from y = `foot` up. */
fn post(foot: i64) -> Vec<Cell<()>> {
    block([0, foot, 0], [4, foot + 40, 4])
}

/** Wakes `cells` on `ground`, which holds on across `ground_holds`: the lattice and what is awake of it. */
fn wake<'a>(cells: Vec<Cell<()>>, ground: &'a dyn Fn([i64; 3]) -> bool, ground_holds: Holds) -> (World<'a>, Matter<()>) {
    let holds = cells.iter().map(|c| (c.site, c.holds)).collect();
    let mut world = World { ground, ground_holds, holds, asleep: FastSet::default() };
    let mut matter = Matter::new(0, UNIT);
    matter.wake(cells, &mut world);
    (world, matter)
}

/** Steps `matter` in `world` till all of it is asleep, or 10 s: the sites it holds then, asleep or not. */
fn run(matter: &mut Matter<()>, world: &mut World) -> Vec<[i64; 3]> {
    while matter.time < 10.0 && matter.awake() {
        for c in matter.step(world) {
            world.holds.insert(c.site, c.holds);
            world.asleep.insert(c.site);
        }
    }
    world.asleep.iter().copied().chain(matter.cells.iter().map(|c| c.site)).collect()
}

fn heights(sites: &[[i64; 3]]) -> (i64, i64) {
    (sites.iter().map(|s| s[1]).min().expect("sites"), sites.iter().map(|s| s[1]).max().expect("sites"))
}

/** The ground under y = −1, and on it a stump under half of a post's foot, its −x half. */
fn stump(s: [i64; 3]) -> bool {
    match s[1] {
        ..-1 => true,
        -1 => (0..2).contains(&s[0]) && (0..4).contains(&s[2]),
        _ => false,
    }
}

#[test]
fn a_post_on_a_stump_under_half_of_it_topples_off_it_and_lies() {
    let (mut world, mut matter) = wake(post(1), &stump, 0);
    let sites = run(&mut matter, &mut world);
    assert!(heights(&sites).1 < 12, "lying: {:?}", heights(&sites));
    assert!(sites.iter().any(|s| s[0] > 25), "off the side it is not borne on, +x");
}

/** Landed, it holds on to itself, not to the ground. */
#[test]
fn a_post_on_nothing_falls_till_it_meets_the_ground_and_stands_on_it() {
    let ground = |s: [i64; 3]| s[1] < -20;
    let (mut world, mut matter) = wake(post(1), &ground, HOLDS_ALL);
    let sites = run(&mut matter, &mut world);
    let (low, high) = heights(&sites);
    assert!(low == -20 && high >= low + 38, "on the ground, upright: {low} to {high}");
    let (down, up) = (1 << 2, 1 << 3);
    assert_eq!(world.holds[&[1, low, 1]] & (down | up), up, "to what is over it, not to the ground");
}

/**
 * A post on another, over its edge, its weight past it: holding on, the
 * two stand as one, their weight over the lower; only touching, the upper
 * topples off, over the edge.
 */
#[test]
fn a_post_over_the_edge_of_another_stands_if_they_hold_on_and_topples_off_if_not() {
    let ground = |s: [i64; 3]| s[1] < 0;
    let posts = |holding: bool| {
        let mut cells = block([0, 0, 0], [6, 10, 2]);
        let upper = block([5, 10, 0], [8, 20, 2])
            .into_iter()
            .map(|c| Cell { holds: if holding || c.site[1] > 10 { HOLDS_ALL } else { HOLDS_ALL & !(1 << 2) }, ..c });
        cells.extend(upper);
        let (mut world, mut matter) = wake(cells, &ground, 0);
        let sites = run(&mut matter, &mut world);
        (heights(&sites).1, sites.iter().map(|s| s[0]).max().expect("sites"), matter.broke)
    };
    let (holding, touching) = (posts(true), posts(false));
    assert!(holding == (19, 7, 0), "standing, as high and as far out, whole: {holding:?}");
    // How far it goes off is chaotic; nothing strikes it hard enough to break it.
    assert!(touching.0 < 19 && touching.1 > 7 && touching.2 == 0, "toppled off, whole: as high and as far out as {touching:?}");
}

/**
 * A beam spinning in the air, end over end, stays as long as it is and
 * whole: each end of a bond is fixed in its cell's own frame.
 */
#[test]
fn a_beam_spinning_in_the_air_keeps_its_length() {
    let nothing = |_: [i64; 3]| false;
    let (mut world, mut matter) = wake(block([0, 0, 0], [16, 1, 1]), &nothing, 0);
    let middle = matter.body.iter().fold([0.0; 3], |m, b| add(m, scale(b.x, 1.0 / matter.body.len() as f64)));
    // Two turns a second about z.
    let spin = [0.0, 0.0, 4.0 * std::f64::consts::PI];
    for b in &mut matter.body {
        (b.v, b.w) = (cross(spin, sub(b.x, middle)), spin);
    }
    // Its end cells, as woken: they hop from site to site as it turns.
    let length = |m: &Matter<()>| len(sub(m.middle(15), m.middle(0)));
    let before = length(&matter);
    while matter.time < 1.0 {
        matter.step(&mut world);
    }
    assert_eq!(matter.broke, 0, "whole");
    let after = length(&matter);
    assert!((after / before - 1.0).abs() < 1e-4, "as long: {before} m, then {after} m");
}

/**
 * A beam a cell thick out of a wall, of wood that holds only 1 MPa
 * pulled: beam theory puts the stress at the wall at 3 ρ g L² / h, its
 * strength reached at about 21 cells. Half that holds, sagging as much
 * as beam theory says within a half; twice that breaks at the wall.
 */
#[test]
fn a_beam_out_of_a_wall_holds_if_short_and_breaks_at_the_wall_if_long() {
    let weak = Material { tension: 1e6, ..WOOD };
    let wall = |s: [i64; 3]| s[0] < 0;
    let beam = |n: i64| wake(of(weak, (0..n).map(|x| [x, 0, 0])), &wall, HOLDS_ALL);
    let (mut world, mut short) = beam(10);
    // How low its tip is, as it last was awake.
    let mut tip = 0.0;
    while short.time < 10.0 && short.awake() {
        tip = short.middle(short.at([9, 0, 0]).expect("awake"))[1] - 0.5 * UNIT;
        short.step(&mut world);
    }
    assert_eq!(short.broke, 0, "nothing breaks");
    // w L⁴ / 8 E I, I = h⁴ / 12, at the middle of the last cell.
    let (l, w) = (9.5 * UNIT, weak.rho * -GRAVITY[1] * UNIT * UNIT);
    let theory = w * l.powi(4) / (8.0 * weak.e * UNIT.powi(4) / 12.0);
    assert!((-tip / theory - 1.0).abs() < 0.5, "sags {:.3e} m, beam theory {theory:.3e}", -tip);
    let (mut world, mut long) = beam(40);
    while long.broke == 0 && long.time < 1.0 && long.awake() {
        long.step(&mut world);
    }
    assert!(long.broke > 0, "it breaks");
    assert!(long.bonds.iter().all(|b| b.at.iter().all(|s| matches!(s, Site::Awake(_)))), "at the wall");
}

/**
 * The same beam a level up, each cell of it a quarter full and holding on
 * across a quarter of each face: what holds on bears the stress, so the
 * beam twice as long as level 0's breaking length breaks, and one half as
 * long holds, as at level 0.
 */
#[test]
fn a_beam_a_level_up_breaks_by_what_of_its_faces_holds_on() {
    let weak = Material { tension: 1e6, ..WOOD };
    struct Quarter;
    impl Lattice for Quarter {
        fn asleep(&mut self, s: [i64; 3]) -> Option<Material> {
            (s[0] < 0).then_some(Material { tension: 1e6, ..WOOD })
        }
        fn joined(&mut self, s: [i64; 3], f: usize) -> f64 {
            // A beam a level-0 site thick in each cell's corner along x: a quarter of the faces across x.
            if f / 2 == 0 && s[1] == 0 && s[2] == 0 { 0.25 } else { 0.0 }
        }
    }
    let broke = |n: i64| {
        let mass = weak.rho * UNIT.powi(3) * 2.0;
        let cells = (0..n).map(|x| Cell { site: [x, 0, 0], mass, material: weak, fill: 0.25, holds: 0, what: () }).collect();
        let mut matter = Matter::new(1, UNIT);
        matter.wake(cells, &mut Quarter);
        while matter.broke == 0 && matter.time < 1.0 && matter.awake() {
            matter.step(&mut Quarter);
        }
        matter.broke > 0
    };
    // 40 and 10 cells of level 0, 20 and 5 here.
    assert!(broke(20), "the long one breaks");
    assert!(!broke(5), "the short one holds");
}

/** A post toppling off a stump, chaotic as that is, moves alike on one thread and on several: each cell sums its springs in one order. */
#[test]
fn threads_change_nothing() {
    let topple = |threads: usize| {
        let (mut world, mut matter) = wake(post(1), &stump, 0);
        matter.threads = threads;
        while matter.time < 0.2 {
            matter.step(&mut world);
        }
        assert!(threads == 1 || matter.bonds.len() / SPRINGS_A_THREAD >= threads, "on {threads} threads");
        (0..matter.cells.len()).map(|i| matter.middle(i)).collect::<Vec<_>>()
    };
    assert_eq!(topple(1), topple(4));
}
