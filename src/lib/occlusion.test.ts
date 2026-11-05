import { describe, it, expect } from 'vitest'
import { ParallaxRenderer } from './renderer'
import { picture } from './picture.test-helper'
import { hills } from './hills.test-helper'

const world = hills(12345)
const [WIDTH, HEIGHT] = [800, 600]

describe('skipping covered chunks', () => {
  // The thickest slices too: in relief, their splats move most with the camera.
  it.each([[0, 0, 6], [20, -25, 6], [-30, 10, 6], [20, -25, 12], [-30, 10, 12]])('leaves the picture unchanged (yaw %s°, pitch %s°, max leak %s°)', (yaw, pitch, maxLeak) => {
    const frames = [false, true].map(skip => {
      const renderer = new ParallaxRenderer(world)
      renderer.resize(WIDTH, HEIGHT)
      renderer.setOrientation({ yaw, pitch })
      renderer.setMaxLeak(maxLeak)
      renderer.setSkipCovered(skip)
      renderer.render()
      return { frame: renderer.render(), covered: renderer.getStats().lastUpdate?.chunksCovered ?? 0 }
    })
    expect(frames[1].covered).toBeGreaterThan(0)
    expect(picture(frames[1].frame, 3)).toEqual(picture(frames[0].frame, 3))
  })
})
