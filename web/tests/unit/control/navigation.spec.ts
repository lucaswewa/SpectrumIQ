// REQ-STG-001, REQ-STG-002: the steps, the wheel's focus, click-to-move's maths and the saved
// positions (ADR-0039).
import { createPinia, setActivePinia } from 'pinia'
import { beforeEach, describe, expect, it } from 'vitest'
import { nextTick } from 'vue'

import { clickMove } from '@/control/clickToMove'
import { NAVIGATION_KEY, useNavigationPreferences } from '@/control/navigation'
import { POSITIONS_KEY, useSavedPositions } from '@/control/positions'
import { notches, wheelFocus } from '@/control/wheel'

beforeEach(() => {
  window.localStorage.clear()
  setActivePinia(createPinia())
})

describe('the navigation preferences', () => {
  it('start from steps of 100 µm in x and y, are signed by the inversion, and are kept', async () => {
    const preferences = useNavigationPreferences()
    expect(preferences.signedSteps()).toEqual({ x: 100, y: 100, z: 50 })
    expect(preferences.steps.focus).toBe(1)
    preferences.steps.xy = 10
    preferences.invert.y = true
    await nextTick()
    expect(preferences.signedSteps()).toEqual({ x: 10, y: -10, z: 50 })

    setActivePinia(createPinia())
    expect(useNavigationPreferences().signedSteps()).toEqual({ x: 10, y: -10, z: 50 })
    expect(useNavigationPreferences().signs()).toEqual({ x: 1, y: -1 })
  })

  it('ignore saved values that make no sense', () => {
    const saved = { steps: { xy: -5, z: 'far', focus: 0.5 }, invert: { x: 'yes', y: true } }
    window.localStorage.setItem(NAVIGATION_KEY, JSON.stringify(saved))
    const preferences = useNavigationPreferences()
    expect(preferences.signedSteps()).toEqual({ x: 100, y: -100, z: 50 })
    expect(preferences.steps.focus).toBe(0.5)
    window.localStorage.setItem(NAVIGATION_KEY, 'not JSON')
    setActivePinia(createPinia())
    expect(useNavigationPreferences().signedSteps()).toEqual({ x: 100, y: 100, z: 50 })
  })
})

describe('wheel focus', () => {
  const wheel = (deltaY: number, deltaMode = 0) =>
    new WheelEvent('wheel', { deltaY, deltaMode, cancelable: true })

  it('counts notches in pixels, lines or pages', () => {
    expect(notches(wheel(100))).toBe(1)
    expect(notches(wheel(-6, 1))).toBe(-2)
    expect(notches(wheel(1, 2))).toBe(1)
  })

  it('focuses up a step a notch away from you, and down towards you', () => {
    const moves: number[] = []
    const onWheel = wheelFocus(
      (dz) => moves.push(dz),
      () => 2,
    )
    const event = wheel(-100)
    onWheel(event)
    onWheel(wheel(150))
    onWheel(wheel(0))
    expect(event.defaultPrevented).toBe(true)
    expect(moves).toEqual([2, -3])
  })
})

describe('click-to-move', () => {
  const optics = {
    region: { offset_x: 0, offset_y: 0, width: 4000, height: 3000 },
    pixelSize: 3.76,
    magnification: 10,
    signs: { x: 1, y: 1 },
  }

  it('moves the stage by the point’s share of the field of view, the same way on both axes', () => {
    // The field of view: 4000 × 0.376 = 1504 µm by 3000 × 0.376 = 1128 µm.
    const move = clickMove({ x: 0.25, y: -0.5 }, optics)
    expect(move.dx).toBeCloseTo(376)
    expect(move.dy).toBeCloseTo(-564)
    expect(clickMove({ x: 0, y: 0 }, optics)).toEqual({ dx: 0, dy: 0 })
  })

  it('takes the region read out, the magnification and the inversion into account', () => {
    const roi = { ...optics, region: { offset_x: 800, offset_y: 0, width: 1000, height: 1000 } }
    expect(clickMove({ x: 0.5, y: 0.5 }, { ...roi, magnification: 20 }).dx).toBeCloseTo(94)
    const inverted = clickMove({ x: 0.5, y: 0.5 }, { ...roi, signs: { x: -1, y: 1 } })
    expect(inverted.dx).toBeCloseTo(-188)
    expect(inverted.dy).toBeCloseTo(188)
  })
})

describe('the saved positions', () => {
  it('are numbered, renamed, forgotten and kept in this browser', async () => {
    const saved = useSavedPositions()
    saved.add({ x: 1, y: 2, z: 3 }, 40)
    saved.add({ x: -5, y: 0, z: 0 })
    saved.positions[0]!.name = 'Cell A'
    saved.remove(2)
    saved.add({ x: 7, y: 7, z: 7 })
    await nextTick()
    setActivePinia(createPinia())
    expect(useSavedPositions().positions).toEqual([
      { id: 1, name: 'Cell A', x: 1, y: 2, z: 3, focus: 40 },
      { id: 2, name: 'Position 2', x: 7, y: 7, z: 7 },
    ])
  })

  it('leave out what makes no sense', () => {
    const entries = [
      { id: 1, name: 'ok', x: 0, y: 0, z: 0 },
      { id: 2, name: 'no z', x: 0, y: 0 },
      7,
    ]
    window.localStorage.setItem(POSITIONS_KEY, JSON.stringify(entries))
    expect(useSavedPositions().positions.map((each) => each.name)).toEqual(['ok'])
    window.localStorage.setItem(POSITIONS_KEY, '{')
    setActivePinia(createPinia())
    expect(useSavedPositions().positions).toEqual([])
  })
})
