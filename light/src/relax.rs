//! Light kept settled site by site as the world and its brightness change:
//! a prototype beside Light's rounds (docs/light.md, "Settling as it changes").
//!
//! Light is linear: what comes to a site is what the sky and what glows
//! send along its lines, the coarser sites' light past them (Line), and what
//! the sites met reflect (Reflects), each a share the world alone sets. So
//! each site finds its light from its neighbours' as they are now
//! (`update`), and again whenever one of them changes by more than the
//! tolerance. Less light spreads as more does, and as nothing is added up,
//! no error is carried on. An edit finds again the lines that read what
//! changed (`edited`); a brightness, the sites the sky or a glow lights
//! (`under`). Sites are found a budget at a time (`settle`): what is lit in
//! between may be shown. Their lines are found first, the nearest first,
//! a batch at a time, on as many threads as the host has (Finder): each
//! is the world's alone.
//!
//! A site is followed as far as its light reaches one asked for by more
//! than the tolerance: each reflection halves it as often as its share
//! does (`deeper`), the coarser sites past a line as often as what gets
//! through it: one light, read between them. A site only ever comes
//! nearer, till a sweep finds each depth anew from those asked for, and
//! forgets what none reaches (`sweep`): what was let go, and what an edit
//! closed off.

use std::collections::VecDeque;
use std::sync::Arc;

use worldgen::World;
use worldgen::grid::Box3;

use super::{Beyond, Brightness, CUBE, F, Light, Line, Map, N, Packed, Reflects, Rgb, Shared, TILE_SHIFT, Touches, pack, sky, unpack};

/** A site of a level. */
type Key = (usize, [i64; 3]);
type Id = u32;

/** The brightness the lines are found under: what glows along them is then a glow of 1's. */
const UNIT: Brightness = Brightness { sky: 1.0, sun: 1.0, glow: 1.0 };
/** A change smaller than this is none, however little light there is: so the dark settles too. */
const FLOOR: f32 = 1e-6;
/** The cube a site's lines are watched by (`watching`): a change is found by cube, as tiles are made again by tile. */
const WATCH: u32 = CUBE.trailing_zeros();

/**
 * A site whose light is kept settled: what it is found from, its lines
 * (Line), and as found last. Light is kept packed, as Light's rounds keep
 * it: a site keeps many, and readers many sites'.
 */
#[derive(Default)]
struct Site {
    key: Key,
    /** Whether a host asked for it (`ask`), and has not let it go since (`forget`). */
    asked: bool,
    /** How many times its light is halved on its way to one asked for, at least (`ways`); u8::MAX till reached. */
    depth: u8,
    /** Whether its lines are to be found, as of its depth. */
    due: bool,
    /** Whether its light was found since it was begun. */
    found: bool,
    queued: bool,
    /** Per direction, what glows along it, at a glow of 1, if anything. */
    glow: Option<Box<[Packed; N]>>,
    /** Per direction, how much of what comes from past its line gets through: the sky's light, where its bit in `skies` is set, else the coarser sites'. */
    through: [Packed; N],
    skies: u32,
    /** The coarser sites past its lines: the direction, the site, and its share of what comes from past there. */
    coarser: Box<[(u8, Id, f32)]>,
    /** The sites met that reflect: the direction, the face seen, the site, and the share of the light on it that comes. */
    reflects: Box<[(u8, u8, Id, Packed)]>,
    touches: Touches,
    /** How many times its lines were found: `watching` keeps what each time read. */
    built: u32,
    /** Its light from each direction, and on each face. */
    light: [Packed; N],
    on: [Packed; F],
    /** The sites that read its light, as often as they do. */
    readers: Vec<Id>,
}

