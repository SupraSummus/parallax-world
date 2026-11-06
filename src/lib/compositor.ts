import { SPLAT } from './chunk-baker'
import type { Chunk, Frame } from './renderer'
import { halfLayer, thickness } from './slice-selection'
import { faceBasis, LAYER, viewBasis, type Basis, type Face } from './view'

// Sky gradient: from the horizon colour at eye level up to SKY_TOP at
// SKY_TOP_TANGENT (tangent of the elevation; 32°), flat above and below.
// Fog has the horizon colour.
export const SKY_TOP = '#3f73b8'
export const SKY_HORIZON = '#b9d3e8'
const SKY_TOP_TANGENT = 0.625

// Seams between faces, when shown: lines this many CSS pixels wide.
const SEAM_COLOR = '#ff2fd0'
const SEAM_WIDTH = 1.5

// Each splat is a voxel's hexagon across the face's axis at its site,
// `halfLayer` past the layer's plane per step of its alpha past 128, and
// as much wider as the ray to it slants across (Splat in
// bake/src/splats.rs), swept along the axis over its run (`run` half
// layers): a prism, an instance of PRISM, its vertices 0–5 the near
// hexagon's and 6–11 the far one's.
// Projected like projector() in view.ts, `view` having the camera's axes
// as columns, in the layer's face coordinates (the world's for the sky).
// `origin` is the chunk's top-left corner, camera-relative. The sky and
// the seams cover the screen.
// Depth: layer i's is `depth`, the sky's behind them all; within ±`slot`
// of it, by how far along the axis the vertex is (±`halfDepth`, the
// slice's half thickness), so the prisms of a layer hide each other about
// as their voxels would, before the fragment shader runs. Interpolated
// linearly on screen, not in perspective: surfaces of one layer much
// nearer each other than a prism is long may show in the wrong order.
const VERTEX_SHADER = `#version 300 es
uniform vec2 viewport;
uniform float focal;
uniform mat3 view;
uniform vec2 origin;
uniform float distance;
uniform float halfLayer;
uniform float depth;
uniform float slot;
uniform float halfDepth;
uniform bool sky;
uniform bool seams;
in vec4 splat; // centre x, y relative to origin, width, run
in vec4 paint;
flat out vec3 colour;
// The hexagon's corners from the top, clockwise as the camera sees them:
// (x · width, y · (height − |x| · width / √2)).
const vec2 CORNERS[6] = vec2[](vec2(0, 1), vec2(1, 1), vec2(1, -1), vec2(0, -1), vec2(-1, -1), vec2(-1, 1));
void main() {
  if (sky || seams) {
    gl_Position = vec4(vec2(gl_VertexID & 1, gl_VertexID >> 1) * 2.0 - 1.0, depth, 1.0);
    return;
  }
  float offset = (paint.a * 255.0 - 128.0) * halfLayer;
  float back = offset + splat.w * halfLayer;
  vec2 site = origin + splat.xy;
  float width = splat.z * (1.0 + abs(site.x / (distance + offset)));
  float height = splat.z * sqrt(2.0);
  vec2 corner = CORNERS[gl_VertexID % 6];
  vec2 u = corner * vec2(width, height - abs(corner.x) * width / sqrt(2.0));
  float z = distance + (gl_VertexID < 6 ? offset : back);
  colour = paint.rgb;
  vec3 p = transpose(view) * vec3(site + u, z);
  // Homogeneous, so the GPU clips what is behind the camera and interpolates in perspective.
  gl_Position = vec4(p.xy * focal / (viewport / 2.0), (depth + slot * clamp((z - distance) / halfDepth, -1.0, 1.0)) * p.z, p.z);
}`

/**
 * A prism's triangles (see VERTEX_SHADER), each clockwise seen from
 * outside: the near hexagon (the first HEXAGON indices, all a flat splat
 * needs), the far one, the six sides.
 */
