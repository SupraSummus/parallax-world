/**
 * Tests' small scene with trees, a world of voxels: rendering the forest
 * takes seconds per test, this moments.
 */
import type { Voxel } from './voxel-grid'
import { World } from './world'

/** Hills 80 blocks wide, grass on dirt on stone, trees here and there; the camera starts south of them, above. */
export function hills(seed: number): World {
  const noise = (x: number, y: number, z: number) => {
    const n = Math.sin(x * 12.9898 + y * 78.233 + z * 45.164 + seed * 0.001) * 43758.5453
    return n - Math.floor(n)
  }
  // Block (x, y, z) is the four sites of the box [2x − 1, 2x] × [2y − 1, 2y] × [2z − 1, 2z]; later blocks win.
  const sites = new Map<string, Voxel>()
  const block = (x: number, y: number, z: number, color: string) => {
    for (const [dx, dy, dz] of [[0, 0, 0], [1, 1, 0], [1, 0, 1], [0, 1, 1]]) {
      const site = { x: 2 * x - dx, y: 2 * y - dy, z: 2 * z - dz, color }
      sites.set(`${String(site.x)},${String(site.y)},${String(site.z)}`, site)
    }
  }
  for (let x = -40; x < 40; x++) {
    for (let z = -40; z < 40; z++) {
      const [nx, nz] = [(x / 40) * 4, (z / 40) * 4]
      const ground = Math.floor(noise(nx, 0, nz) * 8 * 0.8 + noise(nx * 2, 0, nz * 2) * 4 * 0.8 + noise(nx * 4, 0, nz * 4) * 2 * 0.8)
      for (let y = -5; y <= ground; y++) block(x, y, z, y === ground ? '#4CAF50' : y > ground - 3 ? '#8B4513' : '#666666')
      if (noise(x * 0.3, 0, z * 0.3) > 0.75 && ground > 2) {
        const tall = 4 + Math.floor(noise(x, 0, z) * 3)
        for (let y = ground + 1; y < ground + tall; y++) block(x, y, z, '#654321')
        for (let dx = -2; dx <= 2; dx++) {
          for (let dz = -2; dz <= 2; dz++) {
            for (let dy = 0; dy < Math.min(3, 4 - Math.abs(dx) - Math.abs(dz)); dy++) block(x + dx, ground + tall - 1 + dy, z + dz, '#228B22')
          }
        }
      }
    }
  }
  return World.ofVoxels([...sites.values()], { x: 0, y: 40, z: -140 })
}
