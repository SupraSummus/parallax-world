import type { Vec3 } from './world'
import { fbm, hash, rng, smoothstep, valueNoise } from './noise'
import { areaProfile, ball, box, boundsOfShapes, capsule, ellipsoid, lathe, type Box, type Cell, type Child, type Field, type Generator, type Kinds, type Node, type Point, type Population, type Shape } from './generator'

/**
 * Metres per lattice unit: voxels are FCC sites (voxel-grid.ts), nearest
 * neighbours √2 units (0.18 m) apart. Terrain and plant sizes below are in
 * metres, converted with vox(), so the scene keeps its proportions if this
 * changes. Shapes (the ground's height, trunks, balls, branches) are
 * sampled at the lattice's sites.
 * @internal
 */
export const UNIT_SIZE = 0.125
const vox = (metres: number) => Math.round(metres / UNIT_SIZE)
/** Radius of branches, in units: thick enough to stay one piece on the lattice. */
const BRANCH_RADIUS = 1.2
/** Radius of fallen logs. */
const LOG_RADIUS = vox(0.45)
/**
 * The spacing of level 0's sites. Parts this fine (twigs, single leaves,
 * bark) are drawn only there; from level 1 on, things that have them draw
 * what they look like from further away (a ball of leaves, a crown).
 */
const FINE = 1

/** The heights it has anything at, in units: the ground and the trees on it. */
const HEIGHTS = { minY: vox(-36), maxY: vox(80) }

/**
 * How far around the camera the grid of level 0 must hold the forest, in
 * units (44 m): where single voxels are drawn (docs/architecture.md,
 * section 1). The grid of level ℓ reaches 2^ℓ times as far.
 */
const REACH = 352

// Material kinds; each voxel gets one shade of its kind, picked by position.
// `type` ids follow world.ts: 1 grass, 2 dirt, 3 stone, 4 wood, 5 leaves, 6 flower.
const KINDS = {
  moss: { type: 1, colors: ['#4f7a2a', '#5a8a30', '#466e26', '#628f37'] },
  grass: { type: 1, colors: ['#74a83c', '#7fb544', '#6a9c36'] },
  tallGrass: { type: 1, colors: ['#8fbf4d', '#9cc95a', '#a3b84a'] },
  litter: { type: 2, colors: ['#8a5a2b', '#9c6b30', '#7a4e24', '#a8763a'] },
  needles: { type: 2, colors: ['#6b4a2e', '#5e4029', '#76533a'] },
  path: { type: 2, colors: ['#a08262', '#957656', '#ab8c6b'] },
  soil: { type: 2, colors: ['#4a3423', '#3f2c1e', '#54392a'] },
  stone: { type: 3, colors: ['#7d7d78', '#6e6e6a', '#8a8a84'] },
  mossyStone: { type: 3, colors: ['#5f7348', '#687c4f', '#56693f'] },
  bark: { type: 4, colors: ['#4b3621', '#3f2d1c', '#57402a'] },
  spruceBark: { type: 4, colors: ['#4a3426', '#5a4030'] },
  birchBark: { type: 4, colors: ['#e6e2d6', '#dcd7c9', '#efece4'] },
  birchMark: { type: 4, colors: ['#2e2b28', '#3b3733'] },
  deadwood: { type: 4, colors: ['#6a5640', '#5c4a36', '#776149'] },
  spruce: { type: 5, colors: ['#1f4a2c', '#24553a', '#1a3f26', '#2b5e3c'] },
  oakGreen: { type: 5, colors: ['#4c7f2a', '#5a8f30', '#447324', '#679a38'] },
  oakOrange: { type: 5, colors: ['#c46a1c', '#d68526', '#b0521a', '#9c4a1c'] },
  oakGold: { type: 5, colors: ['#d9a521', '#c8941c', '#e3b53a', '#b88a1e'] },
  birchGreen: { type: 5, colors: ['#7fae3c', '#8cbc45', '#9ac24f'] },
  birchGold: { type: 5, colors: ['#d8cf4a', '#e6d65a', '#c9be36', '#efe07a'] },
  fern: { type: 5, colors: ['#3f7a2c', '#4a8a34', '#56953a'] },
  bracken: { type: 5, colors: ['#a8642a', '#b97a35', '#8f5524'] },
  bush: { type: 5, colors: ['#3d6b28', '#477a2e', '#355f22'] },
  // A flower is of one colour.
  flowerYellow: { type: 6, colors: ['#e8d34a', '#f0dc58'] },
  flowerViolet: { type: 6, colors: ['#c9b1ff', '#b89cf0'] },
  flowerWhite: { type: 6, colors: ['#f2f2f2', '#e8e8e0'] },
  flowerRed: { type: 6, colors: ['#e86a6a', '#d95858'] },
  // Finer than the sites of level 1.
  barkFurrow: { type: 4, colors: ['#2e2115', '#352717', '#281c12'] },
  spruceScale: { type: 4, colors: ['#33241a', '#2b1e15', '#3d2b1f'] },
  twig: { type: 4, colors: ['#5a4430', '#4e3a28', '#634b35'] },
  hay: { type: 1, colors: ['#b9a15a', '#c8b06a', '#a89050'] },
  fallenLeaf: { type: 2, colors: ['#c4581c', '#d9861f', '#b8401a', '#e0b030', '#9a3a16', '#8c5a22'] },
  cone: { type: 4, colors: ['#6b4423', '#7a5030', '#5c3a1e'] },
  amanita: { type: 6, colors: ['#c8281e', '#d63a2a', '#b52018'] },
  amanitaSpot: { type: 6, colors: ['#f4efe6', '#ebe4d6'] },
  mushroomStem: { type: 6, colors: ['#e9e2d0', '#ddd4bf'] },
  bolete: { type: 6, colors: ['#7a4a26', '#8a5530', '#6c4020'] },
  berry: { type: 6, colors: ['#c01f2a', '#a81824', '#d43a30'] },
} satisfies Kinds<string>
type Kind = keyof typeof KINDS


