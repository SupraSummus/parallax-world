import { GRID_LEVELS, GRID_REACH } from './slice-selection'
import { BRICK, coarsen, LayerIndex, VoxelGrid, type Box, type Material, type Voxel, type VoxelData } from './voxel-grid'
import { generated, worlds } from './worldgen'

export interface Vec3 {
  x: number
  y: number
  z: number
}

/** A world's name: one of worldTypes()'s values. */
export type WorldType = string

/** The world types, with their labels: worldgen.ts's worlds (Rust), as WORLDS lists them, each labelled with its name. */
export function worldTypes(): { value: WorldType; label: string }[] {
  return worlds().map(value => ({ value, label: value.charAt(0).toUpperCase() + value.slice(1) }))
}

/**
 * A world generated at each level, in any box (worldgen.ts): the voxels of
 * a box at a level, all of `materials`; the y its grid of each level spans;
 * and where the camera starts.
 */
export interface Generated {
  materials: readonly Material[]
  generate: (box: Box, level: number) => VoxelData
  spawn: Vec3
  heights: (level: number) => { minY: number; maxY: number }
}

/**
 * A generated world's grid of each level is made of tiles, generated
 * alone (in workers): columns TILE sites of the level wide along x and z,
 * on a tiling fixed in the world, so a grid moved with the camera keeps
 * the tiles it still has. Tile (i, k) holds x in [i · TILE, (i + 1) · TILE),
 * z likewise.
 */
const TILE = 16 * BRICK

/**
 * A grid moves when the camera is more than this far from the middle of
 * its centre tile along x or z (in the level's sites): past the tile's
 * edge, so that it does not move back and forth there.
 */
const RECENTRE = (3 / 4) * TILE

/** Tiles either way of a grid's centre tile, at most (see World.holds). */
const RADIUS = Math.ceil((GRID_REACH + RECENTRE) / TILE - 0.5)

/** A tile of the grid of `level` of a generated world, its box in the level's units. */
export interface GridPart {
  level: number
  tile: Tile
  box: Box
}

type Tile = readonly [number, number]

/** A grid being generated: its centre tile, the parts it needs, and those received, by tileKey. */
interface Pending {
  centre: Tile
  parts: GridPart[]
  received: Map<string, { tile: Tile; data: VoxelData; index?: LayerIndex }>
}

const tileKey = ([i, k]: Tile) => `${String(i)},${String(k)}`

// The last world generatePart made, kept for the next part.
let lastGenerated: { seed: number; worldType: WorldType; generated: Generated } | null = null

/** The voxels of a part of a generated world's grid (see World.missingParts). */
export function generatePart(seed: number, worldType: WorldType, { level, box }: GridPart): VoxelData {
  if (lastGenerated?.seed !== seed || lastGenerated.worldType !== worldType) {
    lastGenerated = { seed, worldType, generated: generated(worldType, seed) }
  }
  return lastGenerated.generated.generate(box, level)
}

/**
 * A grid at each level of detail the renderer reads (GRID_LEVELS): `grid`,
 * then each the last one coarsened.
 * @internal
 */
export function levelsOf(grid: VoxelGrid): VoxelGrid[] {
  const levels = [grid]
  while (levels.length < GRID_LEVELS) levels.push(coarsen(levels[levels.length - 1]))
  return levels
}

/**
 * A procedurally generated voxel world (worldgen.ts's): a grid at each
 * level of detail the renderer reads, generated level by level, in tiles,
 * around the camera, anywhere (missingParts, receive), or here when asked
 * for (levels). Tests' scenes are worlds of voxels alone (ofVoxels),
 * coarsened.
 */
export class World {
  readonly seed: number
  readonly worldType: WorldType
  /** Where the camera starts. */
  readonly spawn: Readonly<Vec3>

  private readonly source: Generated | undefined
  // By level; generated worlds' only once generated, those of voxels coarsened on first use.
  private grids: (VoxelGrid | undefined)[] = []
  // The tile each generated grid is centred on, by level.
  private centres: (Tile | undefined)[] = []
  // By level, how many grids replaced the first.
  private versions: number[] = []
  // By level, the LayerIndex of each tile of a generated grid, if the workers made them all.
  private tileIndices: ({ tile: Tile; index: LayerIndex }[] | undefined)[] = []
  // By level, the LayerIndex of the grid: one per tile, or one, made here on first use.
  private indices: (LayerIndex[] | undefined)[] = []
  // By level, the grid being generated.
  private pending = new Map<number, Pending>()
  // Voxels as a list, built from the grid on first use.
  private list: Voxel[] | null = null

