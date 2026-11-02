/**
 * Slice selection: decides which slices of a face's layers (view.ts) the
 * camera renders, how they are split into chunks, and along which
 * direction each chunk is baked. Depths count layers, LAYER apart.
 *
 * A slice is a slab of a face's layers between two layers of a coarser
 * lattice 2^k·FCC (k its level), made of the cells of a finer one 2^ℓ·FCC
 * (its atoms, cells.ts) that have their own site in the slab, drawn as
 * the voxels of the world's grid of level ℓ. Up to level ATOM_SHIFT the
 * atoms are single voxels and a slab is 2^k layers of them; above, a slab
 * is 2^ATOM_SHIFT layers of coarser voxels. In grid ℓ's units a slice is
 * one of level k − ℓ: slices are the same at every scale. Voxels of
 * neighbouring layers interlock, so the slab's voxels span a little more
 * than its 2^k layers; its centre plane, where its chunks are baked and
 * drawn, is the middle of that.
 *
 * Constraints (see docs/architecture.md for the derivation and worked examples):
 *
 * 1. Occlusion (physical). A slice is drawn flat, so a thick slice can show
 *    geometry that true perspective hides. A slice at viewing distance d at
 *    most d / detail thick, with detail = leakFreeDetail(...), never shows
 *    more than `max leak` screen pixels of it. Far voxels are small on
 *    screen, so slices grow with distance and covering distance n takes
 *    O(detail · log n) slices.
 *
 * 2. Caching. A slice is named by its level and its slab (and its cutoff,
 *    see SliceId) wherever the camera is, so it always has the same content
 *    and can be cached.
 *
 * Walking away from the camera, each level takes over once its slices are
 * thin enough there. Slabs of levels with the same atoms nest; where the
 * atoms grow, cells of the two sizes do not nest, so the last slices of the
 * nearer level hold only the sites whose cell one size up lies before
 * where the next level begins.
 *
 * Slices are split in x and y into chunks: bricks (chunkCell), each the
 * shadow along the axis of a column of cells of a coarser lattice still.
 * The chunks of a level are baked in perspective from its eye (eyeOf), the
 * site of the level's lattice whose cell holds the camera. The closer the
 * eye is to the camera for the slices' distance, the thicker they may be.
 */
import { owner, ownedLayers, spacing } from './cells'
import { LAYER } from './view'
import type { Vec3 } from './world'

/**
 * A slice: slab `layer` of `level`, i.e. its atoms (cells of
 * atomLevel(level)) with their own site in layers
 * [layer · 2^level, (layer + 1) · 2^level). If `cutoff` is finite, only
 * those whose cell one atom size up has its site in a layer before
 * `cutoff`; the rest belong to the next level's slices.
 */
export interface SliceId {
  level: number
  layer: number
  cutoff: number
}

/** Slabs of up to 4096 layers: from about 8 km on in the forest at 1280×800, slices grow no thicker. */
export const MAX_LEVEL = 12

/**
 * Above level ATOM_SHIFT a slab is 2^ATOM_SHIFT layers of cells, not of
 * voxels. A slice is about 1/detail of its distance thick, so its atoms
 * stay 5.6 to 11 pixels across with the defaults; each step up halves
 * that, and quadruples the bytes of level 0's grid (docs/architecture.md,
 * section 1).
 */
const ATOM_SHIFT = 4

/**
 * The level of the cells (cells.ts) a slice of `level` is made of, and so
 * of the grid it reads: 0, single voxels, up to ATOM_SHIFT.
 */
export function atomLevel(level: number): number {
  return Math.max(0, level - ATOM_SHIFT)
}

/** Levels of a world's grids: one per atomLevel up to MAX_LEVEL's. */
export const GRID_LEVELS = atomLevel(MAX_LEVEL) + 1

/** N: the spacing of its atoms' sites, in layers (and lattice units). */
export function atomSpacing(level: number): number {
  return spacing(atomLevel(level))
}

/**
 * The layers the sites it stands for may be in, relative to the first
 * layer of its slab: its atoms' own sites are in the 2^level layers from
 * there, every N-th, and the cell of each holds the sites of finer grids
 * ownedLayers around its own.
 */
export function siteLayers(level: number): [number, number] {
  const [lo, hi] = ownedLayers(atomLevel(level))
  return [lo, 2 ** level - atomSpacing(level) + hi]
}

/**
 * @internal Whether a slice holds site (a, b, c) of any grid, i.e. its
 * atom (for tests; classify in bake/src/surface.rs).
 */
export function holds({ level, layer, cutoff }: SliceId, site: [number, number, number]): boolean {
  const n = spacing(atomLevel(level))
  return Math.floor(owner(n, site)[2] / 2 ** level) === layer && (cutoff === Infinity || owner(2 * n, site)[2] < cutoff)
}

/**
 * A slice's thickness in layer spacings: its atoms' sites are every N-th
 * of its 2^level layers, and its voxels, N wide, reach N layers past them
 * (see fcc.ts). 2^level + N.
 * @internal
 */
export function thickness(level: number): number {
  return 2 ** level + atomSpacing(level)
}

/** Its near side, the near tips of its nearest voxels, in layers. */
function nearSide({ level, layer }: { level: number; layer: number }): number {
  return layer * 2 ** level - atomSpacing(level)
}

