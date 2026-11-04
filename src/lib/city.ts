import type { Vec3 } from './world'
import { Palette, VoxelGrid } from './voxel-grid'
import { hash, rng } from './noise'

/**
 * Metres per cell of the city's plan: fine enough that a door frame, a
 * window bar or a chair leg is one cell. All sizes below are in plan cells,
 * written as vox(metres). Buildings are boxes on this plan; each plan cell
 * is the box of space holding four voxels of the lattice (voxel-grid.ts),
 * whose unit is half a plan cell.
 * @internal
 */
export const PLAN_CELL_SIZE = 0.25
/** Lattice units per plan cell. */
const UNITS = 2
const vox = (metres: number) => Math.round(metres / PLAN_CELL_SIZE)

// Buildings stand on a grid of square rooms; walls stand on the grid lines.
const CELL = vox(4)
/** Centre of a wall segment, measured from the grid line it starts at. */
const MID = CELL / 2
// A storey: floor layer (row 0), wall rows 1..WALL_TOP, ceiling layer (CEILING).
const STOREY = vox(3.5)
const WALL_TOP = STOREY - 2
const CEILING = STOREY - 1
/** Floor layer of the ground storey, one step above the pavement (y = 0). */
const FLOOR0 = 1
const MAX_STOREYS = 3

// Streets: a 3 × 3 grid of blocks, each a lot ringed by pavement, with roads between and around.
const ROAD = vox(8)
const SIDEWALK = vox(3)
const LOT = vox(22)
const PLOT = LOT + 2 * SIDEWALK
const PERIOD = ROAD + PLOT
const BLOCKS = 3
const SIZE = BLOCKS * PERIOD + ROAD
const MIN_X = -SIZE / 2
const MIN_Z = 0
const MIN_Y = -1
const MAX_Y = FLOOR0 + MAX_STOREYS * STOREY + vox(3.5)

/** A material kind: one of `colors` per plan cell, picked by position unless the caller picks. */
interface Paint {
  colors: readonly string[]
}
const paint = (...colors: string[]): Paint => ({ colors })

const ASPHALT = paint('#3d3f42', '#414346', '#393b3e')
const ROAD_MARK = paint('#e6e2d6')
const PAVING = paint('#aba8a1', '#b4b1aa', '#a29f98')
const KERB = paint('#d0cdc5')
const LAWN = paint('#5f8f3a', '#679a40', '#588536')
const HEDGE = paint('#2f5e2a', '#356a2f', '#2a5426')
const SOIL = paint('#4a3423', '#54392a')
const BARK = paint('#4b3621', '#57402a')
const CANOPY = paint('#4c7f2a', '#5a8f30', '#447324', '#679a38')
const POLE = paint('#2e3236')
const LIGHT = paint('#fff3c4')
const CONCRETE = paint('#9a9690', '#a19d97')
const PLINTH = paint('#5e5a55', '#57534e')
const GRAVEL = paint('#6d6a66', '#75716c', '#686561')
const GLASS = paint('#5b7f97')
const CEILING_PAINT = paint('#f4f2ec')
const SKIRTING = paint('#6b5a48')
const BRASS = paint('#c9a640')
const RAILING = paint('#2b2d30')
const ROOF_UNIT = paint('#9aa0a6', '#a3a9ae')
const WATER = paint('#3f7fa8')
const COURT_PAVING = paint('#c8c0b0', '#bdb5a5')
const TABLE = paint('#6e4a2c')
const CHAIR = paint('#4a3424')
const SHELF = paint('#5c3e26')
const BOOKS = paint('#8c2f2f', '#2f5a8c', '#d8b04a', '#3f7a4a', '#5a3a6e', '#e0dccf')
const POT = paint('#b5603c')
const PLANT = paint('#3f7f35', '#4a8f3d', '#357030')

// Per building: one of each.
const FACADES = ['#c9a27e', '#b5654a', '#d8cfb8', '#8a9aa6', '#a8b08a', '#c47d5a', '#7d8a7a', '#d6b85a']
const TRIMS = ['#f2efe6', '#e6dfcc', '#4a4a4a']
const ENTRANCE_DOORS = ['#2f4f3a', '#7a2b2b', '#2b3a5a', '#3a3a3a']
const INTERIORS = ['#ece6d8', '#e3e8ea', '#efe3d3', '#dfe6d6', '#f3f0ea']
const ROOM_DOORS = ['#8a5a32', '#6e4a2c', '#ece8de']

// Floors, one pattern per room.
const FLOORS = {
  checker: [paint('#e8e4dc'), paint('#2c2c2e')],
  tiles: [paint('#d9d4c8'), paint('#c9c3b5')],
  parquet: [paint('#a9743f', '#b8834a', '#9a6634', '#c0904f')],
  carpet: [paint('#7a2e35', '#2e4a6b', '#3e5e3a', '#6b5a7a'), paint('#c9a85a', '#d8d0bc')],
  concrete: [CONCRETE],
}
type Floor = keyof typeof FLOORS
const FLOOR_KINDS = Object.keys(FLOORS) as Floor[]