  /** The world `worldType` (one of worldTypes()) of `seed`; for a scene, see ofVoxels. */
  constructor(seed: number = Date.now(), worldType: WorldType = 'valley', scene?: { grid: VoxelGrid; spawn: Readonly<Vec3> }) {
    this.seed = seed
    this.worldType = worldType
    if (scene) {
      this.grids = [scene.grid]
      this.spawn = scene.spawn
      return
    }
    // Generated level by level, in parts.
    this.source = generated(worldType, seed)
    this.spawn = this.source.spawn
  }

  /**
   * A world of `voxels` alone, its coarser levels coarsened from them:
   * tests' scenes.
   * @internal
   */
  static ofVoxels(voxels: readonly Voxel[], spawn: Readonly<Vec3> = { x: 0, y: 0, z: 0 }): World {
    return new World(0, 'voxels', { grid: VoxelGrid.fromVoxels(voxels), spawn })
  }

  /** The materials of every grid's ids. */
  get materials(): readonly Material[] {
    return this.source ? this.source.materials : this.level(0)?.materials ?? []
  }

  /**
   * The grid of `level` of detail, if there is one yet: a generated world's
   * once generated (receive), one of voxels' always.
   */
  level(level: number): VoxelGrid | undefined {
    if (!this.source) {
      for (let l = this.grids.length; l <= level; l++) this.grids.push(coarsen(this.grids[l - 1] as VoxelGrid))
    }
    return this.grids[level]
  }

  /** The grid at each level of detail the renderer reads, generated here (around the spawn point) if not yet. */
  get levels(): VoxelGrid[] {
    return Array.from({ length: GRID_LEVELS }, (_, level) => this.generate(level))
  }

  /** The voxels as stored at level 0, generated here (around the spawn point) if not yet. */
  get grid(): VoxelGrid {
    return this.generate(0)
  }

  /**
   * The parts a generated world's grids need around `camera` (lattice
   * units), finest level first: of each level with no grid, and of each
   * whose grid is centred too far from it (RECENTRE), the tiles of a grid
   * centred on the camera's tile that the level's grid lacks. A level's
   * new grid is asked for once, and replaces its old one once all its
   * parts are received.
   */
  missingParts(camera: Readonly<Vec3>): GridPart[] {
    return Array.from({ length: GRID_LEVELS }, (_, level) => this.missingPartsOf(level, camera)).flat()
  }

  /**
   * The grid of `level` around `camera`: missingParts' grid of the level,
   * the parts it misses generated here.
   */
  levelAround(level: number, camera: Readonly<Vec3>): VoxelGrid {
    const { source } = this
    if (!source) return this.level(level) as VoxelGrid
    this.missingPartsOf(level, camera)
    const pending = this.pending.get(level)
    for (const part of pending?.parts ?? []) {
      if (!pending?.received.has(tileKey(part.tile))) this.receive(part, source.generate(part.box, level))
    }
    return this.level(level) as VoxelGrid
  }

  /** missingParts of one level. */
  private missingPartsOf(level: number, camera: Readonly<Vec3>): GridPart[] {
    if (!this.source || this.pending.has(level)) return []
    const centre = this.centres[level]
    const n = 2 ** level
    const off = (v: number, c: number) => Math.abs(v / n - (c + 0.5) * TILE) > RECENTRE
    if (centre && !off(camera.x, centre[0]) && !off(camera.z, centre[1])) return []
    return this.start(level, [Math.floor(camera.x / n / TILE), Math.floor(camera.z / n / TILE)])
  }

  /**
   * Takes a generated part (generatePart), with its LayerIndex if made
   * elsewhere; returns its level's new grid once it has all the grid's
   * parts: those its old grid has, and these.
   */
  receive(part: GridPart, data: VoxelData, index?: LayerIndex): VoxelGrid | undefined {
    const { level, tile } = part
    const pending = this.pending.get(level)
    const key = tileKey(tile)
    if (!pending?.parts.some(p => tileKey(p.tile) === key)) return
    pending.received.set(key, { tile, data, index })
    if (pending.received.size < pending.parts.length) return
    return this.place(level)
  }

