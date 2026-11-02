import { generateCity } from './city'
import { forest } from './forest'
import { generate, materialsOf, type Box, type Generator } from './generator'
import { GRID_LEVELS } from './slice-selection'
import { BRICK, coarsen, VoxelGrid, type Material, type Voxel, type VoxelData } from './voxel-grid'

export interface Vec3 {
  x: number
  y: number
  z: number
}

export type WorldType = 'forest' | 'city' | 'hills' | 'mountains' | 'hills_with_trees' | 'lake_with_island'

export const WORLD_TYPES: { value: WorldType; label: string }[] = [
  { value: 'forest', label: 'Forest' },
  { value: 'city', label: 'City' },
  { value: 'hills', label: 'Hills' },
  { value: 'mountains', label: 'Mountains' },
  { value: 'hills_with_trees', label: 'Hills with Trees' },
  { value: 'lake_with_island', label: 'Lake with Island' },
]

// `type` ids: 1 grass, 2 dirt, 3 stone, 4 wood, 5 leaves, 6 flower, 7 snow, 8 water, 9 sand,
// 10 plaster, 11 glass, 12 metal, 13 fabric
const GRASS: Material = { color: '#4CAF50', type: 1 }
const DIRT: Material = { color: '#8B4513', type: 2 }
const STONE: Material = { color: '#666666', type: 3 }
const LEAVES: Material = { color: '#228B22', type: 5 }
const WATER: Material = { color: '#1E90FF', type: 8 }
const FLOWER_COLORS = ['#FF6B6B', '#FFE66D', '#4ECDC4', '#C9B1FF']

/** A world generator.ts generates at each level, in any box: the box of each level's grid, and where the camera starts. */
interface Generated {
  generator: Generator<string>
  spawn: Vec3
  box: (level: number) => Box
}

/** World types generated level by level, in parts; the grids of the rest are made whole, then coarsened. */
const GENERATED: Partial<Record<WorldType, (seed: number) => Generated>> = { forest }
/** World types with their own scene builder; the rest are built here. */
const SCENES: Partial<Record<WorldType, (seed: number) => { grid: VoxelGrid; spawn: Vec3 }>> = { city: generateCity }

/** A part of the grid of `level` of a generated world: `part` of PARTS, its box in the level's units. */
export interface GridPart {
  level: number
  part: number
  box: Box
}

/** Parts per grid, generated alone (in workers): slabs of its box along z, whole bricks deep but the last. */
const PARTS = 8

/** The parts of a generated world's grid of `level`. */
function partsOf({ box }: Generated, level: number): GridPart[] {
  const whole = box(level)
  const depth = Math.ceil((whole.maxZ - whole.minZ) / PARTS / BRICK) * BRICK
  const parts: GridPart[] = []
  for (let minZ = whole.minZ; minZ < whole.maxZ; minZ += depth) {
    parts.push({ level, part: parts.length, box: { ...whole, minZ, maxZ: Math.min(minZ + depth, whole.maxZ) } })
  }
  return parts
}

// The last world generatePart made, kept for the next part.
let lastGenerated: { seed: number; worldType: WorldType; generated: Generated } | null = null

