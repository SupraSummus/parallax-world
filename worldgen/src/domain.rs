//! Composing a world of domains: parts that never call each other, each
//! named, each a block of what it reads and what it proposes. A domain
//! reads only what is coarser than itself, handed to it when it is made;
//! it bids for land, pulls the ground towards heights of its own, and
//! draws its layers by its hold on the land. A world is its domains, the
//! bands of ground they shape, and the land they share.
//! See "Composing" in docs/generators.md.

use std::rc::Rc;

use crate::generator::Layer;
use crate::noise::{Fx, Text, smoothstep, stream};
use crate::plan::{Bid, Holding, Kept, Land, Plan, Sloped};
use crate::units::Units;

const DOCS: &str = "docs/generators.md";

/**
 * A part of a world by name, and the seeds of what it draws, each from
 * the name and the world's seed: two parts never share one by chance,
 * and a part draws the same in any world that names it alike (the forest
 * alone, and in the valley).
 */
#[derive(Clone)]
pub struct Scope {
    seed: u64,
    /** Its name, as a path of its parts' names joined by '/', hashed: none for the world. */
    name: Option<Text>,
}

impl Scope {
    /** The world of `seed`, which names its parts. */
    pub fn new(seed: u64) -> Scope {
        Scope { seed, name: None }
    }

    /** Its part named `name`. */
    pub fn of(&self, name: &str) -> Scope {
        Scope { seed: self.seed, name: Some(self.name.map_or_else(Text::default, |path| path.then("/")).then(name)) }
    }

    /**
     * The seed of its `what`: a noise, a plan's claims, a stream of random
     * numbers. Cheap: worlds ask in their inner loops.
     */
    pub fn seed(&self, what: &str) -> u64 {
        stream(self.seed, self.name.unwrap_or_default().then(":").then(what).hash(0))
    }
}

/**
 * A part of a world: what it proposes for each column, and what it
 * draws. Each but `layers` has a default, for domains that don't propose it.
 */
pub trait Domain {
    /**
     * Whether it holds land: bids for it, and draws by its share. Relief,
     * which every domain stands on, holds none, and draws everywhere.
     */
    fn holds_land(&self) -> bool {
        true
    }

    /** Its bid for column (x, z), a fraction (see Land): by default none, so it has what no other outbids it for. */
    fn bid(&self, _x: i64, _z: i64) -> Fx {
        Fx::ZERO
    }

    /**
     * How it shapes ground `ground` high at column (x, z), the ground of
     * the bands before its own: a height it pulls towards, and how hard,
     * a fraction. By default it leaves the ground as it is.
     */
    fn shape(&self, _x: i64, _z: i64, _ground: Fx) -> Option<(Fx, Fx)> {
        None
    }

    /** Its layers on `ground`, drawn by `hold`, its hold on the land. */
    fn layers(self: Rc<Self>, ground: &Rc<Sloped>, hold: Holding) -> Vec<Layer>;
}

/**
 * A pull towards `to` (Domain::shape) from `d` away: all of it at none
 * and inside, easing to none `over` past; none further.
 */
pub fn eased(to: Fx, over: Units, d: Units) -> Option<(Fx, Fx)> {
    (d < over).then(|| (to, Fx::ONE - smoothstep(Fx::ZERO, over.to_fx(), d.to_fx())))
}

/**
 * The lie of the land: the ground's height at each column, with its
 * fraction, made band by band. A band is the one before shaped by its
 * domains (`shaped`), which read only bands before it: those they were
 * made with. A function of position, not a plan, so plans of any scale
 * may read it; kept by column, as several read each column. The ground
 * layers stand on is its last band, a plan (`ground`).
 */
#[derive(Clone)]
pub struct Lie(Rc<Band>);

struct Band {
    height: Box<dyn Fn(i64, i64) -> Fx>,
    kept: Kept<Fx>,
}

impl Lie {
    pub fn new(height: impl Fn(i64, i64) -> Fx + 'static) -> Lie {
        Lie(Rc::new(Band { height: Box::new(height), kept: Kept::new() }))
    }

    /** The height at column (x, z), with its fraction. */
    pub fn at(&self, x: i64, z: i64) -> Fx {
        self.0.kept.get(x, z).unwrap_or_else(|| {
            let h = (self.0.height)(x, z);
            self.0.kept.keep(x, z, h);
            h
        })
    }

