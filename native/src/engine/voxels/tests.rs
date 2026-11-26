use std::collections::HashMap;

use super::*;

/**
 * The tiles of level 0 of the forest of seed 1 around `eye`; in every
 * 64th brick of each, its first voxel made to look unlike through each
 * face, so that windows hold their views too.
 */
fn forest_tiles(eye: Vec3) -> Arc<Level> {
    let mut world = World::new((worlds::by_name("forest").expect("forest").make)(1), None, 4);
    // All around the eye asked for, as far as a window's ball reads.
    let reach = std::array::from_fn(|l| (l == 0).then_some(READ));
    let asked = Arc::new(world.lacking(0, READ, eye).into_iter().map(|id| (0, Piece::Tile(id))).collect());
    let wants = Wants { reach, lit: Lit::Not, asked };
    loop {
        world.update(eye, &wants);
        if world.making() == 0 {
            break;
        }
        world.wait();
    }
    let faced = |tile: &Tile| {
        let g = &tile.grid;
        let firsts = (0..g.cells.len() / BRICK_SITES).step_by(64);
        let sites = firsts.filter_map(|first| sites(first).find(|&i| g.cells[i] != 0));
        let views =
            sites.flat_map(|i| std::iter::once(i as u32).chain((0..lattice::FACE_KINDS as u32).map(move |k| g.cells[i] + k))).collect();
        Tile::new(Arc::new(lattice::Grid { views, ..(**g).clone() }))
    };
    Arc::new(Level::new(world.level(0).tiles.iter().map(|(id, tile)| (id, Arc::new(faced(tile))))))
}

#[test]
fn a_site_s_faces_see_their_looks_till_more_than_sights_differ() {
    // What each face sees, as voxels.wgsl's face_look reads it.
    let seen = |own: u32, views: &[u32]| -> Vec<u32> {
        let (sights, _, which) = sights(own, views);
        (0..views.len()).map(|k| which >> (2 * k) & 3).map(|at| if at == 0 { own } else { sights[at as usize - 1] }).collect()
    };
    let few = [7, 5, 5, 9, 11, 7];
    assert_eq!(seen(7, &few), few);
    let many: Vec<u32> = (10..16).collect();
    assert_eq!(seen(7, &many), [10, 11, 12, 7, 7, 7]);
}

/** Where the forest's windows are looked at from. */
const EYE: Vec3 = [10.0, 30.0, -20.0];

/** `forest_tiles(EYE)`, made once: only lit windows read their light, and only one test lights them. */
static FOREST: std::sync::LazyLock<Arc<Level>> = std::sync::LazyLock::new(|| forest_tiles(EYE));

/**
 * Syncs `windows` to level 0's `tiles` around `eye`, rays having asked
 * for every brick of them it may place, keeping what they upload in
 * `slots`; whether any window changed.
 */
fn sync(windows: &mut Windows, tiles: &Arc<Level>, eye: Vec3, slots: &mut HashMap<u32, Vec<u32>>) -> bool {
    let w = &mut windows.windows[0];
    w.asked = (0..TABLE).filter_map(|e| w.asked(e, w.ball)).filter(|(_, held)| *held).map(|(b, _)| b).collect();
    let (added, changed) = windows.sync(std::slice::from_ref(tiles), eye);
    for a in added {
        slots.insert(a.slot, words(&a.tile.grid, a.first, a.part).to_vec());
    }
    changed.contains(&true)
}

/**
 * Syncs `windows` as `sync` does till nothing changes; whether any
 * did. Each sync moves level 0's ball a brick along an axis at most
 * (or anew about the eye, the old one holding nothing around it), and
 * places STEP new tiles' bricks in it at most.
 */
