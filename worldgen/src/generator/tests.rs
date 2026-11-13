use super::*;
use crate::noise::fx;

#[test]
fn a_long_lathe_reaches_its_widest_circle() {
    // 1000 units long, at 3 : 4 : 0, so its widest circle reaches 4, 3 and 5 units along x, y and z.
    let shape = lathe([0.0; 3], [600.0, 800.0, 0.0], |_| 5.0, 1, Fill::Const(1.0), Op::Fill);
    assert_eq!((shape.bounds.min, shape.bounds.max), ([-4, -3, -5], [605, 804, 6]));
}

/** `f` of 0 to `n`, in order, on every core: many seeds take long to measure one by one. */
fn on_threads<T: Send>(n: usize, f: impl Fn(usize) -> T + Sync) -> Vec<T> {
    let per_thread = n.div_ceil(std::thread::available_parallelism().map_or(1, |t| t.get())).max(1);
    let f = &f;
    std::thread::scope(|scope| {
        let parts: Vec<_> = (0..n)
            .step_by(per_thread)
            .map(|from| scope.spawn(move || (from..n.min(from + per_thread)).map(f).collect::<Vec<_>>()))
            .collect();
        parts.into_iter().flat_map(|part| part.join().expect("measured")).collect()
    })
}

#[test]
fn a_measured_far_form_keeps_what_rays_see() {
    // A solid block 64 units wide, painted with a sheet of glass on top two fine sites deep, and a sparse cloud:
    // measured at sites 16 apart from sites 4 apart.
    // Made on each thread: a node is not Sync.
    let block = || Node {
        far: Some(Far::measured(fx(2.0))),
        ..Node::of(vec![cuboid([0.0; 3], [64.0; 3], 1, SOLID, Op::Fill), cuboid([0.0, 56.0, 0.0], [64.0, 64.0, 64.0], 2, SOLID, Op::Paint)])
    };
    let one = block();
    let sites = measure(&one, "block", "blocks", bounds_of(&one), fixed(2.0), 16, 1);
    assert!(
        (1..4).all(|y| (1..4).all(|x| (1..4).all(|z| (x + y + z) % 2 == 1 || sites.iter().any(|s| s.0 == [x, y, z])))),
        "its inside is all drawn"
    );
    // Rays straight down see the glass on its top row of cells, half covered, through the faces towards them;
    // where that is dropped, the cells under it, whose faces up the columns from above credit with the glass
    // too. Rays from the sides see the block through the faces towards them, but in the top row, where the
    // glass shows at their tops. Each block picks its cells' kinds anew.
    let seen = |sites: &Sites, ray: [i64; 3]| {
        let looks: std::collections::HashMap<[i64; 3], Look> = sites.iter().map(|s| (s.0, s.1)).collect();
        let face = |look: &Look, k: usize| look.faces.map_or(look.kind, |faces| faces[k]);
        // The faces a ray along `ray` may enter by: those facing back along it.
        let faces: Vec<usize> = (0..FACE_KINDS).filter(|&k| (0..3).map(|c| VOXEL_FACES[k][c] as i64 * ray[c]).sum::<i64>() < 0).collect();
        let (mut glass, mut hits) = (0, 0);
        for u in (2..64).step_by(4) {
            for v in (2..64).step_by(4) {
                // From above, or from −x, at the middle height.
                let along = |t: i64| if ray[1] < 0 { [u, 127 - t, v] } else { [t - 64, 32 + (u % 16) - 8, v] };
                let Some(look) = (0..192).find_map(|t| looks.get(&nearest_site(along(t).map(|v| v * ONE), 16))) else { continue };
                hits += faces.len();
                glass += faces.iter().filter(|&&k| face(look, k) == 2).count();
            }
        }
        glass as f64 / hits as f64
    };
    let blocks: Vec<Sites> = on_threads(16, |i| {
        let block = block();
        measure(&block, &format!("block {i}"), "blocks", bounds_of(&block), fixed(2.0), 16, 1)
    });
    let from_above = blocks.iter().map(|sites| seen(sites, [0, -1, 0])).sum::<f64>() / 16.0;
    let from_the_side = blocks.iter().map(|sites| seen(sites, [1, 0, 0])).sum::<f64>() / 16.0;
    // Over these 16 blocks, 0.79 from above.
    assert!(from_above > 0.65, "rays from above see the glass on top {from_above} of the time");
    assert!(from_the_side < 0.1, "rays from the side see the glass {from_the_side} of the time");
    // A sparse cloud's shares of its about 134 cells (a ball's volume over a site's, 2 · 16³) add up to about a
    // quarter of them, each cloud's within a cell or two of that: it is not kept or dropped whole. Its fine
    // sites, each kept with a chance of 1 − 0.98⁴, stop 0.27 of the rays along an axis through a cell inside
    // it, traced through their voxels; counted by columns, not pairs, a sixth of them were kept.
    let cloud = || Node { far: Some(Far::measured(fx(2.0))), ..Node::of(vec![ball([0.0; 3], 64.0, 1, Fill::Const(0.02), Op::Fill)]) };
    let kept: Vec<f64> = on_threads(16, |i| {
        let cloud = cloud();
        let sites = measure(&cloud, &format!("cloud {i}"), "clouds", bounds_of(&cloud), fixed(2.0), 16, 1);
        sites.iter().map(|s| s.3 as f64 / SHARE as f64).sum()
    });
    assert!(kept.iter().all(|n| (26.0..38.0).contains(n)), "sparse clouds keep {kept:?} of their about 134 cells");
}

