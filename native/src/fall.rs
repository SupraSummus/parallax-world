//! What a cut brings down, found on a thread of its own, which the thread
//! that draws never waits for (threads.rs), and shown falling. It knows
//! nothing of what things are: matter bears (it stops a walking body, as a
//! world marks its kinds) or is loose (`kind`).
//!
//! Statics (statics/, docs/statics.md) of what bears, joined to the cut,
//! all of one material (MATTER). A cut bears what is over it, so what lies
//! lower is held, and what lies at the edge of what is looked at
//! (`Around`). What stood before the cut stands, MARGIN times as strong as
//! it needed (statics' `as_generated`). One rule decides what falls, by
//! the most: a cell falls if most of its faces do; loose matter goes with
//! the matter that bears nearest it, if most of that fell. Each joined
//! piece falls as a body of its own (motion.rs).

use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};

use lattice::{ALPHA, FastMap, FastSet};
use statics::bodies::{as_generated, settle};
use statics::lattice::{Solid, build};
use statics::model::{Mat, WOOD};
use web_time::Instant;
use worldgen::edits::Sites;
use worldgen::grid::{Held, VoxelGrid};
use worldgen::tiles::{TILE, made_with, sites_as};
use worldgen::units::m;

use crate::motion::{self, Body, Pose};
use crate::threads::Post;
use crate::world::World;

/** What bears is all of one material: the worlds give none. */
const MATTER: Mat = WOOD;
/** How far across from a cut statics looks, each way, and how far over it, lattice units. */
const REGION: i64 = m(10.0);
const ABOVE: i64 = m(30.0);
/** Cells not held, past which it is not looked at: settling takes too long. */
const MOST: usize = 150_000;
/** How far from loose matter what bears, that it goes with, is looked for. */
const REACH: i64 = m(2.0);

/** What a site holds, as statics and motion see it, in the order of how much it bears. */
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Kind {
    Empty,
    Loose,
    Bearing,
}

/** What `held` is: what stops a walking body bears; a medium alone is nothing. */
pub fn kind(held: &Held) -> Kind {
    let ((pieces, _), _, solid, ..) = held;
    match () {
        _ if *pieces == 0 => Kind::Empty,
        _ if *solid > ALPHA / 2 => Kind::Bearing,
        _ => Kind::Loose,
    }
}

/** A world's level-0 sites as its edits had them at some time, made as asked. */
struct Voxels {
    world: Arc<worldgen::World>,
    edits: Arc<Sites>,
    sites: FastMap<[i64; 3], Arc<VoxelGrid>>,
}

impl Voxels {
    fn held(&mut self, site: [i64; 3]) -> Held {
        let key = site.map(|v| v.div_euclid(TILE));
        if !self.sites.contains_key(&key) {
            let sites = Arc::new(sites_as(&self.world, 0, key, &self.edits));
            self.sites.extend(made_with(&self.world, 0, key).map(|with| (with, sites.clone())));
        }
        let [x, y, z] = site;
        self.sites[&key].held(x, y, z)
    }

    fn kind(&mut self, site: [i64; 3]) -> Kind {
        kind(&self.held(site))
    }
}

/** What statics looks at about a cut: its sites, the lowest's height, its middle across. */
struct Around {
    cut: FastSet<[i64; 3]>,
    lowest: i64,
    middle: [i64; 2],
}

impl Around {
    fn of(cut: &[[i64; 3]]) -> Option<Around> {
        let lowest = cut.iter().map(|s| s[1]).min()?;
        let n = cut.len() as i64;
        let middle = [0, 2].map(|k| cut.iter().map(|s| s[k]).sum::<i64>() / n);
        Some(Around { cut: cut.iter().copied().collect(), lowest, middle })
    }

    fn across(&self, s: [i64; 3]) -> i64 {
        (s[0] - self.middle[0]).abs().max((s[2] - self.middle[1]).abs())
    }

    fn inside(&self, s: [i64; 3]) -> bool {
        self.across(s) <= REGION && s[1] <= self.lowest + ABOVE
    }

    /** Lower than the cut, or at the edge. */
    fn held(&self, s: [i64; 3]) -> bool {
        s[1] < self.lowest || self.across(s) == REGION || s[1] == self.lowest + ABOVE
    }
}