// ---------------------------------------------------------------------------
// Fields: what the layers know about any column, from the seed alone.

interface Column {
  height: number
  surface: Kind
  /** 0 = broadleaf, 1 = conifer. */
  conifer: number
  /** Tree density in [0, 1]; low in clearings. */
  density: number
  onPath: boolean
}

type Species = 'spruce' | 'birch' | 'oak'
const SPECIES: Species[] = ['spruce', 'birch', 'oak']
/** A tree with its foot at (x, ground, z); `groundAt` is the ground's height in any column, for its roots. */
type MakeTree = (x: number, ground: number, z: number, random: () => number, groundAt: (x: number, z: number) => number) => Node<Kind>
const TREES: Record<Species, MakeTree> = {
  spruce: spruceTree,
  birch: birchTree,
  oak: oakTree,
}
/** The colour of each species' crowns from far away. */
const FOLIAGE: Record<Species, Kind> = { spruce: 'spruce', birch: 'birchGold', oak: 'oakOrange' }

/**
 * An autumn forest valley: a winding path along the valley floor, wooded
 * slopes rising on both sides. Spruce, birch and oak, with ferns, bushes, boulders and fallen logs, on
 * hills kilometres wide. Endless: the app generates each level's grid
 * around the camera, `reach` sites of the level either way
 * (world.ts), from `spawn`, where the camera starts.
 */
