/**
 * Slice selection: decides which slices of a face's layers (view.ts) the
 * camera renders, how they are split into chunks, and along which
 * direction each chunk is baked. Depths count layers, LAYER apart.
 *
 * A slice is a slab of a face's layers between two layers of a coarser
 * lattice 2^k·FCC (k its level), made of the cells of a finer one 2^ℓ·FCC
 * (its atoms, cells.ts) that have their own site in the slab, drawn as
 * the voxels of the world's grid of level ℓ. Up to level s, the atom shift
 * (atomShift), the atoms are single voxels and a slab is 2^k layers of
 * them; above, a slab is 2^s layers of coarser voxels. In grid ℓ's units a
 * slice is one of level k − ℓ: slices are the same at every scale. Voxels of
 * neighbouring layers interlock, so the slab's voxels span a little more
 * than its 2^k layers; its centre plane, where its chunks are baked and
 * drawn, is the middle of that.
 *
 * Constraints (see docs/architecture.md for the derivation and worked examples):
 *
 * 1. Occlusion (physical). A slice is drawn flat, so a thick slice can show
 *    geometry that true perspective hides. A slice at viewing distance d at
 *    most d / detail thick, with detail = leakFreeDetail(...), never shows
 *    more than the `max leak` angle of it. Far voxels are small on
 *    screen, so slices grow with distance and covering distance n takes
 *    O(detail · log n) slices.
 *
 * 2. Caching. A slice is named by its level, atoms and slab (and its
 *    cutoff, see SliceId) wherever the camera is, so it always has the same
 *    content and can be cached.
 *
 * 3. Grids. Each grid holds the world GRID_REACH of its units around the
 *    camera, and the slices read no farther: the finer the slices (the
 *    higher detail), the sooner their atoms grow (atomShift).
 *
 * Walking away from the camera, each level takes over once its slices are
 * thin enough there. Slabs of levels with the same atoms nest; where the
 * atoms grow, cells of the two sizes do not nest, so the last slices of the
 * nearer level hold only the sites whose cell one size up lies before
 * where the next level begins.
 *
 * Slices are split in x and y into chunks: bricks (chunkCell), each the
 * shadow along the axis of a column of cells of a coarser lattice still.
 * Chunks are baked as seen along the axis, so the camera's position
 * changes none of them.
 */
import { owner, ownedLayers, spacing } from './cells'
import { LAYER } from './view'

/**
 * A slice: slab `layer` of `level`, i.e. its atoms (cells of level
 * `atoms`, see atomLevel) with their own site in layers
 * [layer · 2^level, (layer + 1) · 2^level). If `cutoff` is finite, only
 * those whose cell one atom size up has its site in a layer before
 * `cutoff`; the rest belong to the next level's slices.
 */
export interface SliceId {
  level: number
  atoms: number
  layer: number
  cutoff: number
}

/** What a slice's shape depends on: its level and its atoms'. */
export type Slab = Pick<SliceId, 'level' | 'atoms'>

/** Slabs of up to 4096 layers: from about 4 km on in the forest at 1280×800, slices grow no thicker. @internal */
export const MAX_LEVEL = 12

/**
 * The largest atom shift: slabs of at most 2^ATOM_SHIFT layers of cells,
 * as the baker's splats' depths ride in a byte (bake/src/slicing.rs).
 * @internal
 */
export const ATOM_SHIFT = 4

/** Levels of a world's grids: one per atom level up to MAX_LEVEL's at the largest shift. */
export const GRID_LEVELS = MAX_LEVEL - ATOM_SHIFT + 1

/**
 * How far around the camera each grid holds the world, in its units
 * (World.holds), and so how far slices may read it (atomShift).
 * Its reciprocal is about the angle the farthest atoms of each grid span:
 * larger makes atoms finer on screen, and grids' bytes grow as its square.
 */
export const GRID_REACH = 400

/**
 * The level of the cells (cells.ts) a slice of `level` is made of, and so
 * of the grid it reads, at atom shift `shift`: 0, single voxels, up to
 * level `shift`, then one more per level, up to the coarsest grid's.
 * @internal
 */
