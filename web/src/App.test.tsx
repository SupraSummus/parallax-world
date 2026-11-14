// @vitest-environment jsdom
import './dom.test-helper'
import { describe, it, expect, vi } from 'vitest'
import { act, render, screen, fireEvent, waitFor } from '@testing-library/react'
import App from './App'

// These tests cover the UI; a few voxels render much faster than the full forest.
vi.mock('./lib/worldgen', async () => (await import('./lib/small-forest.test-helper')).smallWorldgen)

const cameraPosition = () =>
  (screen.getByText('Position').nextElementSibling?.textContent ?? '').split(',').map(Number)
const orientation = () =>
  (screen.getByText('Yaw, Pitch').nextElementSibling?.textContent ?? '').split(',').map(parseFloat)
const totalUpdates = () => Number(screen.getByText('Total Updates').nextElementSibling?.textContent)

describe('App', () => {
  it('renders the control panels and the scene on mount', () => {
    const { container } = render(<App />)
    for (const panel of ['Render Statistics', 'Camera', 'World']) expect(screen.getByText(panel)).toBeInTheDocument()
    expect(container.querySelectorAll('canvas').length).toBeGreaterThan(0)
    expect(totalUpdates()).toBeGreaterThan(0)
  })

  it("starts at the view the page's query asks for", () => {
    // As pnpm shot opens it.
    const page = location.href
    history.replaceState(null, '', '?camera=1,2,3&look=30,-10')
    try {
      render(<App />)
      expect(cameraPosition()).toEqual([1, 2, 3])
      expect(orientation()).toEqual([30, -10])
    } finally {
      history.replaceState(null, '', page)
    }
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

  it('moves at one speed however slow the frames are', () => {
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

  it('walks on the ground with F, at eye height over it, and flies again with F', async () => {
    render(<App />)
    const walk = screen.getByRole('switch', { name: /Walk on Ground/ })
    fireEvent.keyDown(window, { key: 'f' })
    fireEvent.keyUp(window, { key: 'f' })
    expect(walk).toBeChecked()
    // The test world's ground is about y = 1 under the spawn point, 6 high; the eye 1.62 m (13 units) over it.
    await waitFor(() => { expect(cameraPosition()[1]).toBeGreaterThan(12) })
    await new Promise(resolve => setTimeout(resolve, 300))
    const standing = cameraPosition()
    expect(standing[1]).toBeLessThan(17)

    fireEvent.keyDown(window, { key: 'w' })
    await waitFor(() => { expect(cameraPosition()[2] - standing[2]).toBeGreaterThanOrEqual(4) })
    fireEvent.keyUp(window, { key: 'w' })
    expect(Math.abs(cameraPosition()[1] - standing[1])).toBeLessThan(4)

    fireEvent.click(walk)
    expect(walk).not.toBeChecked()
  })

  it('ignores unrelated keys', async () => {
    render(<App />)
    const start = cameraPosition()
    fireEvent.keyDown(window, { key: 'q' })
    await new Promise(resolve => setTimeout(resolve, 50))
    fireEvent.keyUp(window, { key: 'q' })
    expect(cameraPosition()).toEqual(start)
  })
})
