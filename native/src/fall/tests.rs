use super::*;

/** Ground under y = 0; two posts of wood half a metre square and 6 m high, 5 m apart; leaves about the first's top. */
fn grove() -> worldgen::World {
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

/** Cuts `sites` of `world`, as a stroke does, and simulates what that sets moving till it rests: what each site changed holds then, and what was found. */
fn cut(world: &Arc<worldgen::World>, sites: Vec<[i64; 3]>) -> (FastMap<[i64; 3], Held>, String) {
    let before = world.edits.sites();
    world.edits.change(sites.iter().map(|&s| (s, Held::default())));
    let (mut changed, mut found) = (FastMap::default(), String::new());
    simulate(world.clone(), &Job { cut: sites, before }, &mut |told| match told {
        Told::Frame(_, changes) => changed.extend(changes),
        Told::Done(f) => found = f,
        Told::Start => {}
    });
    world.edits.change(changed.iter().map(|(&s, &h)| (s, h)));
    (changed, found)
}

/** What `world` holds at `site` now, edits and all. */
fn now(world: &Arc<worldgen::World>, site: [i64; 3]) -> Kind {
    Voxels::new(world, &world.edits.sites(), 0).kind(site)
}

fn layer(y: i64) -> impl Iterator<Item = [i64; 3]> {
    (0..16).map(move |i| [i % 4, y, i / 4])
}

#[test]
fn a_post_cut_through_drops_down_its_kerf_and_stands_its_leaves_with_it() {
    let world = Arc::new(grove());
    let (changed, found) = cut(&world, layer(8).collect());
    let column = |s: &[i64; 3]| (0..4).contains(&s[0]) && (0..4).contains(&s[2]);
    assert!((8..48).all(|y| (y == 47) != (now(&world, [1, y, 1]) == Kind::Bearing)), "a site lower, upright: {found}");
    assert!(changed.keys().any(|s| !column(s) && s[1] > 40), "its leaves moved with it");
    assert!(changed.keys().all(|s| s[0] < 20), "nothing of the other post");
}

#[test]
fn a_post_cut_twice_drops_both_kerfs() {
    let world = Arc::new(grove());
    let (_, found) = cut(&world, layer(8).chain(layer(24)).collect());
    assert!((8..48).all(|y| (y >= 46) != (now(&world, [1, y, 1]) == Kind::Bearing)), "{found}");
}

#[test]
fn a_post_nicked_stands() {
    let world = Arc::new(grove());
    let (changed, found) = cut(&world, vec![[0, 8, 0], [1, 8, 0]]);
    assert!(changed.is_empty(), "{found}");
}

/**
 * A tree of the forest, notched 0.75 m over its foot on the side it
 * leans to, then cut through under the notch, falls towards it, its
 * leaves with it. Cut flat through, its weight over its stump, it would
 * stand on it, as a tree does.
 */
#[test]
fn a_tree_of_the_forest_notched_and_cut_through_falls_towards_the_notch() {
    let world = Arc::new((worlds::by_name("forest").expect("forest").make)(12345));
    let mut voxels = Voxels::new(&world, &world.edits.sites(), 0);
    // The nearest trunk to the spawn: bearing 3 m high and more, with none half a metre to its side.
    let [x0, y0, z0] = world.spawn;
    let foot = (0..80)
        .flat_map(|r: i64| (-r..=r).flat_map(move |dx| (-r..=r).map(move |dz| [dx, dz])).filter(move |d| d[0].abs().max(d[1].abs()) == r))
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
    let (changed, found) = cut(&world, notch);
    assert!(changed.is_empty(), "notched, it stands: {found}");
    // Cut through under it, two sites high, as a stroke cuts at least.
    let (changed, found) = cut(&world, low.iter().flat_map(|&[x, y, z]| [[x, y, z], [x, y - 1, z]]).collect());
    // What bore and is gone, and what bears where nothing was: the tree as it stood, and as it lies.
    let (mut left, mut came) = (Vec::new(), Vec::new());
    for (&s, h) in &changed {
        match (voxels.kind(s), kind(h)) {
            (Kind::Bearing, Kind::Empty) => left.push(s),
            (Kind::Empty, Kind::Bearing) => came.push(s),
            _ => {}
        }
    }
    assert!(came.len() > 500, "{found}");
    assert!(changed.iter().any(|(_, h)| kind(h) == Kind::Loose), "its leaves with it: {found}");
    let top = |sites: &[[i64; 3]]| sites.iter().map(|s| s[1]).max();
    assert!(top(&came) < top(&left), "lower than it stood: {found}");
    let mean = |sites: &[[i64; 3]]| sites.iter().map(lean).sum::<f64>() / sites.len() as f64;
    assert!(mean(&came) > mean(&left) + 8.0, "towards the notch: {} to {}; {found}", mean(&left), mean(&came));
}
