//! A component of a world alone (`Generator::alone`), to check or look at
//! by itself: a layer, or its things' parts down children named as the
//! survey names them (`towns/block/house`).

use super::*;

impl Generator {
    /**
     * The world of its `component` alone: the layer named first, or past it
     * its things' parts down the children named, numbers left out. All
     * they read (plans, the ground, claims) stays, so each stands where it
     * would. Its parents keep none of their own shapes, draw it at every
     * spacing, and their far forms are measured from it. Cutting it again
     * changes nothing. Panics naming the layers if none is named first.
     */
    pub fn alone(self, component: &str) -> Generator {
        let mut names = component.split('/').map(str::trim);
        let first = names.next().unwrap_or_default();
        let path: Arc<[String]> = names.map(String::from).collect();
        let layers: Vec<&str> = self.layers.iter().map(Layer::name).collect();
        let Some(mut layer) = self.layers.into_iter().find(|l| l.name() == first) else {
            panic!("no layer {first} to draw alone; the layers are {}", layers.join(", "));
        };
        if !path.is_empty() {
            let Layer::Population(population) = &mut layer else { panic!("{first} is a field: it has no parts to draw alone") };
            let make = std::mem::replace(&mut population.make, Box::new(|_, _| None));
            population.make = Box::new(move |anchor, cell| part(make(anchor, cell)?, &path));
            (population.size, population.far) = (ANY_SIZE, None);
        }
        Generator { layers: vec![layer], ..self }
    }
}

/**
 * `node` cut down to the parts `path` names, if any: their parents' own
 * shapes left out, their far forms measured from what is left, the
 * children named drawn at any spacing (`Generator::alone`). None if
 * `path` names none of its children, or it has none.
 */
fn part(node: Node, path: &Arc<[String]>) -> Option<Node> {
    let Some(name) = path.first().cloned() else { return Some(node) };
    let rest: Arc<[String]> = path[1..].into();
    let keep = move |children: Vec<Child>| -> Vec<Child> {
        children
            .into_iter()
            .filter(|c| unnumbered(&c.name) == name)
            .filter_map(|c| Some(Child { node: part(c.node, &rest)?, size: ANY_SIZE, name: c.name }))
            .collect()
    };
    let children = match node.children? {
        Children::Fixed(children) => Some(keep(children)).filter(|kept| !kept.is_empty()).map(Children::Fixed)?,
        // Those reaching a box are made only for it: what isn't named is left out of each.
        Children::Reaching(reaching) => Children::Reaching(Box::new(move |b| keep(reaching(b)))),
    };
    let far = node.far.map(|far| Far::measured(far.detail.max(Fx::ONE)));
    Some(Node { shapes: Shapes::Now(Vec::new()), bounds: node.bounds, children: Some(children), far })
}

/** A part's name without its numbers, as components are named: "block 3 1 2" is a block. */
pub(crate) fn unnumbered(name: &str) -> String {
    let words: Vec<&str> = name.split_whitespace().filter(|w| !w.chars().any(|c| c.is_ascii_digit())).collect();
    if words.is_empty() { "?".into() } else { words.join(" ") }
}
