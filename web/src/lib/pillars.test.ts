import { describe, it, expect } from 'vitest'
import { SPLAT } from './chunk-baker'
import { ParallaxRenderer, type Frame } from './renderer'
import { picture } from './picture.test-helper'
import { ofVoxels, type Voxel } from './scene.test-helper'

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
const lawn = ofVoxels(voxels, { x: 0, y: 30, z: 0 })

/** The frame at `yaw`, looking down. */
const view = (yaw: number) => {
  const renderer = new ParallaxRenderer(lawn)
  renderer.resize(320, 240)
  renderer.setOrientation({ yaw, pitch: -22 })
  renderer.setRenderDistance(300)
  renderer.setDetail(2 ** 2.25)
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

/**
 * How many pixels of the picture's lower half show soil: redder than
 * green, as no green (nor the sky, 0) is. Its upper half has the lawn's
 * far edge.
 */
const soil = (colours: Uint32Array) => colours.filter((c, k) => k >= colours.length / 2 && c !== 0 && (c & 255) > ((c >>> 8) & 255)).length

describe('pillars', () => {
  // Looking along a face whose axis lies in the lawn, a column of it runs
  // through the whole slice. Its nearest voxel alone, a slanted ray passes
  // over it into the next slice, under the lawn there: stripes of soil
  // (docs/architecture.md, section 3). Each a pillar over its run, the
  // column is whole again.
  it.each([45, 0])('show a lawn without stripes of the soil under it (yaw %s°)', yaw => {
    const frame = view(yaw)
    expect(soil(picture(withoutRuns(frame), 1))).toBeGreaterThan(1000)
    // A few strays, as drawing every voxel of each slice showed too.
    expect(soil(picture(frame, 1))).toBeLessThan(100)
  }, 60_000)
})
