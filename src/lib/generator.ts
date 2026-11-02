/**
 * World generators: a world is fields and populations of things made of
 * shapes; the framework draws any box of it alone, at any level of detail.
 * See docs/generators.md.
 */

import { VoxelGrid, type Material } from './voxel-grid'
import { hash, rng } from './noise'

/** A point in lattice units. */
export type Point = readonly [number, number, number]

/** Half-open box: x in [minX, maxX), y and z likewise. */
export interface Box {
  minX: number
  minY: number
  minZ: number
  maxX: number
  maxY: number
  maxZ: number
}

/** Material kinds by name; each voxel gets one shade of its kind, picked by position. */
export type Kinds<K extends string> = Record<K, { type: number; colors: readonly string[] }>

/** How something meets what is drawn before it: `fill` empty sites only, `paint` over them, or `carve` them empty. */
type Op = 'fill' | 'paint' | 'carve'

/** The chance to keep each site of a shape at level 0: a number, or one per position. */
type Fill = number | ((x: number, y: number, z: number) => number)

/** A solid, in lattice units. Made by `ball`, `ellipsoid`, `capsule` and `box`; the framework draws it at any level. */
export interface Shape<K extends string> {
  readonly kind: K
  readonly op: Op
  readonly fill: Fill
  /** Every site it draws at level 0 is inside. */
  readonly bounds: Box
  /** Its sizes along its own three axes. */
  readonly extents: readonly [number, number, number]
  readonly centre: Point
  /** Surface area of an opaque body that stops as many rays. */
  readonly area: number
  /** Whether a point is inside the shape with each of its sizes made at least `min`. */
  contains(x: number, y: number, z: number, min: number): boolean
}

/** A thing: its shapes, and finer things in it. */
export interface Node<K extends string> {
  readonly shapes: readonly Shape<K>[]
  /** Required with children: holds everything the node and its children draw. Otherwise the shapes' bounds. */
  readonly bounds?: Box
  /** The finer things reaching `clip` (lattice units); each drawn while sites are at most its `size` apart. */
  children?(clip: Box): readonly Child<K>[]
  /**
   * What the node looks like once its parts are smaller than the sites:
   * drawn instead of its shapes and children while sites are more than
   * `detail` apart, e.g. a crown's outline for its leaf clusters, whose
   * shapes, drawn one by one, would stop rays twice where they overlap.
   */
  readonly far?: { readonly detail: number; readonly shapes: readonly Shape<K>[] }
}

/**
 * A node's finer thing.
 * @internal
 */
export interface Child<K extends string> extends Node<K> {
  /** Unique among its siblings. */
  readonly name: string
  readonly size: number
}

/** Part of a line of sites: [from, to) along the line, solid, or a medium if it has `extinction`. */
interface Span<K extends string> {
  readonly from: number
  readonly to: number
  readonly kind: K
  readonly extinction?: number
}

/** A medium: its sites are kept so that a ray gets d far through it with probability exp(−extinction · d). */
interface Medium<K extends string> {
  readonly kind: K
  readonly extinction: number
}

/**
 * Something everywhere, given by position alone: the ground, caves, fog.
 * Either `at` each site, or `spans` along each line of sites parallel to
 * `axis` (heightfields). `spacing` is the level's: a field may smooth itself
 * to it.
 */
export type Field<K extends string> = {
  readonly name: string
  readonly order: number
  /** `paint` if not given. */
  readonly op?: Op
} & (
  | { at(x: number, y: number, z: number, spacing: number): K | Medium<K> | undefined }
  | { readonly axis: 0 | 1 | 2; spans(u: number, v: number, spacing: number): readonly Span<K>[] }
)

/**
 * A cell of a population: the rhombic dodecahedron of the points nearer to
 * its centre, N · (i, j, k) with i + j + k even, than to any other site of
 * the lattice N times coarser (N·FCC); and random sequences that depend on
 * the cell and a name only.
 */
export interface Cell {
  readonly i: number
  readonly j: number
  readonly k: number
  readonly centre: Point
  random(name: string): () => number
}

/**
 * Things placed in the cells of N·FCC, at most one per cell: the framework
 * asks each cell for an anchor, and a cell owns the thing only if the anchor
 * is in it (a tree belongs to the cell its foot is in). A cell reaches N
 * from its centre along each axis; an anchor picked evenly within that reach
 * gives things as dense as one per cube 2N wide would.
 */