/** A world's light, kept settled as it changes (the module's header). */
pub struct Settling {
    /** What the lines are found with, under UNIT. */
    light: Light,
    brightness: Brightness,
    /** The sky's light per direction, under `brightness`. */
    sky: [Rgb; N],
    tolerance: f32,
    /** The depth past which a site reflects nothing: its light is halved past the tolerance. */
    deepest: u8,
    sites: Vec<Site>,
    ids: Map<Key, Id>,
    /** By the cube of the world's sites, the sites whose lines read it, as they were found (`built`): those found since may not. */
    watching: Map<Key, Vec<(Id, u32)>>,
    /** By depth, the sites whose lines are to be found (`due`), and some found since at a lesser depth. */
    unbuilt: Vec<Vec<Id>>,
    /** The sites whose light is to be found. */
    queue: VecDeque<Id>,
    /** The sites whose light changed, since last taken (`changed`). */
    changed: Vec<Key>,
    found: usize,
    /** How many sites it kept after the last `sweep`, and whether any was let go since. */
    kept: usize,
    let_go: bool,
}

/** A Light to find lines with (Lines::find), on a thread of its own: it shares its Settling's tiles. */
pub struct Finder(Light);

/** The lines of a site, to find on any thread (`find`) as `settle` asks. */
pub struct Lines {
    id: Id,
    key: Key,
    found: Option<Box<Found>>,
}

/** A site's lines as found: what comes along them, what it touches, the sites met that reflect, and the cubes of the world read. */
struct Found {
    lines: [Line; N],
    touches: Touches,
    met: Vec<Reflects>,
    reads: Vec<Key>,
}

impl Lines {
    /** Finds them as the world is now, with `finder`, of the Settling that asks. */
    pub fn find(&mut self, finder: &mut Finder) {
        self.found = Some(Box::new(find(&mut finder.0, self.key)));
    }
}

impl Settling {
    /**
     * `world` under `shared`'s brightness, settling to its tolerance and
     * reading its tiles (made once for its Lights and this), lit nowhere yet.
     */
    pub fn new(world: impl Into<Arc<World>>, shared: Shared) -> Settling {
        let (brightness, tolerance) = (shared.brightness(), shared.tolerance());
        // What a Light finds of its rounds is under its brightness: none of that is read here.
        let light = Light::new(world, Shared { brightness: UNIT, ..shared });
        let sky = sky(&light.world, &light.unit, brightness).0;
        let deepest = halvings(tolerance);
        Settling {
            light,
            brightness,
            sky,
            tolerance,
            deepest,
            sites: Vec::new(),
            ids: Map::default(),
            watching: Map::default(),
            unbuilt: vec![Vec::new(); deepest as usize + 1],
            queue: VecDeque::new(),
            changed: Vec::new(),
            found: 0,
            kept: 0,
            let_go: false,
        }
    }

    /** Asks for the light of the voxel at `site` of `level` (`on_seen`), if light gets out of it, till it is let go (`forget`): its lines are found as the light is (`settle`). */
    pub fn ask(&mut self, level: u32, site: [i64; 3]) {
        if self.light.lets_out(level as usize, site) {
            self.asked((level as usize, site));
        }
    }

    /** Lets go of the voxel at `site` of `level`: what it alone reaches, others asked for not, is forgotten (`sweep`). */
    pub fn forget(&mut self, level: u32, site: [i64; 3]) {
        if let Some(&id) = self.ids.get(&(level as usize, site)) {
            self.let_go |= std::mem::take(&mut self.sites[id as usize].asked);
        }
    }

    /** What lights the voxel at `site` of `level` (Light::on_seen), as found so far, if light gets out of it; asked for, if not yet. */
    pub fn on_seen(&mut self, level: u32, site: [i64; 3]) -> Option<Rgb> {
        let i = level as usize;
        if !self.light.lets_out(i, site) {
            return None;
        }
        let id = self.asked((i, site));
        self.build(|lines| lines);
        let found = &self.sites[id as usize];
        let (light, touches) = (found.light.map(unpack), found.touches.clone());
        Some(self.light.on_surface(i, site, &light, touches.as_deref()))
    }

    /** What the voxel at `site` of `level` glows, under the brightness. */
    pub fn glow(&mut self, level: u32, site: [i64; 3]) -> Rgb {
        self.light.glow(level, site).map(|g| g * self.brightness.glow)
    }