/** What bears, joined to the cut, as it was before it, and whether each is held; None past MOST. */
fn bearing(voxels: &mut Voxels, around: &Around) -> Option<FastMap<[i64; 3], bool>> {
    let mut bearing: FastMap<[i64; 3], bool> = FastMap::default();
    let mut queue: VecDeque<[i64; 3]> = around.cut.iter().copied().filter(|&s| voxels.kind(s) == Kind::Bearing).collect();
    bearing.extend(queue.iter().map(|&s| (s, around.held(s))));
    let mut free = 0;
    while let Some(site) = queue.pop_front() {
        if bearing[&site] {
            continue;
        }
        free += 1;
        if free > MOST {
            return None;
        }
        for next in faces(site) {
            if around.inside(next) && !bearing.contains_key(&next) && voxels.kind(next) == Kind::Bearing {
                bearing.insert(next, around.held(next));
                queue.push_back(next);
            }
        }
    }
    Some(bearing)
}

/** What the cut brings down of `bearing`: the cells most of whose faces fall, the world as generated standing. */
fn fell(bearing: &FastMap<[i64; 3], bool>, cut: &FastSet<[i64; 3]>) -> FastSet<[i64; 3]> {
    let solids = |with_cut: bool| -> Vec<Solid> {
        let kept = bearing.iter().filter(|(s, _)| with_cut || !cut.contains(*s));
        kept.map(|(&site, &held)| Solid { site, mat: MATTER, held }).collect()
    };
    let mut edited = build(0, &solids(false));
    as_generated(&mut edited, &build(0, &solids(true)));
    let mut gone: FastMap<[i64; 3], u32> = FastMap::default();
    for (around, &fell) in edited.around.iter().zip(&settle(&edited).gone) {
        if fell {
            for cell in around {
                *gone.entry(*cell).or_default() += 1;
            }
        }
    }
    gone.into_iter().filter(|(c, n)| 2 * *n > 6 && bearing.get(c) == Some(&false)).map(|(c, _)| c).collect()
}

/** The pieces of `fell`, each joined across faces: by site, and how many. */
fn pieces(fell: &FastSet<[i64; 3]>) -> (FastMap<[i64; 3], usize>, usize) {
    let mut piece: FastMap<[i64; 3], usize> = FastMap::default();
    let mut count = 0;
    for &start in fell {
        if piece.contains_key(&start) {
            continue;
        }
        let mut queue = vec![start];
        piece.insert(start, count);
        while let Some(site) = queue.pop() {
            for next in faces(site) {
                if fell.contains(&next) && !piece.contains_key(&next) {
                    piece.insert(next, count);
                    queue.push(next);
                }
            }
        }
        count += 1;
    }
    (piece, count)
}

/**
 * The loose matter that goes with the pieces (`piece`): that joined to
 * them, across faces, edges or corners, `inside` what is looked at, each
 * with its piece (`goes_with`).
 */
fn loose(piece: &FastMap<[i64; 3], usize>, around: &Around, after: &mut dyn FnMut([i64; 3]) -> Kind) -> FastMap<[i64; 3], usize> {
    let mut loose = FastMap::default();
    let mut seen: FastSet<[i64; 3]> = piece.keys().copied().collect();
    let mut queue: VecDeque<[i64; 3]> = piece.keys().copied().collect();
    while let Some(site) = queue.pop_front() {
        for next in about(site) {
            if around.inside(next)
                && seen.insert(next)
                && after(next) == Kind::Loose
                && let Some(p) = goes_with(next, piece, after)
            {
                loose.insert(next, p);
                queue.push_back(next);
            }
        }
    }
    loose
}

/**
 * The piece loose matter at `site` goes with: that of the most of the
 * matter that bears nearest it (in the first cube about it holding any,
 * within REACH), if most of that fell.
 */
fn goes_with(site: [i64; 3], piece: &FastMap<[i64; 3], usize>, world: &mut dyn FnMut([i64; 3]) -> Kind) -> Option<usize> {
    for r in 1..=REACH {
        let (mut stood, mut fell) = (0, FastMap::<usize, usize>::default());
        let cube = (-r..=r).flat_map(|dx| (-r..=r).flat_map(move |dy| (-r..=r).map(move |dz| [dx, dy, dz])));
        for d in cube.filter(|d| d.iter().any(|v| v.abs() == r)) {
            let near = [site[0] + d[0], site[1] + d[1], site[2] + d[2]];
            match piece.get(&near) {
                Some(&p) => *fell.entry(p).or_default() += 1,
                None if world(near) == Kind::Bearing => stood += 1,
                None => {}
            }
        }
        let fallen: usize = fell.values().sum();
        if fallen + stood > 0 {
            return (fallen > stood).then(|| fell.into_iter().max_by_key(|&(p, n)| (n, std::cmp::Reverse(p))).expect("some").0);
        }
    }
    None
}