#[test]
fn a_cells_pairs_of_columns_are_counted() {
    // Every fine site near a few cells of a level, assigned to its nearest: each cell's columns holding one,
    // per axis, and the pairs of neighbours either of which does, as pairs_of_cell counts them for any cell.
    for r in [2i64, 4, 8, 16] {
        for centre in [[0, 0, 0], [1, 1, 0], [-1, 0, 1], [0, 2, 0], [3, -1, 2]] {
            let mut columns: [std::collections::HashSet<(i64, i64)>; 3] = Default::default();
            let c = centre.map(|v| v * r);
            for x in c[0] - r..=c[0] + r {
                for y in c[1] - r..=c[1] + r {
                    for z in c[2] - r..=c[2] + r {
                        if (x + y + z) & 1 == 0 && nearest_site([x, y, z].map(|v| v * ONE), r) == centre {
                            columns[0].insert((y, z));
                            columns[1].insert((z, x));
                            columns[2].insert((x, y));
                        }
                    }
                }
            }
            let pairs = columns.map(|held| {
                let mut pairs = std::collections::HashSet::new();
                for &(i, j) in &held {
                    for (u, v) in [(i - 1, j), (i + 1, j), (i, j - 1), (i, j + 1)] {
                        pairs.insert(if (i, j) < (u, v) { ((i, j), (u, v)) } else { ((u, v), (i, j)) });
                    }
                }
                pairs.len() as u32
            });
            assert_eq!(pairs, pairs_of_cell(r), "cells {r} sites wide, at {centre:?}");
        }
        println!("{r}: {:?} halves of a column, of a shadow of {}", pairs_of_cell(r), 4 * r * r);
    }
}

/**
 * Levels of detail by construction: at sites s apart, a tube holds
 * what is within its radius of the curve, give or take s / 2, as the
 * curve is drawn through the points of a level it strays from by
 * no more.
 */
#[test]
fn a_tube_is_within_half_a_site_of_its_curve() {
    let curve = Curve::wandering(3, [8.0, 4.0, 2.0, 1.0].map(fx).to_vec());
    let ends = [[0.0, 0.0, 0.0], [96.0, 40.0, -24.0]];
    let line: Vec<Point> = curve.points(ends.map(|e| e.map(fx))).iter().map(|p| p.map(Fx::to_f64)).collect();
    let shape = tube(&curve, ends[0], ends[1], 3.0, 1, SOLID, Op::Fill);
    let distance = |p: Point| {
        let to = |a: Point, b: Point| {
            let (ab, ap) = ([0, 1, 2].map(|k| b[k] - a[k]), [0, 1, 2].map(|k| p[k] - a[k]));
            let t = ((0..3).map(|k| ab[k] * ap[k]).sum::<f64>() / (0..3).map(|k| ab[k] * ab[k]).sum::<f64>()).clamp(0.0, 1.0);
            js::hypot(&[0, 1, 2].map(|k| ap[k] - t * ab[k]))
        };
        line.windows(2).map(|s| to(s[0], s[1])).fold(f64::INFINITY, f64::min)
    };
    let b = shape.bounds;
    for s in [1, 2, 4, 8, 16] {
        let inside = shape.inside(0, s);
        // A hundredth of a unit for rounding.
        let slack = s as f64 / 2.0 + 0.01;
        for x in b.min[0].div_euclid(s)..=b.max[0].div_euclid(s) {
            for y in b.min[1].div_euclid(s)..=b.max[1].div_euclid(s) {
                for z in b.min[2].div_euclid(s)..=b.max[2].div_euclid(s) {
                    let d = distance([x, y, z].map(|v| (v * s) as f64));
                    let holds = inside.holds([x, y, z].map(|v| v * s * ONE));
                    assert!(holds || d > 3.0 - slack, "at sites {s} apart, ({x}, {y}, {z}) is {d:.3} from the curve, not held");
                    assert!(!holds || d <= 3.0 + slack, "at sites {s} apart, ({x}, {y}, {z}) is {d:.3} from the curve, held");
                }
            }
        }
    }
}