    /**
     * The next band: this one pulled by `domains`, each towards its height
     * as hard as it says, the same in whatever order they come. A rule:
     * their pulls on a column add up to all of it at most, so none undoes
     * another's; broken, it panics.
     */
    pub fn shaped(&self, domains: Vec<Rc<dyn Domain>>) -> Lie {
        let under = self.clone();
        Lie::new(move |x, z| {
            let ground = under.at(x, z);
            let (mut pulled, mut how_hard) = (ground, Fx::ZERO);
            for (to, w) in domains.iter().filter_map(|d| d.shape(x, z, ground)) {
                pulled += (to - ground) * w;
                how_hard += w;
            }
            assert!(
                (Fx::ZERO..=Fx::ONE).contains(&how_hard),
                "domains pull column ({x}, {z}) {} hard, not up to all of it; see \"Ground\" in {DOCS}",
                how_hard.to_f64()
            );
            pulled
        })
    }

    /** The ground layers stand on: this band rounded to units, kept by plan at `scale`, rising at most `slope` a column. */
    pub fn ground(&self, scale: i64, slope: i64) -> Rc<Sloped> {
        let lie = self.clone();
        Rc::new(Sloped::new(Rc::new(Plan::new("ground", scale, move |x, z| lie.at(x, z).round_units().0)), slope))
    }
}

/**
 * The layers of `domains` on `ground`: those holding land bid for it,
 * decided at `scale`, their shares easing over `width` (see Land), and
 * each draws by its hold; the rest draw everywhere, as does one holding
 * land alone. In the domains' order, which is that of their layers, and
 * of what wins a tie.
 */
pub fn compose(domains: &[Rc<dyn Domain>], ground: &Rc<Sloped>, scale: i64, width: Fx) -> Vec<Layer> {
    let holding: Vec<Rc<dyn Domain>> = domains.iter().filter(|d| d.holds_land()).cloned().collect();
    let bid = |d: &Rc<dyn Domain>| -> Bid {
        let d = d.clone();
        Box::new(move |x, z| d.bid(x, z))
    };
    let land = (holding.len() > 1).then(|| Rc::new(Land::new(scale, width, holding.iter().map(bid).collect())));
    let mut holds = (0..).map(|d| land.as_ref().map_or_else(Holding::all, |land| land.holding(d)));
    let mut hold = |d: &Rc<dyn Domain>| if d.holds_land() { holds.next().expect("endless") } else { Holding::all() };
    domains.iter().flat_map(|d| d.clone().layers(ground, hold(d))).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::noise::fx;

    /** Pulls towards `to` with `how_hard` where x is at least `from`. */
    struct Pull {
        from: i64,
        to: Fx,
        how_hard: Fx,
    }

    impl Domain for Pull {
        fn shape(&self, x: i64, _: i64, _: Fx) -> Option<(Fx, Fx)> {
            (x >= self.from).then_some((self.to, self.how_hard))
        }

        fn layers(self: Rc<Self>, _: &Rc<Sloped>, _: Holding) -> Vec<Layer> {
            Vec::new()
        }
    }

    fn pull(from: i64, to: f64, how_hard: f64) -> Rc<dyn Domain> {
        Rc::new(Pull { from, to: fx(to), how_hard: fx(how_hard) })
    }

    #[test]
    fn a_parts_seeds_hash_its_path_of_names() {
        // Its name hashed as it grows, as every world's seeds were from the text "forest/trees:what".
        let seed = |path: &str| stream(7, crate::noise::hash_text(path, 0));
        let world = Scope::new(7);
        assert_eq!(world.seed("relief"), seed(":relief"));
        assert_eq!(world.of("valley").of("forest").seed("heights"), seed("valley/forest:heights"));
    }

    #[test]
    fn a_band_pulls_as_hard_as_each_says() {
        let flat = Lie::new(|_, _| fx(10.0));
        let half = flat.shaped(vec![pull(0, 20.0, 0.5)]);
        assert_eq!((half.at(-1, 0), half.at(0, 0)), (fx(10.0), fx(15.0)));
        assert_eq!(flat.shaped(vec![pull(0, 20.0, 0.25), pull(0, 30.0, 0.75)]).at(0, 0), fx(27.5));
    }

    #[test]
    #[should_panic(expected = "domains pull column (0, 0) 1.5 hard")]
    fn refuses_pulls_adding_up_to_more_than_all_of_a_column() {
        Lie::new(|_, _| fx(10.0)).shaped(vec![pull(0, 20.0, 1.0), pull(0, 40.0, 0.5)]).at(0, 0);
    }
}