/** A name for a slice of a face, the same wherever the camera is. */
export function sliceKey({ level, layer, cutoff }: SliceId): string {
  return `${String(level)}:${String(layer)}:${String(cutoff)}`
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
 * the rows of bricks are offset by half a brick, like a wall's, and each
 * is the shadow of one column of cells of M·FCC. In lattice units a brick
 * is 2M across and M high.
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
 * The eye of a slice of `level` for a camera at `camera` (face
 * coordinates): the site of N·FCC, the level's lattice (N = 2^level), whose
 * cell holds the camera, in lattice coordinates. The level's chunks are
 * baked in perspective from it, so they hold while the camera stays in
 * that cell.
 */
export function eyeOf(level: number, camera: Vec3): [number, number, number] {
  return owner(spacing(level), [camera.x / LAYER, camera.y, camera.z / LAYER])
}

/**
 * @internal How far (as a tangent) a ray from the eye of a slice of `level`
 * may be from the camera's ray to the same point, d layers ahead of the
 * eye: 2N / d (for tests; eye_error in bake/src/slicing.rs).
 * Off by δ from the eye, the camera sees a point along tangent σ that the
 * eye sees along τ = σ − (δx − σ · δz) / d (in world units). For σ among
 * the face's rays (|σx| ≤ 1, |σy| ≤ LAYER) and δ in the eye's cell,
 * |δx − σx · δz| ≤ |δx| + |δz| is at most √2 · N, at a corner of the cell
 * N / √2 off in x and N / √2 along z; in y, less. d layers are d · LAYER.
 */
export function eyeError(level: number, d: number): number {
  return (2 * spacing(level)) / d
}

/**
 * The largest eye error baking allows for (EYE_ERROR in bake/src/slicing.rs):
 * every voxel is at least 2N layers ahead of its eye (leakFreeDetail, and
 * NEAR_DEPTH in renderer.ts).
 */
const EYE_ERROR = 1

/**
 * How far past the face's rays (as a tangent) a ray from the eye may be
 * where the camera's rays are the face's, plus how far gap filling may grow
 * faces: EYE_ERROR each (REACH in bake/src/surface.rs).
 */
const REACH = 2 * EYE_ERROR

/**
 * How far around its cell a chunk of a slice of `level` may draw voxels
 * from, in lattice units: `columns` along a, `rows` along b (cell_margin in
 * bake/src/surface.rs, in the units of the slice's grid, N times these).
 * `rows` also bounds how far from a voxel's b its faces may be drawn where
 * the face's rays show them.
 */
export function cellMargin(level: number): { columns: number; rows: number } {
  const n = atomSpacing(level)
  const half = (thickness(level) / n / 2) * LAYER
  return { columns: n * Math.ceil(((1 + REACH) * half + 1.5) / LAYER), rows: n * Math.ceil((LAYER + REACH) * half + 1.5) }
}

/** r: the largest N / thickness, the eye's spacing over its slices' thickness (16/17, from level ATOM_SHIFT on). */
const EYE_RATIO = Math.max(...Array.from({ length: MAX_LEVEL + 1 }, (_, level) => spacing(level) / thickness(level)))

/**
 * Detail (viewing distance / maximum slice thickness) that keeps occlusion
 * leaks at most `maxLeakPixels` on screen, given the focal length in
 * pixels.
 *
 * A slice t layers thick whose near side is a layers from the camera is
 * at least a − N layers from its eye, so its points are drawn with rays
 * at most e = 2N / (a − N) off (eyeError). A leak is at most e · t layer
 * spacings, e · t · focal / a pixels (docs/architecture.md, section 2).
 * With a ≥ t · detail and r = N / t, that is at most
 * 2r · focal / (detail · (detail − r)): ε for
 * detail = (r + √(r² + 8r · focal / ε)) / 2, with r the largest of any
 * level (EYE_RATIO). At least 3r, so that voxels are 2N layers ahead of
 * the eye (EYE_ERROR).
 *
 * Rounded up to a quarter octave (denser slices are always safe), so small
 * changes of the viewport keep the slicing and the cache.
 */
export function leakFreeDetail(focal: number, maxLeakPixels: number): number {
  const r = EYE_RATIO
  const detail = Math.max(3 * r, (r + Math.sqrt(r * r + (8 * r * focal) / maxLeakPixels)) / 2)
  return 2 ** (Math.ceil(4 * Math.log2(detail)) / 4)
}

/**
 * z (face coordinates) of a slice's centre plane, the plane its chunks are
 * baked onto and drawn at: halfway through what its voxels span, so they
 * reach thickness / 2 to either side.
 */
export function sliceCentre(slice: SliceId): number {
  return (nearSide(slice) + thickness(slice.level) / 2) * LAYER
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
  const slices: SliceId[] = []
  // Round up: a slice closer than minRelativeDepth would be blown up over the screen.
  let from = Math.ceil(cameraZ + minRelativeDepth)
  for (let level = 0; ; level++) {
    const slab = 2 ** level
    const n = spacing(atomLevel(level))
    const first = siteLayers(level)[0]
    let until = Infinity
    let grows = false
    if (level < MAX_LEVEL) {
      const next = level + 1
      grows = atomLevel(next) > atomLevel(level)
      until = alignUp(Math.max(cameraZ - nearSide({ level: next, layer: 0 }) + thickness(next) * detail, from + (grows ? 3 * n : 0)), 2 * slab)
      // The next level's sites reach at most its first site layer before its slabs.
      if (until + siteLayers(next)[0] >= far) until = Infinity
    }
    const last = until === Infinity ? (m: number) => m * slab + first < far : grows ? (m: number) => m * slab <= until : (m: number) => m * slab < until
    for (let layer = Math.ceil(from / slab); last(layer); layer++) {
      slices.push({ level, layer, cutoff: grows && (layer + 1) * slab + 4 * n + 2 > until ? until : Infinity })
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