/** The voxel grid, painted cell by cell of the plan, with a material list that grows as paints are used. */
class Canvas {
  private readonly palette = new Palette()
  // Plan cell (x, y, z) holds the sites with x in [UNITS · x − 1, UNITS · x] (y, z likewise).
  readonly grid = new VoxelGrid(
    {
      // EXPERIMENT: wide enough for seed 777's ground too.
      minX: Math.min(UNITS * MIN_X - 1, -401), minY: UNITS * MIN_Y - 1, minZ: Math.min(UNITS * MIN_Z - 1, -201),
      maxX: Math.max(UNITS * (MIN_X + SIZE) - 1, 401), maxY: UNITS * MAX_Y - 1, maxZ: UNITS * (MIN_Z + SIZE) - 1,
    },
    this.palette.materials,
  )
  constructor(private readonly shadeSeed: number) {}

  /** Fills plan cell (x, y, z) with `p` (one of its colours: picked by `pick` in [0, 1), else by position); null clears. Out-of-range cells are ignored. */
  put(x: number, y: number, z: number, p: Paint | null, pick?: number) {
    const id = p === null ? 0 : this.palette.id(p.colors[Math.floor((pick ?? hash(this.shadeSeed, x, y, z)) * p.colors.length)])
    this.grid.fillBox(UNITS * x - 1, UNITS * y - 1, UNITS * z - 1, UNITS * x + 1, UNITS * y + 1, UNITS * z + 1, id)
  }

  /** Fills the box with corners (x0, y0, z0) and (x1, y1, z1), inclusive. */
  box(x0: number, y0: number, z0: number, x1: number, y1: number, z1: number, p: Paint | null) {
    for (let x = Math.min(x0, x1); x <= Math.max(x0, x1); x++) {
      for (let y = Math.min(y0, y1); y <= Math.max(y0, y1); y++) {
        for (let z = Math.min(z0, z1); z <= Math.max(z0, z1); z++) this.put(x, y, z, p)
      }
    }
  }
}

/**
 * A wall segment in its own coordinates: u along the wall from a grid
 * intersection, row up from the storey's floor layer, w across the wall
 * (w = 0 on the grid line, growing `out`wards: outdoors for facades).
 */
interface Span {
  x: number
  y: number
  z: number
  alongX: boolean
  out: 1 | -1
}

class Wall {
  constructor(private readonly canvas: Canvas, private readonly span: Span) {}

  put(u: number, row: number, w: number, p: Paint | null) {
    const { x, y, z, alongX, out } = this.span
    if (alongX) this.canvas.put(x + u, y + row, z + out * w, p)
    else this.canvas.put(x + out * w, y + row, z + u, p)
  }

  /** Fills u0..u1 × row0..row1 × w0..w1 (inclusive, either order). */
  fill(u0: number, u1: number, row0: number, row1: number, w0: number, w1: number, p: Paint | null) {
    for (let u = Math.min(u0, u1); u <= Math.max(u0, u1); u++) {
      for (let row = row0; row <= row1; row++) {
        for (let w = Math.min(w0, w1); w <= Math.max(w0, w1); w++) this.put(u, row, w, p)
      }
    }
  }

  /** A frame one voxel proud of the wall on layer w: posts at u0 and u1 up to `top`, lintel at `top`. */
  architrave(u0: number, u1: number, top: number, w: number, p: Paint) {
    this.fill(u0, u0, 1, top, w, w, p)
    this.fill(u1, u1, 1, top, w, w, p)
    this.fill(u0, u1, top, top, w, w, p)
  }
}

// Openings, centred on the segment (u = MID). Walls are `thick` layers, w = 0 .. thick - 1.
const DOOR_TOP = vox(2.25)
const SILL = vox(0.75)
const WINDOW_TOP = vox(2.75)
const TRANSOM = WINDOW_TOP - 2

type Leaf = 'open-in' | 'open-out' | 'closed' | 'none'