/** The sites across the faces of a site. */
fn faces(s: [i64; 3]) -> impl Iterator<Item = [i64; 3]> {
    lattice::ray::FACES.into_iter().map(move |f| [s[0] + f[0], s[1] + f[1], s[2] + f[2]])
}

/** The sites about a site, across its faces, edges and corners. */
fn about(s: [i64; 3]) -> impl Iterator<Item = [i64; 3]> {
    (0..27).filter(|&d| d != 13).map(move |d| [s[0] + d % 3 - 1, s[1] + d / 3 % 3 - 1, s[2] + d / 9 - 1])
}

/** A cut, as statics is asked of it: the sites it took out, and the world's edits just before. */
pub struct Job {
    pub cut: Vec<[i64; 3]>,
    pub before: Arc<Sites>,
}

/** What a cut brings down: each piece that falls, as a body and its poses (motion.rs); and what was found, told of. */
#[derive(Default)]
pub struct Fallen {
    pub falls: Vec<(Body, Vec<Pose>)>,
    pub told: String,
}

/** What `job`'s cut brings down, in `world`. */
pub fn brought_down(world: Arc<worldgen::World>, job: &Job) -> Fallen {
    let started = Instant::now();
    let Some(around) = Around::of(&job.cut) else { return Fallen::default() };
    let mut voxels = Voxels { world, edits: job.before.clone(), sites: FastMap::default() };
    let Some(bearing) = bearing(&mut voxels, &around) else {
        return Fallen { told: format!("statics: over {MOST} cells about the cut, not looked at"), ..Fallen::default() };
    };
    let fell = fell(&bearing, &around.cut);
    let (piece, count) = pieces(&fell);
    // The world after the cut; and for each piece falling, after it, without what else falls with it.
    let mut after = |s: [i64; 3]| if around.cut.contains(&s) { Kind::Empty } else { voxels.kind(s) };
    let loose = loose(&piece, &around, &mut after);
    let mut cells: Vec<Vec<[i64; 3]>> = vec![Vec::new(); count];
    for (&site, &p) in piece.iter().chain(&loose) {
        cells[p].push(site);
    }
    let mut falls = Vec::new();
    for cells in cells {
        let body = Body { cells: cells.into_iter().map(|s| (s, voxels.held(s))).collect() };
        let falling = |s: &[i64; 3]| piece.contains_key(s) || loose.contains_key(s);
        let poses = motion::fall(&body, &mut |s| if falling(&s) || around.cut.contains(&s) { Kind::Empty } else { voxels.kind(s) });
        falls.push((body, poses));
    }
    let told = format!(
        "statics: {} cells about the cut bear, {} fall in {count} pieces with {} loose, in {:.1} s",
        bearing.len(),
        fell.len(),
        loose.len(),
        started.elapsed().as_secs_f64()
    );
    Fallen { falls, told }
}

/** A body falling: its body and poses, the pose shown, and since when it falls. */
struct Falling {
    body: Body,
    poses: Vec<Pose>,
    pose: usize,
    since: Instant,
}

/**
 * Statics after cuts, on a thread of its own: a cut is posted, what it
 * brings down comes back; and what falls is shown falling, the latest
 * pose due once the world shows the last (World::editing), so a slow
 * world skips poses rather than lags.
 */
pub struct Falls {
    post: Post<Job>,
    done: Receiver<Fallen>,
    /** Cuts posted, not come back yet. */
    pub waiting: usize,
    falling: Vec<Falling>,
    /** What the last that came back found. */
    pub told: String,
}

impl Falls {
    pub fn new(world: Arc<worldgen::World>) -> Falls {
        let (send, done): (Sender<Fallen>, _) = channel();
        let post = Post::spawn(|jobs: Receiver<Job>| {
            vec![move || {
                crate::world::behind_drawing();
                loop {
                    match jobs.try_recv() {
                        Ok(job) => _ = send.send(brought_down(world.clone(), &job)),
                        Err(TryRecvError::Empty) => std::thread::park(),
                        Err(TryRecvError::Disconnected) => return,
                    }
                }
            }]
        });
        Falls { post, done, waiting: 0, falling: Vec::new(), told: String::new() }
    }

    pub fn post(&mut self, job: Job) {
        self.waiting += 1;
        self.post.send(job);
    }