fn settle(windows: &mut Windows, tiles: &Arc<Level>, eye: Vec3, slots: &mut HashMap<u32, Vec<u32>>) -> bool {
    for k in 0..1000 {
        let w = &windows.windows[0];
        let (was, had): (_, HashSet<_>) = (w.ball, w.tiles.keys().copied().collect());
        if !sync(windows, tiles, eye, slots) {
            return k > 0;
        }
        let w = &windows.windows[0];
        let ball = w.ball.expect("a ball");
        if let Some(was) = was {
            let stepped = (0..3).map(|d| (ball.middle[d] - was.middle[d]).abs()).sum::<i32>() <= 1;
            assert!(stepped || was.about(ball.middle) < 0, "{was:?} to {ball:?}, at {eye:?}");
        }
        let new = w.tiles.keys().filter(|&id| !had.contains(id) && in_ball(tile_bricks(*id, w.origin), ball, None).next().is_some());
        assert!(new.count() <= STEP, "new tiles in the ball, at {eye:?}");
    }
    panic!("unsettled at {eye:?}");
}

/**
 * What level 0's window holds, per entry of its ball: EMPTY, UNKNOWN or
 * its slots' words, as `slots` say they were uploaded, and whether it
 * counts in its cells; the slots it takes; and its counts, as its entries say.
 */
type Holds = (Vec<Option<(Result<Vec<u32>, i32>, bool)>>, usize, Vec<u32>);

fn holds(windows: &Windows, slots: &HashMap<u32, Vec<u32>>) -> Holds {
    let w = &windows.windows[0];
    let (mut counts, mut rows) = (vec![0u32; COUNTS], [[0; WINDOW as usize]; 3]);
    for b in window_bricks(w.ball.map_or([WINDOW / 2; 3], |ball| ball.middle)) {
        if w.counted[entry(b)] {
            (0..3).for_each(|d| rows[d][b[d].rem_euclid(WINDOW) as usize] += 1);
            (0..SCALES).for_each(|j| counts[cell_entry(cell_of(b, j), j)] += 1);
        }
    }
    assert!(w.counts == counts && w.rows == rows, "counts and rows as the entries say");
    // A slot's words, and those of the slots after it.
    let words = |mut slot: i32| {
        let mut words = Vec::new();
        while slot >= 0 {
            words.extend(&slots[&(slot as u32)]);
            slot = after(windows.pool.next[slot as usize]);
        }
        words
    };
    let mut held = vec![None; TABLE];
    for b in w.ball.map(|ball| in_ball(ball.bounds(), ball, None)).into_iter().flatten() {
        let e = w.table[entry(b)];
        held[entry(b)] = Some((if e >= 0 { Ok(words(e)) } else { Err(e) }, w.counted[entry(b)]));
    }
    (held, w.slots, counts)
}

/** What windows made anew, of `radius` and room for all, hold of `tiles` around `eye`. */
fn anew(radius: i32, tiles: &Arc<Level>, eye: Vec3) -> Holds {
    let (mut windows, mut slots) = (Windows::new(1 << 20, false), HashMap::new());
    (windows.radius, windows.growing) = (radius, usize::MAX);
    settle(&mut windows, tiles, eye, &mut slots);
    holds(&windows, &slots)
}

/** The bricks of the window about `middle`. */
fn window_bricks(middle: [i32; 3]) -> impl Iterator<Item = [i32; 3]> {
    let (lo, hi) = (middle.map(|v| v - WINDOW / 2), middle.map(|v| v + WINDOW / 2));
    (lo[2]..hi[2]).flat_map(move |z| (lo[1]..hi[1]).flat_map(move |y| (lo[0]..hi[0]).map(move |x| [x, y, z])))
}

