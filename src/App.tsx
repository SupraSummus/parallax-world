import { useEffect, useEffectEvent, useRef, useState, useCallback, type PointerEvent, type ReactNode } from 'react'
import { ParallaxRenderer, DEFAULT_FOV, MAX_FOV, MIN_FOV, DEFAULT_RENDER_DISTANCE, MAX_RENDER_DISTANCE, MIN_RENDER_DISTANCE, DEFAULT_MAX_LEAK, MAX_MAX_LEAK, MIN_MAX_LEAK, type Camera, type SessionStats } from '@/lib/renderer'
import { World, WORLD_TYPES, type WorldType } from '@/lib/world'
import type { Orientation } from '@/lib/view'
import { Compositor } from '@/lib/compositor'
import { startBakeWorkers } from '@/lib/worker-bakery'
import { Card } from '@/components/ui/card'
import { Button } from '@/components/ui/button'
import { Separator } from '@/components/ui/separator'
import { Slider } from '@/components/ui/slider'
import { Switch } from '@/components/ui/switch'
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select'
import { Box, ChartLine, Crosshair, ExternalLink, Eye, EyeOff, History, Move, RefreshCw } from 'lucide-react'

/**
 * Key -> movement direction for held keys: right, up, forward relative to
 * where the camera looks (level: looking up does not lift W). Keys are
 * lower-cased.
 */
const MOVEMENT_KEYS: Record<string, [number, number, number]> = {
  w: [0, 0, 1],
  s: [0, 0, -1],
  a: [-1, 0, 0],
  d: [1, 0, 0],
  ' ': [0, 1, 0],
  shift: [0, -1, 0],
}

/** Key -> turn direction (yaw, pitch) for held keys. */
const LOOK_KEYS: Record<string, [number, number]> = {
  arrowleft: [-1, 0],
  arrowright: [1, 0],
  arrowup: [0, 1],
  arrowdown: [0, -1],
}

const TURN_DEGREES_PER_SECOND = 60

/** Degrees of turn per CSS pixel of mouse drag. */
const DRAG_DEGREES_PER_PIXEL = 0.15