/** A 1 m doorway with a frame on both sides and a leaf: swung open against the wall, closed, or none. */
function door(wall: Wall, thick: number, leaf: Leaf, leafPaint: Paint, trim: Paint, glazed = false) {
  const c = MID
  wall.fill(c - 2, c + 1, 1, DOOR_TOP, 0, thick - 1, null)
  wall.architrave(c - 3, c + 2, DOOR_TOP + 1, -1, trim)
  wall.architrave(c - 3, c + 2, DOOR_TOP + 1, thick, trim)
  if (leaf === 'closed') {
    wall.fill(c - 2, c + 1, 1, DOOR_TOP, 0, 0, leafPaint)
    if (glazed) wall.fill(c - 1, c, 5, 7, 0, 0, GLASS)
    wall.put(c + 1, 4, -1, BRASS)
    wall.put(c + 1, 4, 1, BRASS)
  } else if (leaf !== 'none') {
    // Hinged at u = c - 2, standing perpendicular to the wall.
    const d = leaf === 'open-in' ? -1 : 1
    const w0 = d < 0 ? -1 : thick
    for (let k = 0; k < 4; k++) wall.fill(c - 2, c - 2, 1, DOOR_TOP, w0 + d * k, w0 + d * k, leafPaint)
    if (glazed) wall.fill(c - 2, c - 2, 5, 7, w0 + d, w0 + d * 2, GLASS)
    wall.put(c - 3, 4, w0 + d * 3, BRASS)
    wall.put(c - 1, 4, w0 + d * 3, BRASS)
  }
}

/** A 2.5 m wide opening between rooms, framed on both sides. */
function arch(wall: Wall, trim: Paint) {
  wall.fill(3, CELL - 4, 1, DOOR_TOP + 1, 0, 0, null)
  wall.architrave(2, CELL - 3, DOOR_TOP + 2, -1, trim)
  wall.architrave(2, CELL - 3, DOOR_TOP + 2, 1, trim)
}

/**
 * A 1.75 m window: glass on the inner layer with a mullion and a transom,
 * the outer layers cut back into a reveal, a sill outside and a board inside.
 * A French window (`toFloor`) reaches down to the floor.
 */
function window(wall: Wall, thick: number, trim: Paint, toFloor = false) {
  const c = MID
  const bottom = toFloor ? 1 : SILL + 1
  if (thick > 1) wall.fill(c - 3, c + 3, bottom, WINDOW_TOP, 1, thick - 1, null)
  for (let u = c - 3; u <= c + 3; u++) {
    for (let row = bottom; row <= WINDOW_TOP; row++) {
      const bar = row === TRANSOM || (u === c && row < TRANSOM)
      wall.put(u, row, 0, bar ? trim : GLASS)
    }
  }
  if (thick > 1) wall.fill(c - 4, c + 4, WINDOW_TOP + 1, WINDOW_TOP + 1, thick - 1, thick - 1, trim)
  if (!toFloor) {
    wall.fill(c - 4, c + 4, SILL, SILL, thick, thick, trim)
    wall.fill(c - 3, c + 3, SILL, SILL, -1, -1, trim)
  }
}

/** A French window onto a 1 m deep balcony with a railing. */
function balcony(wall: Wall, thick: number, trim: Paint) {
  window(wall, thick, trim, true)
  const c = MID
  const depth = vox(1)
  wall.fill(c - 4, c + 4, 0, 0, thick, thick + depth - 1, CONCRETE)
  for (let u = c - 4; u <= c + 4; u++) {
    for (let w = thick; w < thick + depth; w++) {
      const edge = u === c - 4 || u === c + 4 || w === thick + depth - 1
      if (!edge) continue
      wall.put(u, 4, w, RAILING)
      if ((u + w) % 2 === 0) wall.fill(u, u, 1, 3, w, w, RAILING)
    }
  }
}

// ---------------------------------------------------------------------------
// Buildings

type EdgeKind = 'wall' | 'door' | 'arch' | 'open' | 'window' | 'balcony' | 'entrance'

interface Edge {
  kind: EdgeKind
  /** Two layers thick with the outer one in facade colours: outside walls and walls onto the courtyard. */
  facade: boolean
  /** On the building's outside (not the courtyard). */
  exterior: boolean
  out: 1 | -1
  leaf: Leaf
}

interface Building {
  /** Grid line x = bx + i · CELL, z = bz + j · CELL. */
  bx: number
  bz: number
  w: number
  d: number
  storeys: number
  court: { i: number; j: number } | null
  /** Cell with the front (-z) entrance. */
  entrance: number
  /** Cell with a back door, if any. */
  backDoor: number | null
  facade: Paint
  trim: Paint
  interior: Paint
  entranceDoor: Paint
}

type Side = 'left' | 'right' | 'front' | 'back'

/** Edges of one storey: on the lines x = const (between cells (i - 1, j) and (i, j)) and z = const. */
class Plan {
  constructor(readonly b: Building, readonly xEdges: Edge[], readonly zEdges: Edge[]) {}
  xEdge(i: number, j: number) { return this.xEdges[i * this.b.d + j] }
  zEdge(i: number, j: number) { return this.zEdges[j * this.b.w + i] }
  side(i: number, j: number, side: Side): Edge {
    if (side === 'left') return this.xEdge(i, j)
    if (side === 'right') return this.xEdge(i + 1, j)
    if (side === 'front') return this.zEdge(i, j)
    return this.zEdge(i, j + 1)
  }
}