#[test]
fn a_gable_narrows_to_its_ridge() {
    // 8 wide across x, 4 high: at height h, the sites within 4 · (1 − h / 4) of x = 4.
    let shape = gable([0.0; 3], [8.0, 4.0, 6.0], 2, 1, SOLID, Op::Fill);
    let inside = shape.inside(0, 1);
    let holds = |x: i64, y: i64| inside.holds([x, y, 2].map(|v| v * ONE));
    assert!(holds(0, 0) && holds(7, 0) && holds(4, 3) && holds(3, 2) && holds(6, 1));
    assert!(!holds(1, 3) && !holds(8, 0) && !holds(4, 4) && !holds(7, 2));
}

/** A generator of `layers`, of one kind. */
fn layers(layers: Vec<Layer>) -> Generator {
    Generator {
        kinds: vec![KindDef { name: "stone", kind_type: 0, colors: &["#808080"], walk_through: false, glows: false, medium: false }],
        seed: 1,
        layers,
    }
}

/** Ground `height(x)` high, said to rise at most `slope` a column: eight units of it about it. */
fn ground(height: fn(i64) -> i64, slope: i64) -> Generator {
    let of = Rc::new(Sloped::new(Rc::new(plan::Plan::new("ground", 1, move |x, _| height(x))), slope));
    let field = Field::columns("ground", 0.0, Op::Fill, move |x, _, _| vec![Span::solid(height(x) - 8, height(x) + 1, 0)]);
    layers(vec![Layer::Field(field.about(of, 8, 1))])
}

#[test]
fn a_field_about_its_ground_is_drawn_only_where_its_lines_may_be() {
    let flat = ground(|_| 0, 0);
    let (across, above) = (Box3 { min: [0, -16, 0], max: [64, 16, 8] }, Box3 { min: [0, 16, 0], max: [64, 48, 8] });
    let drawn = |b: &Box3| checking_skipped(|| generate(&flat, b, 0).pieces.alpha.iter().any(|&a| a > 0));
    assert_eq!(drawn(&across), (true, Vec::new()));
    assert_eq!(drawn(&above), (false, Vec::new()), "skipped above it, and nothing would be drawn there");
}

#[test]
#[should_panic(expected = "field ground: span [-8, 1) at column (0, 0) reaches past 2 under and 1 over its ground 0")]
fn refuses_a_field_reaching_past_what_it_says() {
    let flat = Rc::new(Sloped::new(Rc::new(plan::Plan::new("flat", 1, |_, _| 0)), 0));
    let field = Field::columns("ground", 0.0, Op::Fill, |_, _, _| vec![Span::solid(-8, 1, 0)]).about(flat, 2, 1);
    generate(&layers(vec![Layer::Field(field)]), &Box3 { min: [0, -16, 0], max: [8, 16, 8] }, 0);
}

#[test]
fn checking_finds_what_a_broken_promise_skips() {
    // A spike at x = 12 the slope's rule never sees (no neighbour of it is read): sampled every 8, it is skipped.
    let spiked = ground(|x| if x == 12 { 200 } else { 0 }, 0);
    let b = Box3 { min: [0, 45, 0], max: [32, 55, 4] };
    assert!(generate(&spiked, &b, 2).pieces.alpha.iter().all(|&a| a == 0), "skipped");
    let (_, broken) = checking_skipped(|| generate(&spiked, &b, 2));
    assert!(broken.len() == 1 && broken[0].starts_with("ground draws where it says it may not: at site [3, 49, 0]"), "{broken:?}");
}

/** Ground `height(x)` high, said to rise at most `slope` a column: a surface about it, stone under it. */
fn solid_ground(height: fn(i64) -> i64, slope: i64) -> Generator {
    let of = Rc::new(Sloped::new(Rc::new(plan::Plan::new("ground", 1, move |x, _| height(x))), slope));
    let field = Field::columns("ground", 0.0, Op::Fill, move |x, _, s| crate::column::Column::new(height(x), s).surface(0, 4).under(0));
    layers(vec![Layer::Field(field.about(of, 4, 1).under(0))])
}

