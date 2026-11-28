//! The light kept settled on a thread of its own (light's relax.rs), for
//! the voxels engine, its sites' lines found on as many as the world is
//! made on (Finders). A brick is lit when asked for and again when its
//! light changes, once its voxels' light was found, at most every AGAIN but
//! when all is settled; those of tiles the world lets go, it lets go. The
//! world puts what it lights in their tiles (World::update), so what is
//! shown settles as the light does.

use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};

use lattice::{BRICK, FastMap, FastSet, Grid};
use light::Shared;
use light::relax::{Finder, Lines, Settling};
use web_time::{Duration, Instant};
use worldgen::grid::Box3;

use crate::engine::voxels;
use crate::threads;
use crate::world::{TILE, TileId, behind_drawing};

/** Sites found between looks at the orders: a few milliseconds' worth. */
const BUDGET: usize = 4096;
/** How often bricks are lit, at most, but when all has settled: each lit, its tile is replaced, and the engine places all of it again. */
const AGAIN: Duration = Duration::from_millis(200);

/** What the world has it do. */
pub enum Order {
    /** Light these bricks: by level, where their sites start, and the grid of the tile they are of. */
    Bricks(Vec<(usize, [i32; 3], Arc<Grid>)>),
    /** Light by this from now on. */
    Light(Shared),
    /** The world was edited within this box (level-0 sites). */
    Edited(Box3),
    /** The world let go of these tiles of this level: their bricks are no longer to light. */
    Forget(usize, Vec<TileId>),
}

/** What it tells the world. */
pub enum Told {
    /** Bricks lit, of one tile. */
    Lit(Lit),
    /** Nothing left to do, once it read so many orders. */
    Idle(usize),
}

/** Bricks lit, of one tile: its level, id and grid, and each brick by where its sites start, and in the grid's cells (in bricks), with its light (voxels::light). */
pub struct Lit {
    pub level: usize,
    pub id: TileId,
    pub grid: Arc<Grid>,
    pub bricks: Vec<([i32; 3], usize, Arc<[u32]>)>,
}

/** A brick, by level and where its sites start. */
type Brick = (usize, [i32; 3]);