const PRISM = new Uint8Array([
  ...[1, 2, 3, 4].flatMap(k => [0, k, k + 1]),
  ...[1, 2, 3, 4].flatMap(k => [6, k + 7, k + 6]),
  ...[0, 1, 2, 3, 4, 5].flatMap(k => [k, k + 6, ((k + 1) % 6) + 6, k, ((k + 1) % 6) + 6, (k + 1) % 6]),
])
const HEXAGON = 12

// `ray` is the pixel's direction in face coordinates, the same for every
// layer of a face. A face draws only its own rays (inFace in view.ts), so
// the faces tile the screen; they overlap by a hair, so rounding never
// leaves a seam pixel to neither. A chunk draws only within `clip`, its
// part of the layer's plane (camera-relative left, top, right, bottom),
// decided by where the pixel's ray crosses it, so neighbouring chunks
// share no pixel and leave none out. Splats are opaque: nothing is
// blended, and the depth test hides what is behind. The sky is
// one more rectangle, behind every layer.
// Seams, if shown, are drawn last, over everything, with `view` in world
// coordinates. A ray's face is the neighbour direction (±1, ±1, 0) along its
// two largest components, so faces meet where the second and third largest
// are equal in size.
const FRAGMENT_SHADER = `#version 300 es
precision highp float;
uniform vec2 viewport;
uniform vec2 resolution; // the canvas in device pixels
uniform float focal;
uniform mat3 view;
uniform float distance;
uniform vec4 clip;
uniform vec3 skyTop;
uniform vec3 horizon;
uniform float fog;
uniform bool sky;
uniform bool seams;
uniform vec3 seamColor;
uniform float seamWidth; // device pixels
flat in vec3 colour;
out vec4 color;
void main() {
  vec3 ray = view * vec3((gl_FragCoord.xy / resolution - 0.5) * viewport / focal, 1.0);
  if (seams) {
    vec3 a = abs(ray);
    float second = max(min(a.x, a.y), min(max(a.x, a.y), a.z));
    float third = min(min(a.x, a.y), a.z);
    // Device pixels to the seam, near it.
    float pixels = (second - third) / max(fwidth(second - third), 1e-9);
    color = vec4(seamColor, 1.0 - smoothstep(seamWidth / 2.0 - 0.5, seamWidth / 2.0 + 0.5, pixels));
    return;
  }
  vec3 rgb;
  if (sky) {
    float elevation = ray.y / length(ray.xz);
    rgb = mix(horizon, skyTop, clamp(elevation / ${String(SKY_TOP_TANGENT)}, 0.0, 1.0));
  } else {
    if (abs(ray.x) + sqrt(2.0) * abs(ray.y) > ray.z * 1.001) discard;
    vec2 p = ray.xy / ray.z * distance;
    if (p.x < clip.x || p.x >= clip.z || p.y > clip.y || p.y <= clip.w) discard;
    rgb = colour;
  }
  color = vec4(mix(rgb, horizon, fog), 1.0);
}`

const rgb = (hex: string) => [1, 3, 5].map(i => parseInt(hex.slice(i, i + 2), 16) / 255) as [number, number, number]

const UNIFORMS = ['viewport', 'resolution', 'focal', 'view', 'origin', 'clip', 'distance', 'halfLayer', 'depth', 'slot', 'halfDepth', 'skyTop', 'horizon', 'fog', 'sky', 'seams', 'seamColor', 'seamWidth'] as const
type Uniform = (typeof UNIFORMS)[number]

/** Attribute locations of a splat's numbers (see SPLAT). */
const SPLAT_AT = 0
const PAINT_AT = 1

/**
 * Draws Frames with WebGL 2 onto one canvas, near to far with a depth test,
 * so what is nearer rejects what is behind rather than covering it.
 * A chunk's splats are uploaded as a buffer, near to far,
 * when it comes into view and dropped when it leaves.
 */
export class Compositor {
  private canvas: HTMLCanvasElement
  private gl: WebGL2RenderingContext
  private uniforms = {} as Record<Uniform, WebGLUniformLocation | null>
  private buffers = new Map<Chunk, WebGLBuffer>()
  private frame: Frame | null = null
  private showSeams = false
  // Whether splats are swept over their runs.
  private runs = true