export interface Population<K extends string> {
  readonly name: string
  readonly order: number
  /** N, the spacing of the cells' lattice; a power of two gives the renderer's cells (cells.ts). */
  readonly cell: number
  /** How far a thing reaches past N from its cell's centre: in every direction, or per axis. */
  readonly reach: number | Point
  /** Things are made while sites are at most this far apart; beyond, `far` is drawn instead, if any. */
  readonly size: number
  anchor(cell: Cell): Point | undefined
  make(at: Point, cell: Cell): Node<K> | undefined
  /** The things seen from far away: a medium as dense as they are. */
  readonly far?: Field<K>
}

export interface Generator<K extends string> {
  readonly kinds: Kinds<K>
  /** Seeds the cells' random sequences, the sites kept by fills and media, and shades. */
  readonly seed: number
  readonly layers: readonly (Field<K> | Population<K>)[]
}

const DOCS = 'docs/generators.md'
/** A voxel's surface area at level 0, and its mean chord (Cauchy: 4 · volume / area). */
const VOXEL_AREA = 6 * Math.SQRT2
const CHORD = (4 * 2) / VOXEL_AREA

// ---------------------------------------------------------------------------
// Shapes

interface Options { fill?: Fill; op?: Op }

/** The area of an opaque body stopping as many rays as one of `area` and mean chord `chord`, filled with chance `fill`. */
const effectiveArea = (area: number, chord: number, fill: Fill, centre: Point) => {
  const f = typeof fill === 'number' ? fill : fill(...centre)
  return area * (1 - (1 - f) ** (chord / CHORD))
}

export function ellipsoid<K extends string>(centre: Point, radii: Point, kind: K, { fill = 1, op = 'fill' }: Options = {}): Shape<K> {
  const [cx, cy, cz] = centre
  const [a, b, c] = radii
  const p = 1.6075 // Knud Thomsen's approximation of the surface area
  const area = 4 * Math.PI * (((a * b) ** p + (a * c) ** p + (b * c) ** p) / 3) ** (1 / p)
  return {
    kind, op, fill, centre,
    bounds: { minX: Math.ceil(cx - a), minY: Math.ceil(cy - b), minZ: Math.ceil(cz - c), maxX: Math.floor(cx + a) + 1, maxY: Math.floor(cy + b) + 1, maxZ: Math.floor(cz + c) + 1 },
    extents: [2 * a, 2 * b, 2 * c],
    area: effectiveArea(area, ((16 / 3) * Math.PI * a * b * c) / area, fill, centre),
    contains(x, y, z, min) {
      const r = min / 2
      return ((x - cx) / Math.max(a, r)) ** 2 + ((y - cy) / Math.max(b, r)) ** 2 + ((z - cz) / Math.max(c, r)) ** 2 <= 1
    },
  }
}

export function ball<K extends string>(centre: Point, radius: number, kind: K, options?: Options): Shape<K> {
  return ellipsoid(centre, [radius, radius, radius], kind, options)
}

/** The points within `radius` of the segment from p to q: branches, logs. */
export function capsule<K extends string>(p: Point, q: Point, radius: number, kind: K, { fill = 1, op = 'paint' }: Options = {}): Shape<K> {
  const d = [q[0] - p[0], q[1] - p[1], q[2] - p[2]]
  const length2 = d[0] ** 2 + d[1] ** 2 + d[2] ** 2
  const length = Math.sqrt(length2)
  const area = 2 * Math.PI * radius * length + 4 * Math.PI * radius ** 2
  const volume = Math.PI * radius ** 2 * length + (4 / 3) * Math.PI * radius ** 3
  const centre: Point = [(p[0] + q[0]) / 2, (p[1] + q[1]) / 2, (p[2] + q[2]) / 2]
  const lo = [0, 1, 2].map(i => Math.ceil(Math.min(p[i], q[i]) - radius))
  const hi = [0, 1, 2].map(i => Math.floor(Math.max(p[i], q[i]) + radius) + 1)
  return {
    kind, op, fill, centre,
    bounds: { minX: lo[0], minY: lo[1], minZ: lo[2], maxX: hi[0], maxY: hi[1], maxZ: hi[2] },
    extents: [length + 2 * radius, 2 * radius, 2 * radius],
    area: effectiveArea(area, (4 * volume) / area, fill, centre),
    contains(x, y, z, min) {
      const v = [x - p[0], y - p[1], z - p[2]]
      const t = length2 === 0 ? 0 : Math.max(0, Math.min(1, (v[0] * d[0] + v[1] * d[1] + v[2] * d[2]) / length2))
      return (v[0] - t * d[0]) ** 2 + (v[1] - t * d[1]) ** 2 + (v[2] - t * d[2]) ** 2 <= Math.max(radius, min / 2) ** 2
    },
  }
}

