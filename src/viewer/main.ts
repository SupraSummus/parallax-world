/**
 * Reference viewer: renders a World as an ordinary triangle mesh with
 * three.js, to check what the scene looks like independently of the slice
 * renderer. Opened from the app with the view in its query (parseViewQuery).
 */
import * as THREE from 'three'
import { OrbitControls } from 'three/examples/jsm/controls/OrbitControls.js'
import { World, type Vec3 } from '@/lib/world'
import { DEFAULT_FOV, DEFAULT_RENDER_DISTANCE, FOG_PER_RENDER_DISTANCE, parseViewQuery } from '@/lib/renderer'
import { SKY_HORIZON, SKY_TOP } from '@/lib/compositor'
import { focalLength, viewBasis } from '@/lib/view'
import { GRID_LEVELS } from '@/lib/slice-selection'
import { buildVoxelMesh } from './mesh'

const query = parseViewQuery(location.search)
const seed = query.seed ?? 1
const fov = query.fov ?? DEFAULT_FOV
const renderDistance = query.distance ?? DEFAULT_RENDER_DISTANCE

const started = performance.now()
const world = new World(seed, query.worldType)
const parallaxCamera: Vec3 = query.camera ?? world.spawn
// Level 0 within REACH units in x and z (all of the forest's would take gigabytes of buffers),
// then coarser levels in rings (buildVoxelMesh) to the render distance,
// each its own mesh: all in one would be over WebGL's buffer size limit.
const REACH = 240
const levels = Math.min(GRID_LEVELS, 1 + Math.max(0, Math.ceil(Math.log2(renderDistance / REACH))))
const scene = new THREE.Scene()
const material = new THREE.MeshBasicMaterial({ vertexColors: true })
let faceCount = 0
for (let level = 0; level < levels; level++) {
  const mesh = buildVoxelMesh(world, parallaxCamera, REACH, level)
  const geometry = new THREE.BufferGeometry()
  geometry.setAttribute('position', new THREE.BufferAttribute(mesh.positions, 3))
  geometry.setAttribute('color', new THREE.BufferAttribute(mesh.colors, 3, true))
  geometry.setIndex(new THREE.BufferAttribute(mesh.indices, 1))
  scene.add(new THREE.Mesh(geometry, material))
  faceCount += mesh.faceCount
}
const buildTime = performance.now() - started
const { yaw, pitch } = query.look ?? { yaw: 0, pitch: 0 }

// Linear stand-in for the slice renderer's exponential fog.
const fog = new THREE.Fog(SKY_HORIZON, 0, renderDistance * FOG_PER_RENDER_DISTANCE * 2.5)
scene.fog = fog

const renderer = new THREE.WebGLRenderer({ antialias: true, alpha: true })
renderer.setPixelRatio(window.devicePixelRatio)
renderer.outputColorSpace = THREE.LinearSRGBColorSpace // colours are already sRGB, like the 2D canvas
document.body.append(renderer.domElement)
document.body.style.background = `linear-gradient(${SKY_TOP}, ${SKY_HORIZON} 50%)`

const camera = new THREE.PerspectiveCamera(60, 1, 0.1, REACH * 2 ** levels)
const controls = new OrbitControls(camera, renderer.domElement)
// Draw only when something changes: the mesh can have millions of faces.
const draw = () => { renderer.render(scene, camera) }
controls.addEventListener('change', draw)

/** World +z maps to three.js -z (see mesh.ts). */
function toParallaxView() {
  camera.position.set(parallaxCamera.x, parallaxCamera.y, -parallaxCamera.z)
  // OrbitControls looks at its target; put it ahead, where the parallax camera looks.
  const { forward } = viewBasis({ yaw, pitch })
  controls.target.set(parallaxCamera.x + forward.x * 10, parallaxCamera.y + forward.y * 10, -parallaxCamera.z - forward.z * 10)
  controls.update()
  draw()
}

function resize() {
  const { innerWidth: w, innerHeight: h } = window
  renderer.setSize(w, h)
  camera.aspect = w / h
  // Same perspective as the slice renderer (three.js takes the vertical field of view).
  camera.fov = THREE.MathUtils.radToDeg(2 * Math.atan(h / 2 / focalLength(fov, w, h)))
  camera.updateProjectionMatrix()
  draw()
}

const info = document.getElementById('info')
if (info) {
  info.textContent =
    `${world.worldType} · seed ${String(seed)} · ${String(levels)} levels to ±${String(REACH * 2 ** (levels - 1))} units around the camera · ` +
    `${faceCount.toLocaleString()} faces · built in ${buildTime.toFixed(0)} ms`
}
document.getElementById('reset')?.addEventListener('click', toParallaxView)
document.getElementById('fog')?.addEventListener('change', e => {
  scene.fog = (e.target as HTMLInputElement).checked ? fog : null
  draw()
})
window.addEventListener('resize', resize)

resize()
toParallaxView()