export function atomLevel(level: number, shift: number): number {
  return Math.min(GRID_LEVELS - 1, Math.max(0, level - shift))
}

/**
 * The atom shift for slices of `detail`: the largest (the finest atoms)
 * whose slices read no grid but the coarsest farther than GRID_REACH,
 * for any detail the panel offers.
 *
 * In grid ℓ's units, level s + ℓ, the last with its atoms, is level s.
 * It ends where the next level begins (selectSlices): that level's
 * thickness · detail ahead, rounded up to two slabs of 2^s, and 7 more:
 * the next level's near side (2), sites past the cutoff (3), their
 * neighbours and voxels (2). A face's rays are |x| + √2 · |y| ≤ z, so a
 * site c layers ahead is at most c units away.
 */
function atomShift(detail: number): number {
  const reach = (s: number) => thickness({ level: s + 1, atoms: 1 }) * detail + 2 * 2 ** s + 7
  let shift = ATOM_SHIFT
  while (shift > 0 && reach(shift) > GRID_REACH) shift--
  return shift
}

/** N: the spacing of its atoms' sites, in layers (and lattice units). */
export function atomSpacing({ atoms }: Slab): number {
  return spacing(atoms)
}

/**
 * The layers the sites it stands for may be in, relative to the first
 * layer of its slab: its atoms' own sites are in the 2^level layers from
 * there, every N-th, and the cell of each holds the sites of finer grids
 * ownedLayers around its own.
 */
export function siteLayers(slab: Slab): [number, number] {
  const [lo, hi] = ownedLayers(slab.atoms)
  return [lo, 2 ** slab.level - atomSpacing(slab) + hi]
}

/**
 * @internal Whether a slice holds site (a, b, c) of any grid, i.e. its
 * atom (for tests; classify in bake/src/surface.rs).
 */
export function holds({ level, atoms, layer, cutoff }: SliceId, site: [number, number, number]): boolean {
  const n = spacing(atoms)
  return Math.floor(owner(n, site)[2] / 2 ** level) === layer && (cutoff === Infinity || owner(2 * n, site)[2] < cutoff)
}

/**
 * A slice's thickness in layer spacings: its atoms' sites are every N-th
 * of its 2^level layers, and its voxels, N wide, reach N layers past them
 * (see fcc.ts). 2^level + N.
 */
export function thickness(slab: Slab): number {
  return 2 ** slab.level + atomSpacing(slab)
}

/** Its near side, the near tips of its nearest voxels, in layers. */
function nearSide(slice: Slab & { layer: number }): number {
  return slice.layer * 2 ** slice.level - atomSpacing(slice)
}

/** A name for a slice of a face, the same wherever the camera is. */
export function sliceKey({ level, atoms, layer, cutoff }: SliceId): string {
  return `${String(level)}/${String(atoms)}:${String(layer)}:${String(cutoff)}`
}

/**
 * M: the spacing of the lattice M·FCC whose columns of cells a slice of
 * `level` is split along, 8 · 2^level: a chunk is about 16 times as wide
 * as its slice is thick, so about as wide on screen at every distance.
 */
function brickSpacing(level: number): number {
  return 8 * 2 ** level
}

/** A part of a slice's plane, in face coordinates: [x, x + width) × [y, y + height). */
export interface Rect {
  x: number
  y: number
  width: number
  height: number
}

/**
 * Chunk (i, j) of a slice of `level`: brick i of row j. Seen along the
 * axis, the sites of M·FCC (M = brickSpacing) form rows M apart in b,
 * their a multiples of 2M in even rows and odd multiples of M in odd ones.
 * The brick around a site reaches M either way in a and M / 2 in b, so
 * the rows of bricks are offset by half a brick, like a wall's. In
 * lattice units a brick is 2M across and M high.
 */