/**
 * A solid of revolution: the points whose distance from the segment from p
 * to q is at most `radius(t)`, t from 0 at p to 1 at q (crowns, cones).
 */
export function lathe<K extends string>(p: Point, q: Point, radius: (t: number) => number, kind: K, { fill = 1, op = 'fill' }: Options = {}): Shape<K> {
  const d = [q[0] - p[0], q[1] - p[1], q[2] - p[2]]
  const length2 = d[0] ** 2 + d[1] ** 2 + d[2] ** 2
  const length = Math.sqrt(length2)
  // Area and volume by slices. The shadow it casts is its convex hull's, so the area is the hull's:
  // a crown's whorls have undersides that cast no more shadow (Cauchy holds for convex bodies).
  const n = Math.max(8, Math.ceil(length))
  const r = Array.from({ length: n + 1 }, (_, i) => radius(i / n))
  const hull: number[] = []
  for (let i = 0; i <= n; i++) {
    const cross = (a: number, b: number) => (b - a) * (r[i] - r[a]) - (i - a) * (r[b] - r[a])
    while (hull.length >= 2 && cross(hull[hull.length - 2], hull[hull.length - 1]) >= 0) hull.pop()
    hull.push(i)
  }
  let [area, volume] = [Math.PI * (r[0] ** 2 + r[n] ** 2), 0]
  for (let h = 0; h + 1 < hull.length; h++) {
    const [a, b] = [hull[h], hull[h + 1]]
    area += Math.PI * (r[a] + r[b]) * Math.hypot(((b - a) * length) / n, r[b] - r[a])
  }
  for (let i = 0; i < n; i++) volume += (Math.PI * (r[i] ** 2 + r[i + 1] ** 2) / 2) * (length / n)
  const widest = Math.max(...r)
  const centre: Point = [(p[0] + q[0]) / 2, (p[1] + q[1]) / 2, (p[2] + q[2]) / 2]
  // Discs across the axis reach out by widest · sin(angle to the axis) along each world axis.
  const out = d.map(v => widest * Math.sqrt(Math.max(0, 1 - (v * v) / length2)))
  const lo = [0, 1, 2].map(i => Math.ceil(Math.min(p[i], q[i]) - out[i]))
  const hi = [0, 1, 2].map(i => Math.floor(Math.max(p[i], q[i]) + out[i]) + 1)
  return {
    kind, op, fill, centre,
    bounds: { minX: lo[0], minY: lo[1], minZ: lo[2], maxX: hi[0], maxY: hi[1], maxZ: hi[2] },
    extents: [length, 2 * widest, 2 * widest],
    area: effectiveArea(area, (4 * volume) / area, fill, centre),
    contains(x, y, z, min) {
      const [vx, vy, vz] = [x - p[0], y - p[1], z - p[2]]
      const t = (vx * d[0] + vy * d[1] + vz * d[2]) / length2
      const reach = (length * Math.max(0, min - length)) / 2 / length2
      if (t < -reach || t > 1 + reach) return false
      const c = Math.max(0, Math.min(1, t))
      const away = (vx - c * d[0]) ** 2 + (vy - c * d[1]) ** 2 + (vz - c * d[2]) ** 2
      // Past the widest disc, without asking for the radius.
      const limit = Math.max(widest, min / 2) ** 2
      return away <= limit && away <= Math.max(radius(c), min / 2) ** 2
    },
  }
}

