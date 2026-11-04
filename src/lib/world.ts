import { generateCity } from './city'
import { GRID_LEVELS } from './slice-selection'
import { BRICK, coarsen, LayerIndex, Palette, VoxelGrid, type Box, type Material, type Voxel, type VoxelData } from './voxel-grid'
import { generated, worlds } from './worldgen'

export interface Vec3 {
  x: number
  y: number
  z: number
}

/** A world's name: one of worldTypes()'s values. */
export type WorldType = string

/** The worlds of TS, made here or by their own scene builder (SCENES). */
const TS_WORLDS: { value: WorldType; label: string }[] = [
  { value: 'city', label: 'City' },
  { value: 'hills', label: 'Hills' },
  { value: 'mountains', label: 'Mountains' },
  { value: 'hills_with_trees', label: 'Hills with Trees' },
  { value: 'lake_with_island', label: 'Lake with Island' },
]

/**
 * The world types, with their labels: worldgen.ts's worlds (Rust), as
 * WORLDS lists them, each labelled with its name; then the worlds of TS.
 */
export function worldTypes(): { value: WorldType; label: string }[] {
  return [...worlds().map(value => ({ value, label: value.charAt(0).toUpperCase() + value.slice(1) })), ...TS_WORLDS]
}

const GRASS = '#4CAF50'
const DIRT = '#8B4513'
const STONE = '#666666'
const LEAVES = '#228B22'
const WATER = '#1E90FF'
const FLOWER_COLORS = ['#FF6B6B', '#FFE66D', '#4ECDC4', '#C9B1FF']

/**
 * A world generated at each level, in any box (worldgen.ts): the voxels of
 * a box at a level, all of `materials`; how far around the camera each
 * level's grid must hold it, in sites of the level; the y its grid of each
 * level spans; and where the camera starts.
 */
export interface Generated {
  materials: readonly Material[]
  generate: (box: Box, level: number) => VoxelData
  spawn: Vec3
  reach: number
  heights: (level: number) => { minY: number; maxY: number }
}

/** World types with their own scene builder; the rest are built here. */
const SCENES: Partial<Record<WorldType, (seed: number) => { grid: VoxelGrid; spawn: Vec3 }>> = { city: generateCity }

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
 * Toy worlds are built of blocks BLOCK lattice units wide: block (x, y, z)
 * holds the sites with x in [BLOCK · x − 1, BLOCK · x] (y, z likewise).
 */
