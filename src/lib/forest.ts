import type { Vec3 } from './world'
import { isSite, type VoxelGrid } from './voxel-grid'
import { fbm, hash, rng, smoothstep, valueNoise } from './noise'
import { areaProfile, ball, box, boundsOfShapes, capsule, ellipsoid, generate, lathe, type Box, type Cell, type Field, type Generator, type Kinds, type Node, type Point, type Population, type Shape } from './generator'

/**
 * Metres per lattice unit: voxels are FCC sites (voxel-grid.ts), nearest
 * neighbours √2 units (0.35 m) apart. Terrain and plant sizes below are in
 * metres, converted with vox(), so the scene keeps its proportions if this
 * changes. Shapes (the ground's height, trunks, balls, branches) are
 * sampled at the lattice's sites.
 * @internal
 */
export const UNIT_SIZE = 0.25
const vox = (metres: number) => Math.round(metres / UNIT_SIZE)
/** Radius of branches and fallen logs, in units: thick enough to stay one piece on the lattice. */
const BRANCH_RADIUS = 0.3 / UNIT_SIZE

/**
 * The part of the (endless) forest the app shows, in units: 96 m wide,
 * 160 m deep, heights from -8 m to 64 m.
 */
const EXTENT: Box = { minX: vox(-48), minY: vox(-8), minZ: vox(-32), maxX: vox(48), maxY: vox(64), maxZ: vox(128) }

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
  flower: { type: 6, colors: ['#e8d34a', '#c9b1ff', '#f2f2f2', '#e86a6a'] },
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
const TREES: Record<Species, (x: number, ground: number, z: number, random: () => number) => Node<Kind>> = {
  spruce: spruceTree,
  birch: birchTree,
  oak: oakTree,
}
/** The colour of each species' crowns from far away. */
const FOLIAGE: Record<Species, Kind> = { spruce: 'spruce', birch: 'birchGold', oak: 'oakOrange' }

/**
 * An autumn forest valley: a winding path along the valley floor, wooded
 * slopes rising on both sides, and a ridge closing the view at the far end.
 * Spruce, birch and oak, with ferns, bushes, boulders and fallen logs.
 * Endless: the app shows `EXTENT` of it.
 * @internal
 */