function App() {
  const canvasRef = useRef<HTMLCanvasElement>(null)
  const rendererRef = useRef<ParallaxRenderer | null>(null)
  const compositorRef = useRef<Compositor | null>(null)
  // A render scheduled to show chunks baked since the last one.
  const bakedFrameRef = useRef<number | undefined>(undefined)
  // See the effect setting it.
  const renderBehindMessagesRef = useRef<() => void>(() => undefined)
  // Without it (e.g. jsdom) the panels still work, but the scene isn't drawn.
  const [hasWebGL2] = useState(() => document.createElement('canvas').getContext('webgl2') !== null)

  const [sessionStats, setSessionStats] = useState<SessionStats | null>(null)
  // Replaced by the world's spawn point on the first render.
  const [camera, setCamera] = useState<Camera>({ x: 0, y: 0, z: 0 })
  const [orientation, setOrientation] = useState<Orientation>({ yaw: 0, pitch: 0 })
  const [showStats, setShowStats] = useState(true)
  // Lattice units per second.
  const [moveSpeed, setMoveSpeed] = useState(60)
  const [worldType, setWorldType] = useState<WorldType>('forest')
  const [fov, setFov] = useState(DEFAULT_FOV)
  const [renderDistance, setRenderDistance] = useState(DEFAULT_RENDER_DISTANCE)
  const [maxLeak, setMaxLeak] = useState(DEFAULT_MAX_LEAK)
  const [fillGaps, setFillGaps] = useState(true)
  const [skipCovered, setSkipCovered] = useState(true)
  const [showSeams, setShowSeams] = useState(false)
  // Generating a world blocks this thread for a second or two; a spinner shows meanwhile.
  const [generating, setGenerating] = useState(true)

  // Read from the movement loop without restarting it.
  const getMoveSpeed = useEffectEvent(() => moveSpeed)

  const requestRender = useCallback(() => {
    const renderer = rendererRef.current
    if (!renderer) return
    // This render shows the baked chunks too.
    if (bakedFrameRef.current !== undefined) cancelAnimationFrame(bakedFrameRef.current)
    bakedFrameRef.current = undefined
    const frame = renderer.render()
    compositorRef.current?.draw(frame)
    setSessionStats(renderer.getStats())
    setCamera(renderer.getCamera())
    setOrientation(renderer.getOrientation())
  }, [])

  // Renders asked for in an animation frame (moving, chunks baked) run as a
  // task queued behind the workers' messages received by then: in the frame,
  // while a key was held, the browser ran only input and frames.
  useEffect(() => {
    const channel = new MessageChannel()
    let queued = false
    channel.port1.onmessage = () => {
      queued = false
      requestRender()
    }
    renderBehindMessagesRef.current = () => {
      if (queued) return
      queued = true
      channel.port2.postMessage(null)
    }
    return () => { channel.port1.close() }
  }, [requestRender])

  // Renderer lifecycle.
  useEffect(() => {
    const canvas = canvasRef.current
    if (!canvas) return

    if (hasWebGL2) compositorRef.current = new Compositor(canvas)
    // Chunks are baked in workers, leaving a core for this thread; without
    // workers (e.g. jsdom) render() bakes them itself. Each holds a copy of
    // the world (35–50 MB for the scenes) and its cell surfaces, so at most 8.
    const bakery = typeof Worker === 'undefined'
      ? undefined
      : startBakeWorkers(Math.max(1, Math.min(8, navigator.hardwareConcurrency - 1)))
    // Baked chunks arrive one by one; show them at most once per frame.
    const onBaked = () => {
      bakedFrameRef.current ??= requestAnimationFrame(() => { renderBehindMessagesRef.current() })
    }
    const renderer = new ParallaxRenderer(new World(Date.now(), 'forest'), { bakery, onBaked })
    rendererRef.current = renderer

    const handleResize = () => {
      renderer.resize(window.innerWidth, window.innerHeight)
      requestRender()
    }
    handleResize()
    setGenerating(false)

    window.addEventListener('resize', handleResize)
    return () => {
      window.removeEventListener('resize', handleResize)
      bakery?.dispose()
      if (bakedFrameRef.current !== undefined) cancelAnimationFrame(bakedFrameRef.current)
      rendererRef.current = null
      compositorRef.current = null
    }
  }, [requestRender, hasWebGL2])

  // Keyboard movement and turning: one animation-frame loop runs while any such key is held.
  useEffect(() => {
    const held = new Set<string>()
    let frame: number | undefined
    let lastTime: number | undefined

    const tick = (time: number) => {
      if (held.size === 0) {
        frame = undefined
        return
      }
      // Seconds since the previous frame, so a slow frame moves further.
      // Capped, so that a stall doesn't jump the camera.
      const dt = lastTime === undefined ? 0 : Math.min(0.1, (time - lastTime) / 1000)
      lastTime = time
      const renderer = rendererRef.current
      if (renderer) {
        const speed = getMoveSpeed() * dt
        const cam = renderer.getCamera()
        const { yaw, pitch } = renderer.getOrientation()
        const [sin, cos] = [Math.sin((yaw * Math.PI) / 180), Math.cos((yaw * Math.PI) / 180)]
        let [turnYaw, turnPitch] = [0, 0]
        for (const key of held) {
          if (key in LOOK_KEYS) {
            turnYaw += LOOK_KEYS[key][0] * TURN_DEGREES_PER_SECOND * dt
            turnPitch += LOOK_KEYS[key][1] * TURN_DEGREES_PER_SECOND * dt
            continue
          }
          const [right, up, forward] = MOVEMENT_KEYS[key]
          cam.x += (right * cos + forward * sin) * speed
          cam.y += up * speed
          cam.z += (forward * cos - right * sin) * speed
        }
        renderer.setCamera(cam)
        renderer.setOrientation({ yaw: yaw + turnYaw, pitch: pitch + turnPitch })
        setCamera(renderer.getCamera())
        setOrientation(renderer.getOrientation())
        renderBehindMessagesRef.current()
      }
      frame = requestAnimationFrame(tick)
    }

    const stop = () => {
      held.clear()
      if (frame !== undefined) cancelAnimationFrame(frame)
      frame = undefined
      lastTime = undefined
    }

    const handleKeyDown = (e: KeyboardEvent) => {
      const key = e.key.toLowerCase()
      if (!(key in MOVEMENT_KEYS) && !(key in LOOK_KEYS)) return
      // Don't hijack typing/space activation inside form controls.
      if (e.target instanceof HTMLElement && e.target.closest('input, textarea, select, [role="combobox"]')) return
      e.preventDefault()
      held.add(key)
      frame ??= requestAnimationFrame(tick)
    }

    const handleKeyUp = (e: KeyboardEvent) => {
      held.delete(e.key.toLowerCase())
      if (held.size === 0) stop()
    }

    window.addEventListener('keydown', handleKeyDown)
    window.addEventListener('keyup', handleKeyUp)
    // Releasing keys while the window is unfocused never fires keyup.
    window.addEventListener('blur', stop)
    return () => {
      window.removeEventListener('keydown', handleKeyDown)
      window.removeEventListener('keyup', handleKeyUp)
      window.removeEventListener('blur', stop)
      stop()
    }
  }, [])

  // Dragging on the scene turns the camera: right turns right, up looks up.
  const handlePointerMove = (e: PointerEvent) => {
    const renderer = rendererRef.current
    if (!renderer || (e.buttons & 1) === 0) return
    const { yaw, pitch } = renderer.getOrientation()
    renderer.setOrientation({
      yaw: yaw + e.movementX * DRAG_DEGREES_PER_PIXEL,
      pitch: pitch - e.movementY * DRAG_DEGREES_PER_PIXEL,
    })
    requestRender()
  }

  const handleRegenerateWorld = (type: WorldType = worldType) => {
    setWorldType(type)
    setGenerating(true)
    // Once the spinner is painted: after the next frame.
    requestAnimationFrame(() => {
      setTimeout(() => {
        rendererRef.current?.regenerateWorld(Date.now(), type)
        requestRender()
        setGenerating(false)
      })
    })
  }

  // The viewer rebuilds the same world from its seed and type.
  const openViewer = () => {
    const renderer = rendererRef.current
    if (!renderer) return
    const { seed, worldType } = renderer.getWorld()
    const { x, y, z } = renderer.getCamera()
    const { yaw, pitch } = renderer.getOrientation()
    const query = new URLSearchParams({
      seed: String(seed),
      world: worldType,
      camera: [x, y, z].join(','),
      look: [yaw, pitch].join(','),
      fov: String(renderer.getFov()),
      distance: String(renderer.getRenderDistance()),
    })
    window.open(`${import.meta.env.BASE_URL}viewer.html?${query.toString()}`, '_blank', 'noopener')
  }

  const handleFovChange = ([value]: number[]) => {
    setFov(value)
    rendererRef.current?.setFov(value)
    requestRender()
  }

  const handleRenderDistanceChange = ([value]: number[]) => {
    setRenderDistance(value)
    rendererRef.current?.setRenderDistance(value)
    requestRender()
  }

  const handleMaxLeakChange = ([value]: number[]) => {
    setMaxLeak(value)
    rendererRef.current?.setMaxLeak(value)
    requestRender()
  }

  const handleSkipCoveredChange = (value: boolean) => {
    setSkipCovered(value)
    rendererRef.current?.setSkipCovered(value)
    requestRender()
  }

  const handleShowSeamsChange = (value: boolean) => {
    setShowSeams(value)
    compositorRef.current?.setShowSeams(value)
  }

  const handleFillGapsChange = (value: boolean) => {
    setFillGaps(value)
    rendererRef.current?.setFillGaps(value)
    requestRender()
  }

  const totalUpdates = sessionStats?.totalUpdates ?? 0
  const averageCacheEfficiency = sessionStats?.averageCacheEfficiency ?? 0
  const lastUpdate = sessionStats?.lastUpdate
  const avgRenderTime = totalUpdates > 0 && sessionStats
    ? (sessionStats.totalRenderTime / totalUpdates).toFixed(2)
    : '0.00'

  return (
    <div className="relative h-screen w-screen overflow-hidden bg-background text-foreground">
      <canvas
        ref={canvasRef}
        // Keep the drag when it passes over a panel.
        onPointerDown={e => { e.currentTarget.setPointerCapture(e.pointerId) }}
        onPointerMove={handlePointerMove}
        className="absolute inset-0 h-full w-full touch-none" />
      {!hasWebGL2 && (
        <div className="absolute inset-0 flex items-center justify-center p-4 text-center text-sm text-white">
          Can't draw the scene: this browser has no WebGL 2.
        </div>
      )}

      <div className="pointer-events-none absolute left-1/2 top-1/2 -translate-x-1/2 -translate-y-1/2">
        <Crosshair size={20} className="text-white/60" />
      </div>
      {generating && <Generating />}

      <div className="pointer-events-none absolute inset-0 flex justify-between gap-4 p-4">
        <aside className="flex w-72 min-h-0 flex-col gap-3 overflow-y-auto">
          <header className="px-1">
            <h1 className="font-mono text-2xl font-bold tracking-tight text-white drop-shadow">PARALLAX ENGINE</h1>
            <p className="text-xs text-white/80 drop-shadow">Event-driven 2.5D voxel rendering</p>
          </header>

          <Panel icon={<Crosshair size={16} />} title="Camera">
            <StatRow label="Position">
              {camera.x.toFixed(1)}, {camera.y.toFixed(1)}, {camera.z.toFixed(1)}
            </StatRow>
            <StatRow label="Yaw, Pitch">
              {orientation.yaw.toFixed(1)}°, {orientation.pitch.toFixed(1)}°
            </StatRow>
            <SliderRow
              label={`Move Speed: ${String(moveSpeed)} units/s`}
              value={moveSpeed}
              min={10}
              max={240}
              step={10}
              onChange={([v]) => { setMoveSpeed(v) }}
            />
            <SliderRow
              label={`Field of View: ${String(fov)}°`}
              value={fov}
              min={MIN_FOV}
              max={MAX_FOV}
              step={1}
              onChange={handleFovChange}
            />
            <SliderRow
              label={`Render Distance: ${String(renderDistance)}`}
              value={renderDistance}
              min={MIN_RENDER_DISTANCE}
              max={MAX_RENDER_DISTANCE}
              step={10}
              onChange={handleRenderDistanceChange}
            />
            <SliderRow
              label={`Max Occlusion Leak: ${maxLeak.toFixed(1)}°`}
              value={maxLeak}
              min={MIN_MAX_LEAK}
              max={MAX_MAX_LEAK}
              step={0.1}
              onChange={handleMaxLeakChange}
            />
            <label className="flex items-center justify-between text-xs text-muted-foreground">
              Fill Gaps
              <Switch checked={fillGaps} onCheckedChange={handleFillGapsChange} aria-label="Fill gaps" />
            </label>
            <label className="flex items-center justify-between text-xs text-muted-foreground">
              Skip Covered Chunks
              <Switch checked={skipCovered} onCheckedChange={handleSkipCoveredChange} aria-label="Skip covered chunks" />
            </label>
            <label className="flex items-center justify-between text-xs text-muted-foreground">
              Show Face Seams
              <Switch checked={showSeams} onCheckedChange={handleShowSeamsChange} aria-label="Show face seams" />
            </label>
            <div className="space-y-0.5 text-xs text-muted-foreground">
              <div className="flex items-center gap-2"><Move size={14} /><span>W/S - Forward/Back</span></div>
              <div className="pl-[22px]">A/D - Left/Right</div>
              <div className="pl-[22px]">Space/Shift - Up/Down</div>
              <div className="pl-[22px]">Arrows/Drag - Look around</div>
            </div>
          </Panel>

          <Panel icon={<Box size={16} />} title="World">
            <Select value={worldType} onValueChange={(v) => { handleRegenerateWorld(v as WorldType) }}>
              <SelectTrigger className="w-full" aria-label="World type">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {WORLD_TYPES.map(type => (
                  <SelectItem key={type.value} value={type.value}>{type.label}</SelectItem>
                ))}
              </SelectContent>
            </Select>
            <Button onClick={() => { handleRegenerateWorld() }} className="w-full">
              <RefreshCw size={16} />
              Regenerate World
            </Button>
            <Button onClick={openViewer} variant="outline" className="w-full">
              <ExternalLink size={16} />
              Open in 3D Viewer
            </Button>
            {!showStats && (
              <Button onClick={() => { setShowStats(true) }} variant="outline" className="w-full">
                <Eye size={16} />
                Stats
              </Button>
            )}
          </Panel>
        </aside>

        <aside className="flex w-72 min-h-0 flex-col gap-3">
          {showStats && (
            <Panel
              icon={<ChartLine size={16} />}
              title="Render Statistics"
              action={
                <Button
                  size="sm"
                  variant="ghost"
                  onClick={() => { setShowStats(false) }}
                  className="h-6 w-6 p-0"
                  aria-label="Hide statistics"
                >
                  <EyeOff size={14} />
                </Button>
              }
            >
              <div className="space-y-1">
                <h3 className="flex items-center gap-1.5 text-xs font-semibold">
                  <History size={12} className="text-muted-foreground" />
                  Session
                </h3>
                <StatRow label="Total Updates">{totalUpdates}</StatRow>
                <StatRow label="Avg Render Time">{avgRenderTime}ms</StatRow>
                <StatRow label="Avg Cache Efficiency">{averageCacheEfficiency.toFixed(1)}%</StatRow>
                <StatRow label="Total Voxels">{(sessionStats?.totalVoxelsRendered ?? 0).toLocaleString()}</StatRow>
              </div>
              {lastUpdate && (
                <>
                  <Separator />
                  <div className="space-y-1">
                    <h3 className="text-xs font-semibold">Last Update</h3>
                    <StatRow label="Render Time">{lastUpdate.renderTime.toFixed(2)}ms</StatRow>
                    <StatRow label="Active Layers">{lastUpdate.layerCount}</StatRow>
                    <StatRow label="Slice Detail">{lastUpdate.detail.toFixed(1)}</StatRow>
                    <StatRow label="Leak Bound">{lastUpdate.leakPixels.toFixed(1)} px</StatRow>
                    <StatRow label="Chunks in View">{lastUpdate.chunkCount}</StatRow>
                    <StatRow label="Reused / Regenerated">
                      <span className="text-primary">{lastUpdate.chunksReused}</span>
                      {' / '}
                      <span className="text-destructive">{lastUpdate.chunksRegenerated}</span>
                    </StatRow>
                    <StatRow label="Chunks Baking">{lastUpdate.chunksPending}</StatRow>
                    <StatRow label="Chunks Covered">{lastUpdate.chunksCovered}</StatRow>
                    <StatRow label="Cache Efficiency">{lastUpdate.cacheEfficiency.toFixed(1)}%</StatRow>
                    <StatRow label="Voxels Rendered">{lastUpdate.voxelsRendered.toLocaleString()}</StatRow>
                    <StatRow label="Cached Pixels">{(lastUpdate.cachedPixels / 1e6).toFixed(1)} Mpx</StatRow>
                  </div>
                </>
              )}
            </Panel>
          )}
        </aside>
      </div>
    </div>
  )
}