#[test]
fn a_window_moved_and_its_tiles_changed_holds_what_a_new_one_would() {
    let eye = EYE;
    let all = FOREST.clone();
    let mut ids: Vec<_> = all.tiles.iter().map(|(id, _)| id).collect();
    ids.sort();
    assert!(ids.len() > 4, "{} tiles", ids.len());
    // Some tiles of all, then others, as the eye moves through bricks and across the torus's seams,
    // then farther than the ball reaches, and back: begun anew each time.
    let some =
        |skip: usize| Arc::new(Level::new(ids.iter().skip(skip).step_by(2).map(|id| (*id, all.tiles.get(*id).expect("the tile").clone()))));
    let steps = [
        (some(0), eye),
        (all.clone(), [13.0, 31.0, -9.0]),
        (some(1), [-140.0, 30.0, 70.0]),
        (all.clone(), [-130.0, 22.0, 75.0]),
        (all.clone(), [900.0, 22.0, 75.0]),
        (all.clone(), eye),
    ];
    let (mut windows, mut slots) = (Windows::new(1 << 20, false), HashMap::new());
    // No tiles yet: all it reads unknown, each brick's tile for rays to ask for.
    settle(&mut windows, &Arc::new(Level::new(HashMap::new())), eye, &mut slots);
    let w = &windows.windows[0];
    let ball = w.ball.expect("a ball");
    assert!(w.shape().gpu(0, eye, f64::MAX).reach > 0.0 && w.slots == 0);
    assert!(
        in_ball(ball.bounds(), ball, None).all(|b| w.asked(entry(b), w.ball) == Some((b, false))),
        "every brick's tile may be asked for"
    );
    for (tiles, eye) in &steps {
        assert!(settle(&mut windows, tiles, *eye, &mut slots), "a change");
        assert_eq!(windows.radius, RADIUS, "room for all");
        assert!(holds(&windows, &slots) == anew(RADIUS, tiles, *eye), "at {eye:?}");
        assert!(!sync(&mut windows, tiles, *eye, &mut slots), "nothing moved");
    }
}

#[test]
fn lit_a_window_places_the_bricks_rays_asked_for_once_lit() {
    let eye = EYE;
    let tiles = FOREST.clone();
    let mut windows = Windows::new(1 << 20, true);
    let sync = |windows: &mut Windows| windows.sync(std::slice::from_ref(&tiles), eye).1.contains(&true);
    while sync(&mut windows) {}
    // No light found: the bricks with voxels unknown, rays may ask for them, the empty ones empty.
    let w = &windows.windows[0];
    let ball = w.ball.expect("a ball");
    let held: Vec<[i32; 3]> = window_bricks(ball.middle).filter(|&b| ball.contains(b) && tiles.tiles.get(w.site(b).1).is_some()).collect();
    let unknown: Vec<[i32; 3]> = held.iter().copied().filter(|&b| w.table[entry(b)] == UNKNOWN).collect();
    assert!(held.iter().all(|&b| matches!(w.table[entry(b)], UNKNOWN | EMPTY)));
    assert!(unknown.len() > 1000 && unknown.len() < held.len(), "{} of {} bricks unknown", unknown.len(), held.len());
    assert!(unknown.iter().all(|&b| w.asked(entry(b), w.ball) == Some((b, true))), "every unknown brick may be asked for");
    // Rays ask for half, and some of all are lit (with them the buried bricks of a tile, which share their sites):
    // those placed that were asked for and are lit, no other.
    let world = (worlds::by_name("forest").expect("forest").make)(1);
    let mut light = Light::new(world, light::Shared::new(light::Brightness { sky: 1.0, sun: 1.0, glow: 0.0 }));
    for &b in unknown.iter().step_by(3) {
        let (corner, id) = w.site(b);
        tiles.tiles.get(id).expect("the tile").lit(0, corner, &mut light);
    }
    let lit = |b: [i32; 3]| {
        let ([x, y, z], id) = w.site(b);
        let tile = tiles.tiles.get(id).expect("the tile");
        tile.light(tile.grid.brick(x, y, z) as usize).is_some()
    };
    let lit: Vec<bool> = unknown.iter().map(|&b| lit(b)).collect();
    windows.windows[0].asked = unknown.iter().copied().step_by(2).collect();
    assert!(sync(&mut windows), "the lit placed");
    let w = &windows.windows[0];
    for (k, &b) in unknown.iter().enumerate() {
        let placed = w.table[entry(b)] >= 0;
        assert_eq!(placed, k % 2 == 0 && lit[k], "brick {b:?}");
        assert_eq!(w.asked(entry(b), w.ball).is_some(), !placed, "brick {b:?}");
    }
    assert!(!sync(&mut windows), "nothing more to place");
}

