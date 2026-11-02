// @vitest-environment jsdom
import './dom.test-helper'
import { describe, it, expect, vi } from 'vitest'
import { act, render, screen, fireEvent, waitFor } from '@testing-library/react'
import App from './App'

// These tests cover the UI; a few voxels render much faster than the full forest.
// A strip of ground 10 units wide and 100 long, in a small box at every level.
vi.mock('./lib/forest', () => ({
  forest: (seed: number) => ({
    generator: {
      kinds: { moss: { type: 1, colors: ['#4f7a2a'] } },
      seed,
      layers: [{ name: 'ground', order: 0, axis: 1, spans: (x: number, z: number) => (x >= 0 && x < 10 && z >= 0 && z < 100 ? [{ from: -1, to: 1, kind: 'moss' }] : []) }],
    },
    spawn: { x: 0, y: 6, z: -4 },
    box: () => ({ minX: -16, minY: -4, minZ: -16, maxX: 16, maxY: 4, maxZ: 112 }),
  }),
}))

const cameraPosition = () =>
  (screen.getByText('Position').nextElementSibling?.textContent ?? '').split(',').map(Number)
const totalUpdates = () => Number(screen.getByText('Total Updates').nextElementSibling?.textContent)

describe('App', () => {
  it('renders the title and control panels', () => {
    render(<App />)
    expect(screen.getByText('PARALLAX ENGINE')).toBeInTheDocument()
    expect(screen.getByText('Render Statistics')).toBeInTheDocument()
    expect(screen.getByText('Camera')).toBeInTheDocument()
    expect(screen.getByText('World')).toBeInTheDocument()
  })

  it('renders the scene on mount', () => {
    const { container } = render(<App />)
    expect(container.querySelectorAll('canvas').length).toBeGreaterThan(0)
    expect(totalUpdates()).toBeGreaterThan(0)
  })

  it('shows a spinner while generating a world', async () => {
    render(<App />)
    expect(screen.queryByRole('status')).not.toBeInTheDocument()
    fireEvent.click(screen.getByRole('button', { name: /Regenerate World/ }))
    expect(screen.getByRole('status')).toHaveTextContent('Generating world')
    await waitFor(() => { expect(screen.queryByRole('status')).not.toBeInTheDocument() })
  })

  it('hides and re-shows the statistics panel', () => {
    render(<App />)
    fireEvent.click(screen.getByRole('button', { name: 'Hide statistics' }))
    expect(screen.queryByText('Render Statistics')).not.toBeInTheDocument()

    fireEvent.click(screen.getByRole('button', { name: /Stats/ }))
    expect(screen.getByText('Render Statistics')).toBeInTheDocument()
  })

  it.each([
    ['w', 2],
    ['s', 2],
    ['d', 0],
    ['a', 0],
    [' ', 1],
    ['Shift', 1],
  ])('moves the camera along one axis while %j is held', async (key, axis) => {
    const sign = ['s', 'a', 'Shift'].includes(key) ? -1 : 1
    render(<App />)
    const [start, updates] = [cameraPosition(), totalUpdates()]

    fireEvent.keyDown(window, { key })
    await waitFor(() => { expect(sign * (cameraPosition()[axis] - start[axis])).toBeGreaterThanOrEqual(1) })
    // The scene is rendered too, not only the camera moved.
    await waitFor(() => { expect(totalUpdates()).toBeGreaterThan(updates) })
    fireEvent.keyUp(window, { key })

    // Let React flush the state from the last frame, then check movement stopped.
    const settle = () => new Promise(resolve => setTimeout(resolve, 50))
    await settle()
    const moved = cameraPosition()
    expect(moved.filter((_, i) => i !== axis)).toEqual(start.filter((_, i) => i !== axis))
    await settle()
    expect(cameraPosition()).toEqual(moved)
  })

  it('moves at the set speed however slow the frames are', () => {
    vi.useFakeTimers()
    // 10 frames per second.
    vi.spyOn(window, 'requestAnimationFrame').mockImplementation(callback =>
      window.setTimeout(() => { callback(performance.now()) }, 100))
    vi.spyOn(window, 'cancelAnimationFrame').mockImplementation(id => { window.clearTimeout(id) })
    try {
      render(<App />)
      const start = cameraPosition()
      fireEvent.keyDown(window, { key: 'w' })
      act(() => { vi.advanceTimersByTime(1000) })
      fireEvent.keyUp(window, { key: 'w' })
      // 60 units per second, less the first frame, which only starts the clock.
      expect(cameraPosition()[2] - start[2]).toBeCloseTo(54)
    } finally {
      vi.useRealTimers()
      vi.restoreAllMocks()
    }
  })

  it('turns with the arrow keys, and then moves forward where it looks', async () => {
    render(<App />)
    const orientation = () => (screen.getByText('Yaw, Pitch').nextElementSibling?.textContent ?? '').split(',').map(parseFloat)
    expect(orientation()).toEqual([0, 0])

    fireEvent.keyDown(window, { key: 'ArrowRight' })
    await waitFor(() => { expect(orientation()[0]).toBeGreaterThanOrEqual(10) })
    fireEvent.keyUp(window, { key: 'ArrowRight' })
    expect(orientation()[1]).toBe(0)

    const start = cameraPosition()
    fireEvent.keyDown(window, { key: 'w' })
    await waitFor(() => { expect(cameraPosition()[2] - start[2]).toBeGreaterThanOrEqual(1) })
    fireEvent.keyUp(window, { key: 'w' })
    // Turned right, so forward also goes towards +x; level, so not up.
    expect(cameraPosition()[0]).toBeGreaterThan(start[0])
    expect(cameraPosition()[1]).toBe(start[1])
  })

  it('ignores unrelated keys', async () => {
    render(<App />)
    const start = cameraPosition()
    fireEvent.keyDown(window, { key: 'q' })
    await new Promise(resolve => setTimeout(resolve, 50))
    fireEvent.keyUp(window, { key: 'q' })
    expect(cameraPosition()).toEqual(start)
  })

  it('lists movement controls', () => {
    render(<App />)
    expect(screen.getByText('W/S - Forward/Back')).toBeInTheDocument()
    expect(screen.getByText('A/D - Left/Right')).toBeInTheDocument()
    expect(screen.getByText('Space/Shift - Up/Down')).toBeInTheDocument()
    expect(screen.getByText('Arrows/Drag - Look around')).toBeInTheDocument()
  })
})