export function forest(seed: number): { generator: Generator<Kind>; spawn: Vec3; reach: number; heights: (level: number) => { minY: number; maxY: number } } {
  const s = (k: number) => (seed | 0) + k * 1013904223
  const pathPhase = hash(s(1), 0, 0) * 100
  /** Path centre line, in metres. */
  const pathX = (zm: number) => 9 * Math.sin(zm / 31 + pathPhase) + 5 * Math.sin(zm / 13 + pathPhase * 2)

  const groundMetres = (xm: number, zm: number) => {
    const fromPath = Math.abs(xm - pathX(zm))
    const hills = (fbm(s(2), xm / 45, zm / 45) - 0.5) * 10 + (valueNoise(s(3), xm / 6, zm / 6) - 0.5) * 0.8
    const valley = smoothstep(8, 42, fromPath) * 18
    const far = (fbm(s(9), xm / 900, zm / 900) - 0.5) * 50
    const pathDip = (1 - smoothstep(1, 2.2, fromPath)) * 0.3
    return hills + valley + far - pathDip
  }
  // Every cell of a column of a population's cells asks for the same
  // spots (see spot()), so recent heights are kept, by a hash of (x, z).
  const keptX = new Float64Array(1 << 14).fill(NaN)
  const keptZ = new Float64Array(1 << 14)
  const keptHeight = new Int32Array(1 << 14)
  /** Ground height in units. */
  const ground = (x: number, z: number) => {
    const k = (Math.imul(x, 0x9e3779b1) ^ Math.imul(z, 0x85ebca77)) >>> 18
    if (keptX[k] !== x || keptZ[k] !== z) {
      keptX[k] = x
      keptZ[k] = z
      keptHeight[k] = vox(groundMetres(x * UNIT_SIZE, z * UNIT_SIZE))
    }
    return keptHeight[k]
  }

  /** Column (x, z), given the ground's height there and at its four neighbours. */
  const columnOf = (x: number, z: number, h: number, neighbours: readonly number[]): Column => {
    const xm = x * UNIT_SIZE
    const zm = z * UNIT_SIZE
    const slope = Math.max(...neighbours.map(n => Math.abs(n - h)))
    const conifer = smoothstep(0.4, 0.6, fbm(s(5), xm / 40, zm / 40)) * 0.75 + smoothstep(6, 16, h * UNIT_SIZE) * 0.25
    const density = smoothstep(0.3, 0.48, fbm(s(6), xm / 22, zm / 22))
    const onPath = Math.abs(xm - pathX(zm)) < 1.1 + valueNoise(s(7), zm / 3, 0) * 0.5
    const floor = valueNoise(s(8), xm / 2.5, zm / 2.5)
    let surface: Kind
    if (onPath) surface = 'path'
    else if (slope >= 3) surface = floor < 0.5 ? 'stone' : 'mossyStone'
    else if (density < 0.2) surface = floor < 0.75 ? 'grass' : 'moss'
    else if (conifer > 0.5) surface = floor < 0.55 ? 'needles' : 'moss'
    else surface = floor < 0.6 ? 'litter' : 'moss'
    return { height: h, surface, conifer, density, onPath }
  }
  const column = (x: number, z: number) =>
    columnOf(x, z, ground(x, z), [ground(x - 1, z), ground(x + 1, z), ground(x, z - 1), ground(x, z + 1)])

  // Start on the path at the near end, at about the height of a tall shrub.
  const spawnZ = vox(-31)
  const spawnX = Math.round(pathX(spawnZ * UNIT_SIZE) / UNIT_SIZE)
  // On a site, so it is in a voxel's cell, which is empty.
  const spawnY = ground(spawnX, spawnZ) + vox(3)
  const spawn = { x: spawnX, y: spawnY + ((spawnX + spawnY + spawnZ) & 1), z: spawnZ }

  /**
   * A random spot on the ground under the square a cell reaches over (2N
   * wide): every spot on the ground is in one cell, whose column of cells
   * picks it as often as any, so things are as even as one per square 2N
   * wide. The cells of a column pick the same spot, one cell owns it.
   */
  const spot = (cell: Cell, n: number): Point => {
    const [cx, , cz] = cell.centre
    const pick = rng(Math.floor(hash(s(19), n, cell.i, cell.k) * 4294967296))
    const [x, z] = [cx - n + Math.floor(pick() * 2 * n), cz - n + Math.floor(pick() * 2 * n)]
    return [x, ground(x, z), z]
  }

  /** The chance a cell of trees around column (x, z) has one. */
  const treeChance = (x: number, z: number, c: Column) => {
    if (c.surface === 'stone' || Math.abs(x * UNIT_SIZE - pathX(z * UNIT_SIZE)) < 2.8) return 0
    // Keep the view from the spawn point clear of low branches.
    if (Math.hypot(x - spawn.x, z - spawn.z) < vox(7)) return 0
    return 0.15 + c.density * 0.75
  }
  const speciesChances = (c: Column): Record<Species, number> =>
    ({ spruce: c.conifer, birch: (1 - c.conifer) * 0.45, oak: (1 - c.conifer) * 0.55 })

  // -------------------------------------------------------------------------
  // Layers, drawn in this order.

  // The ground: along each column, soil or stone under a layer of its surface.
  const terrain: Field<Kind> = {
    name: 'terrain',
    order: 0,
    axis: 1,
    spans(x, z, spacing) {
      const h = ground(x, z)
      const around = [ground(x - spacing, z), ground(x + spacing, z), ground(x, z - spacing), ground(x, z + spacing)]
      const { surface } = spacing === 1 ? columnOf(x, z, h, around) : column(x, z)
      // Deep enough that no gaps open between slices or on slopes; the top site shows the surface.
      const bottom = Math.min(h - vox(1.5), ...around)
      const top = h + 1 - Math.max(vox(1), 2 * spacing)
      return [
        { from: bottom, to: top, kind: surface === 'stone' || surface === 'mossyStone' ? 'stone' : 'soil' },
        { from: top, to: h + 1, kind: surface },
      ]
    },
  }

  // Trees, each in the cell its foot is in: at most one per 4 m × 4 m.
  const TREE_CELL = vox(2)
  // Per species, its area below each height up to the trees' reach: the area between two is a difference.
  const TREE_TOP = vox(26)
  const below = Object.fromEntries(SPECIES.map(species => {
    const samples = Array.from({ length: 64 }, (_, i) => TREES[species](0, 0, 0, rng(s(17) + i), () => 0))
    const profile = areaProfile(samples, 1)
    const sums = new Float64Array(TREE_TOP + 1)
    for (let h = 0; h < TREE_TOP; h++) sums[h + 1] = sums[h] + profile(h)
    return [species, sums]
  })) as Record<Species, Float64Array>
  const trees: Population<Kind> = {
    name: 'trees',
    order: 1,
    cell: TREE_CELL,
    reach: [vox(9), TREE_TOP, vox(9)],
    // One by one, trees cost as much at every level per tree, and a box of
    // a coarser level holds more of them: from level 3 on, the canopy.
    size: vox(1),
    anchor: cell => spot(cell, TREE_CELL),
    make([x, y, z], cell) {
      const random = cell.random('tree')
      const c = column(x, z)
      if (random() > treeChance(x, z, c)) return undefined
      const chances = speciesChances(c)
      let pick = random()
      const species = SPECIES.find(sp => (pick -= chances[sp]) < 0) ?? 'oak'
      return TREES[species](x, y, z, random, ground)
    },
    // A medium as dense as the trees' area per volume over each column (a quarter of it shadows a ray).
    far: {
      name: 'canopy',
      order: 1,
      op: 'fill',
      axis: 1,
      spans(x, z, spacing) {
        const c = column(x, z)
        const perArea = treeChance(x, z, c) / (2 * TREE_CELL) ** 2
        if (perArea === 0) return []
        const chances = speciesChances(c)
        let pick = hash(s(18), x, z)
        const kind = FOLIAGE[SPECIES.find(sp => (pick -= chances[sp]) < 0) ?? 'oak']
        const spans = []
        for (let from = 0; from < TREE_TOP; from += spacing) {
          const to = Math.min(from + spacing, TREE_TOP)
          const area = SPECIES.reduce((sum, sp) => sum + chances[sp] * (below[sp][to] - below[sp][from]), 0)
          if (area > 0) spans.push({ from: c.height + from, to: c.height + from + spacing, kind, extinction: (perArea * area) / spacing / 4 })
        }
        return spans
      },
    },
  }

  // Boulders and fallen logs, about one per 430 m² and 830 m².
  const ROCK_CELL = vox(4)
  const boulders: Population<Kind> = {
    name: 'boulders',
    order: 2,
    cell: ROCK_CELL,
    reach: vox(3),
    size: vox(2),
    anchor: cell => spot(cell, ROCK_CELL),
    make: ([x, y, z], cell) => {
      const random = cell.random('boulder')
      return random() < 0.15 ? { shapes: boulder(x, y, z, random) } : undefined
    },
  }
  const logs: Population<Kind> = {
    name: 'logs',
    order: 2,
    cell: ROCK_CELL,
    reach: vox(10),
    size: vox(2),
    anchor: cell => spot(cell, ROCK_CELL),
    make: ([x, , z], cell) => {
      const random = cell.random('log')
      return random() < 0.077 && !column(x, z).onPath ? { shapes: fallenLog(x, z, ground, random) } : undefined
    },
  }

  // Undergrowth, per column of (0.25 m)², in square patches N wide around
  // each cell's centre (the patches of one column of cells tile the ground).
  // A patch belongs to the cell its centre's ground is in: half a unit up,
  // that is never on the boundary, so always in one of the column's cells.
  // It fills only empty sites, but for fallen leaves, which colour the ground.
  const PLANTS = 16
  const COLUMN = vox(0.25)
  const plants: Population<Kind> = {
    name: 'plants',
    order: 3,
    cell: PLANTS,
    reach: [0, vox(10), 0],
    // Up to sites 0.25 m apart; it is mostly finer than that.
    size: COLUMN,
    anchor: ({ centre: [cx, , cz] }) => [cx, ground(cx, cz) + 0.5, cz],
    make(_, { centre: [cx, , cz] }) {
      const shapes: Shape<Kind>[] = []
      for (let x = cx - PLANTS / 2; x < cx + PLANTS / 2; x += COLUMN) {
        for (let z = cz - PLANTS / 2; z < cz + PLANTS / 2; z += COLUMN) {
          const c = column(x, z)
          const y = c.height + 1
          const r = hash(s(15), x, z)
          const random = rng(Math.floor(hash(s(16), x, z) * 4294967296))
          if (c.onPath) {
            if (r < 0.05) shapes.push(box([x, y, z], [x + 2, y + 1, z + 1], 'stone'))
          } else if (c.surface === 'grass') {
            if (r < 0.02) shapes.push(...tuft(x, y, z, 3 + Math.floor(random() * 3), vox(0.3), random() < 0.3 ? 'hay' : 'tallGrass', random))
            else if (r < 0.0275) shapes.push(...flower(x, y, z, random))
            else if (r < 0.2) shapes.push(...tuft(x, y, z, 1 + Math.floor(random() * 2), vox(0.12), 'grass', random))
          } else if (c.surface !== 'stone' && c.surface !== 'mossyStone') {
            if (r < 0.00625) shapes.push(...fern(x, y, z, c.conifer > 0.5 ? 'fern' : 'bracken', random))
            else if (r < 0.008) shapes.push(...bush(x, y, z, random))
            else if (r < 0.0085) shapes.push(...flower(x, y, z, random))
            else if (r < 0.011 && c.density > 0.3) shapes.push(...mushrooms(x, y, z, c.conifer > 0.5 ? 'amanita' : 'bolete', random))
            else if (c.surface === 'needles' && r < 0.04) shapes.push(box([x, y, z], [x + 2, y + 2, z + 1], 'cone'))
            else if (c.surface === 'litter' && r < 0.4) shapes.push(box([x - 1, y - 1, z - 1], [x + 1, y, z + 1], 'fallenLeaf', { op: 'paint' }))
          }
        }
      }
      return shapes.length > 0 ? { shapes } : undefined
    },
  }

  return {
    generator: { kinds: KINDS, seed: s(4), layers: [terrain, trees, boulders, logs, plants] },
    spawn,
    reach: REACH,
    heights: level => ({ minY: Math.floor(HEIGHTS.minY / 2 ** level), maxY: Math.ceil(HEIGHTS.maxY / 2 ** level) }),
  }
}