const isCourt = (b: Building, i: number, j: number) => b.court?.i === i && b.court.j === j
const inside = (b: Building, i: number, j: number) => i >= 0 && i < b.w && j >= 0 && j < b.d

/**
 * Lays out one storey as a maze: a random spanning tree of doorways joins
 * all rooms, plus a few extra openings so there are loops. Outside walls
 * get windows (and on the ground floor the entrances), upper storeys
 * sometimes balconies.
 */
function planStorey(b: Building, storey: number, random: () => number): Plan {
  const between: { e: Edge; a: number; c: number }[] = []
  /** The edge between cell a and its +x or +z neighbour c; at most one of them is outside. */
  const edge = (ai: number, aj: number, ci: number, cj: number): Edge => {
    const aIn = inside(b, ai, aj)
    if (!aIn || !inside(b, ci, cj)) {
      const roll = random()
      const kind: EdgeKind = storey > 0 && roll < 0.2 ? 'balcony' : roll > 0.92 ? 'wall' : 'window'
      return { kind, facade: true, exterior: true, out: aIn ? 1 : -1, leaf: 'none' }
    }
    const cCourt = isCourt(b, ci, cj)
    if (cCourt || isCourt(b, ai, aj)) return { kind: 'window', facade: true, exterior: false, out: cCourt ? 1 : -1, leaf: 'none' }
    const e: Edge = { kind: 'wall', facade: false, exterior: false, out: random() < 0.5 ? 1 : -1, leaf: 'none' }
    between.push({ e, a: aj * b.w + ai, c: cj * b.w + ci })
    return e
  }
  const plan = new Plan(
    b,
    Array.from({ length: (b.w + 1) * b.d }, (_, k) => edge(Math.floor(k / b.d) - 1, k % b.d, Math.floor(k / b.d), k % b.d)),
    Array.from({ length: b.w * (b.d + 1) }, (_, k) => edge(k % b.w, Math.floor(k / b.w) - 1, k % b.w, Math.floor(k / b.w))),
  )

  // Kruskal over the room-to-room walls in random order.
  const parent = Array.from({ length: b.w * b.d }, (_, k) => k)
  const root = (k: number): number => (parent[k] === k ? k : (parent[k] = root(parent[k])))
  for (let k = between.length - 1; k > 0; k--) {
    const r = Math.floor(random() * (k + 1))
    ;[between[k], between[r]] = [between[r], between[k]]
  }
  for (const { e, a, c } of between) {
    if (root(a) !== root(c)) parent[root(a)] = root(c)
    else if (random() > 0.15) continue
    const roll = random()
    e.kind = roll < 0.72 ? 'door' : roll < 0.87 ? 'arch' : 'open'
    const l = random()
    e.leaf = l < 0.3 ? 'open-in' : l < 0.6 ? 'open-out' : l < 0.75 ? 'closed' : 'none'
  }

  if (storey === 0) {
    const front = plan.zEdge(b.entrance, 0)
    front.kind = 'entrance'
    front.leaf = 'open-in'
    if (b.backDoor !== null) {
      const back = plan.zEdge(b.backDoor, b.d)
      back.kind = 'entrance'
      back.leaf = 'closed'
    }
    // One door from a neighbouring room out into the courtyard.
    if (b.court) {
      const e = plan.side(b.court.i, b.court.j, (['left', 'right', 'front', 'back'] as const)[Math.floor(random() * 4)])
      e.kind = 'door'
      e.leaf = 'open-out'
    }
  }
  return plan
}