export function chunkCell(level: number, i: number, j: number): Rect {
  const m = brickSpacing(level)
  return { x: (2 * i + (j & 1) - 1) * m * LAYER, y: (j - 0.5) * m, width: 2 * m * LAYER, height: m }
}

/** The chunks (i, j) of a slice of `level` that meet [left, right) × [bottom, top) of its plane, row by row. */
export function chunkCells(level: number, left: number, right: number, bottom: number, top: number): [number, number][] {
  const m = brickSpacing(level)
  const cells: [number, number][] = []
  for (let j = Math.floor(bottom / m + 0.5); (j - 0.5) * m < top; j++) {
    // Brick i of row j spans a in [(2i + odd − 1)·M, (2i + odd + 1)·M).
    const brick = (a: number) => (a / m - (j & 1) + 1) / 2
    for (let i = Math.floor(brick(left / LAYER)); i < brick(right / LAYER); i++) cells.push([i, j])
  }
  return cells
}

/**
 * How far a face's rays are from its axis, as a tangent, at most: they are
 * |x| + √2 · |y| ≤ z (inFace in view.ts). Seen along them, a point d ahead
 * of its slice's centre plane is up to MAX_SLANT · d off its site there:
 * where drawn flat it would be off (docs/architecture.md, section 2),
 * where drawn in relief its splat moves (MAX_SLANT in bake/src/slicing.rs).
 */
const MAX_SLANT = 1

/** How far past its slice's centre plane a splat is per step of its depth byte (see ChunkSplats), in world units: half a layer of its grid. */
export function halfLayer(slab: Slab): number {
  return (atomSpacing(slab) * LAYER) / 2
}

/**
 * How far around its cell a chunk of a slice of `slab` may draw voxels
 * from, in lattice units: `columns` along a, `rows` along b (cell_margin in
 * bake/src/surface.rs, in the units of the slice's grid, N times these).
 * A voxel's splat, less than half the slice's thickness off the centre
 * plane, is drawn up to MAX_SLANT times that off its site, widened (reach
 * in bake/src/splats.rs). `rows` also bounds how far from a voxel's b it
 * may be drawn.
 */
export function cellMargin(slab: Slab): { columns: number; rows: number } {
  const n = atomSpacing(slab)
  const off = MAX_SLANT * (thickness(slab) / n / 2) * LAYER
  return { columns: n * Math.ceil((off + 1.5) / LAYER), rows: n * Math.ceil(off * Math.SQRT1_2 + 1.5) }
}

/**
 * Detail (viewing distance / maximum slice thickness) that keeps occlusion
 * leaks at most `maxLeak` radians (maxLeak · focal pixels on screen).
 *
 * A slice t layers thick whose near side is a layers from the camera has
 * its centre plane at a + t / 2. Its points are drawn up to t / 2 layer
 * spacings before or behind where they are, along the axis, which the
 * camera sees at most MAX_SLANT off; a leak is at most MAX_SLANT · t
 * spacings, MAX_SLANT · t / (a + t / 2) radians (docs/architecture.md,
 * section 2). With a ≥ t · detail that is at most
 * MAX_SLANT / (detail + 1/2) (leakAngle): ε for
 * detail = MAX_SLANT / ε − 1/2, at least 1.
 *
 * Rounded up to a quarter octave (denser slices are always safe), so small
 * changes of the tolerance keep the slicing and the cache.
 */
export function leakFreeDetail(maxLeak: number): number {
  const detail = Math.max(1, MAX_SLANT / maxLeak - 1 / 2)
  return 2 ** (Math.ceil(4 * Math.log2(detail)) / 4)
}

/** The largest occlusion leak slices of `detail` would show drawn flat, in radians (see leakFreeDetail). @internal */
export function leakAngle(detail: number): number {
  return MAX_SLANT / (detail + 1 / 2)
}

/**
 * z (face coordinates) of a slice's centre plane, the plane its chunks are
 * baked onto and drawn at: halfway through what its voxels span, so they
 * reach thickness / 2 to either side.
 */
export function sliceCentre(slice: SliceId): number {
  return (nearSide(slice) + thickness(slice) / 2) * LAYER
}