// ---------------------------------------------------------------------------
// Things. `random` picks their shapes.

/**
 * A tree of `shapes`, with the parts its crown hides as a child: from far
 * away, hidden parts would show, as each shape is drawn on its own.
 * `fine` children are drawn only near. Its crown of `clusters`, once their
 * balls are smaller than the sites, is drawn as one solid of their volume.
 */
function tree(shapes: Shape<Kind>[], hidden: Shape<Kind>[], fine: Child<Kind>[], crown?: { trunk: Shape<Kind>; clusters: Child<Kind>[]; fill: number }): Node<Kind> {
  const children = [{ name: 'hidden', size: vox(0.5), shapes: hidden }, ...fine, ...(crown?.clusters ?? [])]
  // Children's bounds, not shapes, so fine parts are not made (see lazily).
  const node = { shapes, bounds: boundsOfShapes([...shapes, ...children.map(child => ({ bounds: child.bounds ?? boundsOfShapes(child.shapes) }))]), children: () => children }
  if (!crown) return node
  const { trunk, fill } = crown
  const clusters = crown.clusters.flatMap(c => c.far?.shapes ?? [])
  // An ellipsoid of the clusters' outline, shrunk to their volume (overlapping as if at random) and solid.
  const b = boundsOfShapes(clusters)
  const box: Point = [(b.maxX - b.minX) / 2, (b.maxY - b.minY) / 2, (b.maxZ - b.minZ) / 2]
  const volume = (4 / 3) * Math.PI * box[0] * box[1] * box[2]
  const filled = clusters.reduce((sum, { extents: [a, c, d] }) => sum + (Math.PI / 6) * a * c * d * fill, 0)
  const shrink = (1 - Math.exp(-filled / volume)) ** (1 / 3)
  const outline = ellipsoid([(b.minX + b.maxX) / 2, (b.minY + b.maxY) / 2, (b.minZ + b.maxZ) / 2], [box[0] * shrink, box[1] * shrink, box[2] * shrink], clusters[0].kind)
  return { ...node, far: { detail: Math.min(...clusters.map(c => Math.min(...c.extents))), shapes: [trunk, outline] } }
}