function drawBuilding(canvas: Canvas, b: Building, seed: number) {
  const random = rng(seed)
  const x1 = b.bx + b.w * CELL
  const z1 = b.bz + b.d * CELL
  const top = FLOOR0 + b.storeys * STOREY
  const courtInterior = (x: number, z: number) => {
    if (!b.court) return false
    const u = x - b.bx - b.court.i * CELL
    const v = z - b.bz - b.court.j * CELL
    return u > 0 && u < CELL && v > 0 && v < CELL
  }

  canvas.box(b.bx, 0, b.bz, x1, 0, z1, CONCRETE)

  for (let s = 0; s < b.storeys; s++) {
    const y0 = FLOOR0 + s * STOREY
    const plan = planStorey(b, s, random)

    // Floor and ceiling layers.
    for (let x = b.bx; x <= x1; x++) {
      for (let z = b.bz; z <= z1; z++) {
        if (courtInterior(x, z)) {
          if (s === 0) canvas.put(x, y0, z, COURT_PAVING, ((x >> 1) + (z >> 1)) % 2 === 0 ? 0 : 0.99)
          continue
        }
        canvas.put(x, y0, z, CONCRETE)
        canvas.put(x, y0 + CEILING, z, CEILING_PAINT)
      }
    }

    // Walls.
    const segment = (i: number, j: number, alongX: boolean, e: Edge) =>
      new Wall(canvas, { x: b.bx + i * CELL, y: y0, z: b.bz + j * CELL, alongX, out: e.out })
    const eachEdge = (f: (wall: Wall, e: Edge) => void) => {
      for (let i = 0; i <= b.w; i++) for (let j = 0; j < b.d; j++) f(segment(i, j, false, plan.xEdge(i, j)), plan.xEdge(i, j))
      for (let j = 0; j <= b.d; j++) for (let i = 0; i < b.w; i++) f(segment(i, j, true, plan.zEdge(i, j)), plan.zEdge(i, j))
    }
    eachEdge((wall, e) => {
      if (e.kind === 'open') return
      wall.fill(1, CELL - 1, 2, WALL_TOP, 0, 0, b.interior)
      wall.fill(1, CELL - 1, 1, 1, 0, 0, SKIRTING)
      if (!e.facade) return
      // The outer layer wraps the building's corners; on the courtyard it stays inside the court.
      const [u0, u1] = e.exterior ? [-1, CELL + 1] : [1, CELL - 1]
      const r0 = e.exterior && s === 0 ? -1 : 0
      wall.fill(u0, u1, r0, CEILING, 1, 1, b.facade)
      if (s === 0) wall.fill(u0, u1, r0, 1, 1, 1, PLINTH)
      if (e.exterior && s < b.storeys - 1) wall.fill(u0 - 1, u1 + 1, CEILING, CEILING, 2, 2, b.trim)
    })
    for (let i = 0; i <= b.w; i++) {
      for (let j = 0; j <= b.d; j++) {
        const x = b.bx + i * CELL
        const z = b.bz + j * CELL
        canvas.box(x, y0 + 2, z, x, y0 + WALL_TOP, z, b.interior)
        canvas.put(x, y0 + 1, z, SKIRTING)
      }
    }

    // Openings.
    eachEdge((wall, e) => {
      const thick = e.facade ? 2 : 1
      switch (e.kind) {
        case 'door':
          door(wall, thick, e.leaf, paint(ROOM_DOORS[Math.floor(random() * ROOM_DOORS.length)]), b.trim)
          break
        case 'arch':
          arch(wall, b.trim)
          break
        case 'window':
          window(wall, thick, b.trim)
          break
        case 'balcony':
          balcony(wall, thick, b.trim)
          break
        case 'entrance':
          door(wall, thick, e.leaf, b.entranceDoor, b.trim, true)
          // Landing, step down to the pavement, and a canopy.
          wall.fill(MID - 3, MID + 2, 0, 0, 2, 3, PLINTH)
          wall.fill(MID - 4, MID + 3, DOOR_TOP + 2, DOOR_TOP + 2, 2, 4, b.trim)
          break
        case 'wall':
        case 'open':
          break
      }
    })

    // Rooms: floor pattern, ceiling lamp, furniture.
    for (let i = 0; i < b.w; i++) {
      for (let j = 0; j < b.d; j++) {
        if (isCourt(b, i, j)) {
          if (s === 0) courtyard(canvas, b.bx + i * CELL, y0, b.bz + j * CELL, random)
          continue
        }
        const front = s === 0 && i === b.entrance && j === 0
        room(canvas, plan, i, j, y0, front ? 'checker' : FLOOR_KINDS[Math.floor(random() * FLOOR_KINDS.length)], random)
      }
    }
  }

  // Roof: gravel, a parapet with coping, a stair housing and a few machines.
  for (let x = b.bx; x <= x1; x++) {
    for (let z = b.bz; z <= z1; z++) if (!courtInterior(x, z)) canvas.put(x, top, z, GRAVEL)
  }
  const parapet = (alongX: boolean, x: number, z: number, out: 1 | -1, length: number) => {
    const wall = new Wall(canvas, { x, y: top, z, alongX, out })
    wall.fill(-1, length + 1, 0, 2, 0, 1, b.facade)
    wall.fill(-1, length + 1, 3, 3, 0, 1, b.trim)
  }
  parapet(true, b.bx, b.bz, -1, b.w * CELL)
  parapet(true, b.bx, z1, 1, b.w * CELL)
  parapet(false, b.bx, b.bz, -1, b.d * CELL)
  parapet(false, x1, b.bz, 1, b.d * CELL)

  const rooms: [number, number][] = []
  for (let i = 0; i < b.w; i++) for (let j = 0; j < b.d; j++) if (!isCourt(b, i, j)) rooms.push([i, j])
  const pickRoom = () => rooms[Math.floor(random() * rooms.length)]
  {
    const [i, j] = pickRoom()
    const x = b.bx + i * CELL
    const z = b.bz + j * CELL
    canvas.box(x + 4, top + 1, z + 4, x + CELL - 4, top + vox(2.5), z + CELL - 4, b.facade)
    canvas.box(x + 3, top + vox(2.5) + 1, z + 3, x + CELL - 3, top + vox(2.5) + 1, z + CELL - 3, b.trim)
    canvas.box(x + MID - 2, top + 1, z + 4, x + MID + 1, top + DOOR_TOP, z + 4, POLE)
  }
  const units = 1 + Math.floor(random() * 3)
  for (let k = 0; k < units; k++) {
    const [i, j] = pickRoom()
    const x = b.bx + i * CELL + 2 + Math.floor(random() * 8)
    const z = b.bz + j * CELL + 2 + Math.floor(random() * 8)
    canvas.box(x, top + 1, z, x + 2 + Math.floor(random() * 3), top + 2 + Math.floor(random() * 3), z + 2 + Math.floor(random() * 2), ROOF_UNIT)
  }
}