/** The smallest multiple of n ≥ x. */
const alignUp = (x: number, n: number) => Math.ceil(x / n) * n

/**
 * Slices holding every site with c in [cameraZ + minRelativeDepth,
 * cameraZ + maxRelativeDepth), each in exactly one, sorted near to far;
 * all in layers. Beyond level 0 (single layers), no slice is thicker than
 * a / detail, where a is the distance to its near side.
 *
 * Level k holds the slabs from where it begins, `from`, to where the next
 * level begins, `until` (a multiple of the next level's slab, 2^(k+1)):
 * the first layer far enough for the next level's thickness. Where the
 * next level's atoms are larger, cells of two sizes meet at `until`: a
 * site's cell of the larger size may lie before it while its cell of the
 * smaller size lies after, and the other way round, by less than 3N (N the
 * smaller cells' spacing). So level k goes on to the slab at `until`, and
 * its slabs within reach (ending past until − 4N − 2) get `until` as their
 * cutoff; and it is at least 3N long, so that a site of its first slabs
 * never belongs to an earlier level.
 */
export function selectSlices(cameraZ: number, minRelativeDepth: number, maxRelativeDepth: number, detail: number): SliceId[] {
  if (minRelativeDepth >= maxRelativeDepth) return []
  const far = cameraZ + maxRelativeDepth
  const shift = atomShift(detail)
  const slabOf = (level: number): Slab => ({ level, atoms: atomLevel(level, shift) })
  const slices: SliceId[] = []
  // Round up: a slice closer than minRelativeDepth would be blown up over the screen.
  let from = Math.ceil(cameraZ + minRelativeDepth)
  for (let level = 0; ; level++) {
    const slab = 2 ** level
    const { atoms } = slabOf(level)
    const n = spacing(atoms)
    const first = siteLayers(slabOf(level))[0]
    let until = Infinity
    let grows = false
    if (level < MAX_LEVEL) {
      const next = slabOf(level + 1)
      grows = next.atoms > atoms
      until = alignUp(Math.max(cameraZ - nearSide({ ...next, layer: 0 }) + thickness(next) * detail, from + (grows ? 3 * n : 0)), 2 * slab)
      // The next level's sites reach at most its first site layer before its slabs.
      if (until + siteLayers(next)[0] >= far) until = Infinity
    }
    const last = until === Infinity ? (m: number) => m * slab + first < far : grows ? (m: number) => m * slab <= until : (m: number) => m * slab < until
    for (let layer = Math.ceil(from / slab); last(layer); layer++) {
      slices.push({ level, atoms, layer, cutoff: grows && (layer + 1) * slab + 4 * n + 2 > until ? until : Infinity })
    }
    if (until === Infinity) return slices
    from = until
  }
}

/**
 * Pairs up two slicings of a face (near to far, as selectSlices returns
 * them): segments of consecutive `wanted` slices, each with the `previous`
 * slices it replaces. Slices in both are segments of their own, paired
 * with themselves; the ones between two of them, or before the first, are
 * paired with the previous slices there, which hold the same sites but for
 * a few at the ends. Wanted slices past the last shared one get none.
 */
export function matchTilings(wanted: SliceId[], previous: SliceId[]): { wanted: SliceId[]; previous: SliceId[] }[] {
  const index = new Map(previous.map((s, k) => [sliceKey(s), k]))
  const segments: { wanted: SliceId[]; previous: SliceId[] }[] = []
  let current: SliceId[] = []
  // The last slice found in both, by its index in `previous`.
  let shared = -1
  for (const slice of wanted) {
    const k = index.get(sliceKey(slice))
    if (k === undefined || k <= shared) {
      current.push(slice)
      continue
    }
    if (current.length > 0) segments.push({ wanted: current, previous: previous.slice(shared + 1, k) })
    segments.push({ wanted: [slice], previous: [previous[k]] })
    current = []
    shared = k
  }
  if (current.length > 0) segments.push({ wanted: current, previous: [] })
  return segments
}
