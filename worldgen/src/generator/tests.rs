use super::*;
use crate::noise::fx;

#[test]
fn the_sites_around_a_point_keep_its_place() {
    // On a site, between two, in a tetrahedron's middle, in an octahedron's, and anywhere.
    for p in [[2.0, 1.0, -1.0], [0.5, 0.5, 0.0], [0.5, 0.5, 0.5], [1.0, 0.0, 0.0], [0.3, 1.9, -0.6], [-2.25, 0.75, 7.5]] {
        let around = shapes::around(p);
        assert!(
            around.iter().all(|(site, near)| (site[0] + site[1] + site[2]) & 1 == 0 && (0.0..=1.0).contains(near)),
            "{p:?}: {around:?}"
        );
        let centre: [f64; 3] = std::array::from_fn(|a| around.iter().map(|(site, near)| near * site[a] as f64).sum());
        let all: f64 = around.iter().map(|(_, near)| near).sum();
        assert!((all - 1.0).abs() < 1e-12 && (0..3).all(|a| (centre[a] - p[a]).abs() < 1e-12), "{p:?}: {around:?}");
    }
    assert!(shapes::around([2.0, 1.0, -1.0]).contains(&([2, 1, -1], 1.0)), "all of it on the site it is at");
}

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
    let block = || Node {
        far: Some(Far::measured(fx(2.0))),
        ..Node::of(vec![cuboid([0.0; 3], [64.0; 3], 1, SOLID, Op::Fill), cuboid([0.0, 56.0, 0.0], [64.0, 64.0, 64.0], 2, SOLID, Op::Paint)])
    };
    let one = block();
    let sites = measure(&one, "block", "blocks", bounds_of(&one), fixed(2.0), 16, 1, &Measured::default());
    assert!(
        (1..4).all(|y| (1..4).all(|x| (1..4).all(|z| (x + y + z) % 2 == 1 || sites.iter().any(|s| s.0 == [x, y, z])))),
        "its inside is all drawn"
    );
    // Rays straight down see the glass on its top row of cells, half covered, through the faces towards them;
    // where that is dropped, the cells under it, whose faces up the columns from above credit with the glass
    // too. Rays from the sides see the block through the faces towards them, but in the top row, where the
    // glass shows at their tops. Each block picks its cells' kinds anew.
    let seen = |sites: &Sites, ray: [i64; 3]| {
        let looks: std::collections::HashMap<[i64; 3], Faces<Kind>> = sites.iter().map(|s| (s.0, s.2)).collect();
        // The faces a ray along `ray` may enter by: those facing back along it.
        let faces: Vec<usize> = (0..FACE_KINDS).filter(|&k| (0..3).map(|c| VOXEL_FACES[k][c] as i64 * ray[c]).sum::<i64>() < 0).collect();
        let (mut glass, mut hits) = (0, 0);
        for u in (2..64).step_by(4) {
            for v in (2..64).step_by(4) {
                // From above, or from −x, at the middle height.
                let along = |t: i64| if ray[1] < 0 { [u, 127 - t, v] } else { [t - 64, 32 + (u % 16) - 8, v] };
                let Some(look) = (0..192).find_map(|t| looks.get(&nearest_site(along(t).map(|v| v * ONE), 16))) else { continue };
                hits += faces.len();
                glass += faces.iter().filter(|&&k| look.get(k) == 2).count();
            }
        }
        glass as f64 / hits as f64
    };
    let blocks: Vec<Sites> = on_threads(16, |i| {
        let block = block();
        measure(&block, &format!("block {i}"), "blocks", bounds_of(&block), fixed(2.0), 16, 1, &Measured::default())
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
        let sites = measure(&cloud, &format!("cloud {i}"), "clouds", bounds_of(&cloud), fixed(2.0), 16, 1, &Measured::default());
        sites.iter().map(|s| s.4 as f64 / SHARE as f64).sum()
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
fn a_sheet_is_its_middle_on_the_planes_of_sites_within_half_a_site() {
    // A disc and a plate 2 thick and 16 wide across each axis, at sites 4 apart, grown to a site thick as a sheet is
    // (draw_shape): on a plane of sites, near one, and midway between two.
    let s = 4;
    for (axis, c) in (0..3).flat_map(|axis| [8.0, 9.5, 11.0, 10.0].map(move |c| (axis, c))) {
        let along = |thick: f64, wide: f64| std::array::from_fn(|i| if i == axis { thick } else { wide });
        let disc = ellipsoid(along(c, 1.0), along(1.0, 8.0), 1, SOLID, Op::Fill);
        let plate = cuboid(along(c - 1.0, -7.0), along(c + 1.0, 9.0), 1, SOLID, Op::Fill);
        for sheet in [disc, plate] {
            let inside = sheet.inside(s * ONE, s);
            for site in (-3..4).flat_map(|u| (-3..4).flat_map(move |v| (-3..4).map(move |w| [u, v, w]))) {
                let p = site.map(|v| (v * s) as f64);
                let middle = std::array::from_fn(|i| if i == axis { c } else { p[i] });
                let held = (p[axis] - c).abs() <= s as f64 / 2.0 && sheet.holds(middle);
                assert_eq!(inside.holds(site.map(|v| v * s * ONE)), held, "across {axis}, at {c}: {site:?}");
            }
        }
    }
}

#[test]
fn a_gable_narrows_to_its_ridge() {
    // 8 wide across x, 4 high: at height h, the sites within 4 · (1 − h / 4) of x = 4.
    let shape = gable([0.0; 3], [8.0, 4.0, 6.0], 2, [0, 1, 0], 1, SOLID, Op::Fill);
    let inside = shape.inside(0, 1);
    let holds = |x: i64, y: i64| inside.holds([x, y, 2].map(|v| v * ONE));
    assert!(holds(0, 0) && holds(7, 0) && holds(4, 3) && holds(3, 2) && holds(6, 1));
    assert!(!holds(1, 3) && !holds(8, 0) && !holds(4, 4) && !holds(7, 2));
    // Standing along −x, its ridge along y, it holds the same points turned (off its ends, which a box rounds one way).
    let turned = gable([-4.0, 0.0, 0.0], [0.0, 6.0, 8.0], 1, [-1, 0, 0], 1, SOLID, Op::Fill);
    let inside = turned.inside(0, 1);
    for (x, y) in (-1..9).flat_map(|x| (1..4).map(move |y| (x, y))) {
        assert_eq!(inside.holds([-y, 2, x].map(|v| v * ONE)), holds(x, y), "({x}, {y})");
    }
}

/** A generator of `layers`, of one kind. */
fn layers(layers: Vec<Layer>) -> Generator {
    Generator {
        kinds: vec![KindDef { name: "stone", kind_type: 0, colors: &["#808080"], walk_through: false, glows: false, medium: false }],
        seed: 1,
        layers,
    }
}

/**
 * Stone up along `up` to `height(r)`, r along the first axis across it
 * (x for y), which it never falls along, solid to the bottom: saying where
 * a box is all of it or none (Field::meets).
 */
fn solid_ground(up: usize, height: fn(i64) -> i64) -> Generator {
    let rise = across(up)[0];
    let field = Field::spans("ground", 0.0, Op::Fill, up, move |u, _| vec![Span::solid(DEEP, height(u) + 1, 0)]);
    let field = field.meets(move |b, _| {
        if b.min[up] > height(b.max[rise] - 1) {
            Meets::Nothing
        } else if b.max[up] <= height(b.min[rise]) {
            Meets::All(0)
        } else {
            Meets::Any
        }
    });
    layers(vec![Layer::Field(field)])
}

/** Point (x, y, z) of a ground standing along y, turned to stand along `up`. */
fn standing(up: usize, [x, y, z]: [i64; 3]) -> [i64; 3] {
    turned(up, [x, z], y)
}

/** Box `b` of a ground standing along y, turned as `standing`. */
fn standing_box(up: usize, b: Box3) -> Box3 {
    Box3 { min: standing(up, b.min), max: standing(up, b.max) }
}

/**
 * Solid where x < 0, as spans along x or, `at` each site, by x alone,
 * saying so if it `says`: nothing from x = 0 on, all solid under x = -8.
 */
fn wall(at: bool, says: bool) -> Generator {
    let field = if at {
        Field::at("wall", 0.0, Op::Fill, |x, _, _| (x < 0).then_some((0, None)))
    } else {
        Field::spans("wall", 0.0, Op::Fill, 0, |_, _| vec![Span::solid(-(1 << 20), 0, 0)])
    };
    let meets = |b: &Box3, _: i64| match b {
        _ if b.min[0] >= 0 => Meets::Nothing,
        _ if b.max[0] <= -8 => Meets::All(0),
        _ => Meets::Any,
    };
    layers(vec![Layer::Field(if says { field.meets(meets) } else { field })])
}

#[test]
fn a_field_says_where_it_is_along_any_axis() {
    for at in [false, true] {
        let world = wall(at, true);
        for level in 0..4 {
            let s = 1 << level;
            let box_at = |x: i64| Box3 { min: [x / s, -16, -16], max: [x / s + 32, 16, 16] };
            let (past, across, behind) = (box_at(256), box_at(-16 * s), box_at(-512));
            assert_eq!(whole(&world, &past, level), Some(Whole::Empty), "level {level}");
            assert_eq!(whole(&world, &across, level), None, "level {level}");
            assert_eq!(whole(&world, &behind, level), Some(Whole::Buried(0)), "level {level}");
            for b in [past, across, behind] {
                // Drawn as it would be without asking it.
                let (grid, broken) = checking_skipped(|| generate(&world, &b, level));
                assert_eq!(broken, Vec::<String>::new());
                assert_eq!(grid.pieces.alpha, generate(&wall(at, false), &b, level).pieces.alpha, "level {level}, {b:?}");
            }
            // All buried, it is made as one brick, every site of it whole.
            let buried = generate_shown(&world, &behind, level, 1);
            assert_eq!(buried.sites(), (BRICK * BRICK * BRICK / 2) as usize);
            assert!(buried.bricks.iter().all(|&at| at == 0) && buried.pieces.alpha.iter().all(|&a| a == ALPHA));
        }
    }
}

#[test]
fn a_field_at_each_site_is_drawn_only_where_it_says_and_held_to_it() {
    // Solid where x < 0, saying the opposite: nothing there, all solid from x = 0 on (a box is asked two sites wider).
    let meets = |b: &Box3, _: i64| match b {
        _ if b.max[0] <= 2 => Meets::Nothing,
        _ if b.min[0] >= -2 => Meets::All(0),
        _ => Meets::Any,
    };
    let world = layers(vec![Layer::Field(Field::at("wall", 0.0, Op::Fill, |x, _, _| (x < 0).then_some((0, None))).meets(meets))]);
    let (behind, past) = (Box3 { min: [-32, -4, -4], max: [0, 4, 4] }, Box3 { min: [0, -4, -4], max: [32, 4, 4] });
    assert!(generate(&world, &behind, 0).pieces.alpha.iter().all(|&a| a == 0), "skipped where it says nothing");
    assert!(generate(&world, &past, 0).pieces.alpha.contains(&ALPHA), "filled where it says all, not asked");
    let (_, broken) = checking_skipped(|| generate(&world, &behind, 0));
    assert!(broken.len() == 1 && broken[0].starts_with("wall draws where it says it may not"), "{broken:?}");
    let (_, broken) = checking_skipped(|| generate(&world, &past, 0));
    assert!(broken.len() == 1 && broken[0].starts_with("wall isn't all kind 0, as it says"), "{broken:?}");
}

#[test]
fn a_tile_holds_its_ground_by_its_surface_not_its_volume() {
    // Ground rising g sites a column: a column of bricks holds its surface and what its block of columns rises by,
    // some 2 + 2g, whatever the tile's size; the rock under them, some g·t/16 more, is held once, a brick.
    for g in [0, 1, 2, 4] {
        let height: fn(i64) -> i64 = match g {
            0 => |_| 0,
            1 => |x| x,
            2 => |x| 2 * x,
            _ => |x| 4 * x,
        };
        for (up, t) in (0..3).flat_map(|up| [64, 256].map(|t| (up, t))) {
            let world = solid_ground(up, height);
            let b = standing_box(up, Box3 { min: [0, -64, 0], max: [t, g * t + 64, t] });
            let sites = (BRICK * BRICK * BRICK / 2) as usize;
            let column = |grid: VoxelGrid| (grid.sites() / sites - 1) as f64 / (t * t / BRICK / BRICK) as f64;
            let held = column(generate_shown(&world, &b, 0, up));
            assert!(held <= 2.5 + 2.0 * g as f64, "up {up}, rising {g} a column, a tile {t} sites a side holds {held} bricks a column");
        }
    }
}

#[test]
fn what_hosts_make_of_a_box_differs_from_all_of_it_only_in_buried_rock() {
    // Ground rising two sites a column along x: no layer of the box's bricks is buried, its blocks of columns are.
    // Holes in it and rocks all about, a field's shapes and things, are asked once for all the parts hosts ask about.
    for (up, level) in (0..3).flat_map(|up| (0..3).map(move |level| (up, level))) {
        let mut world = solid_ground(up, |x| 2 * x);
        world.layers.extend([Layer::Field(holes()), Layer::Population(rocks())]);
        let b = standing_box(up, Box3 { min: [0, -64, 0], max: [128, 320, 64] });
        let (all, made) = (generate(&world, &b, level), generate_shown(&world, &b, level, up));
        let whole = |grid: &VoxelGrid, [x, y, z]: [i64; 3]| grid.alpha(x, y, z) == ALPHA;
        let sites = (b.min[2]..b.max[2]).flat_map(|z| (b.min[1]..b.max[1]).flat_map(move |y| (b.min[0]..b.max[0]).map(move |x| [x, y, z])));
        for [x, y, z] in sites.filter(|&[x, y, z]| crate::grid::is_site(x, y, z)) {
            let light = |grid: &VoxelGrid| std::array::from_fn::<_, FACE_KINDS, _>(|k| grid.light(x, y, z, k));
            let look = |grid: &VoxelGrid| (grid.get(x, y, z), grid.solid(x, y, z), light(grid));
            let around = VOXEL_FACES.map(|[i, j, k]| [x + i as i64, y + j as i64, z + k as i64]);
            // Rock no ray reaches: whole, as is every site around it.
            let buried = whole(&made, [x, y, z]) && whole(&all, [x, y, z]) && around.iter().all(|&p| !b.holds(p) || whole(&all, p));
            assert!(look(&made) == look(&all) || buried, "up {up}, level {level}: site ({x}, {y}, {z})");
        }
    }
}

#[test]
fn what_shows_of_a_box_is_all_but_its_whole_bottom_and_top() {
    for up in 0..3 {
        // Ground from 0 to 16 high.
        let world = solid_ground(up, |x| x / 4);
        let ground = |min: [i64; 3], max: [i64; 3]| standing_box(up, Box3 { min, max });
        let b = ground([0, -509, 0], [64, 509, 16]);
        let part = shown(&world, &b, 0, up);
        assert!(part.min[up] < 0 && part.min[up] > -64 && part.max[up] > 16 && part.max[up] < 64, "up {up}: {part:?}");
        assert_eq!((part.min[up] - b.min[up]) % BRICK, 0, "in bricks from the bottom");
        assert_eq!(whole(&world, &ground([0, -509, 0], [64, part.min[up], 16]), 0), Some(Whole::Buried(0)));
        assert_eq!(whole(&world, &ground([0, part.max[up], 0], [64, 509, 16]), 0), Some(Whole::Empty));
        // A brick more of either is not.
        assert_eq!(whole(&world, &ground([0, -509, 0], [64, part.min[up] + BRICK, 16]), 0), None);
        assert_eq!(whole(&world, &ground([0, part.max[up] - BRICK, 0], [64, 509, 16]), 0), None);
        let buried = shown(&world, &ground([0, -509, 0], [64, -100, 16]), 0, up);
        assert_eq!(buried.min[up], buried.max[up], "all buried");
        // Hosts keep its whole bottom as rock, not a hollow rays from a finer level's tunnels would see through.
        let grid = generate_shown(&world, &b, 0, up);
        let [x, y, z] = standing(up, [1, b.min[up], 0]);
        // Its rows start on an even x, so along x it may reach a site further.
        assert!(grid.clip().min[up] <= b.min[up] && grid.alpha(x, y, z) == ALPHA, "up {up}");
    }
}

/** Balls carving the world at the sites of 32·FCC: a field of shapes. */
fn holes() -> Field {
    let near = |b: &Box3| lattice(32, &b.widen(6)).into_iter().map(|p| ball(p.map(|v| v as f64), 5.0, 0, SOLID, Op::Carve));
    Field::shapes("holes", 0.5, Op::Carve, move |b| near(b).collect::<Vec<_>>())
}

/** The seed of the rocks' sizes. */
const SIZES: u64 = 7;

/** Two rocks at each site of 16·FCC, one a little apart in its random sequence. */
fn rocks() -> Population {
    let at = |b: &Box3| lattice(16, b).into_iter().flat_map(|p| [0, 1].map(|n| (p.map(|v| v as f64), n))).collect();
    Population::new("rocks", 1.0, 16).reach(3).at(at, |&n, anchor| {
        let r = 1.0 + n as f64 + anchor.random(SIZES).next();
        Some(Node::of(vec![ball(anchor.at, r, 0, SOLID, Op::Fill)]))
    })
}

#[test]
fn a_thing_is_known_by_its_anchor_and_number_there_whichever_box_asks() {
    let rocks = rocks();
    let made = |b: Box3| {
        let mut made = Vec::new();
        each_made(&rocks, &b, |name, bounds, _, _| made.push((name, bounds)));
        made.sort_by(|a, b| a.0.cmp(&b.0));
        made
    };
    let whole = made(Box3 { min: [-40; 3], max: [40; 3] });
    let mut halves = [[-40, 0], [0, 40]].map(|[x0, x1]| made(Box3 { min: [x0, -40, -40], max: [x1, 40, 40] })).concat();
    halves.sort_by(|a, b| a.0.cmp(&b.0));
    halves.dedup();
    assert!(whole.len() > 100 && whole == halves);
    // Each point's two, each its own.
    let at_origin: Vec<_> = whole.iter().filter(|(name, _)| name.starts_with("rocks 0,0,0")).collect();
    assert_eq!(at_origin.iter().map(|(name, _)| name.as_str()).collect::<Vec<_>>(), ["rocks 0,0,0", "rocks 0,0,0 1"]);
    assert_ne!(at_origin[0].1, at_origin[1].1);
}

#[test]
fn a_spans_face_is_where_it_is_at_every_level() {
    // Ground up to y = h, on a plane of sites or off it. Along the plane, a line's sites stop as much of their rays as
    // ground up to h holds of their voxels, a site standing for two spacings of its line: on average over the lines.
    let kinds = vec![KindDef { name: "stone", kind_type: 1, colors: &["#808080"], walk_through: false, glows: false, medium: false }];
    let along = (0..FACE_KINDS).find(|&k| VOXEL_FACES[k][1] == 0).expect("a face along the plane");
    for h in [0, 3, -7] {
        let ground = Layer::Field(Field {
            name: "ground",
            order: 0.0,
            op: Op::Fill,
            values: FieldValues::Spans { axis: 1, spans: Box::new(move |_, _| vec![Span::solid(-4096, h, 0)]) },
            meets: None,
        });
        let world = Generator { kinds: kinds.clone(), seed: 1, layers: vec![ground] };
        for level in 0..6 {
            let s = 1i64 << level;
            let b = Box3 { min: [0, -32 / s - 2, 0], max: [32, 32 / s + 2, 32] };
            let grid = generate(&world, &b, level);
            let stopped = |x: i64, y: i64, z: i64| grid.light(x, y, z, along).0.0 as f64 / ALPHA as f64;
            let lines = (0..32).flat_map(|x| (0..32).map(move |z| (x, z)));
            let tops = lines.map(|(x, z)| {
                let sites = (b.min[1]..b.max[1]).filter(|y| (x + y + z) & 1 == 0);
                // The lowest site's voxel starts a spacing under it.
                let bottom = (b.min[1] + ((b.min[1] + x + z) & 1) - 1) * s;
                bottom as f64 + sites.map(|y| stopped(x, y, z)).sum::<f64>() * 2.0 * s as f64
            });
            let mean = tops.sum::<f64>() / (32.0 * 32.0);
            assert!((mean - h as f64).abs() <= s as f64 / 16.0, "level {level}: ground up to {h} has its top at {mean} on average");
        }
    }
}

#[test]
fn slabs_of_a_voxel_stop_the_chords_that_meet_them() {
    let near = |a: f64, b: f64| (a - b).abs() < 1e-3;
    for across in [true, false] {
        // All of it stops all; its halves, by symmetry, alike; a carve of both halves frees what lies wholly in either.
        assert!(near(slabs(across, [(-1.0, 1.0)], false), 1.0));
        assert!(near(slabs(across, [(-1.0, 0.0)], false), slabs(across, [(0.0, 1.0)], false)));
        assert!(near(slabs(across, [(-1.0, 0.0), (0.0, 1.0)], true), 2.0 * slabs(across, [(-1.0, 0.0)], true)));
    }
    // Chords along a plane across the axis lie at one height each, half of them below the middle; those crossing such
    // planes span heights, so a thin slab across the middle stops most of them and few of the others.
    assert!(near(slabs(false, [(-1.0, 0.0)], true), 0.5));
    let thin = |across| slabs(across, [(-0.1, 0.1)], false);
    assert!(thin(true) > 0.5 && thin(false) < 0.2, "{} across, {} along", thin(true), thin(false));
}

#[test]
fn slabs_that_share_a_voxel_out_share_out_its_mean_chord() {
    // However it is cut across an axis, what its rays along the axis run in its slabs adds up to their whole way:
    // so a medium is as dense along a ray whatever sites its span is cut by.
    for cuts in [vec![-1.0, 1.0], vec![-1.0, 0.0, 1.0], vec![-1.0, -0.9, -0.3, 0.05, 0.6, 1.0]] {
        let run: f64 = cuts.windows(2).map(|cut| crossed(cut[0], cut[1])).map(|(share, length)| share * length).sum();
        assert!((run - 1.0).abs() < 1e-9, "{cuts:?}: {run}");
    }
    // A thin slab is crossed by all of them across the middle, where each runs its height in it, and by few at a tip.
    let ((middle, height), (tip, _)) = (crossed(-0.05, 0.05), crossed(0.9, 1.0));
    assert!((middle - 1.0).abs() < 1e-9 && (height - 0.1).abs() < 0.01 && tip < 0.05, "{middle} {height} {tip}");
}