/** The voxels of a part of a generated world's grid (see World.missingParts). */
export function generatePart(seed: number, worldType: WorldType, { level, box }: GridPart): VoxelGrid {
  if (lastGenerated?.seed !== seed || lastGenerated.worldType !== worldType) {
    const make = GENERATED[worldType]
    if (!make) throw new RangeError(`${worldType} is not generated in parts`)
    lastGenerated = { seed, worldType, generated: make(seed) }
  }
  return generate(lastGenerated.generated.generator, box, level)
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
 * the renderer reads. Generated worlds (GENERATED) are generated level by
 * level, in parts, anywhere (missingParts, receive), or here when asked
 * for (levels); the rest are made here whole, and coarsened.
 */
export class World {
  readonly seed: number
  readonly worldType: WorldType
  /** Where the camera starts. */
  readonly spawn: Readonly<Vec3>

  private readonly source: Generated | undefined
  // By level; generated worlds' only once generated, the rest's coarsened on first use.
  private grids: (VoxelGrid | undefined)[] = []
  // Parts received of grids not whole yet, by level.
  private received = new Map<number, VoxelData[]>()
  // Voxels as a list, built from the grid on first use.
  private list: Voxel[] | null = null
  // Toy world blocks while generating.
  private generated: Voxel[] = []

  constructor(seed: number = Date.now(), worldType: WorldType = 'forest') {
    this.seed = seed
    this.worldType = worldType

    this.source = GENERATED[worldType]?.(seed)
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
    return this.source ? materialsOf(this.source.generator) : this.level(0)?.materials ?? []
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

  /** The grid at each level of detail the renderer reads, generated here if not yet. */
  get levels(): VoxelGrid[] {
    return Array.from({ length: GRID_LEVELS }, (_, level) => this.generate(level))
  }

  /** The voxels as stored at level 0, generated here if not yet. */
  get grid(): VoxelGrid {
    return this.generate(0)
  }

  /** The parts of the grids a generated world has not received, finest level first. */
  missingParts(): GridPart[] {
    const { source } = this
    if (!source) return []
    return Array.from({ length: GRID_LEVELS }, (_, level) => level).filter(level => !this.grids[level]).flatMap(level => partsOf(source, level))
  }

  /** Takes a generated part (generatePart); returns its level's grid once it has all the level's parts. */
  receive({ level }: GridPart, part: VoxelData): VoxelGrid | undefined {
    if (!this.source || this.grids[level]) return
    const parts = this.received.get(level) ?? []
    parts.push(part)
    this.received.set(level, parts)
    if (parts.length < partsOf(this.source, level).length) return
    this.received.delete(level)
    return (this.grids[level] = VoxelGrid.stack(parts.sort((a, b) => a.minZ - b.minZ)))
  }

  /** The grid of `level`, generated here if not yet. */
  private generate(level: number): VoxelGrid {
    let grid = this.level(level)
    if (!grid && this.source) {
      for (const part of partsOf(this.source, level)) grid = this.receive(part, generate(this.source.generator, part.box, level))
    }
    return grid as VoxelGrid
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

  private push(x: number, y: number, z: number, material: Material) {
    this.generated.push({ x, y, z, color: material.color, type: material.type })
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
    const materials: Material[] = [{ color: '#000000', type: 0 }]
    const ids = new Map<string, number>()
    const grid = new VoxelGrid(box, materials)
    for (const { x, y, z, color, type } of blocks) {
      const key = `${color}|${String(type)}`
      let id = ids.get(key)
      if (id === undefined) {
        id = materials.push({ color, type }) - 1
        ids.set(key, id)
      }
      grid.fillBox(BLOCK * x - 1, BLOCK * y - 1, BLOCK * z - 1, BLOCK * x + 1, BLOCK * y + 1, BLOCK * z + 1, id)
    }
    return grid
  }

  /** Fills a column from bedrock to `top`: surface block, 2 subsoil blocks, rock below. */
  private soilColumn(x: number, z: number, top: number, surface: Material, subsoil = DIRT, rock = STONE) {
    for (let y = BEDROCK_Y; y <= top; y++) {
      this.push(x, y, z, y === top ? surface : y > top - 3 ? subsoil : rock)
    }
  }

  private tree(x: number, ground: number, z: number, height: number) {
    for (let y = ground + 1; y < ground + height; y++) {
      this.push(x, y, z, { color: '#654321', type: 4 })
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
      this.push(x, y, z, { color: '#8B7355', type: 4 })
    }
    for (let dx = -2; dx <= 2; dx++) {
      for (let dz = -2; dz <= 2; dz++) {
        if (Math.abs(dx) + Math.abs(dz) <= 2) {
          this.push(x + dx, ground + height, z + dz, { color: '#32CD32', type: 5 })
        }
      }
    }
  }

  private generateColumn(x: number, z: number) {
    switch (this.worldType) {
      case 'hills': {
        const ground = this.terrainHeight(x, z, 0.7)
        this.soilColumn(x, z, ground, { color: '#7CFC00', type: 1 })
        if (this.noise(x * 0.4, 30, z * 0.4) > 0.8) {
          const color = FLOWER_COLORS[Math.floor(this.noise(x, z, 0) * FLOWER_COLORS.length)]
          this.push(x, ground + 1, z, { color, type: 6 })
        }
        break
      }

      case 'mountains': {
        const ground = this.terrainHeight(x, z, 2.5)
        for (let y = BEDROCK_Y; y <= ground; y++) {
          let material: Material
          // Snow caps: the top two blocks of peaks above y=15.
          if (y > 15 && y > ground - 2) material = { color: '#FFFFFF', type: 7 }
          else if (y === ground && y > 8) material = { color: '#808080', type: 3 }
          else if (y === ground) material = GRASS
          else if (y > ground - 3) material = DIRT
          else material = { color: '#555555', type: 3 }
          this.push(x, y, z, material)
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
          this.soilColumn(x, z, top, { color: '#90EE90', type: 1 }, { color: '#C4A76D', type: 2 })
          if (distFromCenter < 5 && this.noise(x * 0.5, 0, z * 0.5) > 0.7) {
            this.palm(x, top, z, 5 + Math.floor(this.noise(x, 0, z) * 2))
          }
        } else if (distFromCenter < 25) {
          this.soilColumn(x, z, 0, WATER, STONE)
        } else {
          const sand = { color: '#F4D03F', type: 9 }
          this.soilColumn(x, z, ground, distFromCenter < 28 ? sand : GRASS)
        }
        break
      }
    }
  }
}