  /** Throws if the browser has no WebGL 2. */
  constructor(canvas: HTMLCanvasElement) {
    const gl = canvas.getContext('webgl2', { alpha: false, antialias: false })
    if (!gl) throw new Error('WebGL 2 is not available')
    this.canvas = canvas
    this.gl = gl
    this.setup()
    // The GPU may drop the context (driver reset, backgrounded tab); start
    // over when it comes back.
    canvas.addEventListener('webglcontextlost', e => {
      e.preventDefault()
      this.buffers.clear()
    })
    canvas.addEventListener('webglcontextrestored', () => {
      this.setup()
      if (this.frame) this.draw(this.frame)
    })
  }

  /** Whether to draw the seams between faces over the picture; redraws the last frame. */
  setShowSeams(show: boolean) {
    this.showSeams = show
    if (this.frame) this.draw(this.frame)
  }

  /** Whether to draw splats with runs as pillars over them, or as their nearest voxel alone; redraws the last frame. */
  setRuns(on: boolean) {
    this.runs = on
    if (this.frame) this.draw(this.frame)
  }

  draw(frame: Frame) {
    this.frame = frame
    const { gl, canvas, uniforms: u } = this
    if (gl.isContextLost()) return

    // Full device resolution keeps voxel edges sharp; capped to bound the fill cost.
    const dpr = Math.min(2, window.devicePixelRatio)
    const width = Math.round(frame.width * dpr)
    const height = Math.round(frame.height * dpr)
    if (canvas.width !== width || canvas.height !== height) {
      canvas.width = width
      canvas.height = height
    }
    gl.viewport(0, 0, width, height)
    // The sky covers every pixel, so only depth needs clearing.
    gl.clear(gl.DEPTH_BUFFER_BIT)
    gl.uniform2f(u.viewport, frame.width, frame.height)
    gl.uniform2f(u.resolution, width, height)
    gl.uniform1f(u.focal, frame.focal)
    // Columns: the camera's right, up and forward axes.
    const setView = ({ right, up, forward }: Basis) => {
      gl.uniformMatrix3fv(u.view, false, [right, up, forward].flatMap(v => [v.x, v.y, v.z]))
    }

    // Layer i at depth z(i), strictly inside (-1, 1) so rounding never clips
    // it, its splats within `slot` of that; the sky behind all of them,
    // still in front of the cleared depth. A chunk's splats come near to
    // far, so the GPU rejects most of those its nearer ones hide; where two
    // are as near, the first drawn (the nearer in painting order) shows (LESS).
    const z = (i: number) => (2 * i + 1) / (frame.layers.length + 1) - 1
    gl.uniform1f(u.slot, 0.9 / (frame.layers.length + 1))
    gl.uniform1i(u.sky, 0)
    const drawn = new Set<Chunk>()
    gl.enable(gl.CULL_FACE)
    gl.enableVertexAttribArray(SPLAT_AT)
    gl.enableVertexAttribArray(PAINT_AT)
    let face: Face | null = null
    frame.layers.forEach((layer, i) => {
      if (layer.face !== face) {
        face = layer.face
        setView(faceBasis(face, frame.orientation))
      }
      const { distance, placements, fog, slice } = layer
      gl.uniform1f(u.distance, distance)
      gl.uniform1f(u.halfLayer, halfLayer(slice))
      gl.uniform1f(u.halfDepth, (thickness(slice) * LAYER) / 2)
      gl.uniform1f(u.fog, fog)
      gl.uniform1f(u.depth, z(i))
      for (const { chunk, x, y } of placements) {
        gl.bindBuffer(gl.ARRAY_BUFFER, this.buffer(chunk))
        drawn.add(chunk)
        gl.uniform2f(u.origin, x, y)
        gl.uniform4f(u.clip, x, y, x + chunk.extentWidth, y - chunk.extentHeight)
        gl.vertexAttribPointer(SPLAT_AT, 4, gl.FLOAT, false, SPLAT * 4, 0)
        gl.vertexAttribPointer(PAINT_AT, 4, gl.UNSIGNED_BYTE, true, SPLAT * 4, 16)
        gl.drawElementsInstanced(gl.TRIANGLES, this.runs ? PRISM.length : HEXAGON, gl.UNSIGNED_BYTE, 0, chunk.splats.length / SPLAT)
      }
    })
    gl.disableVertexAttribArray(SPLAT_AT)
    gl.disableVertexAttribArray(PAINT_AT)
    gl.disable(gl.CULL_FACE)
    gl.uniform1i(u.sky, 1)
    setView(viewBasis(frame.orientation))
    gl.uniform1f(u.depth, z(frame.layers.length))
    gl.uniform1f(u.fog, frame.fog)
    gl.drawArrays(gl.TRIANGLE_STRIP, 0, 4)

    if (this.showSeams) {
      gl.uniform1i(u.seams, 1)
      gl.uniform1f(u.seamWidth, SEAM_WIDTH * dpr)
      gl.disable(gl.DEPTH_TEST)
      gl.enable(gl.BLEND)
      gl.drawArrays(gl.TRIANGLE_STRIP, 0, 4)
      gl.disable(gl.BLEND)
      gl.enable(gl.DEPTH_TEST)
      gl.uniform1i(u.seams, 0)
    }

    for (const [chunk, buffer] of this.buffers) {
      if (drawn.has(chunk)) continue
      gl.deleteBuffer(buffer)
      this.buffers.delete(chunk)
    }
  }