/** Interior of room (i, j) on the storey whose floor layer is y0. */
function room(canvas: Canvas, plan: Plan, i: number, j: number, y0: number, floor: Floor, random: () => number) {
  const { bx, bz } = plan.b
  const x0 = bx + i * CELL
  const z0 = bz + j * CELL
  const [a, b] = FLOORS[floor]
  const roomSeed = Math.floor(random() * 1e9)
  const pick = random()
  for (let u = 1; u < CELL; u++) {
    for (let v = 1; v < CELL; v++) {
      const x = x0 + u
      const z = z0 + v
      switch (floor) {
        case 'checker':
          canvas.put(x, y0, z, ((u >> 1) + (v >> 1)) % 2 === 0 ? a : b)
          break
        case 'tiles':
          canvas.put(x, y0, z, ((u >> 2) + (v >> 2)) % 2 === 0 ? a : b)
          break
        case 'parquet': {
          // Planks 1 m long, staggered row by row, each in one shade.
          const plank = Math.floor((u + (v % 2) * 2) / 4)
          canvas.put(x, y0, z, a, hash(roomSeed, plank, v))
          break
        }
        case 'carpet': {
          const ring = Math.min(u, v, CELL - u, CELL - v)
          canvas.put(x, y0, z, ring === 2 ? b : ring > 2 ? a : CONCRETE, pick)
          break
        }
        case 'concrete':
          canvas.put(x, y0, z, a)
          break
      }
    }
  }
  canvas.box(x0 + MID - 1, y0 + WALL_TOP, z0 + MID - 1, x0 + MID, y0 + WALL_TOP, z0 + MID, LIGHT)

  // Furniture. Local (u, v) to world; `along` places things against a side, t along it, depth d from it.
  const at = (u: number, v: number, row: number, p: Paint, pk?: number) => { canvas.put(x0 + u, y0 + row, z0 + v, p, pk) }
  const along = (side: Side, t: number, d: number): [number, number] =>
    side === 'left' ? [d, t] : side === 'right' ? [CELL - d, t] : side === 'front' ? [t, d] : [t, CELL - d]

  if (random() < 0.5) {
    // Table 1.25 m × 0.75 m with a chair on each long side.
    for (let u = MID - 2; u <= MID + 2; u++) for (let v = MID - 1; v <= MID + 1; v++) at(u, v, 3, TABLE)
    for (const [u, v] of [[MID - 2, MID - 1], [MID + 2, MID - 1], [MID - 2, MID + 1], [MID + 2, MID + 1]]) {
      at(u, v, 1, TABLE)
      at(u, v, 2, TABLE)
    }
    for (const [v, back] of [[MID - 3, MID - 4], [MID + 3, MID + 4]]) {
      at(MID, v, 1, CHAIR)
      at(MID, v, 2, CHAIR)
      for (let row = 1; row <= 4; row++) at(MID, back, row, CHAIR)
    }
  }
  for (const side of ['left', 'right', 'front', 'back'] as const) {
    const e = plan.side(i, j, side)
    if (e.kind !== 'wall' || e.facade || random() > 0.4) continue
    // Bookcase: shelves every 0.75 m, books in between with gaps.
    for (let t = 3; t <= CELL - 4; t++) {
      for (let row = 1; row <= 8; row++) {
        const [u, v] = along(side, t, 1)
        if (row % 3 === 1 || t === 3 || t === CELL - 4) at(u, v, row, SHELF)
        else if (random() < 0.8) at(u, v, row, BOOKS, random())
      }
    }
  }
  if (random() < 0.4) {
    // Potted plant in a corner.
    const [u, v] = [[1, 1], [CELL - 1, 1], [1, CELL - 1], [CELL - 1, CELL - 1]][Math.floor(random() * 4)]
    at(u, v, 1, POT)
    at(u, v, 2, POT)
    for (let row = 3; row <= 6; row++) at(u, v, row, PLANT)
    const du = u === 1 ? 1 : -1
    const dv = v === 1 ? 1 : -1
    for (const [a2, b2] of [[du, 0], [0, dv], [du, dv]]) for (let row = 5; row <= 6; row++) at(u + a2, v + b2, row, PLANT)
  }
}

