import { describe, it, expect } from 'vitest'
import { ParallaxRenderer } from './renderer'
import { picture } from './picture.test-helper'
import { hills } from './hills.test-helper'

const world = hills(12345)
const [WIDTH, HEIGHT] = [800, 600]

describe('skipping covered chunks and clusters', () => {
  // The thickest slices too: in relief, their splats move most with the camera.
  it.each([[0, 0, 2 ** 3.25], [20, -25, 2 ** 3.25], [-30, 10, 2 ** 3.25], [20, -25, 2 ** 2.25], [-30, 10, 2 ** 2.25]])('leaves the picture unchanged (yaw %s°, pitch %s°, detail %s)', (yaw, pitch, detail) => {
    const frames = [false, true].map(skip => {
      const renderer = new ParallaxRenderer(world)
      renderer.resize(WIDTH, HEIGHT)
      renderer.setOrientation({ yaw, pitch })
      renderer.setDetail(detail)
      renderer.setSkipCovered(skip)
      renderer.render()
      const frame = renderer.render()
      // Clusters hidden in chunks drawn (see Placement.show).
      const clusters = frame.layers.flatMap(layer => layer.placements).reduce((n, { show }) => n + (show?.filter(s => s === 0).length ?? 0), 0)
      return { frame, covered: renderer.getStats().lastUpdate?.chunksCovered ?? 0, clusters }
    })
    expect(frames[1].covered).toBeGreaterThan(0)
    expect(frames[1].clusters).toBeGreaterThan(0)
    expect(picture(frames[1].frame, 3)).toEqual(picture(frames[0].frame, 3))
  })
})