/** The points of [min, max): at level 0, the sites with min ≤ site < max. */
export function box<K extends string>(min: Point, max: Point, kind: K, { fill = 1, op = 'fill' }: Options = {}): Shape<K> {
  const size = [0, 1, 2].map(i => max[i] - min[i])
  const area = 2 * (size[0] * size[1] + size[1] * size[2] + size[0] * size[2])
  const centre: Point = [(min[0] + max[0]) / 2, (min[1] + max[1]) / 2, (min[2] + max[2]) / 2]
  return {
    kind, op, fill, centre,
    bounds: { minX: Math.ceil(min[0]), minY: Math.ceil(min[1]), minZ: Math.ceil(min[2]), maxX: Math.ceil(max[0]), maxY: Math.ceil(max[1]), maxZ: Math.ceil(max[2]) },
    extents: [size[0], size[1], size[2]],
    area: effectiveArea(area, (4 * size[0] * size[1] * size[2]) / area, fill, centre),
    contains(x, y, z, m) {
      return [x, y, z].every((v, i) => (size[i] >= m ? min[i] <= v && v < max[i] : Math.abs(v - centre[i]) <= m / 2))
    },
  }
}

/**
 * Draws a shape at sites `s` apart, by how many of its sizes are under s:
 * - none: the sites inside, each kept with the chance that one of the s
 *   sites of level 0 it stands for along a ray is, so as many rays get through;
 * - one, a sheet (a wall): grown to a site thick, each site kept with the
 *   chance that a ray crossing the sheet is stopped;
 * - two or three, a rod or a speck: the sites near it, each kept with one
 *   chance that makes them stop as many rays as the shape does on average
 *   over directions (Cauchy: a body's mean shadow is a quarter of its area).
 * A site's chance is compared with a hash of its position in lattice units,
 * so the sites kept at one level are kept at the coarser ones with as high a chance.
 */
function drawShape<K extends string>(shape: Shape<K>, seed: number, s: number, clip: Box, put: (x: number, y: number, z: number) => void) {
  const small = shape.extents.filter(e => e < s).length
  const fill = (x: number, y: number, z: number) => (typeof shape.fill === 'number' ? shape.fill : shape.fill(x, y, z))
  const kept = (x: number, y: number, z: number, chance: number) => chance >= 1 || hash(seed, x * s, y * s, z * s) < chance
  if (small === 3) {
    const [x, y, z] = nearestSite(shape.centre.map(v => v / s))
    if (inBox(clip, x, y, z) && kept(x, y, z, shape.area / (VOXEL_AREA * s * s))) put(x, y, z)
    return
  }
  const min = [0, s, 1.6 * s][small]
  const b = shape.bounds
  const [x0, y0, z0] = [b.minX, b.minY, b.minZ].map(v => Math.ceil((v - min / 2) / s))
  const [x1, y1, z1] = [b.maxX, b.maxY, b.maxZ].map(v => Math.floor((v - 1 + min / 2) / s))
  const sites = (within: Box, visit: (x: number, y: number, z: number) => void) => {
    for (let z = Math.max(z0, within.minZ); z <= Math.min(z1, within.maxZ - 1); z++) {
      for (let y = Math.max(y0, within.minY); y <= Math.min(y1, within.maxY - 1); y++) {
        const from = Math.max(x0, within.minX)
        for (let x = from + ((from + y + z) & 1); x <= Math.min(x1, within.maxX - 1); x += 2) {
          if (shape.contains(x * s, y * s, z * s, min)) visit(x, y, z)
        }
      }
    }
  }
  if (small === 0) {
    sites(clip, (x, y, z) => { if (kept(x, y, z, 1 - (1 - fill(x * s, y * s, z * s)) ** s)) put(x, y, z) })
  } else if (small === 1) {
    const thickness = Math.min(...shape.extents)
    sites(clip, (x, y, z) => { if (kept(x, y, z, 1 - (1 - fill(x * s, y * s, z * s)) ** (thickness / CHORD))) put(x, y, z) })
  } else {
    // Counted whole, so every box gets the same chance.
    let count = 0
    sites(EVERYWHERE, () => { count++ })
    const chance = shape.area / (count * VOXEL_AREA * s * s)
    sites(clip, (x, y, z) => { if (kept(x, y, z, chance)) put(x, y, z) })
  }
}

// ---------------------------------------------------------------------------
// Generating

type Putter<K extends string> = (kind: K, op: Op) => (x: number, y: number, z: number) => void

/** The materials of the grids `generate` makes: ids are the same in all of them. */
export function materialsOf<K extends string>(generator: Generator<K>): Material[] {
  return new Palette(generator.kinds, generator.seed).materials
}