    /** Whether the light of the voxel at `site` of `level` was found since it was asked for, if it was. */
    pub fn found_at(&self, level: u32, site: [i64; 3]) -> bool {
        self.ids.get(&(level as usize, site)).is_none_or(|&id| self.sites[id as usize].found)
    }

    /** A Finder of its lines, for `settle`'s `find`. */
    pub fn finder(&self) -> Finder {
        Finder(Light::new(self.light.world.clone(), self.light.shared.clone()))
    }

    /**
     * Forgets what no site asked for reaches, once some was let go and it
     * keeps a quarter more sites than after it last did (`sweep`, which
     * costs as much as all kept: four times what was begun since); finds
     * the lines due, the nearest first, each batch by `find`
     * (any of them it leaves, here); then the light of at most `most`
     * sites, each from its neighbours' as they are now. Returns how many
     * it found.
     */
    pub fn settle(&mut self, most: usize, find: impl FnMut(Vec<Lines>) -> Vec<Lines>) -> usize {
        if self.let_go && self.sites.len() * 4 >= self.kept * 5 {
            self.sweep();
        }
        self.build(find);
        let mut n = 0;
        while n < most {
            let Some(id) = self.queue.pop_front() else { break };
            self.sites[id as usize].queued = false;
            self.update(id);
            n += 1;
        }
        self.found += n;
        n
    }

    /** Whether no site's light is left to find again. */
    pub fn settled(&self) -> bool {
        self.queue.is_empty()
    }

    /** How little a change of light is that is followed (Shared's tolerance). */
    pub fn tolerance(&self) -> f32 {
        self.tolerance
    }

    /** How many sites it keeps. */
    pub fn sites(&self) -> usize {
        self.sites.len()
    }

    /** How many times a site's light was found, so far. */
    pub fn found(&self) -> usize {
        self.found
    }

    /** The sites, by level, whose light changed by more than the tolerance since this was last asked. */
    pub fn changed(&mut self) -> Vec<(u32, [i64; 3])> {
        let mut changed: Vec<_> = self.changed.drain(..).map(|(level, site)| (level as u32, site)).collect();
        changed.sort_unstable();
        changed.dedup();
        changed
    }

    /** The same world under `brightness`: the sites the sky or a glow lights are found again. */
    pub fn under(&mut self, brightness: Brightness) {
        self.brightness = brightness;
        self.sky = sky(&self.light.world, &self.light.unit, brightness).0;
        for id in 0..self.sites.len() as Id {
            let site = &self.sites[id as usize];
            if site.glow.is_some() || site.skies != 0 {
                self.queue(id);
            }
        }
    }

    /**
     * The world was edited within `b` (level-0 sites; worldgen's Edits):
     * the tiles there are made again, and the lines of the sites that read
     * them found again.
     */
    pub fn edited(&mut self, b: Box3) {
        let tiles = |level: usize, a: usize| (b.min[a] >> level >> TILE_SHIFT)..=((b.max[a] - 1) >> level >> TILE_SHIFT);
        drop(self.light.shared.edited(|level, key| (0..3).all(|a| tiles(level, a).contains(&key[a]))));
        let mut touched = Vec::new();
        for level in 0..self.light.levels.len() {
            let (lo, hi) = (b.min.map(|v| v >> level >> WATCH), b.max.map(|v| (v - 1) >> level >> WATCH));
            for x in lo[0]..=hi[0] {
                for y in lo[1]..=hi[1] {
                    for z in lo[2]..=hi[2] {
                        if let Some(watching) = self.watching.get_mut(&(level, [x, y, z])) {
                            watching.retain(|&(id, built)| self.sites[id as usize].built == built);
                            touched.extend(watching.iter().map(|&(id, _)| id));
                        }
                    }
                }
            }
        }
        touched.sort_unstable();
        touched.dedup();
        for id in touched {
            self.due(id, self.sites[id as usize].depth);
        }
    }

    /** The id of `key`, asked for. */
    fn asked(&mut self, key: Key) -> Id {
        let id = self.id(key);
        self.reach(id, 0);
        self.sites[id as usize].asked = true;
        id
    }

