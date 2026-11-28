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
//! between may be shown.
//!
//! A site is followed as far as its light reaches one asked for by more
//! than the tolerance: each reflection halves it as often as its share
//! does (`halvings`). Coarser sites past a line count as asked for, as the
//! rounds read them settled. A site only ever comes nearer: after an edit
//! closes a way, what was past it is still followed.

use std::collections::VecDeque;
use std::sync::Arc;

use worldgen::World;
use worldgen::grid::Box3;

use super::{Beyond, Brightness, CUBE, F, Light, Map, N, Rgb, Shared, TILE_SHIFT, Touches, sky, unpack};

/** A site of a level. */
type Key = (usize, [i64; 3]);
type Id = u32;

/** The brightness the lines are found under: what glows along them is then a glow of 1's. */
const UNIT: Brightness = Brightness { sky: 1.0, sun: 1.0, glow: 1.0 };
/** A change smaller than this is none, however little light there is: so the dark settles too. */
const FLOOR: f32 = 1e-6;
/** The cube a site's lines are watched by (`watching`): a change is found by cube, as tiles are made again by tile. */
const WATCH: u32 = CUBE.trailing_zeros();

/** A site whose light is kept settled: what it is found from, and as found last. */
#[derive(Default)]
struct Site {
    key: Key,
    /** How many times its light is halved on its way to one asked for, at least. */
    depth: u8,
    /** Whether its lines are to be found, as of its depth. */
    due: bool,
    /** Whether its light was found since it was begun. */
    found: bool,
    /** Per direction, what glows along it, at a glow of 1, if anything; and how much of the sky's light gets through. */
    glow: Option<Box<[Rgb; N]>>,
    sky: [Rgb; N],
    /** The coarser sites past its lines: the direction, the site, and how much of its light from there comes. */
    coarser: Vec<(u8, Id, Rgb)>,
    /** The sites met that reflect: the direction, the site, the face seen, and the share of the light on it that comes. */
    reflects: Vec<(u8, Id, u8, Rgb)>,
    touches: Touches,
    /** The cubes of the world's sites its lines read, each once: it is found from them again when they change. */
    reads: Vec<Key>,
    /** Its light from each direction, and on each face. */
    light: [Rgb; N],
    on: [Rgb; F],
    /** The sites that read its light, as often as they do. */
    readers: Vec<Id>,
    queued: bool,
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
    /** By the cube of the world's sites, the sites whose lines read it. */
    watching: Map<Key, Vec<Id>>,
    /** By depth, the sites whose lines are to be found (`due`), and some found since at a lesser depth. */
    unbuilt: Vec<Vec<Id>>,
    /** The sites whose light is to be found. */
    queue: VecDeque<Id>,
    /** The sites whose light changed, since last taken (`changed`). */
    changed: Vec<Key>,
    found: usize,
}

impl Settling {
    /**
     * `world` under `shared`'s brightness, taking its steps, settling to
     * its tolerance and reading its tiles (made once for its Lights and
     * this), lit nowhere yet.
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
        }
    }

    /** Asks for the light of the voxel at `site` of `level` (`on_seen`), if light gets out of it: its lines are found as the light is (`settle`). */
    pub fn ask(&mut self, level: u32, site: [i64; 3]) {
        if self.light.lets_out(level as usize, site) {
            self.add((level as usize, site), 0);
        }
    }