  /** How many levels' grids are being generated: asked for (missingParts), not all their parts received. */
  get generating(): number {
    return this.pending.size
  }

  /** How many grids replaced the first of `level` (see ChunkRequest). */
  version(level: number): number {
    return this.versions[level] ?? 0
  }

  /** Where each face's layers of the grid of `level` hold voxels, once it has one. */
  layerIndex(level: number): LayerIndex[] | undefined {
    const grid = this.level(level)
    if (grid) this.indices[level] ??= this.tileIndices[level]?.map(t => t.index) ?? [LayerIndex.of(grid)]
    return this.indices[level]
  }

  /**
   * Whether a grid centred on tile `centre` holds tile `tile`: whether the
   * tile comes within GRID_REACH of where the camera may be while the grid
   * is centred there (RECENTRE about the centre tile's middle). The tiles
   * make a square with its corners cut, RADIUS either way.
   */
  private holds(centre: Tile | undefined, [i, k]: Tile): boolean {
    if (!centre) return false
    const gap = (d: number) => Math.max(0, (Math.abs(d) - 0.5) * TILE - RECENTRE)
    return Math.hypot(gap(i - centre[0]), gap(k - centre[1])) < GRID_REACH
  }

  /** Starts a grid of `level` centred on tile `centre`; returns the parts it needs. */
  private start(level: number, centre: Tile): GridPart[] {
    const { minY, maxY } = (this.source as Generated).heights(level)
    const parts: GridPart[] = []
    for (let k = centre[1] - RADIUS; k <= centre[1] + RADIUS; k++) {
      for (let i = centre[0] - RADIUS; i <= centre[0] + RADIUS; i++) {
        const part: GridPart = { level, tile: [i, k], box: { minX: i * TILE, minY, minZ: k * TILE, maxX: (i + 1) * TILE, maxY, maxZ: (k + 1) * TILE } }
        if (this.holds(centre, part.tile) && !this.holds(this.centres[level], part.tile)) parts.push(part)
      }
    }
    this.pending.set(level, { centre, parts, received: new Map() })
    if (parts.length === 0) this.place(level)
    return parts
  }

  /** Puts the grid of `level` being generated in place, from its parts and the old grid's. */
  private place(level: number): VoxelGrid {
    const { centre, received } = this.pending.get(level) as Pending
    const { minY, maxY } = (this.source as Generated).heights(level)
    const [reach, old] = [(RADIUS + 0.5) * TILE, this.grids[level]]
    const [x, z] = centre.map(c => (c + 0.5) * TILE)
    const box = { minX: x - reach, minY, minZ: z - reach, maxX: x + reach, maxY, maxZ: z + reach }
    const parts = [...received.values()]
    // The old grid's tiles that this one does not hold are left out.
    const keeps = (x: number, z: number) => this.holds(centre, [Math.floor(x / TILE), Math.floor(z / TILE)])
    const grid = VoxelGrid.compose(box, this.materials, [...(old ? [old] : []), ...parts.map(p => p.data)], keeps)
    // Each tile's LayerIndex, while the workers made them all.
    const kept = old ? this.tileIndices[level]?.filter(t => this.holds(centre, t.tile)) : []
    const fresh = parts.flatMap(({ tile, index }) => (index ? [{ tile, index }] : []))
    this.tileIndices[level] = kept && fresh.length === parts.length ? [...kept, ...fresh] : undefined
    this.indices[level] = undefined
    this.pending.delete(level)
    if (this.centres[level]) this.versions[level] = this.version(level) + 1
    this.centres[level] = centre
    if (level === 0) this.list = null
    return (this.grids[level] = grid)
  }

  /** The grid of `level`; if there is none, generated here around the spawn point. */
  private generate(level: number): VoxelGrid {
    return this.level(level) ?? this.levelAround(level, this.spawn)
  }

  /** The voxels as a list, ordered by z, y, x. */
  get voxels(): Voxel[] {
    this.list ??= this.grid.toVoxels()
    return this.list
  }
}