  private setup() {
    const { gl } = this
    const program = gl.createProgram()
    for (const [type, source] of [[gl.VERTEX_SHADER, VERTEX_SHADER], [gl.FRAGMENT_SHADER, FRAGMENT_SHADER]] as const) {
      const shader = gl.createShader(type)
      if (!shader) throw new Error('WebGL could not create a shader')
      gl.shaderSource(shader, source)
      gl.compileShader(shader)
      gl.attachShader(program, shader)
    }
    gl.bindAttribLocation(program, SPLAT_AT, 'splat')
    gl.bindAttribLocation(program, PAINT_AT, 'paint')
    gl.linkProgram(program)
    if (!gl.getProgramParameter(program, gl.LINK_STATUS)) throw new Error(gl.getProgramInfoLog(program) ?? 'WebGL link failed')
    gl.useProgram(program)
    for (const name of UNIFORMS) {
      this.uniforms[name] = gl.getUniformLocation(program, name)
    }
    gl.uniform3f(this.uniforms.skyTop, ...rgb(SKY_TOP))
    gl.uniform3f(this.uniforms.horizon, ...rgb(SKY_HORIZON))
    gl.uniform3f(this.uniforms.seamColor, ...rgb(SEAM_COLOR))
    // The prism's triangles, for every splat; the default vertex array keeps them bound.
    gl.bindBuffer(gl.ELEMENT_ARRAY_BUFFER, gl.createBuffer())
    gl.bufferData(gl.ELEMENT_ARRAY_BUFFER, PRISM, gl.STATIC_DRAW)
    gl.frontFace(gl.CW)
    gl.blendFunc(gl.SRC_ALPHA, gl.ONE_MINUS_SRC_ALPHA)
    gl.enable(gl.DEPTH_TEST)
    gl.depthFunc(gl.LESS)
    for (const at of [SPLAT_AT, PAINT_AT]) gl.vertexAttribDivisor(at, 1)
  }

  private buffer(chunk: Chunk): WebGLBuffer {
    let buffer = this.buffers.get(chunk)
    if (buffer) return buffer
    const { gl } = this
    buffer = gl.createBuffer()
    gl.bindBuffer(gl.ARRAY_BUFFER, buffer)
    // Near to far: the baker's painting order reversed.
    const { splats } = chunk
    const reversed = new Float32Array(splats.length)
    for (let k = 0; k < splats.length; k += SPLAT) reversed.set(splats.subarray(k, k + SPLAT), splats.length - SPLAT - k)
    gl.bufferData(gl.ARRAY_BUFFER, reversed, gl.STATIC_DRAW)
    this.buffers.set(chunk, buffer)
    return buffer
  }
}