#[test]
fn windows_short_of_slots_hold_as_large_a_ball_as_fits() {
    let eye = EYE;
    let tiles = FOREST.clone();
    let (all, wanted, _) = anew(RADIUS, &tiles, eye);
    // Buried bricks of a tile share a slot: fewer slots than bricks held.
    let entries = all.iter().flatten().filter(|(e, _)| e.is_ok()).count();
    assert!(wanted < entries, "{wanted} slots for {entries} bricks");
    let pool = wanted / 3;
    let (mut windows, mut slots) = (Windows::new(pool, false), HashMap::new());
    for eye in [eye, [13.0, 31.0, -9.0], [21.0, 33.0, -3.0], eye] {
        settle(&mut windows, &tiles, eye, &mut slots);
        let w = &windows.windows[0];
        let ball = w.ball.expect("a ball");
        assert!(ball.radius == windows.radius && windows.radius < RADIUS && w.slots <= pool, "at {eye:?}");
        // Every brick of a tile made in the ball held, every other unknown.
        for b in window_bricks(ball.middle) {
            let inside = ball.contains(b) && tiles.tiles.get(w.site(b).1).is_some();
            assert_eq!(w.table[entry(b)] != UNKNOWN, inside, "brick {b:?}, at {eye:?}");
        }
        assert!(holds(&windows, &slots) == anew(windows.radius, &tiles, eye), "at {eye:?}");
    }
    // More room: they grow, a brick at a sync, to all.
    windows.pool.free.extend(pool as u32..2 * wanted as u32);
    windows.pool.next.resize(2 * wanted, EMPTY);
    for _ in 0..RADIUS {
        sync(&mut windows, &tiles, eye, &mut slots);
    }
    assert_eq!(windows.radius, RADIUS);
    assert!(holds(&windows, &slots) == anew(RADIUS, &tiles, eye));
}

#[test]
fn a_window_holds_the_bricks_of_every_tile_made_past_the_heights_too() {
    // Over the forest's heights, where its tiles are made empty (world.rs): each brick of a tile made is held,
    // as empty or with voxels, not unknown, so rays read on through it, not the next level. Those over
    // the heights are empty.
    let tall = (worlds::by_name("forest").expect("forest").make)(1).heights_at(0).1 as f64;
    let eye = [10.0, tall + 100.0, -20.0];
    let tiles = forest_tiles(eye);
    let mut windows = Windows::new(1 << 20, false);
    settle(&mut windows, &tiles, eye, &mut HashMap::new());
    let w = &windows.windows[0];
    let ball = w.ball.expect("a ball");
    let (mut made, mut over) = (0, 0);
    for b in window_bricks(ball.middle).filter(|&b| ball.contains(b)) {
        let (site, id) = w.site(b);
        if tiles.tiles.get(id).is_none() {
            continue;
        }
        made += 1;
        assert_ne!(w.table[entry(b)], UNKNOWN, "brick {b:?} of a tile made");
        if f64::from(site[1]) >= tall {
            over += 1;
            assert_eq!(w.table[entry(b)], EMPTY, "brick {b:?}, over the heights");
        }
    }
    assert!(made > 10_000 && over > 1000, "{made} bricks of tiles made, {over} over the heights");
}