/**
 * The voxels of `box` at `level`: sites 2^level apart, site (x, y, z) at
 * 2^level · (x, y, z) in lattice units, the box in the level's units. The
 * grid may reach one site further in x (its rows start and end on an even x).
 */
export function generate<K extends string>(generator: Generator<K>, box: Box, level = 0): VoxelGrid {
  const s = 2 ** level
  const palette = new Palette(generator.kinds, generator.seed)
  const grid = new VoxelGrid(box, palette.materials)
  const clip: Box = { minX: grid.minX, minY: grid.minY, minZ: grid.minZ, maxX: grid.minX + grid.nx, maxY: grid.minY + grid.ny, maxZ: grid.minZ + grid.nz }
  const putters: Record<Op, Map<K, (x: number, y: number, z: number) => void>> = { fill: new Map(), paint: new Map(), carve: new Map() }
  const putter: Putter<K> = (kind, op) => {
    let put = putters[op].get(kind)
    if (!put) {
      const shade = palette.shade(kind)
      put = (x, y, z) => {
        if (op === 'fill' && grid.get(x, y, z) !== 0) return
        grid.set(x, y, z, op === 'carve' ? 0 : shade(x * s, y * s, z * s))
      }
      putters[op].set(kind, put)
    }
    return put
  }
  for (const { draw } of drawablesIn(generator, clip, s)) draw(clip, putter)
  return grid
}

interface Drawable<K extends string> { order: number; key: string; draw: (clip: Box, putter: Putter<K>) => void }

/** What draws into `clip` (sites s apart), in drawing order: by layer, then key. */
function drawablesIn<K extends string>({ layers, seed }: Generator<K>, clip: Box, s: number): Drawable<K>[] {
  // The lattice units the sites of `clip` cover, with what a shape may grow by at this level.
  const units = widen({ minX: clip.minX * s, minY: clip.minY * s, minZ: clip.minZ * s, maxX: (clip.maxX - 1) * s + 1, maxY: (clip.maxY - 1) * s + 1, maxZ: (clip.maxZ - 1) * s + 1 }, s)
  const drawables: Drawable<K>[] = []
  for (const layer of layers) {
    if (!('anchor' in layer)) {
      drawables.push({ order: layer.order, key: layer.name, draw: (c, putter) => { drawField(layer, seed, s, c, putter) } })
      continue
    }
    if (s > layer.size) {
      const { far } = layer
      if (far) drawables.push({ order: layer.order, key: layer.name, draw: (c, putter) => { drawField(far, seed, s, c, putter) } })
      continue
    }
    const visit = (node: Node<K>, key: string, bounds: Box) => {
      if (!overlaps(bounds, units)) return
      const far = node.far && s > node.far.detail
      const shapes = far ? (node.far as { shapes: readonly Shape<K>[] }).shapes : node.shapes
      drawables.push({
        order: layer.order,
        key,
        draw: (c, putter) => {
          shapes.forEach((shape, i) => { drawShape(shape, hashText(`${key}#${far ? 'far' : ''}${String(i)}`, seed), s, c, putter(shape.kind, shape.op)) })
        },
      })
      if (far) return
      const names = new Set<string>()
      for (const child of node.children?.(units) ?? []) {
        if (names.has(child.name)) throw new RangeError(`${key} has two children named ${child.name}; see "Things" in ${DOCS}`)
        names.add(child.name)
        const childBounds = boundsOf(child)
        if (!inside(childBounds, bounds)) throw new RangeError(`${key}/${child.name} reaches past its parent: ${boxText(childBounds)} is not in ${boxText(bounds)}; see "Things" in ${DOCS}`)
        if (s <= child.size) visit(child, `${key}/${child.name}`, childBounds)
      }
    }
    const { cell: n, name } = layer
    const [rx, ry, rz] = typeof layer.reach === 'number' ? [layer.reach, layer.reach, layer.reach] : layer.reach
    // Cells whose reach meets `units`: centres within N + reach of it.
    const range = (min: number, max: number, reach: number) => [Math.ceil((min - n - reach) / n), Math.floor((max - 1 + n + reach) / n)]
    const [[i0, i1], [j0, j1], [k0, k1]] = [range(units.minX, units.maxX, rx), range(units.minY, units.maxY, ry), range(units.minZ, units.maxZ, rz)]
    for (let i = i0; i <= i1; i++) {
      for (let j = j0; j <= j1; j++) {
        for (let k = k0 + ((k0 + i + j) & 1); k <= k1; k += 2) {
          const centre: Point = [i * n, j * n, k * n]
          const at: Cell = { i, j, k, centre, random: stream => rng(Math.floor(hash(hashText(`${name}:${stream}`, seed), i, j, k) * 4294967296)) }
          const anchor = layer.anchor(at)
          if (!anchor) continue
          const [oi, oj, ok] = nearestSite([anchor[0] / n, anchor[1] / n, anchor[2] / n])
          if (oi !== i || oj !== j || ok !== k) continue
          const node = layer.make(anchor, at)
          if (!node) continue
          const key = `${name} ${String(i)},${String(j)},${String(k)}`
          const bounds = boundsOf(node)
          const reach = { minX: centre[0] - n - rx, minY: centre[1] - n - ry, minZ: centre[2] - n - rz, maxX: centre[0] + n + rx, maxY: centre[1] + n + ry, maxZ: centre[2] + n + rz }
          if (!inside(bounds, reach)) throw new RangeError(`${key} reaches past its population's reach: ${boxText(bounds)} is not in ${boxText(reach)}; see "Populations" in ${DOCS}`)
          visit(node, key, bounds)
        }
      }
    }
  }
  return drawables.sort((a, b) => a.order - b.order || (a.key < b.key ? -1 : a.key > b.key ? 1 : 0))
}