/** The courtyard floor's centrepiece: a tree in a planter, or a fountain. */
function courtyard(canvas: Canvas, x0: number, y0: number, z0: number, random: () => number) {
  const cx = x0 + MID
  const cz = z0 + MID
  if (random() < 0.5) {
    canvas.box(cx - 2, y0 + 1, cz - 2, cx + 2, y0 + 2, cz + 2, PLINTH)
    canvas.box(cx - 1, y0 + 2, cz - 1, cx + 1, y0 + 2, cz + 1, SOIL)
    canvas.box(cx, y0 + 3, cz, cx, y0 + vox(2.5), cz, BARK)
    treeCrown(canvas, cx, y0 + vox(2.5), cz)
  } else {
    canvas.box(cx - 3, y0 + 1, cz - 3, cx + 3, y0 + 2, cz + 3, PLINTH)
    canvas.box(cx - 2, y0 + 1, cz - 2, cx + 2, y0 + 2, cz + 2, WATER)
    canvas.box(cx, y0 + 1, cz, cx, y0 + 4, cz, PLINTH)
    canvas.put(cx, y0 + 5, cz, WATER)
  }
}

/** A boxy crown, 1.25 m across with its edges trimmed, sitting on `y`. */
function treeCrown(canvas: Canvas, x: number, y: number, z: number) {
  for (let dx = -2; dx <= 2; dx++) {
    for (let dz = -2; dz <= 2; dz++) {
      for (let dy = 0; dy <= 4; dy++) {
        const edges = Number(Math.abs(dx) === 2) + Number(Math.abs(dz) === 2) + Number(dy === 0 || dy === 4)
        if (edges < 2) canvas.put(x + dx, y + dy, z + dz, CANOPY)
      }
    }
  }
}

// ---------------------------------------------------------------------------
// Streets

/** Where t (voxels from the city's edge) falls: on a road or in a plot, and how far into it. */
function strip(t: number): { road: boolean; offset: number } {
  const offset = t % PERIOD
  return offset < ROAD ? { road: true, offset } : { road: false, offset: offset - ROAD }
}

/** Road markings: a dashed centre line, and zebra crossings next to the junctions. */
function roadPaint(across: number, along: number): Paint {
  const zebra = along >= 2 && along < 2 + vox(3)
  const zebraFar = along < PLOT - 2 && along >= PLOT - 2 - vox(3)
  if (zebra || zebraFar) return across >= 2 && across < ROAD - 2 && (across >> 1) % 2 === 0 ? ROAD_MARK : ASPHALT
  return across === ROAD / 2 && along % vox(6) < vox(3) ? ROAD_MARK : ASPHALT
}

function streetLamp(canvas: Canvas, x: number, z: number, dx: number, dz: number) {
  const height = vox(4)
  canvas.box(x, 1, z, x, height, z, POLE)
  for (let k = 1; k <= 3; k++) canvas.put(x + dx * k, height, z + dz * k, POLE)
  canvas.put(x + dx * 3, height - 1, z + dz * 3, LIGHT)
}

function streetTree(canvas: Canvas, x: number, z: number) {
  canvas.box(x - 1, 0, z - 1, x + 1, 0, z + 1, SOIL)
  canvas.box(x, 1, z, x, vox(2.25), z, BARK)
  treeCrown(canvas, x, vox(2.25), z)
}

/**
 * A district of streets and buildings for looking at walls, doors and
 * windows: a 3 × 3 grid of blocks, each with a building of 1 to 3 storeys
 * whose rooms form a maze joined by doorways, some with a courtyard.
 * Pavements with kerbs, street lamps and trees; zebra crossings at the
 * junctions. The camera starts on the road in front of an open entrance.
 */