#[test]
fn a_field_under_its_ground_is_solid_to_the_bottom_and_says_so() {
    let world = solid_ground(|x| x / 4, 1);
    for level in 0..4 {
        let s = 1 << level;
        // Deep under it, across it, over it.
        for (y, whole) in [(-4096, Some(Whole::Buried)), (-16, None), (1024, Some(Whole::Empty))] {
            let b = Box3 { min: [0, y / s, 0], max: [64, y / s + 32, 16] };
            assert_eq!(super::whole(&world, &b, level), whole, "level {level}, at {y}");
            let (grid, broken) = checking_skipped(|| generate(&world, &b.widen(1), level));
            assert_eq!(broken, Vec::<String>::new());
            let sites =
                || (b.min[2]..b.max[2]).flat_map(|z| (b.min[1]..b.max[1]).flat_map(move |y| (b.min[0]..b.max[0]).map(move |x| [x, y, z])));
            let alpha = |[x, y, z]: [i64; 3]| grid.alpha(x, y, z);
            match whole {
                Some(Whole::Buried) => assert!(sites().filter(|&[x, y, z]| crate::grid::is_site(x, y, z)).all(|p| alpha(p) == ALPHA)),
                Some(Whole::Empty) => assert!(sites().all(|p| alpha(p) == 0)),
                None => {}
            }
            // Drawn as the field would, without asking it.
            assert_eq!(generate(&world, &b, level).alpha(0, b.min[1], 0), grid.alpha(0, b.min[1], 0));
        }
    }
}

#[test]
fn a_tile_holds_its_ground_by_its_surface_not_its_volume() {
    // Ground rising g sites a column: a column of bricks holds its surface and the ring of whole bricks under it,
    // some 2 + 4g, whatever the tile's size; the rock under them, some g·t/16 more, is held once.
    for g in [0, 1, 2, 4] {
        let height: fn(i64) -> i64 = match g {
            0 => |_| 0,
            1 => |x| x,
            2 => |x| 2 * x,
            _ => |x| 4 * x,
        };
        let world = solid_ground(height, g);
        for t in [64, 256] {
            let b = Box3 { min: [0, -64, 0], max: [t, g * t + 64, t] };
            let sites = (BRICK * BRICK * BRICK / 2) as usize;
            let column = |grid: VoxelGrid| (grid.cells.len() / sites) as f64 / (t * t / BRICK / BRICK) as f64;
            let held = column(generate_shown(&world, &b, 0));
            assert!(held <= 2.0 + 4.5 * g as f64, "rising {g} a column, a tile {t} sites a side holds {held} bricks a column");
        }
    }
}

#[test]
fn what_shows_of_a_box_is_all_but_its_whole_bottom_and_top() {
    // Ground from 0 to 16 high.
    let world = solid_ground(|x| x / 4, 1);
    let b = Box3 { min: [0, -509, 0], max: [64, 509, 16] };
    let part = shown(&world, &b, 0);
    assert!(part.min[1] < 0 && part.min[1] > -64 && part.max[1] > 16 && part.max[1] < 64, "{part:?}");
    assert_eq!((part.min[1] - b.min[1]) % BRICK, 0, "in bricks from the bottom");
    assert_eq!(whole(&world, &Box3 { max: [64, part.min[1], 16], ..b }, 0), Some(Whole::Buried));
    assert_eq!(whole(&world, &Box3 { min: [0, part.max[1], 0], ..b }, 0), Some(Whole::Empty));
    // A brick more of either is not.
    assert_eq!(whole(&world, &Box3 { max: [64, part.min[1] + BRICK, 16], ..b }, 0), None);
    assert_eq!(whole(&world, &Box3 { min: [0, part.max[1] - BRICK, 0], ..b }, 0), None);
    let buried = shown(&world, &Box3 { max: [64, -100, 16], ..b }, 0);
    assert_eq!(buried.min[1], buried.max[1], "all buried");
}

#[test]
#[should_panic(expected = "field ground: at column (0, 0), kind Some(0) doesn't reach up from the bottom to -6, 4 under its ground 0")]
fn refuses_a_field_not_under_its_ground_as_it_says() {
    let flat = Rc::new(Sloped::new(Rc::new(plan::Plan::new("flat", 1, |_, _| 0)), 0));
    let field = Field::columns("ground", 0.0, Op::Fill, |_, _, _| vec![Span::solid(-4, 1, 0)]).about(flat, 4, 1).under(0);
    generate(&layers(vec![Layer::Field(field)]), &Box3 { min: [0, -16, 0], max: [8, 16, 8] }, 0);
}