#[test]
fn the_window_and_the_tiles_wanted_hold_all_a_ray_reads_of_what_it_meets() {
    // The voxel a ray is in, wherever it is up to its level's reach, and that voxel's neighbours, whose faces
    // shade it: eyes along a line across a tile, rays every way, balls of any radius.
    let world = World::new((worlds::by_name("forest").expect("forest").make)(1), None, 1);
    let ways = (0..125).map(|i| [i % 5 - 2, i / 5 % 5 - 2, i / 25 - 2].map(f64::from)).filter(|w| w != &[0.0; 3]);
    for step in 0..TILE * 4 / 3 {
        let t = step as f64 * 0.75;
        let eye = [t, 0.6 * t + 31.3, 0.3 * t - 17.9];
        let tiles: HashSet<_> = world.lacking(0, READ, eye).into_iter().collect();
        // Balls about the eye's brick, or behind it, rays reading as far as they hold around it.
        let balls = [0, 1, 13, RADIUS].into_iter().flat_map(|radius| {
            [[0, 0, 0], [1, -1, 1], [3, 0, -2], [-7, 5, 4]]
                .map(|behind| Ball { middle: std::array::from_fn(|d| eye_brick(0, eye, [0; 3])[d] + behind[d]), radius })
        });
        for ball in balls.filter(|b| b.about(eye_brick(0, eye, [0; 3])) >= 0) {
            let read = reach_of(ball.about(eye_brick(0, eye, [0; 3])));
            for way in ways.clone() {
                let length = way.iter().map(|v| v * v).sum::<f64>().sqrt();
                let site = lattice::ray::nearest_site(std::array::from_fn(|d| eye[d] + way[d] / length * read));
                for s in std::iter::once([0; 3]).chain(lattice::VOXEL_FACES).map(|n| std::array::from_fn(|d| site[d] as i32 + n[d])) {
                    let b = s.map(|v: i32| v.div_euclid(BRICK));
                    assert!(ball.contains(b), "eye {eye:?}: site {s:?} out of {ball:?}");
                    assert!(tiles.contains(&s.map(|v| v.div_euclid(TILE as i32))), "eye {eye:?}: site {s:?} in no tile wanted");
                }
            }
        }
    }
}

#[test]
fn no_voxel_the_window_holds_is_out_of_its_box_or_in_a_cell_counted_hollow() {
    let eye = EYE;
    let tiles = FOREST.clone();
    // Moved, so that bricks of both sides of the torus have been in it.
    let mut windows = Windows::new(1 << 20, false);
    settle(&mut windows, &tiles, [eye[0] - 300.0, eye[1], eye[2] + 200.0], &mut HashMap::new());
    settle(&mut windows, &tiles, eye, &mut HashMap::new());
    let window = &windows.windows[0];
    let (ball, origin, (low, high)) = (window.ball.expect("a ball"), window.origin, window.bounds());
    let (lo, hi) = ball.bounds();
    let (lo, hi) = (lo.map(|v| v * BRICK), hi.map(|v| v * BRICK));
    // Every voxel of the ball, and the cell of each scale it is in, at the ball's edge too.
    let mut voxels = 0;
    for z in lo[2]..hi[2] {
        for y in lo[1]..hi[1] {
            for x in lo[0]..hi[0] {
                let p = [x + origin[0], y + origin[1], z + origin[2]];
                let b = [x, y, z].map(|v| v.div_euclid(BRICK));
                if !ball.contains(b) || tiles.grids.at(p[0], p[1], p[2]).is_none_or(|v| v == 0) {
                    continue;
                }
                voxels += 1;
                assert!((0..3).all(|d| low[d] <= p[d] as f32 && (p[d] as f32) < high[d]), "a voxel at {p:?} out of {low:?}..{high:?}");
                for j in 0..SCALES {
                    assert_ne!(window.counts[cell_entry(cell_of(b, j), j)], 0, "a voxel at {p:?} in a cell of scale {j} counted hollow");
                }
            }
        }
    }
    assert!(voxels > 100_000, "{voxels} voxels");
}

#[test]
fn the_shader_is_valid_and_lays_out_what_this_file_writes() {
    use crate::gpu::wgsl;
    let module = wgsl::shader(&shader());
    wgsl::assert_layout!(
        &module,
        "Globals",
        Globals { resolution, extent, fog, sky_fog, lit, exposure, sky_top, horizon, view, white, pointer }
    );
    wgsl::assert_layout!(&module, "Level", GpuLevel { min, offset, corner, reach, low, filled, high });
    wgsl::assert_layout!(&module, "Told", Told { metered, probe });
    wgsl::assert_layout!(&module, "Probe", Probe { site, level, on, distance, glow, crossed, through, path, media });
    wgsl::assert_layout!(&module, "Crossed", Crossed { site, level, through, path });
}