    /** Takes what came back, and has `world` show what falls as it is by now. Whether anything is still found or falling. */
    pub fn falling(&mut self, world: &mut World) -> bool {
        for fallen in self.done.try_iter() {
            self.waiting -= 1;
            self.told = fallen.told;
            self.falling.extend(fallen.falls.into_iter().map(|(body, poses)| Falling { body, poses, pose: 0, since: Instant::now() }));
        }
        if !world.editing() {
            for f in &mut self.falling {
                let t = f.since.elapsed().as_secs_f64();
                let due = f.poses.iter().rposition(|p| p.t <= t).unwrap_or(0);
                if due > f.pose {
                    world.change(motion::changes(&f.poses[f.pose].sites, &f.poses[due].sites, &f.body));
                    f.pose = due;
                }
            }
            self.falling.retain(|f| f.pose + 1 < f.poses.len());
        }
        self.waiting > 0 || !self.falling.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /** Ground under y = 0; two posts of wood half a metre square and 6 m high, 5 m apart; leaves about the first's top. */
    pub(super) fn grove() -> worldgen::World {
        use worldgen::generator::{Field, Generator, KindDef, Layer, Op};
        let kinds = vec![
            KindDef { name: "ground", kind_type: 10, colors: &["#806040"], walk_through: false, glows: false, medium: false },
            KindDef { name: "wood", kind_type: 4, colors: &["#604020"], walk_through: false, glows: false, medium: false },
            KindDef { name: "leaves", kind_type: 5, colors: &["#40a040"], walk_through: true, glows: false, medium: false },
        ];
        let field = Field::at("grove", 0.0, Op::Fill, |x, y, z| {
            let post = |x0: i64| (x0..x0 + 4).contains(&x) && (0..4).contains(&z) && (0..48).contains(&y);
            let leaves = (x - 2).pow(2) + (y - 52).pow(2) + (z - 2).pow(2) <= 100;
            match () {
                _ if y < 0 => Some((0, None)),
                _ if post(0) || post(40) => Some((1, None)),
                _ if leaves => Some((2, None)),
                _ => None,
            }
        });
        let generator = Generator { kinds, seed: 1, layers: vec![Layer::Field(field)] };
        worldgen::World { generator, spawn: [0; 3], heights: Some((-64, 128)), points: Vec::new(), edits: Default::default() }
    }

    /** What cutting `sites` of `world` brings down, as a stroke does, and the sites of what falls. */
    fn cut(world: &Arc<worldgen::World>, sites: Vec<[i64; 3]>) -> (Fallen, FastSet<[i64; 3]>) {
        let before = world.edits.sites();
        world.edits.change(sites.iter().map(|&s| (s, Held::default())));
        let fallen = brought_down(world.clone(), &Job { cut: sites, before });
        let fell = fallen.falls.iter().flat_map(|f| f.0.cells.iter().map(|c| c.0)).collect();
        (fallen, fell)
    }

    #[test]
    fn a_post_cut_through_falls_with_its_leaves_and_nothing_else() {
        let world = Arc::new(grove());
        let (fallen, fell) = cut(&world, (0..16).map(|i| [i % 4, 8, i / 4]).collect());
        // The post's column holds wood up to 48, the leaves the ball about its top but that.
        let column = |[x, _, z]: [i64; 3]| (0..4).contains(&x) && (0..4).contains(&z);
        let post = |s: [i64; 3]| column(s) && (9..48).contains(&s[1]);
        let leaves = |s: [i64; 3]| (s[0] - 2).pow(2) + (s[1] - 52).pow(2) + (s[2] - 2).pow(2) <= 100 && !(column(s) && s[1] < 48);
        let about = (-8..13).flat_map(|x| (0..63).flat_map(move |y| (-8..13).map(move |z| [x, y, z])));
        let expected: FastSet<[i64; 3]> = about.filter(|&s| post(s) || leaves(s)).collect();
        assert!(fell == expected, "the post over the cut, and its leaves: {}", fallen.told);
        assert_eq!(fallen.falls.len(), 1, "one piece");
    }

    #[test]
    fn a_post_cut_twice_falls_in_two_pieces() {
        let world = Arc::new(grove());
        let layer = |y: i64| (0..16).map(move |i| [i % 4, y, i / 4]);
        let (fallen, _) = cut(&world, layer(8).chain(layer(24)).collect());
        let mut lowest: Vec<i64> = fallen.falls.iter().map(|f| f.0.cells.iter().map(|c| c.0[1]).min().expect("cells")).collect();
        lowest.sort();
        assert_eq!(lowest, [9, 25], "a body over each cut: {}", fallen.told);
    }

    #[test]
    fn a_post_nicked_stands() {
        let world = Arc::new(grove());
        let (fallen, fell) = cut(&world, vec![[0, 8, 0], [1, 8, 0]]);
        assert!(fell.is_empty(), "{}", fallen.told);
    }
    /**
     * A tree of the forest, notched 0.75 m over its foot on the side it
     * leans to, then cut through under the notch, falls towards it: what
     * bears over the cut, and loose matter. Cut flat through, its weight
     * over its stump, it would stand on it, as a tree does.
     */
    #[test]
    fn a_tree_of_the_forest_notched_and_cut_through_falls_towards_the_notch() {
        let world = Arc::new((worlds::by_name("forest").expect("forest").make)(12345));
        let mut voxels = Voxels { world: world.clone(), edits: world.edits.sites(), sites: FastMap::default() };
        // The nearest trunk to the spawn: bearing 3 m high and more, with none half a metre to its side.
        let [x0, y0, z0] = world.spawn;
        let foot = (0..80)
            .flat_map(|r: i64| {
                (-r..=r).flat_map(move |dx| (-r..=r).map(move |dz| [dx, dz])).filter(move |d| d[0].abs().max(d[1].abs()) == r)
            })
            .find_map(|[dx, dz]| {
                let (x, z) = (x0 + dx, z0 + dz);
                let top = (y0 - 200..y0 + 40).rev().find(|&y| voxels.kind([x, y, z]) == Kind::Bearing)?;
                let foot = (top - 200..=top).rev().find(|&y| voxels.kind([x, y, z]) != Kind::Bearing)? + 1;
                (top - foot > 24 && voxels.kind([x + 6, foot + 10, z]) != Kind::Bearing).then_some([x, foot, z])
            })
            .expect("a tree near the spawn");
        let y = foot[1] + m(0.75);
        let trunk = |y: i64, voxels: &mut Voxels| -> Vec<[i64; 3]> {
            let layer = (-8..=8).flat_map(|dx| (-8..=8).map(move |dz| [foot[0] + dx, y, foot[2] + dz]));
            layer.filter(|&s| voxels.kind(s) == Kind::Bearing).collect()
        };
        let low = trunk(y, &mut voxels);
        let middle = |sites: &[[i64; 3]]| [0, 2].map(|k| sites.iter().map(|s| s[k] as f64).sum::<f64>() / sites.len() as f64);
        let foot_middle = middle(&low);
        // The way it leans: where its trunk is 4 m up.
        let up = middle(&trunk(y + m(4.0), &mut voxels));
        let (dx, dz) = (up[0] - foot_middle[0], up[1] - foot_middle[1]);
        let length = (dx * dx + dz * dz).sqrt().max(1e-9);
        let lean = |s: &[i64; 3]| ((s[0] as f64 - foot_middle[0]) * dx + (s[2] as f64 - foot_middle[1]) * dz) / length;
        let across = low.iter().map(lean).fold(f64::NEG_INFINITY, f64::max) - low.iter().map(lean).fold(f64::INFINITY, f64::min);
        // The notch: two thirds of the trunk on the side it leans to, half a metre high, over the cut.
        let notch: Vec<[i64; 3]> = (y + 1..=y + 4).flat_map(|y| trunk(y, &mut voxels)).filter(|s| lean(s) > -across / 6.0).collect();
        let (notched, fell) = cut(&world, notch);
        assert!(fell.is_empty(), "notched, it stands: {}", notched.told);
        let (fallen, fell) = cut(&world, low);
        assert!(fell.iter().filter(|s| voxels.kind(**s) == Kind::Bearing).count() > 1000, "{}", fallen.told);
        assert!(fell.iter().any(|s| voxels.kind(*s) == Kind::Loose), "{}", fallen.told);
        assert!(fell.iter().all(|s| s[1] > y), "nothing under the cut");
        // The piece the most falls with: the tree.
        let (_, poses) = fallen.falls.iter().max_by_key(|f| f.0.cells.len()).expect("a piece");
        let last = &poses[poses.len() - 1];
        let tops = |pose: &Pose| pose.sites.keys().map(|s| s[1]).max();
        assert!(tops(last) < tops(&poses[0]), "lower than it stood: {}", fallen.told);
        let mean = |pose: &Pose| pose.sites.keys().map(lean).sum::<f64>() / pose.sites.len() as f64;
        assert!(mean(last) > mean(&poses[0]) + 8.0, "towards the notch: {} to {}", mean(&poses[0]), mean(last));
        println!("{}; {} poses", fallen.told, poses.len());
    }
}