/** Draws a field into `clip`; a medium keeps a site with the chance a ray crossing it is stopped. */
function drawField<K extends string>(field: Field<K>, seed: number, s: number, clip: Box, putter: Putter<K>) {
  const op = field.op ?? 'paint'
  const fieldSeed = hashText(field.name, seed)
  const kept = (x: number, y: number, z: number, extinction?: number) =>
    extinction === undefined || hash(fieldSeed, x * s, y * s, z * s) < 1 - Math.exp(-extinction * CHORD * s)
  if ('at' in field) {
    for (let z = clip.minZ; z < clip.maxZ; z++) {
      for (let y = clip.minY; y < clip.maxY; y++) {
        for (let x = clip.minX + ((clip.minX + y + z) & 1); x < clip.maxX; x += 2) {
          const value = field.at(x * s, y * s, z * s, s)
          if (value === undefined) continue
          const [kind, extinction] = typeof value === 'string' ? [value, undefined] : [value.kind, value.extinction]
          if (kept(x, y, z, extinction)) putter(kind, op)(x, y, z)
        }
      }
    }
    return
  }
  // Lines of sites along the axis; (u, v) are the other two coordinates, in order.
  const { axis } = field
  const lo = [clip.minX, clip.minY, clip.minZ]
  const hi = [clip.maxX, clip.maxY, clip.maxZ]
  const [a, b] = [0, 1, 2].filter(i => i !== axis)
  const site = [0, 0, 0]
  for (let u = lo[a]; u < hi[a]; u++) {
    for (let v = lo[b]; v < hi[b]; v++) {
      site[a] = u
      site[b] = v
      for (const { from, to, kind, extinction } of field.spans(u * s, v * s, s)) {
        const put = putter(kind, op)
        const first = Math.max(Math.ceil(from / s), lo[axis])
        for (let w = first + ((first + u + v) & 1); w * s < to && w < hi[axis]; w += 2) {
          site[axis] = w
          if (kept(site[0], site[1], site[2], extinction)) put(site[0], site[1], site[2])
        }
      }
    }
  }
}

/**
 * The mean area per unit length of `nodes` along `axis`, as a function of
 * the coordinate: for a population's far form, from things made at the
 * origin, as seen from far away.
 */
export function areaProfile<K extends string>(nodes: readonly Node<K>[], axis: 0 | 1 | 2): (at: number) => number {
  const profile = new Map<number, number>()
  for (const node of nodes) {
    for (const shape of node.far?.shapes ?? node.shapes) {
      const lo = [shape.bounds.minX, shape.bounds.minY, shape.bounds.minZ][axis]
      const hi = [shape.bounds.maxX, shape.bounds.maxY, shape.bounds.maxZ][axis]
      for (let at = lo; at < hi; at++) profile.set(at, (profile.get(at) ?? 0) + shape.area / (hi - lo) / nodes.length)
    }
  }
  return at => profile.get(Math.floor(at)) ?? 0
}

