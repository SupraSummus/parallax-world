/** What UI tests need besides jsdom: cleanup, matchers and stubs of what jsdom lacks. */
import { afterEach, vi } from 'vitest'
import { cleanup } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'

afterEach(() => {
  cleanup()
})

globalThis.ResizeObserver = vi.fn(function(this: ResizeObserver) {
  this.observe = vi.fn()
  this.unobserve = vi.fn()
  this.disconnect = vi.fn()
}) as unknown as typeof ResizeObserver

// Plain no-ops, not vi.fn(): mocks record every call, and rendering a
// scene makes millions of them.
const noop = () => undefined

// A 2D context only: jsdom has no WebGL, so the scene isn't drawn.
HTMLCanvasElement.prototype.getContext = vi.fn(function(type: string) {
  if (type !== '2d') return null
  return {
    fillStyle: '',
    strokeStyle: '',
    lineWidth: 1,
    fillRect: noop,
    strokeRect: noop,
    clearRect: noop,
    drawImage: noop,
    beginPath: noop,
    closePath: noop,
    moveTo: noop,
    lineTo: noop,
    arc: noop,
    fill: noop,
    stroke: noop,
    save: noop,
    restore: noop,
    translate: noop,
    rotate: noop,
    scale: noop,
    globalAlpha: 1,
    // No pixels are kept, so drawing costs no memory.
    createImageData: (width: number, height: number) => ({ width, height, data: new Uint8ClampedArray(0) }),
    putImageData: noop,
    canvas: document.createElement('canvas'),
  }
}) as unknown as typeof HTMLCanvasElement.prototype.getContext

globalThis.requestAnimationFrame = (cb: FrameRequestCallback) =>
  window.setTimeout(() => { cb(performance.now()) }, 16)
globalThis.cancelAnimationFrame = (id: number) => { window.clearTimeout(id) }