export function forest(seed: number): { generator: Generator<Kind>; spawn: Vec3 } {
  const s = (k: number) => (seed | 0) + k * 1013904223
  const pathPhase = hash(s(1), 0, 0) * 100
  /** Path centre line, in metres. */
  const pathX = (zm: number) => 9 * Math.sin(zm / 31 + pathPhase) + 5 * Math.sin(zm / 13 + pathPhase * 2)

  const groundMetres = (xm: number, zm: number) => {
    const fromPath = Math.abs(xm - pathX(zm))
    const hills = (fbm(s(2), xm / 45, zm / 45) - 0.5) * 10 + (valueNoise(s(3), xm / 6, zm / 6) - 0.5) * 0.8
    const valley = smoothstep(8, 42, fromPath) * 18
    const ridge = smoothstep(95, 128, zm) * 22
    const pathDip = (1 - smoothstep(1, 2.2, fromPath)) * 0.3
    return hills + valley + ridge - pathDip
  }
  // Every cell of a column of a population's cells asks for the same
  // spots (see spot()), so heights are kept a while.
  const heights = new Map<number, number>()
  /** Ground height in units. */
  const ground = (x: number, z: number) => {
    // Unique while |z| < 2^21 units (500 km).
    const key = x * 4194304 + z
    let h = heights.get(key)
    if (h === undefined) {
      if (heights.size >= 1 << 16) heights.clear()
      h = vox(groundMetres(x * UNIT_SIZE, z * UNIT_SIZE))
      heights.set(key, h)
    }
    return h
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
  const spawnZ = EXTENT.minZ + vox(1)
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
  const profiles = Object.fromEntries(SPECIES.map(species => {
    const samples = Array.from({ length: 64 }, (_, i) => TREES[species](0, 0, 0, rng(s(17) + i)))
    return [species, areaProfile(samples, 1)]
  })) as Record<Species, (height: number) => number>
  const trees: Population<Kind> = {
    name: 'trees',
    order: 1,
    cell: TREE_CELL,
    reach: [vox(9), vox(26), vox(9)],
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
      return TREES[species](x, y, z, random)
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
        for (let from = 0; from < vox(26); from += spacing) {
          let area = 0
          for (let h = from; h < from + spacing; h++) area += SPECIES.reduce((sum, sp) => sum + chances[sp] * profiles[sp](h), 0)
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

  // Undergrowth, per column of units: (0.25 m)², in square patches N wide
  // around each cell's centre (the patches of one column of cells tile the ground).
  // A patch belongs to the cell its centre's ground is in: half a unit up,
  // that is never on the boundary, so always in one of the column's cells.
  // It fills only empty sites.
  const PLANTS = 16
  const plants: Population<Kind> = {
    name: 'plants',
    order: 3,
    cell: PLANTS,
    reach: [0, vox(10), 0],
    size: 1,
    anchor: ({ centre: [cx, , cz] }) => [cx, ground(cx, cz) + 0.5, cz],
    make(_, { centre: [cx, , cz] }) {
      const shapes: Shape<Kind>[] = []
      for (let x = cx - PLANTS / 2; x < cx + PLANTS / 2; x++) {
        for (let z = cz - PLANTS / 2; z < cz + PLANTS / 2; z++) {
          const c = column(x, z)
          const y = c.height + 1
          if (c.onPath || !isSite(x, y, z)) continue
          const r = hash(s(15), x, z)
          const random = rng(Math.floor(hash(s(16), x, z) * 4294967296))
          const site = (kind: Kind) => box([x, y, z], [x + 1, y + 1, z + 1], kind)
          if (c.surface === 'grass') {
            if (r < 0.04) shapes.push(site('tallGrass'))
            else if (r < 0.055) shapes.push(site('flower'))
          } else if (c.surface !== 'stone' && c.surface !== 'mossyStone') {
            if (r < 0.0125) shapes.push(...fern(x, y, z, c.conifer > 0.5 ? 'fern' : 'bracken', random))
            else if (r < 0.016) shapes.push(ball([x, y, z], vox(0.5 + random() * 0.5), 'bush', { fill: 0.8 }))
            else if (r < 0.017) shapes.push(site('flower'))
          }
        }
      }
      return shapes.length > 0 ? { shapes } : undefined
    },
  }

  return {
    generator: { kinds: KINDS, seed: s(4), layers: [terrain, trees, boulders, logs, plants] },
    spawn,
  }
}

/** The forest's `EXTENT`, and where the camera starts. */
export function generateForest(seed: number): { grid: VoxelGrid; spawn: Vec3 } {
  const { generator, spawn } = forest(seed)
  return { grid: generate(generator, EXTENT), spawn }
}

// ---------------------------------------------------------------------------
// Things. `random` picks their shapes.

/**
 * A tree of `shapes`, with the parts its crown hides as a child: from far
 * away, hidden parts would show, as each shape is drawn on its own. Its
 * crown of `clusters`, once they are smaller than the sites, is drawn as
 * one solid of their volume.
 */
function tree(shapes: Shape<Kind>[], hidden: Shape<Kind>[], crown?: { trunk: Shape<Kind>; clusters: Shape<Kind>[]; fill: number }): Node<Kind> {
  const node = { shapes, bounds: boundsOfShapes([...shapes, ...hidden]), children: () => [{ name: 'hidden', size: vox(0.5), shapes: hidden }] }
  if (!crown) return node
  const { trunk, clusters, fill } = crown
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

/** Conical spruce with drooping whorls of branches. */
function spruceTree(x: number, ground: number, z: number, random: () => number): Node<Kind> {
  const height = vox(14 + random() * 10)
  const width = height >= vox(20) ? vox(1) : vox(0.5)
  const top = ground + height
  const base = ground + Math.round(height * (0.15 + random() * 0.15))
  const shapes = [trunk(x, ground, z, base, width, 'spruceBark')]
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
  const fill = (px: number, py: number, pz: number) => 0.95 - 0.35 * (Math.sqrt((px - cx) ** 2 + (pz - cz) ** 2) / r(py)) ** 3
  shapes.push(lathe([cx, from, cz], [cx, to, cz], t => r(Math.round(from + t * (to - from))), 'spruce', { fill }))
  return tree(shapes, hidden)
}

/** Slender white birch with a wavy trunk and a loose, clumpy crown. */
function birchTree(x: number, ground: number, z: number, random: () => number): Node<Kind> {
  const height = vox(12 + random() * 7)
  const [shapes, marks]: Shape<Kind>[][] = [[], []]
  // Two units square, so the trunk stays one piece on the lattice; it shifts by a unit now and then.
  let [tx, tz, from] = [x, z, ground - 1]
  for (let y = ground - 1; y <= ground + height + 1; y++) {
    const end = y === ground + height + 1
    if (end || ((y - ground) % vox(3) === 0 && y - ground > vox(4) && random() < 0.35)) {
      shapes.push(box([tx, from, tz], [tx + 2, y, tz + 2], 'birchBark', { op: 'paint' }))
      if (end) break
      if (random() < 0.5) tx += random() < 0.5 ? -1 : 1
      else tz += random() < 0.5 ? -1 : 1
      from = y
    }
    // Black marks in bands half a metre tall.
    if ((y - ground) % vox(0.5) === 0 && random() < 0.18) marks.push(box([tx, y, tz], [tx + 2, y + vox(0.5), tz + 2], 'birchMark', { op: 'paint' }))
  }
  shapes.push(...marks)
  const foliage: Kind = random() < 0.65 ? 'birchGold' : 'birchGreen'
  const crownBottom = ground + height * 0.45
  const bole = box([x, ground - 1, z], [x + 2, crownBottom, z + 2], 'birchBark', { op: 'paint' })
  const clusterShapes: Shape<Kind>[] = []
  const crownTop = ground + height + vox(1)
  const cy = (crownBottom + crownTop) / 2
  const ry = (crownTop - crownBottom) / 2
  const rx = vox(2 + random() * 1.2)
  const clusters = 12 + Math.floor(random() * 6)
  const branches: Shape<Kind>[] = []
  for (let i = 0; i < clusters; i++) {
    // Uniform point in the crown ellipsoid.
    let ox, oy, oz
    do {
      ox = random() * 2 - 1
      oy = random() * 2 - 1
      oz = random() * 2 - 1
    } while (ox * ox + oy * oy + oz * oz > 1)
    const p: Point = [tx + ox * rx, cy + oy * ry, tz + oz * rx]
    branches.push(capsule([tx + 0.5, p[1] - vox(0.5), tz + 0.5], p, BRANCH_RADIUS, 'birchBark'))
    clusterShapes.push(ball(p, vox(0.8 + random() * 0.6), foliage, { fill: 0.7 }))
  }
  return tree([...shapes, ...clusterShapes], branches, { trunk: bole, clusters: clusterShapes, fill: 0.7 })
}

/** Broad oak: thick trunk, a few branches each ending in a leaf cluster. */
function oakTree(x: number, ground: number, z: number, random: () => number): Node<Kind> {
  const height = vox(12 + random() * 8)
  const trunkTop = ground + Math.round(height * (0.45 + random() * 0.1))
  const width = vox(1)
  const shapes = [trunk(x, ground, z, trunkTop, width, 'bark')]
  const roll = random()
  const foliage: Kind = roll < 0.45 ? 'oakGreen' : roll < 0.75 ? 'oakOrange' : 'oakGold'
  const cx = x + (width - 1) / 2
  const cz = z + (width - 1) / 2
  const branches = 4 + Math.floor(random() * 3)
  const ends: Point[] = [[cx, ground + height - vox(2), cz]]
  const hidden: Shape<Kind>[] = []
  for (let i = 0; i < branches; i++) {
    const angle = (i / branches) * Math.PI * 2 + random() * 0.8
    const reach = vox(2.5 + random() * 2.5)
    const startY = trunkTop - Math.floor(random() * vox(2))
    const end: Point = [cx + Math.cos(angle) * reach, Math.min(startY + vox(2 + random() * 3), ground + height - vox(1.5)), cz + Math.sin(angle) * reach]
    hidden.push(capsule([cx, startY, cz], end, BRANCH_RADIUS, 'bark'))
    ends.push(end)
  }
  const clusters = ends.map(end => ball(end, vox(1.8 + random() * 1.2), foliage, { fill: 0.8 }))
  return tree([...shapes, ...clusters], hidden, { trunk: shapes[0], clusters, fill: 0.8 })
}

/** A fern or bracken clump about a metre across: fronds spreading from the middle. */
function fern(x: number, y: number, z: number, kind: Kind, random: () => number): Shape<Kind>[] {
  const shapes = [ball([x, y, z], 1.5, kind)]
  if (random() < 0.5) shapes.push(ball([x, y + vox(0.5), z], 1.5, kind))
  for (const [dx, dz] of [[1, 0], [-1, 0], [0, 1], [0, -1]]) {
    if (random() < 0.6) shapes.push(ball([x + dx * vox(0.5), y, z + dz * vox(0.5)], 1.5, kind))
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
  const lift = BRANCH_RADIUS * 1.5
  const shapes: Shape<Kind>[] = [capsule([x, ground(x, z) + lift, z], [ex, ground(Math.round(ex), Math.round(ez)) + lift, ez], BRANCH_RADIUS * 1.5, 'deadwood')]
  // Moss in patches on top.
  for (let i = 0; i <= length; i += vox(0.5)) {
    const px = Math.round(x + dx * i)
    const pz = Math.round(z + dz * i)
    if (random() < 0.5) shapes.push(ball([px, ground(px, pz) + lift * 2 + 1, pz], 1.2, 'moss'))
  }
  return shapes
}
