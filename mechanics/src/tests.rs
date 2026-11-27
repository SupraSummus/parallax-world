use super::*;
use lattice::HOLDS_ALL;

const UNIT: f64 = 0.125;

/** Cells of wood at `sites`, at rest, holding on across every face. */
fn wood(sites: impl Iterator<Item = [i64; 3]>) -> Vec<Cell<()>> {
    let mass = WOOD.rho * UNIT * UNIT * UNIT;
    sites.map(|site| Cell { site, mass, material: WOOD, holds: HOLDS_ALL, what: () }).collect()
}

/** A block of wood: sites from `lo` to `hi`, not that. */
fn block(lo: [i64; 3], hi: [i64; 3]) -> Vec<Cell<()>> {
    wood((lo[0]..hi[0]).flat_map(move |x| (lo[1]..hi[1]).flat_map(move |y| (lo[2]..hi[2]).map(move |z| [x, y, z]))))
}

/** A post 4 sites square and 40 high, from y = `foot` up. */
fn post(foot: i64) -> Vec<Cell<()>> {
    block([0, foot, 0], [4, foot + 40, 4])
}

/** Steps `matter` in `world` till it is at rest, or 10 s: the sites it holds then. */
fn run(matter: &mut Matter<()>, world: &dyn Fn([i64; 3]) -> bool) -> Vec<[i64; 3]> {
    while matter.time < 10.0 && !matter.at_rest() {
        matter.step(&mut |s| world(s));
    }
    matter.cells.iter().map(|c| c.site).collect()
}

fn heights(sites: &[[i64; 3]]) -> (i64, i64) {
    (sites.iter().map(|s| s[1]).min().expect("sites"), sites.iter().map(|s| s[1]).max().expect("sites"))
}

#[test]
fn a_post_on_a_stump_under_half_of_it_topples_off_it_and_lies() {
    let world = |s: [i64; 3]| match s[1] {
        ..-1 => true,
        -1 => (0..2).contains(&s[0]) && (0..4).contains(&s[2]),
        _ => false,
    };
    let sites = run(&mut Matter::new(0, UNIT, post(1), &mut |_| None), &world);
    assert!(heights(&sites).1 < 12, "lying: {:?}", heights(&sites));
    assert!(sites.iter().any(|s| s[0] > 25), "off the side it is not borne on, +x");
}

/** Landed, it holds on to itself, not to the ground. */
#[test]
fn a_post_on_nothing_falls_till_it_meets_the_ground_and_stands_on_it() {
    let world = |s: [i64; 3]| s[1] < -20;
    let mut matter = Matter::new(0, UNIT, post(1), &mut |_| None);
    let sites = run(&mut matter, &world);
    let (low, high) = heights(&sites);
    assert!(low == -20 && high >= low + 38, "on the ground, upright: {low} to {high}");
    let foot = matter.cells.iter().find(|c| c.site == [1, low, 1]).expect("there").holds;
    let (down, up) = (1 << 2, 1 << 3);
    assert_eq!(foot & (down | up), up, "to what is over it, not to the ground");
}

/**
 * A post on another, over its edge, its weight past it: holding on, the
 * two stand as one, their weight over the lower; only touching, the upper
 * topples off, over the edge.
 */
#[test]
fn a_post_over_the_edge_of_another_stands_if_they_hold_on_and_topples_off_if_not() {
    let world = |s: [i64; 3]| s[1] < 0;
    let posts = |holding: bool| {
        let mut cells = block([0, 0, 0], [6, 10, 2]);
        let upper = block([5, 10, 0], [8, 20, 2])
            .into_iter()
            .map(|c| Cell { holds: if holding || c.site[1] > 10 { HOLDS_ALL } else { HOLDS_ALL & !(1 << 2) }, ..c });
        cells.extend(upper);
        let mut matter = Matter::new(0, UNIT, cells, &mut |_| None);
        let sites = run(&mut matter, &world);
        (heights(&sites).1, sites.iter().map(|s| s[0]).max().expect("sites"))
    };
    let (holding, touching) = (posts(true), posts(false));
    assert!(holding == (19, 7), "standing, as high and as far out: {holding:?}");
    assert!(touching.0 < 19 && touching.1 > 10, "toppled off: as high and as far out as {touching:?}");
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
    let beam = |n: i64| -> Matter<()> {
        let cells =
            (0..n).map(|x| Cell { site: [x, 0, 0], mass: weak.rho * UNIT.powi(3), material: weak, holds: HOLDS_ALL, what: () }).collect();
        Matter::new(0, UNIT, cells, &mut |s| (s[0] < 0).then_some((weak, HOLDS_ALL)))
    };
    let mut short = beam(10);
    run(&mut short, &|s| s[0] < 0);
    assert_eq!(short.welds(), beam(10).welds(), "nothing breaks");
    let tip = short.middle(9)[1] - 0.5 * UNIT;
    // w L⁴ / 8 E I, I = h⁴ / 12, at the middle of the last cell.
    let (l, w) = (9.5 * UNIT, weak.rho * GRAVITY * UNIT * UNIT);
    let theory = w * l.powi(4) / (8.0 * weak.e * UNIT.powi(4) / 12.0);
    assert!((-tip / theory - 1.0).abs() < 0.5, "sags {:.3e} m, beam theory {theory:.3e}", -tip);
    let mut long = beam(40);
    let whole = long.welds();
    while long.welds() == whole && long.time < 1.0 {
        long.step(&mut |s| s[0] < 0);
    }
    assert!(long.welds() < whole, "it breaks");
    assert!(long.welds.iter().all(|w| matches!(w.j, Other::Cell(_))), "at the wall");
}
