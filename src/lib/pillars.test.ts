import { describe, it, expect } from 'vitest'
import { SPLAT } from './chunk-baker'
import { ParallaxRenderer, type Frame } from './renderer'
import { picture } from './picture.test-helper'
import type { Voxel } from './voxel-grid'
import { World } from './world'

// One surface: a lawn of three greens, a layer of sites on soil.
const GREENS = ['#5f8f3a', '#679a40', '#588536']
const voxels: Voxel[] = []
for (let x = -200; x <= 200; x++) {
  for (let z = -200; z <= 200; z++) {
    for (let y = -((x + z) & 1); y >= -3; y -= 2) {
      voxels.push({ x, y, z, color: y < 0 ? '#4a3423' : GREENS[(((x * 73856093) ^ (z * 19349663)) >>> 0) % GREENS.length] })
    }
  }
}
const lawn = World.ofVoxels(voxels, { x: 0, y: 30, z: 0 })

/** The frame at `yaw`, looking down, with one voxel per column or not. */
const view = (yaw: number, firstInColumn: boolean) => {
  const renderer = new ParallaxRenderer(lawn)
  renderer.resize(320, 240)
  renderer.setOrientation({ yaw, pitch: -22 })
  renderer.setRenderDistance(300)
  renderer.setMaxLeak(12)
  renderer.setFirstInColumn(firstInColumn)
  renderer.render()
  return renderer.render()
}

/** The frame with its chunks' splats drawn single, without their runs. */
const withoutRuns = (frame: Frame): Frame => ({
  ...frame,
  layers: frame.layers.map(layer => ({
    ...layer,
    placements: layer.placements.map(placement => {
      const splats = placement.chunk.splats.slice()
      for (let k = 3; k < splats.length; k += SPLAT) splats[k] = 0
      return { ...placement, chunk: { ...placement.chunk, splats } }
    }),
  })),
})

describe('one voxel per column', () => {
  // Looking along a face whose axis lies in the lawn, a column of it runs
  // through the whole slice. Its nearest voxel alone, a slanted ray passes
  // over it into the next slice, under the lawn there: stripes of soil
  // (docs/architecture.md, section 3). Each a pillar over its run, the
  // column is whole again.
  it.each([45, 0])('shows a lawn as all voxels do (yaw %s°)', yaw => {
    const shown = new Set(picture(view(yaw, false), 1))
    const stray = (colours: Uint32Array) => colours.filter(colour => !shown.has(colour)).length
    const first = view(yaw, true)
    expect(stray(picture(withoutRuns(first), 1))).toBeGreaterThan(500)
    expect(stray(picture(first, 1))).toBe(0)
  }, 60_000)
})
