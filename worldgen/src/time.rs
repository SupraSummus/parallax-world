//! Time, a prototype: a world that moves is a world at each time, as it is
//! one at each place and level. A layer reads where what moves has got to
//! (`drift`): a site of the lattice it draws, so what drifts goes from
//! site to site, the world then the same world a site on. A level's sites
//! change as often as what moves crosses one, a coarser level's half as often.
//! What is made says while when it stays as made (`at`): while each of
//! its reads would answer the same, always if it read none. So a host's
//! tile still never changes; it is a tile of a time, and one that holds
//! nothing moving is made once.
//!
//! Only layers may read it, as they draw: plans and things are kept from
//! box to box, and would keep what they read.

use std::cell::Cell;
use std::ops::Range;

use fcc::ray::FACES;

use crate::sites;

/** While what read no time stays as made. */
pub const ALWAYS: Range<i64> = i64::MIN..i64::MAX;

thread_local! {
    /** The time what is made on this thread is made at, and while when it stays as made, milliseconds. */
    static NOW: Cell<(i64, i64, i64)> = const { Cell::new((0, ALWAYS.start, ALWAYS.end)) };
}

/** Runs `make` at time `now` (milliseconds): what it makes, and while when that stays as made, `now` within (ALWAYS: it does not move). */
pub fn at<R>(now: i64, make: impl FnOnce() -> R) -> (R, Range<i64>) {
    let outer = NOW.replace((now, ALWAYS.start, ALWAYS.end));
    let made = make();
    let (_, from, until) = NOW.replace(outer);
    (made, from..until)
}

/** The time what is made on this thread is made at (`at`); 0 outside it. */
pub fn now() -> i64 {
    NOW.get().0
}

/** The site of the lattice `s` apart whose voxel holds `p`, in units of `s`. */
fn site(p: [i64; 3], s: i64) -> [i64; 3] {
    let mut r = p.map(|v| (v + s / 2).div_euclid(s));
    if (r[0] + r[1] + r[2]) & 1 != 0 {
        // The coordinate furthest from its whole number goes the other way.
        let off = |i: usize| p[i] - r[i] * s;
        let i = (0..3).max_by_key(|&i| (off(i).abs(), 2 - i)).expect("an axis");
        r[i] += if off(i) > 0 { 1 } else { -1 };
    }
    r
}

/**
 * Where what moves at `velocity` (lattice units a second) has got to from
 * where it was at time 0, lattice units: the site whose voxel it is in, of
 * the lattice read for (`sites::spacing`). What is made stays as made
 * while it is in that voxel.
 */
pub fn drift(velocity: [i64; 3]) -> [i64; 3] {
    let spacing = sites::read_for().map(|(spacing, _)| spacing).expect("only layers read the time: plans and things are kept");
    let (now, from, until) = NOW.get();
    // In thousandths of a unit: as far as it goes in a millisecond at a unit a second.
    let (s, at) = (1000 * spacing, |t: i64| velocity.map(|v| v * t));
    let here = site(at(now), s);
    // It is in the voxel while within each face: face · (at(t) − here · s) ≤ s.
    let dot = |a: [i64; 3], b: [i64; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    let (mut enters, mut leaves) = (ALWAYS.start, ALWAYS.end - 1);
    for face in FACES {
        let (pace, room) = (dot(face, velocity), s + s * dot(face, here));
        if pace > 0 {
            leaves = leaves.min(room.div_euclid(pace));
        } else if pace < 0 {
            enters = enters.max(-room.div_euclid(-pace));
        }
    }
    // On a face it is in one voxel or the other, as `site` says.
    let in_it = |t: i64| site(at(t), s) == here;
    let (enters, left) = (enters + i64::from(!in_it(enters)), leaves + i64::from(in_it(leaves)));
    NOW.set((now, from.max(enters), until.min(left)));
    here.map(|v| v * spacing)
}

#[cfg(test)]
mod tests {
    use super::*;

    /** `drift` of `velocity` at `now`, read for sites `spacing` apart. */
    fn read(spacing: i64, velocity: [i64; 3], now: i64) -> ([i64; 3], Range<i64>) {
        at(now, || sites::footprint(spacing, [0, 0, 0], || drift(velocity)))
    }

    #[test]
    fn what_drifts_goes_from_site_to_site_and_says_while_when_it_stays() {
        for spacing in [1, 2, 16] {
            for velocity in [[3, 0, 0], [-40, 0, 17], [7, -5, 11], [48, 0, 20]] {
                let mut now = -5000;
                while now < 5000 {
                    let (gone, stays) = read(spacing, velocity, now);
                    let units = gone.map(|v| v / spacing);
                    assert!(gone == units.map(|v| v * spacing) && (units[0] + units[1] + units[2]) % 2 == 0, "a site of the lattice");
                    // In that site's voxel.
                    let off: Vec<i64> = (0..3).map(|i| (velocity[i] * now - gone[i] * 1000).abs()).collect();
                    assert!((0..3).all(|i| off[i] + off[(i + 1) % 3] <= 1000 * spacing), "{off:?}");
                    // There while it says, and not before or after.
                    assert!(stays.contains(&now));
                    for (t, there) in [(stays.start, true), (stays.end - 1, true), (stays.start - 1, false), (stays.end, false)] {
                        let (then, while_then) = read(spacing, velocity, t);
                        assert_eq!(then == gone, there, "at {t}, of {stays:?}");
                        assert!(!there || while_then == stays);
                    }
                    now = stays.end;
                }
            }
        }
    }

    #[test]
    fn a_wind_along_no_axis_keeps_to_its_plane() {
        // Level with the ground it stays level, stepping to the lattice's neighbours across it.
        assert!((0..20_000).step_by(7).all(|now| read(4, [48, 0, 20], now).0[1] == 0));
    }

    #[test]
    fn what_reads_no_time_stays_always() {
        assert_eq!(at(1234, || 5), (5, ALWAYS));
        assert_eq!(read(4, [0; 3], 1234), ([0; 3], ALWAYS));
    }
}
