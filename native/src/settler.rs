//! The light kept settled on a thread of its own (light's relax.rs), for
//! the voxels engine. A brick is lit when asked for and again when its
//! light changes, once its voxels' light was found, at most every AGAIN but
//! when all is settled. The world puts what it lights in their tiles
//! (World::update), so what is shown settles as the light does.

use std::sync::Arc;
use std::sync::mpsc::{Receiver, TryRecvError};

use lattice::{BRICK, FastMap, FastSet, Grid};
use light::Shared;
use light::relax::Settling;
use web_time::{Duration, Instant};
use worldgen::grid::Box3;

use crate::engine::voxels;
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

/** Lights what `orders` say of `world`, lit as `shared` says, telling what it does by `tell`, till the orders are gone. */
pub fn run(world: Arc<worldgen::World>, shared: Shared, orders: Receiver<Order>, tell: impl Fn(Told)) {
    behind_drawing();
    let mut light = (shared.clone(), Settling::new(world.clone(), shared));
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
                                ask(&mut light.1, level, corner, &grid);
                                shown.insert((level, corner), grid);
                                due.insert((level, corner));
                            }
                        }
                        Order::Light(shared) if (shared.steps(), shared.tolerance()) == (light.0.steps(), light.0.tolerance()) => {
                            light.1.under(shared.brightness());
                            light.0 = shared;
                        }
                        // Its lines are others: all found anew, and shown as they are.
                        Order::Light(shared) => {
                            light = (shared.clone(), Settling::new(world.clone(), shared));
                            for (&(level, corner), grid) in &shown {
                                ask(&mut light.1, level, corner, grid);
                            }
                            due.extend(shown.keys());
                        }
                        Order::Edited(b) => light.1.edited(b),
                    }
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => return,
            }
        }
        let settling = &mut light.1;
        let found = settling.settle(BUDGET);
        for (level, site) in settling.changed() {
            let brick = (level as usize, site.map(|v| v as i32 & !(BRICK - 1)));
            if shown.contains_key(&brick) {
                due.insert(brick);
            }
        }
        if !due.is_empty() && (lit_at.elapsed() >= AGAIN || settling.settled()) {
            lit_at = Instant::now();
            for lit in light_bricks(settling, &shown, &mut due) {
                tell(Told::Lit(lit));
            }
        }
        if found == 0 && due.is_empty() {
            tell(Told::Idle(read));
            std::thread::park();
        }
    }
}

/** Asks `settling` for the light of the voxels of brick `corner` of `level`, of `grid`. */
fn ask(settling: &mut Settling, level: usize, corner: [i32; 3], grid: &Grid) {
    for site in voxels_of(grid, corner) {
        settling.ask(level as u32, site);
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