/** A vertical trunk, `width` units square (at least 2, to stay one piece on the lattice), from just below `ground` up to `top`. */
const trunk = (x: number, ground: number, z: number, top: number, width: number, kind: Kind) =>
  box([x, ground - 1, z], [x + width, top + 1, z + width], kind, { op: 'paint' })

/** A random direction, evenly over the sphere. */
function direction(random: () => number): Point {
  const y = 2 * random() - 1
  const a = 2 * Math.PI * random()
  const r = Math.sqrt(1 - y * y)
  return [r * Math.cos(a), y, r * Math.sin(a)]
}

/**
 * A cluster of leaves around `centre`: at level 0, a core and sprays of
 * leaves on twigs radiating from it, with light between them at its edge;
 * further, the ball they fill. `droop` bends the sprays down (birch);
 * some are of the `turning` colour, for trees between green and gold.
 */
function leafCluster(name: string, centre: Point, radius: number, kind: Kind, fill: number, random: () => number, { droop = 0, turning = kind }: { droop?: number; turning?: Kind } = {}): Child<Kind> {
  const sprays = lazily(random, random => {
    // Sprays a bit under half a metre thick, enough to fill the outer shell about as densely as the ball.
    const spray = Math.min(vox(0.45), radius * 0.4)
    const count = Math.max(4, Math.round(0.95 * (radius / spray) ** 2))
    const [twigs, leaves]: Shape<Kind>[][] = [[], []]
    const at = (d: Point, r: number): Point => [centre[0] + d[0] * r, centre[1] + d[1] * r, centre[2] + d[2] * r]
    for (let i = 0; i < count; i++) {
      const [dx, dy, dz] = direction(random)
      const length = Math.hypot(dx, dy - droop, dz)
      const d: Point = [dx / length, (dy - droop) / length, dz / length]
      const tip = radius - spray
      twigs.push(capsule(at(d, radius * 0.1), at(d, tip - spray / 2), FINE, 'twig', { op: 'fill' }))
      leaves.push(capsule(at(d, radius * 0.3), at(d, tip), spray, random() < 0.25 ? turning : kind, { op: 'fill', fill: 0.75 }))
    }
    return [...twigs, ball(centre, radius * 0.55, kind, { fill }), ...leaves]
  })
  const far = ball(centre, radius, kind, { fill })
  return {
    name,
    size: Infinity,
    bounds: far.bounds,
    get shapes() { return sprays() },
    far: { detail: FINE, shapes: [far] },
  }
}

/**
 * Shapes made only once asked for: those of fine parts, which a thing
 * drawn far away never draws. From their own random sequence, seeded now,
 * so the thing is the same whether they are made or not.
 */
function lazily(random: () => number, make: (random: () => number) => Shape<Kind>[]): () => Shape<Kind>[] {
  const seed = Math.floor(random() * 4294967296)
  let shapes: Shape<Kind>[] | undefined
  return () => (shapes ??= make(rng(seed)))
}

/** A child of a thing within `bounds`, drawn only at level 0, its shapes made only then (see lazily). */
function fineChild(name: string, bounds: Box, random: () => number, make: (random: () => number) => Shape<Kind>[]): Child<Kind> {
  const shapes = lazily(random, make)
  return { name, size: FINE, bounds, get shapes() { return shapes() } }
}

/**
 * Bark on the four faces of a trunk [x, x + width) × [from, to) × [z, z + width):
 * `furrows`, dark cracks up it in pieces, or `dashes`, a birch's short
 * black lenticels across it. Bark is finer than the sites of level 1.
 */
function bark(x: number, z: number, width: number, from: number, to: number, style: 'furrows' | 'dashes', kind: Kind, random: () => number): Shape<Kind>[] {
  const shapes: Shape<Kind>[] = []
  // Face f: its layer, and the box of a mark from u to u + length along it, y0 to y1 up.
  const faces = [0, 1, 2, 3].map(f => {
    const layer = f % 2 === 0 ? (f < 2 ? z : x) : (f < 2 ? z + width - 1 : x + width - 1)
    return (u: number, length: number, y0: number, y1: number) => f < 2
      ? box([u, y0, layer], [u + length, y1, layer + 1], kind, { op: 'paint' })
      : box([layer, y0, u], [layer + 1, y1, u + length], kind, { op: 'paint' })
  })
  const start = (f: number) => (f < 2 ? x : z)
  for (const [f, mark] of faces.entries()) {
    if (style === 'furrows') {
      for (let n = Math.max(1, Math.round(width / 3)); n > 0; n--) {
        let u = start(f) + Math.floor(random() * (width - 1))
        for (let y = from; y < to;) {
          const length = vox(0.4 + random() * 1.2)
          shapes.push(mark(u, 2, y, Math.min(y + length, to)))
          y += length + Math.floor(random() * vox(0.3))
          u = Math.max(start(f), Math.min(start(f) + width - 2, u + Math.floor(random() * 3) - 1))
        }
      }
    } else {
      for (let y = from; y < to; y += vox(0.25)) {
        if (random() < 0.5) shapes.push(mark(start(f) + Math.floor(random() * (width - 1)), 2 + Math.floor(random() * 2), y, y + 1))
      }
    }
  }
  return shapes
}

