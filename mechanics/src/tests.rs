use super::*;

const UNIT: f64 = 0.125;

/** Cells of wood at `sites`, at rest. */
fn wood(sites: impl Iterator<Item = [i64; 3]>) -> Vec<Cell<()>> {
    let mass = WOOD.rho * UNIT * UNIT * UNIT;
    sites.map(|site| Cell { site, mass, material: WOOD, what: () }).collect()
}

/** A post 4 sites square and 40 high, from y = `foot` up. */
fn post(foot: i64) -> Vec<Cell<()>> {
    wood((0..4).flat_map(|x| (foot..foot + 40).flat_map(move |y| (0..4).map(move |z| [x, y, z]))))
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

#[test]
fn a_post_on_nothing_falls_till_it_meets_the_ground_and_stands_on_it() {
    let world = |s: [i64; 3]| s[1] < -20;
    let mut matter = Matter::new(0, UNIT, post(1), &mut |_| None);
    let (low, high) = heights(&run(&mut matter, &world));
    assert!(low == -20 && high >= low + 38, "on the ground, upright: {low} to {high}");
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
        let cells = (0..n).map(|x| Cell { site: [x, 0, 0], mass: weak.rho * UNIT.powi(3), material: weak, what: () }).collect();
        Matter::new(0, UNIT, cells, &mut |s| (s[0] < 0).then_some(weak))
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