/** Lights what `orders` say of `world`, lit as `shared` says, its lines found on `threads` threads, telling what it does by `tell`, till the orders are gone. */
pub fn run(world: Arc<worldgen::World>, shared: Shared, threads: usize, orders: Receiver<Order>, tell: impl Fn(Told)) {
    behind_drawing();
    let finders = Finders::spawn(threads);
    let mut settling = Settling::new(world.clone(), shared);
    finders.lend(&settling);
    // The bricks asked for, each with its tile's grid, and those to light.
    let (mut shown, mut due): (FastMap<Brick, Arc<Grid>>, FastSet<Brick>) = Default::default();
    let (mut read, mut lit_at) = (0, Instant::now());
    loop {
        loop {
            match orders.try_recv() {
                Ok(order) => {
                    read += 1;
                    match order {
                        Order::Bricks(bricks) => {
                            for (level, corner, grid) in bricks {
                                // A tile made again: its voxels as they were are let go, and asked for as they are.
                                if let Some(was) = shown.insert((level, corner), grid.clone()) {
                                    voxels_of(&was, corner).for_each(|site| settling.forget(level as u32, site));
                                }
                                voxels_of(&grid, corner).for_each(|site| settling.ask(level as u32, site));
                                due.insert((level, corner));
                            }
                        }
                        Order::Light(shared) if shared.tolerance() == settling.tolerance() => {
                            settling.under(shared.brightness());
                        }
                        // Its lines are others: all found anew, and shown as they are.
                        Order::Light(shared) => {
                            settling = Settling::new(world.clone(), shared);
                            finders.lend(&settling);
                            for (&(level, corner), grid) in &shown {
                                voxels_of(grid, corner).for_each(|site| settling.ask(level as u32, site));
                            }
                            due.extend(shown.keys());
                        }
                        Order::Edited(b) => settling.edited(b),
                        Order::Forget(level, tiles) => {
                            let tiles: FastSet<TileId> = tiles.into_iter().collect();
                            shown.retain(|&(l, corner), grid| {
                                let kept = l != level || !tiles.contains(&corner.map(|v| v.div_euclid(TILE as i32)));
                                if !kept {
                                    due.remove(&(l, corner));
                                    voxels_of(grid, corner).for_each(|site| settling.forget(level as u32, site));
                                }
                                kept
                            });
                        }
                    }
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => return,
            }
        }
        let found = settling.settle(BUDGET, |lines| finders.find(lines));
        for (level, site) in settling.changed() {
            let brick = (level as usize, site.map(|v| v as i32 & !(BRICK - 1)));
            if shown.contains_key(&brick) {
                due.insert(brick);
            }
        }
        if !due.is_empty() && (lit_at.elapsed() >= AGAIN || settling.settled()) {
            lit_at = Instant::now();
            for lit in light_bricks(&mut settling, &shown, &mut due) {
                tell(Told::Lit(lit));
            }
        }
        if found == 0 && due.is_empty() {
            tell(Told::Idle(read));
            std::thread::park();
        }
    }
}

/** Threads that find lines, each with a Finder of the Settling it was last lent (`lend`); dropped, they end. */
struct Finders {
    jobs: Vec<Sender<Job>>,
    found: Receiver<(usize, Vec<Lines>)>,
}

enum Job {
    Lend(Box<Finder>),
    Find(usize, Vec<Lines>),
}

impl Finders {
    /** As many threads, for the settler's life: a browser's do not end. */
    fn spawn(threads: usize) -> Finders {
        let (give, found) = channel();
        let jobs = (0..threads)
            .map(|_| {
                let ((jobs, taken), give) = (channel(), give.clone());
                threads::spawn(move || {
                    behind_drawing();
                    let mut finder = None;
                    for job in taken {
                        match job {
                            Job::Lend(lent) => finder = Some(*lent),
                            Job::Find(i, mut lines) => {
                                lines.iter_mut().for_each(|lines| lines.find(finder.as_mut().expect("a Finder lent")));
                                if give.send((i, lines)).is_err() {
                                    return;
                                }
                            }
                        }
                    }
                });
                jobs
            })
            .collect();
        Finders { jobs, found }
    }

    /** Has its threads find `settling`'s lines from now on. */
    fn lend(&self, settling: &Settling) {
        self.jobs.iter().for_each(|jobs| jobs.send(Job::Lend(Box::new(settling.finder()))).expect("finders"));
    }

    /**
     * `lines` found on its threads, given back in order: four runs a
     * thread, each of sites near one another, which read the same tiles,
     * and one message each way.
     */
    fn find(&self, lines: Vec<Lines>) -> Vec<Lines> {
        let run = lines.len().div_ceil(4 * self.jobs.len());
        let (mut lines, mut runs) = (lines.into_iter(), 0);
        loop {
            let some: Vec<Lines> = lines.by_ref().take(run).collect();
            if some.is_empty() {
                break;
            }
            self.jobs[runs % self.jobs.len()].send(Job::Find(runs, some)).expect("finders");
            runs += 1;
        }
        let mut found: Vec<_> = self.found.iter().take(runs).collect();
        found.sort_unstable_by_key(|&(i, _)| i);
        found.into_iter().flat_map(|(_, lines)| lines).collect()
    }
}

/** Lights the bricks `due` whose voxels' light was found, by tile (and its grid: a tile made again is another), and lets them go. */
fn light_bricks(settling: &mut Settling, shown: &FastMap<Brick, Arc<Grid>>, due: &mut FastSet<Brick>) -> Vec<Lit> {
    let ready: Vec<Brick> = due
        .iter()
        .copied()
        .filter(|&(level, corner)| voxels_of(&shown[&(level, corner)], corner).all(|site| settling.found_at(level as u32, site)))
        .collect();
    let mut tiles: FastMap<(usize, TileId, usize), Lit> = FastMap::default();
    for (level, corner) in ready {
        due.remove(&(level, corner));
        let grid = shown[&(level, corner)].clone();
        let Ok(first) = usize::try_from(grid.brick(corner[0], corner[1], corner[2])) else { continue };
        let light = voxels::light(&grid, corner, |at| settling.on_seen(level as u32, at).map(|on| (on, settling.glow(level as u32, at))));
        let id = corner.map(|v| v.div_euclid(TILE as i32));
        let key = (level, id, Arc::as_ptr(&grid) as usize);
        tiles.entry(key).or_insert_with(|| Lit { level, id, grid, bricks: Vec::new() }).bricks.push((corner, first, light));
    }
    tiles.into_values().collect()
}

/** The sites of the brick of `grid` whose sites start at `corner` that hold a voxel. */
fn voxels_of(grid: &Grid, corner: [i32; 3]) -> impl Iterator<Item = [i64; 3]> + '_ {
    (0..BRICK * BRICK * BRICK).filter_map(move |r| {
        let site: [i32; 3] = std::array::from_fn(|d| corner[d] + [r % BRICK, r / BRICK % BRICK, r / BRICK / BRICK][d]);
        (grid.get(site[0], site[1], site[2]) != 0).then(|| site.map(i64::from))
    })
}