/** Also in index.html, shown until the app first renders. */
function Generating() {
  return (
    <div role="status" className="pointer-events-none absolute inset-0 flex items-center justify-center gap-3 font-mono text-sm text-white">
      <span className="size-5 animate-spin rounded-full border-2 border-white border-t-transparent" />
      Generating world…
    </div>
  )
}

function Panel({ icon, title, action, children }: {
  icon: ReactNode
  title: string
  action?: ReactNode
  children: ReactNode
}) {
  return (
    <Card className="pointer-events-auto gap-3 bg-card/80 p-4 backdrop-blur-md">
      <div className="flex items-center justify-between">
        <h2 className="flex items-center gap-2 text-sm font-semibold">
          <span className="text-primary">{icon}</span>
          {title}
        </h2>
        {action}
      </div>
      {children}
    </Card>
  )
}

function StatRow({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="flex justify-between font-mono text-xs">
      <span className="text-muted-foreground">{label}</span>
      <span>{children}</span>
    </div>
  )
}

function SliderRow({ label, value, min, max, step, onChange }: {
  label: string
  value: number
  min: number
  max: number
  step: number
  onChange: (value: number[]) => void
}) {
  return (
    <div className="space-y-2">
      <label className="block text-xs text-muted-foreground">{label}</label>
      <Slider value={[value]} onValueChange={onChange} min={min} max={max} step={step} />
    </div>
  )
}

export default App
