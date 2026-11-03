/**
 * Dumps a view of the forest for the native spike (native/src/dump.rs),
 * without WebAssembly: the renderer lists the chunks in view through the
 * recording Bakery below, and the faces' lattices and the world's grids are
 * written as chunk-baker.ts hands them to the baker.
 * Runs only when asked for: DUMP=frame.bin pnpm vitest run native/dump.test.ts
 * (SEED picks the forest). Outside tsconfig and ESLint: it needs Node's fs.
 */
import { writeFileSync } from 'fs'
import { test, vi } from 'vitest'
import type { ChunkRequest } from '../src/lib/chunk-baker'
import { ParallaxRenderer, type Bakery } from '../src/lib/renderer'
import { sliceCentre } from '../src/lib/slice-selection'
import { FACES } from '../src/lib/view'
import { LayerIndex, usedCells, type LayerIndexData, type VoxelData } from '../src/lib/voxel-grid'
import { generatePart, World, type GridPart, type WorldType } from '../src/lib/world'

vi.mock('../src/lib/chunk-baker', () => ({ ChunkBaker: class {} }))
vi.mock('../src/lib/occlusion', () => ({ Coverage: class {} }))

const WIDTH = 1280
const HEIGHT = 800

test.skipIf(!process.env.DUMP)('dump a view for the native spike', () => {
  const grids: (VoxelData | undefined)[] = []
  let requests: ChunkRequest[] = []
  let generated: (part: GridPart, grid: VoxelData, index: LayerIndexData) => void = () => undefined
  const queue: { seed: number; worldType: WorldType; part: GridPart }[] = []
  const bakery: Bakery = {
    connect(_, g) { generated = g },
    setWorld(_, levels) { levels.forEach((g, l) => { grids[l] = g }) },
    addGrid(level, grid) { grids[level] = grid },
    generate(seed, worldType, parts) { for (const part of parts) queue.push({ seed, worldType, part }) },
    want(r) { requests = r },
  }
  const world = new World(Number(process.env.SEED ?? 12345), 'forest')
  const renderer = new ParallaxRenderer(world, { bakery })
  renderer.resize(WIDTH, HEIGHT)
  renderer.setSkipCovered(false)
  const start = performance.now()
  for (;;) {
    renderer.render()
    if (queue.length === 0) break
    for (const { seed, worldType, part } of queue.splice(0)) {
      const grid = generatePart(seed, worldType, part)
      const { keys, tables } = new LayerIndex(grid, FACES.map(f => f.lattice))
      generated(part, grid, { keys, tables })
    }
  }
  console.log(`generated in ${((performance.now() - start) / 1000).toFixed(1)} s, ${requests.length} chunks wanted`)

  const parts: Uint8Array[] = []
  const bytes = (a: Uint8Array | Int32Array | Uint32Array | Float64Array) => parts.push(new Uint8Array(a.buffer, a.byteOffset, a.byteLength))
  const u32 = (...v: number[]) => bytes(Uint32Array.from(v))
  const i32 = (...v: number[]) => bytes(Int32Array.from(v))
  const f64 = (...v: number[]) => bytes(Float64Array.from(v))
  const array = (a: Uint8Array | Int32Array | Uint32Array | Float64Array) => { u32(a.length); bytes(a) }

  u32(0x32445850)
  const camera = renderer.getCamera()
  const { yaw, pitch } = renderer.getOrientation()
  f64(camera.x, camera.y, camera.z, yaw, pitch, renderer.getFov(), WIDTH, HEIGHT, renderer.getRenderDistance())
  for (const face of FACES) f64(...[face.right, face.up, face.forward].flatMap(v => [v.x, v.y, v.z]))
  array(Int32Array.from(FACES.flatMap(({ lattice: { right, up, forward } }) => [right, up, forward].flatMap(v => [v.x, v.y, v.z]))))
  u32(grids.length)
  for (const grid of grids) {
    if (!grid) { u32(0); continue }
    u32(1)
    i32(grid.minX, grid.minY, grid.minZ, grid.nx, grid.ny, grid.nz)
    array(grid.bricks)
    array(usedCells(grid))
  }
  const faceIndex = new Map(FACES.map((f, i) => [f.name, i]))
  u32(requests.length)
  for (const r of requests) {
    u32(faceIndex.get(r.face) ?? 0)
    i32(r.level, r.layer)
    f64(r.cutoff)
    i32(r.i, r.j, ...r.eye, r.fill ? 1 : 0)
    f64(sliceCentre(r))
  }
  writeFileSync(process.env.DUMP ?? '', Buffer.concat(parts))
}, 600_000)