/** Roots spreading from a trunk's foot around (x, z) into the ground, `half` its half-width. */
function roots(x: number, ground: number, z: number, half: number, kind: Kind, random: () => number, groundAt: (x: number, z: number) => number): Shape<Kind>[] {
  const count = 4 + Math.floor(random() * 3)
  return Array.from({ length: count }, (_, i) => {
    const angle = (i / count) * Math.PI * 2 + random() * 0.6
    const [c, s] = [Math.cos(angle), Math.sin(angle)]
    const out = half + vox(0.5 + random() * 0.7)
    const [ex, ez] = [x + c * out, z + s * out]
    return capsule([x + c * half * 0.6, ground + vox(0.35), z + s * half * 0.6], [ex, groundAt(Math.round(ex), Math.round(ez)) - 1, ez], vox(0.15 + random() * 0.08), kind, { op: 'fill' })
  })
}

/** Conical spruce with drooping whorls of branches: near, the branches of each whorl fan out with gaps between. */
function spruceTree(x: number, ground: number, z: number, random: () => number, groundAt: (x: number, z: number) => number): Node<Kind> {
  const height = vox(14 + random() * 10)
  const width = height >= vox(20) ? vox(1) : vox(0.5)
  const top = ground + height
  const base = ground + Math.round(height * (0.15 + random() * 0.15))
  const bole = trunk(x, ground, z, base, width, 'spruceBark')
  const hidden = [box([x, base + 1, z], [x + width, top, z + width], 'spruceBark', { op: 'paint' })]
  const radius = height * (0.17 + random() * 0.05)
  const whorl = vox(1.5 + random() * 0.6)
  const [cx, cz] = [x + (width - 1) / 2, z + (width - 1) / 2]
  const crownTop = top + vox(0.5)
  const r = (y: number) => {
    // Each whorl is widest at its bottom, giving the layered silhouette.
    const phase = (((top - y) % whorl) + whorl) % whorl / whorl
    return radius * (1 - (y - base) / (crownTop - base)) * (0.5 + 0.5 * phase) + vox(0.35)
  }
  // From half a unit below the lowest whorl to half above the top, so level 0 has a disc of sites at each y.
  const [from, to] = [base - 0.5, crownTop + 0.5]
  const density = (distance: number, py: number) => 0.95 - 0.35 * (distance / r(py)) ** 3
  // Each whorl's branches: fans widening outwards, turned by the golden angle from the whorl above.
  const spokes = 6 + Math.floor(random() * 3)
  const sector = (2 * Math.PI) / spokes
  const twist = random() * sector
  const halfWidth = (distance: number) => vox(0.15) + 0.07 * distance
  // The share of a circle the fans cover: the crown from further away is as dense on average.
  const covered = (distance: number) => Math.min(1, (2 * halfWidth(distance)) / (distance * sector))
  const fans = (px: number, py: number, pz: number) => {
    const distance = Math.hypot(px - cx, pz - cz)
    if (distance < vox(0.5)) return density(distance, py)
    const a = Math.atan2(pz - cz, px - cx) - twist - Math.floor((top - py) / whorl) * 2.39996
    const off = ((a % sector) + sector) % sector
    return Math.min(off, sector - off) * distance < halfWidth(distance) ? density(distance, py) : 0
  }
  const crown = (f: (px: number, py: number, pz: number) => number) => lathe([cx, from, cz], [cx, to, cz], t => r(Math.round(from + t * (to - from))), 'spruce', { fill: f })
  const foot = roots(cx, ground, cz, width / 2, 'spruceBark', random, groundAt)
  const fine = [fineChild('bark', bole.bounds, random, random => bark(x, z, width, ground, base, 'furrows', 'spruceScale', random))]
  const near = tree([bole, ...foot, crown(fans)], hidden, fine)
  // Its hidden parts are not drawn beyond level 0: inside a crown this dense they don't show.
  return { ...near, far: { detail: FINE, shapes: [bole, ...foot, crown((px, py, pz) => {
    const distance = Math.hypot(px - cx, pz - cz)
    return density(distance, py) * covered(Math.max(vox(0.5), distance))
  })] } }
}