#[test]
fn checking_finds_a_field_not_under_its_ground_where_it_says() {
    // A hole at x = 12 the slope's rule never sees, sampled every 8: filled under the ground all the same.
    let holed = solid_ground(|x| if x == 12 { -200 } else { 0 }, 0);
    let b = Box3 { min: [0, -55, 0], max: [32, -45, 4] };
    let (_, broken) = checking_skipped(|| generate(&holed, &b, 2));
    assert!(
        broken.len() == 1 && broken[0].starts_with("ground isn't all kind 0 under its ground, as it says: at site [3, -55, 0]"),
        "{broken:?}"
    );
}

#[test]
#[should_panic(expected = "rocks 0,0,0 is anchored at [0.0, 0.0, 0.0], 100 units off the ground it stands on (Population::on)")]
fn refuses_a_thing_anchored_off_the_ground_it_stands_on() {
    // In its cell, but the ground is 100 units under it.
    let low = Rc::new(Sloped::new(Rc::new(plan::Plan::new("low", 1, |_, _| -100)), 0));
    let rocks = Population::new("rocks", 1.0, 16)
        .on(low)
        .anchor(|cell| (cell.centre == [0; 3]).then_some([0.0; 3]))
        .make(|p, _| Some(Node::of(vec![ball(p, 2.0, 0, SOLID, Op::Fill)])));
    generate(&layers(vec![Layer::Population(rocks)]), &Box3 { min: [-8, -100, -8], max: [8, 8, 8] }, 0);
}

#[test]
fn faces_on_a_plane_of_sites_stay_where_level_0_has_them() {
    // Ground up to y = 0 and boxes from y = 0 up, on a plane of sites at every level, each drawn alone.
    let ground = Layer::Field(Field {
        name: "ground",
        order: 0.0,
        op: Op::Fill,
        values: FieldValues::Spans { axis: 1, spans: Box::new(|_, _, _| vec![Span { from: -4096, to: 0, kind: 0, extinction: None }]) },
        about: None,
    });
    let n = 1 << 14;
    let block = |top: f64| {
        Layer::Population(Population {
            name: "block",
            order: 1.0,
            cell: n,
            reach: [0; 3],
            size: ANY_SIZE,
            anchor: Anchors::Cells(Box::new(|cell| (cell.centre == [0; 3]).then_some([0.0; 3]))),
            make: Box::new(move |_, _| Some(Node::of(vec![cuboid([-8192.0, 0.0, -8192.0], [8192.0, top, 8192.0], 0, SOLID, Op::Fill)]))),
            far: None,
            on: None,
        })
    };
    let kinds = vec![KindDef { name: "stone", kind_type: 1, colors: &["#808080"], walk_through: false, glows: false, medium: false }];
    let alone = |layer: Layer| Generator { kinds: kinds.clone(), seed: 1, layers: vec![layer] };
    let (ground, solid) = (alone(ground), alone(block(4096.0)));
    for level in 0..6 {
        let s = 1i64 << level;
        let b = Box3 { min: [0, -32 / s - 2, 0], max: [128, 32 / s + 2, 128] };
        let (below, above) = (generate(&ground, &b, level), generate(&solid, &b, level));
        let (mut tops, mut lines) = (0, 0);
        for x in 0..128 {
            for z in 0..128 {
                let sites = (b.min[1]..b.max[1]).filter(|y| (x + y + z) & 1 == 0);
                let (mut top, mut bottom) = (i64::MIN, i64::MAX);
                for y in sites {
                    let (g, h) = (below.get(x, y, z) != 0, above.get(x, y, z) != 0);
                    assert!(g != h, "level {level}: ({x}, {y}, {z}) is in {} of the two", g as u8 + h as u8);
                    if g { top = top.max(y) } else { bottom = bottom.min(y) }
                }
                assert_eq!(top + 2, bottom, "level {level}: the line at ({x}, {z}) is split once");
                // A site's voxel reaches a site's spacing along the line.
                tops += (top + 1) * s;
                lines += 1;
            }
        }
        // Level 0's ground is half a unit low; a site on the plane counts as in it with chance ½ − 1/(2s), so
        // the mean is within a few s / √lines of that, not (s − 1) / 2 lower.
        let mean = tops as f64 / lines as f64;
        assert!((mean + 0.5).abs() <= s as f64 / 16.0, "level {level}: the ground's top is at {mean} on average");
        // A slab one site thick keeps the whole plane: half of it and half the next would let rays through.
        let slab = generate(&alone(block(s as f64)), &b, level);
        assert!((0..128).all(|x| (0..128).all(|z| (x + z) & 1 != 0 || slab.get(x, 0, z) != 0)), "level {level}: the slab has holes");
    }
}