    /** The id of `key`: begun if new, reached by none yet. */
    fn id(&mut self, key: Key) -> Id {
        *self.ids.entry(key).or_insert_with(|| {
            self.sites.push(Site { key, depth: u8::MAX, ..Site::default() });
            self.sites.len() as Id - 1
        })
    }

    /**
     * Has site `id` reached at `depth` (none deeper than `deepest`): if
     * nearer than it was, so too what it reads (`ways`), or, if it has
     * its lines yet to find, or found as of `deepest`, without what they
     * meet that reflects, has them found as of that depth.
     */
    fn reach(&mut self, id: Id, depth: u8) {
        let mut reached = vec![(id, depth)];
        while let Some((id, depth)) = reached.pop() {
            let (depth, site) = (depth.min(self.deepest), &self.sites[id as usize]);
            if depth >= site.depth {
                continue;
            }
            if site.due || site.depth >= self.deepest {
                self.due(id, depth);
            } else {
                self.sites[id as usize].depth = depth;
                reached.extend(self.ways(id, depth));
            }
        }
    }

    /**
     * The sites site `id` reads the light of, each with the depth it is
     * reached at so, from `depth`: as many halvings deeper as of the
     * share of its light that comes. What it reflects, only short of
     * `deepest`; the coarser sites past a line, as often as what gets
     * through it: one light, read between them.
     */
    fn ways(&self, id: Id, depth: u8) -> Vec<(Id, u8)> {
        let site = &self.sites[id as usize];
        let coarser = site.coarser.iter().map(|&(d, q, _)| (q, deeper(depth, unpack(site.through[d as usize]))));
        let reflects = site.reflects.iter().filter(|_| depth < self.deepest).map(|&(_, _, q, share)| (q, deeper(depth, unpack(share))));
        coarser.chain(reflects).collect()
    }

    /** Has site `id`'s lines found again as of `depth`: whose light they read is forgotten till then. */
    fn due(&mut self, id: Id, depth: u8) {
        let site = &mut self.sites[id as usize];
        let (coarser, reflects) = (std::mem::take(&mut site.coarser), std::mem::take(&mut site.reflects));
        (site.depth, site.due) = (depth, true);
        for q in coarser.iter().map(|c| c.1).chain(reflects.iter().map(|r| r.2)) {
            self.sites[q as usize].readers.retain(|&r| r != id);
        }
        self.unbuilt[depth as usize].push(id);
    }

    /** Finds the lines due, the nearest first, each batch by `find` (any it leaves, here). */
    fn build(&mut self, mut find: impl FnMut(Vec<Lines>) -> Vec<Lines>) {
        loop {
            let due = self.nearest();
            if due.is_empty() {
                return;
            }
            self.lined(find(due));
        }
    }

    /** The lines due nearest, each once as of its depth: none once all are found. */
    fn nearest(&mut self) -> Vec<Lines> {
        for depth in 0..self.unbuilt.len() {
            let mut ids = std::mem::take(&mut self.unbuilt[depth]);
            ids.sort_unstable();
            ids.dedup();
            ids.retain(|&id| self.sites[id as usize].due && self.sites[id as usize].depth as usize == depth);
            if !ids.is_empty() {
                return ids.into_iter().map(|id| Lines { id, key: self.sites[id as usize].key, found: None }).collect();
            }
        }
        Vec::new()
    }