// ---------------------------------------------------------------------------
// Helpers

/** Shades of the kinds' colours, as material ids picked by a hash of the position in lattice units. */
class Palette<K extends string> {
  readonly materials: Material[] = [{ color: '', type: 0 }] // index 0 = empty
  private readonly shades = new Map<K, number[]>()

  constructor(kinds: Kinds<K>, private readonly seed: number) {
    for (const [kind, { type, colors }] of Object.entries(kinds) as [K, Kinds<K>[K]][]) {
      this.shades.set(kind, colors.map(color => this.materials.push({ color, type }) - 1))
    }
  }

  shade(kind: K): (x: number, y: number, z: number) => number {
    const shades = this.shades.get(kind)
    if (!shades) throw new RangeError(`unknown kind ${kind}`)
    return (x, y, z) => shades[Math.floor(hash(this.seed, x, y, z) * shades.length)]
  }
}

const EVERYWHERE: Box = { minX: -Infinity, minY: -Infinity, minZ: -Infinity, maxX: Infinity, maxY: Infinity, maxZ: Infinity }

/** The site nearest to a point: the one whose voxel holds it (one, also on a boundary). */
function nearestSite(p: readonly number[]): [number, number, number] {
  const r: [number, number, number] = [Math.round(p[0]), Math.round(p[1]), Math.round(p[2])]
  if ((r[0] + r[1] + r[2]) & 1) {
    // Round the coordinate furthest from its integer the other way.
    const [e0, e1, e2] = [Math.abs(p[0] - r[0]), Math.abs(p[1] - r[1]), Math.abs(p[2] - r[2])]
    const i = e0 >= e1 && e0 >= e2 ? 0 : e1 >= e2 ? 1 : 2
    r[i] += p[i] > r[i] ? 1 : -1
  }
  return r
}

function boundsOf<K extends string>(node: Node<K>): Box {
  if (node.bounds) return node.bounds
  if (node.children) throw new RangeError(`a node with children needs bounds; see "Things" in ${DOCS}`)
  return boundsOfShapes(node.shapes)
}

/** The box holding the shapes. */
export function boundsOfShapes<K extends string>(shapes: readonly Shape<K>[]): Box {
  const b = { minX: Infinity, minY: Infinity, minZ: Infinity, maxX: -Infinity, maxY: -Infinity, maxZ: -Infinity }
  for (const { bounds: s } of shapes) {
    b.minX = Math.min(b.minX, s.minX)
    b.minY = Math.min(b.minY, s.minY)
    b.minZ = Math.min(b.minZ, s.minZ)
    b.maxX = Math.max(b.maxX, s.maxX)
    b.maxY = Math.max(b.maxY, s.maxY)
    b.maxZ = Math.max(b.maxZ, s.maxZ)
  }
  return b
}

/** A seed from a text and a seed. */
function hashText(text: string, seed: number): number {
  let h = seed | 0
  for (let i = 0; i < text.length; i++) h = Math.imul(h ^ text.charCodeAt(i), 16777619)
  return h
}

const inBox = (b: Box, x: number, y: number, z: number) =>
  x >= b.minX && x < b.maxX && y >= b.minY && y < b.maxY && z >= b.minZ && z < b.maxZ

const overlaps = (a: Box, b: Box) =>
  a.minX < b.maxX && b.minX < a.maxX && a.minY < b.maxY && b.minY < a.maxY && a.minZ < b.maxZ && b.minZ < a.maxZ

const inside = (a: Box, b: Box) =>
  a.minX >= b.minX && a.minY >= b.minY && a.minZ >= b.minZ && a.maxX <= b.maxX && a.maxY <= b.maxY && a.maxZ <= b.maxZ

const widen = (b: Box, m: number): Box =>
  ({ minX: b.minX - m, minY: b.minY - m, minZ: b.minZ - m, maxX: b.maxX + m, maxY: b.maxY + m, maxZ: b.maxZ + m })

const boxText = (b: Box) =>
  `[${String(b.minX)}, ${String(b.maxX)}) × [${String(b.minY)}, ${String(b.maxY)}) × [${String(b.minZ)}, ${String(b.maxZ)})`