    /** What lights the voxel at `site` of `level` (Light::on_seen), as found so far, if light gets out of it; asked for, if not yet. */
    pub fn on_seen(&mut self, level: u32, site: [i64; 3]) -> Option<Rgb> {
        let i = level as usize;
        if !self.light.lets_out(i, site) {
            return None;
        }
        let id = self.add((i, site), 0);
        self.build();
        let found = &self.sites[id as usize];
        let (light, touches) = (found.light, found.touches.clone());
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

    /** Finds the lines due, then the light of at most `most` sites, each from its neighbours' as they are now; returns how many it found. */
    pub fn settle(&mut self, most: usize) -> usize {
        self.build();
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
            if site.glow.is_some() || site.sky.iter().flatten().any(|&v| v > 0.0) {
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
                        touched.extend(self.watching.get(&(level, [x, y, z])).into_iter().flatten());
                    }
                }
            }
        }
        touched.sort_unstable();
        touched.dedup();
        for id in touched {
            self.unbuild(id);
            let depth = self.sites[id as usize].depth;
            self.due(id, depth);
        }
    }

    /**
     * The id of `key`, of `depth`: begun if new;
     * if nearer than it was, its lines to find again as of that.
     */
    fn add(&mut self, key: Key, depth: u8) -> Id {
        let id = match self.ids.get(&key) {
            Some(&id) if self.sites[id as usize].depth <= depth => return id,
            Some(&id) => {
                self.unbuild(id);
                id
            }
            None => {
                let id = self.sites.len() as Id;
                self.sites.push(Site { key, ..Site::default() });
                self.ids.insert(key, id);
                id
            }
        };
        self.due(id, depth);
        id
    }

    /** Has site `id`'s lines found as of `depth`. */
    fn due(&mut self, id: Id, depth: u8) {
        let site = &mut self.sites[id as usize];
        (site.depth, site.due) = (depth, true);
        self.unbuilt[depth as usize].push(id);
    }

    /** Finds the lines due, the nearest first: each found once as of its depth, unless something nearer than that asks for it. */
    fn build(&mut self) {
        while let Some((depth, id)) = self.unbuilt.iter_mut().enumerate().find_map(|(d, ids)| Some((d as u8, ids.pop()?))) {
            let site = &self.sites[id as usize];
            if site.due && site.depth == depth {
                self.build_one(id);
            }
        }
    }

    /** Finds the lines of site `id` as the world is now (Light's `lines`), the sites they read and those whose light they read, and queues it. */
    fn build_one(&mut self, id: Id) {
        let ((level, p), depth) = (self.sites[id as usize].key, self.sites[id as usize].depth);
        self.light.reads = Some(Vec::new());
        let mut work = self.light.work.pop().unwrap_or_default();
        let (lines, touches) = self.light.lines(level, p, &mut work);
        let met = work.reflects.clone();
        self.light.work.push(work);
        let mut reads: Vec<Key> = self.light.reads.take().expect("kept").into_iter().map(|(l, s)| (l, s.map(|v| v >> WATCH))).collect();
        reads.sort_unstable();
        reads.dedup();
        let (mut glow, mut sky, mut coarser) = (None::<Box<[Rgb; N]>>, [[0.0; 3]; N], Vec::new());
        for (d, line) in lines.iter().enumerate() {
            if line.glow.iter().any(|&g| g > 0.0) {
                glow.get_or_insert_with(|| Box::new([[0.0; 3]; N]))[d] = line.glow;
            }
            match line.beyond {
                Beyond::Nothing => {}
                Beyond::Sky => sky[d] = line.through,
                Beyond::Coarser { sites, total } => {
                    for (site, w) in sites.into_iter().filter(|&(_, w)| w > 0.0) {
                        coarser.push((d as u8, self.add((level + 1, site), 0), line.through.map(|t| t * w / total)));
                    }
                }
            }
        }
        let mut reflects = Vec::new();
        if depth < self.deepest {
            for r in met {
                let ((q, d, face), share) = (r.site(p), unpack(r.share));
                let deeper = (depth + halvings(share.into_iter().fold(0.0, f32::max))).min(self.deepest);
                reflects.push((d as u8, self.add((level, q), deeper), face as u8, share));
            }
        }
        for &key in &reads {
            self.watching.entry(key).or_default().push(id);
        }
        for q in coarser.iter().map(|c| c.1).chain(reflects.iter().map(|r| r.1)) {
            self.sites[q as usize].readers.push(id);
        }
        let site = &mut self.sites[id as usize];
        (site.glow, site.sky, site.coarser, site.reflects, site.touches, site.reads, site.due) =
            (glow, sky, coarser, reflects, touches, reads, false);
        self.queue(id);
    }

    /** Forgets what site `id`'s lines read, to find them again. */
    fn unbuild(&mut self, id: Id) {
        let site = &mut self.sites[id as usize];
        let (reads, coarser, reflects) =
            (std::mem::take(&mut site.reads), std::mem::take(&mut site.coarser), std::mem::take(&mut site.reflects));
        for key in reads {
            if let Some(watching) = self.watching.get_mut(&key) {
                watching.retain(|&w| w != id);
            }
        }
        for q in coarser.iter().map(|c| c.1).chain(reflects.iter().map(|r| r.1)) {
            self.sites[q as usize].readers.retain(|&r| r != id);
        }
    }

    fn queue(&mut self, id: Id) {
        if !std::mem::replace(&mut self.sites[id as usize].queued, true) {
            self.queue.push_back(id);
        }
    }

    /** Finds site `id`'s light from what it reads as it is now; if it changed, those that read it are to find theirs again. */
    fn update(&mut self, id: Id) {
        let site = &self.sites[id as usize];
        let (glowing, skies) = (self.brightness.glow, &self.sky);
        let mut light: [Rgb; N] = std::array::from_fn(|d| std::array::from_fn(|c| site.sky[d][c] * skies[d][c]));
        if let Some(glow) = &site.glow {
            (0..N).for_each(|d| (0..3).for_each(|c| light[d][c] += glow[d][c] * glowing));
        }
        for &(d, q, w) in &site.coarser {
            let past = self.sites[q as usize].light[d as usize];
            (0..3).for_each(|c| light[d as usize][c] += w[c] * past[c]);
        }
        for &(d, q, face, share) in &site.reflects {
            let on = self.sites[q as usize].on[face as usize];
            (0..3).for_each(|c| light[d as usize][c] += share[c] * on[c]);
        }
        let on = std::array::from_fn(|f| self.light.on_face(&light, site.touches.as_deref(), f));
        let changed = changed(&site.light, &light, self.tolerance);
        let site = &mut self.sites[id as usize];
        (site.light, site.on, site.found) = (light, on, true);
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

/** How many times halving 1 takes to reach `share`, at least once: none can be skipped. */
fn halvings(share: f32) -> u8 {
    (-share.log2()).ceil().clamp(1.0, u8::MAX as f32) as u8
}

/** Whether any channel of `after` is off `before` by more than `tolerance` of the larger, and more than FLOOR. */
fn changed(before: &[Rgb; N], after: &[Rgb; N], tolerance: f32) -> bool {
    before.iter().flatten().zip(after.iter().flatten()).any(|(&b, &a)| {
        let off = (a - b).abs();
        off > FLOOR && off > tolerance * a.max(b)
    })
}