    /** Takes `lines` (`nearest`), found or found here: the sites they read and those whose light they read; and queues each. */
    fn lined(&mut self, lines: Vec<Lines>) {
        for Lines { id, key, found } in lines {
            let Found { lines, touches, met, reads } = found.map_or_else(|| find(&mut self.light, key), |found| *found);
            let ((level, p), depth) = (key, self.sites[id as usize].depth);
            let (mut glow, mut through, mut skies, mut coarser) = (None::<Box<[Packed; N]>>, [[0; 3]; N], 0, Vec::new());
            for (d, line) in lines.iter().enumerate() {
                if line.glow.iter().any(|&g| g > 0.0) {
                    glow.get_or_insert_with(|| Box::new([[0; 3]; N]))[d] = pack(line.glow);
                }
                through[d] = pack(line.through);
                match line.beyond {
                    Beyond::Nothing => {}
                    Beyond::Sky => skies |= 1 << d,
                    Beyond::Coarser { sites, total } => {
                        for (site, w) in sites.into_iter().filter(|&(_, w)| w > 0.0) {
                            coarser.push((d as u8, self.id((level + 1, site)), w / total));
                        }
                    }
                }
            }
            let mut reflects = Vec::new();
            if depth < self.deepest {
                for r in met {
                    let (q, d, face) = r.site(p);
                    reflects.push((d as u8, face as u8, self.id((level, q)), r.share));
                }
            }
            let built = self.sites[id as usize].built + 1;
            for key in reads {
                self.watching.entry(key).or_default().push((id, built));
            }
            for q in coarser.iter().map(|c| c.1).chain(reflects.iter().map(|r| r.2)) {
                self.sites[q as usize].readers.push(id);
            }
            let site = &mut self.sites[id as usize];
            (site.glow, site.through, site.skies, site.touches, site.due) = (glow, through, skies, touches, false);
            (site.coarser, site.reflects, site.built) = (coarser.into(), reflects.into(), built);
            for (q, depth) in self.ways(id, depth) {
                self.reach(q, depth);
            }
            self.queue(id);
        }
    }

    /**
     * Forgets the sites no site asked for reaches, and has each other at
     * its depth from those (`reached`): as if only these had ever been
     * asked for. As `reach` never finds a site deeper than it is, a site
     * only comes up: one come up to `deepest` lets go of what it reflects.
     */
    fn sweep(&mut self) {
        let depths = self.reached();
        // Each site's id among those kept, in the order they were begun; Id::MAX if forgotten.
        let mut kept = 0;
        let ids: Vec<Id> = depths
            .iter()
            .map(|&depth| {
                kept += Id::from(depth != u8::MAX);
                if depth == u8::MAX { Id::MAX } else { kept - 1 }
            })
            .collect();
        let (sites, mut again) = (std::mem::take(&mut self.sites), Vec::new());
        for (mut site, depth) in sites.into_iter().zip(depths).filter(|&(_, depth)| depth != u8::MAX) {
            debug_assert!(depth >= site.depth, "a site comes nearer only as reached");
            if depth == self.deepest && !site.reflects.is_empty() {
                site.reflects = Box::default();
                again.push(self.sites.len() as Id);
            }
            site.depth = depth;
            site.coarser.iter_mut().for_each(|c| c.1 = ids[c.1 as usize]);
            site.reflects.iter_mut().for_each(|r| r.2 = ids[r.2 as usize]);
            site.readers.clear();
            self.sites.push(site);
        }
        self.ids.clear();
        self.unbuilt.iter_mut().for_each(Vec::clear);
        for id in 0..self.sites.len() as Id {
            let site = &self.sites[id as usize];
            self.ids.insert(site.key, id);
            if site.due {
                self.unbuilt[site.depth as usize].push(id);
            }
            let read: Vec<Id> = site.coarser.iter().map(|c| c.1).chain(site.reflects.iter().map(|r| r.2)).collect();
            read.into_iter().for_each(|q| self.sites[q as usize].readers.push(id));
        }
        self.watching.retain(|_, watching| {
            watching.retain_mut(|(id, built)| {
                *id = ids[*id as usize];
                *id != Id::MAX && self.sites[*id as usize].built == *built
            });
            !watching.is_empty()
        });
        self.queue.retain_mut(|id| {
            *id = ids[*id as usize];
            *id != Id::MAX
        });
        // Its light found again, without what it reflected.
        again.into_iter().for_each(|id| self.queue(id));
        (self.kept, self.let_go) = (self.sites.len(), false);
    }