/** Slender white birch with a wavy trunk and a loose, clumpy crown of drooping twigs. */
function birchTree(x: number, ground: number, z: number, random: () => number): Node<Kind> {
  const height = vox(12 + random() * 7)
  const width = vox(0.375)
  const [shapes, marks]: Shape<Kind>[][] = [[], []]
  const segments: [number, number, number, number][] = []
  // The trunk shifts by a unit now and then.
  let [tx, tz, from] = [x, z, ground - 1]
  for (let y = ground - 1; y <= ground + height + 1; y++) {
    const end = y === ground + height + 1
    if (end || ((y - ground) % vox(3) === 0 && y - ground > vox(4) && random() < 0.35)) {
      shapes.push(box([tx, from, tz], [tx + width, y, tz + width], 'birchBark', { op: 'paint' }))
      segments.push([tx, tz, Math.max(from, ground), y])
      if (end) break
      if (random() < 0.5) tx += random() < 0.5 ? -1 : 1
      else tz += random() < 0.5 ? -1 : 1
      from = y
    }
    // Black marks in bands half a metre tall.
    if ((y - ground) % vox(0.5) === 0 && random() < 0.18) marks.push(box([tx, y, tz], [tx + width, y + vox(0.5), tz + width], 'birchMark', { op: 'paint' }))
  }
  shapes.push(...marks)
  const foliage: Kind = random() < 0.65 ? 'birchGold' : 'birchGreen'
  const turning: Kind = foliage === 'birchGold' ? 'birchGreen' : 'birchGold'
  const crownBottom = ground + height * 0.45
  const bole = box([x, ground - 1, z], [x + width, crownBottom, z + width], 'birchBark', { op: 'paint' })
  const crownTop = ground + height + vox(1)
  const cy = (crownBottom + crownTop) / 2
  const ry = (crownTop - crownBottom) / 2
  const rx = vox(2 + random() * 1.2)
  const count = 12 + Math.floor(random() * 6)
  const [mid, branches, clusters]: [number, Shape<Kind>[], Child<Kind>[]] = [(width - 1) / 2, [], []]
  for (let i = 0; i < count; i++) {
    // Uniform point in the crown ellipsoid.
    let ox, oy, oz
    do {
      ox = random() * 2 - 1
      oy = random() * 2 - 1
      oz = random() * 2 - 1
    } while (ox * ox + oy * oy + oz * oz > 1)
    const p: Point = [tx + ox * rx, cy + oy * ry, tz + oz * rx]
    branches.push(capsule([tx + mid, p[1] - vox(0.5), tz + mid], p, BRANCH_RADIUS, 'birchBark'))
    clusters.push(leafCluster(`leaves ${String(i)}`, p, vox(0.8 + random() * 0.6), foliage, 0.7, random, { droop: 0.6, turning }))
  }
  const dashes = fineChild('bark', boundsOfShapes(shapes), random, random => segments.flatMap(([sx, sz, y0, y1]) => bark(sx, sz, width, y0, y1, 'dashes', 'birchMark', random)))
  return tree(shapes, branches, [dashes], { trunk: bole, clusters, fill: 0.7 })
}

/** Broad oak: thick trunk, a few branches each ending in a leaf cluster. */
function oakTree(x: number, ground: number, z: number, random: () => number, groundAt: (x: number, z: number) => number): Node<Kind> {
  const height = vox(12 + random() * 8)
  const trunkTop = ground + Math.round(height * (0.45 + random() * 0.1))
  const width = vox(1)
  const shapes = [trunk(x, ground, z, trunkTop, width, 'bark')]
  const roll = random()
  const foliage: Kind = roll < 0.45 ? 'oakGreen' : roll < 0.75 ? 'oakOrange' : 'oakGold'
  const turning: Kind = foliage === 'oakGreen' ? 'oakGold' : foliage === 'oakOrange' ? 'oakGold' : 'oakOrange'
  const cx = x + (width - 1) / 2
  const cz = z + (width - 1) / 2
  shapes.push(...roots(cx, ground, cz, width / 2, 'bark', random, groundAt))
  const branches = 4 + Math.floor(random() * 3)
  const ends: Point[] = [[cx, ground + height - vox(2), cz]]
  const hidden: Shape<Kind>[] = []
  for (let i = 0; i < branches; i++) {
    const angle = (i / branches) * Math.PI * 2 + random() * 0.8
    const reach = vox(2.5 + random() * 2.5)
    const startY = trunkTop - Math.floor(random() * vox(2))
    const end: Point = [cx + Math.cos(angle) * reach, Math.min(startY + vox(2 + random() * 3), ground + height - vox(1.5)), cz + Math.sin(angle) * reach]
    hidden.push(capsule([cx, startY, cz], end, BRANCH_RADIUS * 1.5, 'bark'))
    ends.push(end)
  }
  const clusters = ends.map((end, i) => leafCluster(`leaves ${String(i)}`, end, vox(1.8 + random() * 1.2), foliage, 0.8, random, { turning }))
  const fine = [fineChild('bark', shapes[0].bounds, random, random => bark(x, z, width, ground, trunkTop, 'furrows', 'barkFurrow', random))]
  return tree(shapes, hidden, fine, { trunk: shapes[0], clusters, fill: 0.8 })
}

// Undergrowth on the ground at (x, y, z): blades, stems and leaves a unit or
// two thick. A box two units wide in x holds a site of it at every y.

/** A tuft of `count` blades about `tall` high. */
function tuft(x: number, y: number, z: number, count: number, tall: number, kind: Kind, random: () => number): Shape<Kind>[] {
  return Array.from({ length: count }, () => {
    const [dx, dz] = [Math.floor(random() * 3) - 1, Math.floor(random() * 3) - 1]
    return box([x + dx, y, z + dz], [x + dx + 2, y + Math.max(1, Math.round(tall * (0.6 + random() * 0.6))), z + dz + 1], kind)
  })
}