export function generateCity(seed: number): { grid: VoxelGrid; spawn: Vec3 } {
  const s = (k: number) => (seed | 0) + k * 1013904223
  const canvas = new Canvas(s(1))

  // EXPERIMENT: seed 777 is flat ground ruled in straight lines, every 16 and 64 units, to see how slices bend them.
  if (seed === 777) {
    const [white, red, dark] = [paint('#f0f0f0'), paint('#d04040'), paint('#404040')]
    const ruled = (v: number, period: number, width: number) => ((v % period) + period) % period < width
    for (let x = -200; x < 200; x++) {
      for (let z = -100; z < 300; z++) {
        canvas.put(x, -1, z, ruled(x, 32, 4) || ruled(z, 32, 4) ? white : ruled(x, 8, 2) || ruled(z, 8, 2) ? red : dark)
      }
    }
    return { grid: canvas.grid, spawn: { x: 0, y: 200, z: 0 } }
  }
  // Ground.
  for (let tx = 0; tx < SIZE; tx++) {
    const sx = strip(tx)
    for (let tz = 0; tz < SIZE; tz++) {
      const sz = strip(tz)
      const x = MIN_X + tx
      const z = MIN_Z + tz
      if (sx.road || sz.road) {
        const p = sx.road && sz.road ? ASPHALT : sx.road ? roadPaint(sx.offset, sz.offset) : roadPaint(sz.offset, sx.offset)
        canvas.put(x, -1, z, p)
        continue
      }
      const ring = Math.min(sx.offset, sz.offset, PLOT - 1 - sx.offset, PLOT - 1 - sz.offset)
      if (ring < SIDEWALK) {
        canvas.put(x, -1, z, CONCRETE)
        // Paving slabs 1 m square, a kerb along the road.
        canvas.put(x, 0, z, ring === 0 ? KERB : PAVING, ring === 0 ? 0 : hash(s(2), x >> 2, z >> 2))
      } else {
        canvas.put(x, -1, z, SOIL)
        canvas.put(x, 0, z, LAWN)
      }
    }
  }

  let spawn: Vec3 = { x: 0, y: 0, z: 0 }
  for (let bi = 0; bi < BLOCKS; bi++) {
    for (let bj = 0; bj < BLOCKS; bj++) {
      const random = rng(s(3) + bi * 7349 + bj * 15731)
      const px = MIN_X + bi * PERIOD + ROAD
      const pz = MIN_Z + bj * PERIOD + ROAD

      // Lamps at the corners and trees between them, along every side of the block.
      for (const t of [vox(2), PLOT - 1 - vox(2)]) {
        streetLamp(canvas, px + t, pz + 1, 0, -1)
        streetLamp(canvas, px + t, pz + PLOT - 2, 0, 1)
        streetLamp(canvas, px + 1, pz + t, -1, 0)
        streetLamp(canvas, px + PLOT - 2, pz + t, 1, 0)
      }
      for (const t of [vox(8), PLOT - 1 - vox(8)]) {
        streetTree(canvas, px + t, pz + vox(1.5))
        streetTree(canvas, px + t, pz + PLOT - 1 - vox(1.5))
        streetTree(canvas, px + vox(1.5), pz + t)
        streetTree(canvas, px + PLOT - 1 - vox(1.5), pz + t)
      }

      const front = bi === 1 && bj === 0
      const w = 3 + Math.floor(random() * 3)
      const d = 3 + Math.floor(random() * 3)
      const lx = px + SIDEWALK
      const lz = pz + SIDEWALK
      const pick = <T,>(list: readonly T[]) => list[Math.floor(random() * list.length)]
      const building: Building = {
        bx: lx + Math.floor((LOT - w * CELL) / 2),
        bz: lz + 2,
        w,
        d,
        storeys: front ? 2 + Math.floor(random() * 2) : 1 + Math.floor(random() * MAX_STOREYS),
        court: random() < 0.6 ? { i: 1 + Math.floor(random() * (w - 2)), j: 1 + Math.floor(random() * (d - 2)) } : null,
        entrance: Math.floor(w / 2),
        backDoor: random() < 0.5 ? Math.floor(random() * w) : null,
        facade: paint(pick(FACADES)),
        trim: paint(pick(TRIMS)),
        interior: paint(pick(INTERIORS)),
        entranceDoor: paint(pick(ENTRANCE_DOORS)),
      }

      // Hedges round the back and sides of the lot, with a gap behind the back door.
      const z1 = building.bz + d * CELL
      const gate = building.backDoor === null ? null : building.bx + building.backDoor * CELL + MID
      for (let k = 0; k < LOT; k++) {
        if (gate === null || Math.abs(lx + k - gate) > 3) canvas.box(lx + k, 1, lz + LOT - 1, lx + k, 3, lz + LOT - 1, HEDGE)
        if (lz + k > z1 + 2) {
          canvas.box(lx, 1, lz + k, lx, 3, lz + k, HEDGE)
          canvas.box(lx + LOT - 1, 1, lz + k, lx + LOT - 1, 3, lz + k, HEDGE)
        }
      }

      drawBuilding(canvas, building, s(4) + bi * 31 + bj)
      if (front) {
        // In the road before the entrance, at eye height.
        spawn = { x: building.bx + building.entrance * CELL + MID - 1, y: -1 + vox(1.75), z: MIN_Z + vox(2) }
      }
    }
  }

  // In lattice units, at a site of the plan cell (the upper of its two along each axis).
  return { grid: canvas.grid, spawn: { x: UNITS * spawn.x, y: UNITS * spawn.y, z: UNITS * spawn.z } }
}