const BLOCK = 2
/** The world spans blocks x, z in [-HALF_SIZE, HALF_SIZE). */
const HALF_SIZE = 40
/** Every column is filled from this y up to its surface. */
const BEDROCK_Y = -5

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
 * A procedurally generated voxel world: a grid at each level of detail
 * the renderer reads. Generated worlds (worldgen.ts's) are generated level
 * by level, in tiles, around the camera, anywhere (missingParts, receive),
 * or here when asked for (levels); the rest are made here whole, and
 * coarsened.
 */
export class World {
  readonly seed: number
  readonly worldType: WorldType
  /** Where the camera starts. */
  readonly spawn: Readonly<Vec3>

  private readonly source: Generated | undefined
  // By level; generated worlds' only once generated, the rest's coarsened on first use.
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
  // Toy world blocks while generating.
  private generated: Voxel[] = []

  constructor(seed: number = Date.now(), worldType: WorldType = 'forest') {
    this.seed = seed
    this.worldType = worldType

    // Generated level by level, in parts; the grids of the rest are made whole, then coarsened.
    this.source = worlds().includes(worldType) ? generated(worldType, seed) : undefined
    if (this.source) {
      this.spawn = this.source.spawn
      return
    }
    const scene = SCENES[worldType]
    if (scene) {
      const { grid, spawn } = scene(seed)
      this.grids = [grid]
      this.spawn = spawn
      return
    }

    this.spawn = { x: 0, y: 20 * BLOCK, z: -70 * BLOCK }

    for (let x = -HALF_SIZE; x < HALF_SIZE; x++) {
      for (let z = -HALF_SIZE; z < HALF_SIZE; z++) {
        this.generateColumn(x, z)
      }
    }
    this.grids = [World.fromBlocks(this.generated)]
    this.generated = []
  }

  /** The materials of every grid's ids. */
  get materials(): readonly Material[] {
    return this.source ? this.source.materials : this.level(0)?.materials ?? []
  }

  /**
   * The grid of `level` of detail, if there is one yet: a generated world's
   * once generated (receive), the rest's always.
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

  /** Whether a grid centred on tile `centre` holds tile `tile`. */
  private holds(centre: Tile | undefined, [i, k]: Tile): boolean {
    const radius = this.radius()
    return !!centre && Math.abs(i - centre[0]) <= radius && Math.abs(k - centre[1]) <= radius
  }

  /** Tiles either way of a grid's centre tile: enough for it to reach the generator's reach around the camera (see RECENTRE). */
  private radius(): number {
    return Math.ceil(((this.source?.reach ?? 0) + RECENTRE) / TILE - 0.5)
  }

  /** Starts a grid of `level` centred on tile `centre`; returns the parts it needs. */
  private start(level: number, centre: Tile): GridPart[] {
    const { minY, maxY } = (this.source as Generated).heights(level)
    const radius = this.radius()
    const parts: GridPart[] = []
    for (let k = centre[1] - radius; k <= centre[1] + radius; k++) {
      for (let i = centre[0] - radius; i <= centre[0] + radius; i++) {
        const part: GridPart = { level, tile: [i, k], box: { minX: i * TILE, minY, minZ: k * TILE, maxX: (i + 1) * TILE, maxY, maxZ: (k + 1) * TILE } }
        if (!this.holds(this.centres[level], part.tile)) parts.push(part)
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
    const [reach, old] = [(this.radius() + 0.5) * TILE, this.grids[level]]
    const [x, z] = centre.map(c => (c + 0.5) * TILE)
    const box = { minX: x - reach, minY, minZ: z - reach, maxX: x + reach, maxY, maxZ: z + reach }
    const parts = [...received.values()]
    const grid = VoxelGrid.compose(box, this.materials, [...(old ? [old] : []), ...parts.map(p => p.data)])
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

  /** The voxels as a list, ordered by z, y, x. Assigning replaces the world's contents (tests build worlds this way). */
  get voxels(): Voxel[] {
    this.list ??= this.grid.toVoxels()
    return this.list
  }

  set voxels(voxels: Voxel[]) {
    if (this.source) throw new RangeError('a generated world\'s voxels are its generator\'s')
    this.grids = [VoxelGrid.fromVoxels(voxels)]
    this.list = null
  }

  /** Deterministic hash noise in [0, 1). */
  private noise(x: number, y: number, z: number): number {
    const n = Math.sin(x * 12.9898 + y * 78.233 + z * 45.164 + this.seed * 0.001) * 43758.5453
    return n - Math.floor(n)
  }

  private terrainHeight(x: number, z: number, heightMultiplier: number): number {
    const nx = (x / HALF_SIZE) * 4
    const nz = (z / HALF_SIZE) * 4
    return Math.floor(
      this.noise(nx, 0, nz) * 8 * heightMultiplier +
      this.noise(nx * 2, 0, nz * 2) * 4 * heightMultiplier +
      this.noise(nx * 4, 0, nz * 4) * 2 * heightMultiplier
    )
  }

  private push(x: number, y: number, z: number, color: string) {
    this.generated.push({ x, y, z, color })
  }

  /** The grid of the generated blocks: block (x, y, z) is filled with the sites it holds; later blocks win. */
  private static fromBlocks(blocks: readonly Voxel[]): VoxelGrid {
    if (blocks.length === 0) return VoxelGrid.fromVoxels([])
    const box = { minX: Infinity, minY: Infinity, minZ: Infinity, maxX: -Infinity, maxY: -Infinity, maxZ: -Infinity }
    for (const { x, y, z } of blocks) {
      box.minX = Math.min(box.minX, BLOCK * x - 1)
      box.minY = Math.min(box.minY, BLOCK * y - 1)
      box.minZ = Math.min(box.minZ, BLOCK * z - 1)
      box.maxX = Math.max(box.maxX, BLOCK * x + 1)
      box.maxY = Math.max(box.maxY, BLOCK * y + 1)
      box.maxZ = Math.max(box.maxZ, BLOCK * z + 1)
    }
    const palette = new Palette()
    const grid = new VoxelGrid(box, palette.materials)
    for (const { x, y, z, color } of blocks) {
      grid.fillBox(BLOCK * x - 1, BLOCK * y - 1, BLOCK * z - 1, BLOCK * x + 1, BLOCK * y + 1, BLOCK * z + 1, palette.id(color))
    }
    return grid
  }

  /** Fills a column from bedrock to `top`: surface block, 2 subsoil blocks, rock below. */
  private soilColumn(x: number, z: number, top: number, surface: string, subsoil = DIRT, rock = STONE) {
    for (let y = BEDROCK_Y; y <= top; y++) {
      this.push(x, y, z, y === top ? surface : y > top - 3 ? subsoil : rock)
    }
  }

  private tree(x: number, ground: number, z: number, height: number) {
    for (let y = ground + 1; y < ground + height; y++) {
      this.push(x, y, z, '#654321')
    }
    for (let dx = -2; dx <= 2; dx++) {
      for (let dz = -2; dz <= 2; dz++) {
        for (let dy = 0; dy < 3; dy++) {
          if (Math.abs(dx) + Math.abs(dz) + dy < 4) {
            this.push(x + dx, ground + height - 1 + dy, z + dz, LEAVES)
          }
        }
      }
    }
  }

  private palm(x: number, ground: number, z: number, height: number) {
    for (let y = ground + 1; y < ground + height; y++) {
      this.push(x, y, z, '#8B7355')
    }
    for (let dx = -2; dx <= 2; dx++) {
      for (let dz = -2; dz <= 2; dz++) {
        if (Math.abs(dx) + Math.abs(dz) <= 2) {
          this.push(x + dx, ground + height, z + dz, '#32CD32')
        }
      }
    }
  }

  private generateColumn(x: number, z: number) {
    switch (this.worldType) {
      case 'hills': {
        const ground = this.terrainHeight(x, z, 0.7)
        this.soilColumn(x, z, ground, '#7CFC00')
        if (this.noise(x * 0.4, 30, z * 0.4) > 0.8) {
          const color = FLOWER_COLORS[Math.floor(this.noise(x, z, 0) * FLOWER_COLORS.length)]
          this.push(x, ground + 1, z, color)
        }
        break
      }

      case 'mountains': {
        const ground = this.terrainHeight(x, z, 2.5)
        for (let y = BEDROCK_Y; y <= ground; y++) {
          let color: string
          // Snow caps: the top two blocks of peaks above y=15.
          if (y > 15 && y > ground - 2) color = '#FFFFFF'
          else if (y === ground && y > 8) color = '#808080'
          else if (y === ground) color = GRASS
          else if (y > ground - 3) color = DIRT
          else color = '#555555'
          this.push(x, y, z, color)
        }
        break
      }

      case 'hills_with_trees': {
        const ground = this.terrainHeight(x, z, 0.8)
        this.soilColumn(x, z, ground, GRASS)
        if (this.noise(x * 0.3, 0, z * 0.3) > 0.75 && ground > 2) {
          this.tree(x, ground, z, 4 + Math.floor(this.noise(x, 0, z) * 3))
        }
        break
      }

      case 'lake_with_island': {
        const ground = this.terrainHeight(x, z, 0.6)
        const distFromCenter = Math.sqrt(x * x + z * z)
        if (distFromCenter <= 8) {
          const top = ground + 3
          this.soilColumn(x, z, top, '#90EE90', '#C4A76D')
          if (distFromCenter < 5 && this.noise(x * 0.5, 0, z * 0.5) > 0.7) {
            this.palm(x, top, z, 5 + Math.floor(this.noise(x, 0, z) * 2))
          }
        } else if (distFromCenter < 25) {
          this.soilColumn(x, z, 0, WATER, STONE)
        } else {
          const sand = '#F4D03F'
          this.soilColumn(x, z, ground, distFromCenter < 28 ? sand : GRASS)
        }
        break
      }
    }
  }
}