const FLOWERS: Kind[] = ['flowerYellow', 'flowerViolet', 'flowerWhite', 'flowerRed']

/** A flower: a stem and a head two sites wide. */
function flower(x: number, y: number, z: number, random: () => number): Shape<Kind>[] {
  const stem = 2 + Math.floor(random() * 2)
  const colour = FLOWERS[Math.floor(random() * FLOWERS.length)]
  return [box([x, y, z], [x + 2, y + stem, z + 1], 'tallGrass'), box([x - 1, y + stem, z - 1], [x + 1, y + stem + 1, z + 1], colour)]
}

/** A fern or bracken clump about a metre across: arching fronds spreading from the middle. */
function fern(x: number, y: number, z: number, kind: Kind, random: () => number): Shape<Kind>[] {
  const fronds = 5 + Math.floor(random() * 4)
  const shapes: Shape<Kind>[] = []
  for (let i = 0; i < fronds; i++) {
    const angle = (i / fronds) * Math.PI * 2 + random() * 0.5
    const [c, s] = [Math.cos(angle), Math.sin(angle)]
    const [length, rise] = [vox(0.45 + random() * 0.35), vox(0.25 + random() * 0.2)]
    const bend: Point = [x + c * length * 0.5, y + rise, z + s * length * 0.5]
    shapes.push(capsule([x, y, z], bend, 1.1, kind, { op: 'fill', fill: 0.85 }))
    shapes.push(capsule(bend, [x + c * length, y + rise * 0.3, z + s * length], 1, kind, { op: 'fill', fill: 0.7 }))
  }
  return shapes
}

/** A bush, half the time with red berries on its upper side. */
function bush(x: number, y: number, z: number, random: () => number): Shape<Kind>[] {
  const radius = vox(0.5 + random() * 0.5)
  const shapes: Shape<Kind>[] = [ball([x, y, z], radius, 'bush', { fill: 0.8 })]
  if (random() < 0.5) {
    for (let n = 4 + Math.floor(random() * 6); n > 0; n--) {
      const [dx, dy, dz] = direction(random)
      const [px, py, pz] = [Math.round(x + dx * radius * 0.9), Math.round(y + Math.abs(dy) * radius * 0.9), Math.round(z + dz * radius * 0.9)]
      // On a site.
      const at: Point = [px, py + ((px + py + pz) & 1), pz]
      shapes.push(box(at, [at[0] + 1, at[1] + 1, at[2] + 1], 'berry', { op: 'paint' }))
    }
  }
  return shapes
}

/** One to three mushrooms: fly agarics (white-spotted red caps) or boletes. */
function mushrooms(x: number, y: number, z: number, kind: Kind, random: () => number): Shape<Kind>[] {
  const shapes: Shape<Kind>[] = []
  for (let n = 1 + Math.floor(random() * 3); n > 0; n--) {
    const [mx, mz] = [x + Math.floor(random() * 5) - 2, z + Math.floor(random() * 5) - 2]
    const stem = 1 + Math.floor(random() * 2)
    const cap: Point = [mx + 0.5, y + stem, mz]
    const radii: Point = [1.4 + random() * 0.4, 1, 1.4 + random() * 0.4]
    shapes.push(box([mx, y, mz], [mx + 2, y + stem, mz + 1], 'mushroomStem'), ellipsoid(cap, radii, kind))
    if (kind === 'amanita') shapes.push(ellipsoid(cap, radii, 'amanitaSpot', { op: 'paint', fill: (_x, py) => (py > cap[1] ? 0.35 : 0) }))
  }
  return shapes
}

/** A half-buried rock with moss on its upper side. */
function boulder(x: number, ground: number, z: number, random: () => number): Shape<Kind>[] {
  const rx = vox(0.8 + random() * 1.4)
  const ry = rx * (0.6 + random() * 0.3)
  const rz = vox(0.8 + random() * 1.4)
  return [
    ellipsoid([x, ground, z], [rx, ry, rz], 'stone', { op: 'paint' }),
    ellipsoid([x, ground, z], [rx, ry, rz], 'mossyStone', { op: 'paint', fill: (_x, y) => (y - ground > ry * 0.4 ? 0.7 : 0) }),
  ]
}

/** A fallen trunk lying across the ground, with moss on top. */
function fallenLog(x: number, z: number, ground: (x: number, z: number) => number, random: () => number): Shape<Kind>[] {
  const length = vox(4 + random() * 5)
  const angle = random() * Math.PI
  const dx = Math.cos(angle)
  const dz = Math.sin(angle)
  const [ex, ez] = [x + dx * length, z + dz * length]
  const lift = LOG_RADIUS
  const shapes: Shape<Kind>[] = [capsule([x, ground(x, z) + lift, z], [ex, ground(Math.round(ex), Math.round(ez)) + lift, ez], LOG_RADIUS, 'deadwood')]
  // Moss in patches on top.
  for (let i = 0; i <= length; i += vox(0.5)) {
    const px = Math.round(x + dx * i)
    const pz = Math.round(z + dz * i)
    if (random() < 0.5) shapes.push(ball([px, ground(px, pz) + lift * 2 + vox(0.25), pz], vox(0.3), 'moss'))
  }
  return shapes
}