    /** Each site's depth from those asked for, by the lines as they are now (`ways`), the nearest first; u8::MAX where none reaches it. */
    fn reached(&self) -> Vec<u8> {
        let mut depths: Vec<u8> = self.sites.iter().map(|site| if site.asked { 0 } else { u8::MAX }).collect();
        let mut by_depth = vec![Vec::new(); self.deepest as usize + 1];
        by_depth[0] = (0..self.sites.len() as Id).filter(|&id| depths[id as usize] == 0).collect();
        for depth in 0..=self.deepest {
            while let Some(id) = by_depth[depth as usize].pop() {
                if depths[id as usize] != depth {
                    continue;
                }
                for (q, at) in self.ways(id, depth) {
                    let at = at.min(self.deepest);
                    if at < depths[q as usize] {
                        depths[q as usize] = at;
                        by_depth[at as usize].push(q);
                    }
                }
            }
        }
        depths
    }

    fn queue(&mut self, id: Id) {
        if !std::mem::replace(&mut self.sites[id as usize].queued, true) {
            self.queue.push_back(id);
        }
    }

    /** Finds site `id`'s light from what it reads as it is now; if it changed, those that read it are to find theirs again. */
    fn update(&mut self, id: Id) {
        let site = &self.sites[id as usize];
        // Each line's: what glows along it, and what gets through of what comes from past it.
        let mut past: [Rgb; N] = std::array::from_fn(|d| if site.skies >> d & 1 == 1 { self.sky[d] } else { [0.0; 3] });
        for &(d, q, share) in &site.coarser {
            let light = unpack(self.sites[q as usize].light[d as usize]);
            (0..3).for_each(|c| past[d as usize][c] += share * light[c]);
        }
        let glow = |d: usize| site.glow.as_ref().map_or([0.0; 3], |glow| unpack(glow[d]).map(|g| g * self.brightness.glow));
        let mut light: [Rgb; N] = std::array::from_fn(|d| {
            let (glow, through) = (glow(d), unpack(site.through[d]));
            std::array::from_fn(|c| glow[c] + through[c] * past[d][c])
        });
        for &(d, face, q, share) in &site.reflects {
            let (on, share) = (unpack(self.sites[q as usize].on[face as usize]), unpack(share));
            (0..3).for_each(|c| light[d as usize][c] += share[c] * on[c]);
        }
        let on = std::array::from_fn(|f| pack(self.light.on_face(&light, site.touches.as_deref(), f)));
        let changed = changed(&site.light.map(unpack), &light, self.tolerance);
        let site = &mut self.sites[id as usize];
        (site.light, site.on, site.found) = (light.map(pack), on, true);
        if changed {
            self.changed.push(site.key);
            let readers = std::mem::take(&mut site.readers);
            for &r in &readers {
                self.queue(r);
            }
            self.sites[id as usize].readers = readers;
        }
    }
}

/** The lines of the site `key` as the world is now (Light's `lines`), by `light`, sharing the Settling's tiles. */
fn find(light: &mut Light, (level, p): Key) -> Found {
    light.reads = Some(Vec::new());
    let mut work = light.work.pop().unwrap_or_default();
    let (lines, touches) = light.lines(level, p, &mut work);
    let met = work.reflects.clone();
    light.work.push(work);
    let mut reads: Vec<Key> = light.reads.take().expect("kept").into_iter().map(|(l, s)| (l, s.map(|v| v >> WATCH))).collect();
    reads.sort_unstable();
    reads.dedup();
    reads.shrink_to_fit();
    Found { lines, touches, met, reads }
}

/** How many times halving 1 takes to reach `share`, so little as it is. */
fn halvings(share: f32) -> u8 {
    (-share.log2()).ceil().clamp(0.0, u8::MAX as f32) as u8
}

/** The depth of a site whose light comes by `share`, by channel, to one of `depth`. */
fn deeper(depth: u8, share: Rgb) -> u8 {
    depth.saturating_add(halvings(share.into_iter().fold(0.0, f32::max)))
}

/** Whether any channel of `after` is off `before` by more than `tolerance` of the larger, and more than FLOOR. */
fn changed(before: &[Rgb; N], after: &[Rgb; N], tolerance: f32) -> bool {
    before.iter().flatten().zip(after.iter().flatten()).any(|(&b, &a)| {
        let off = (a - b).abs();
        off > FLOOR && off > tolerance * a.max(b)
    })
}
