// REQ-VIS-001: the viewer's LUT store: the image's own mappings, changes applied once the controls
// rest, visibility at once, mappings the routes take, and Reset.
import { createPinia, setActivePinia } from 'pinia'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { SETTLE_MS, useViewerLuts } from '@/viewer2d/lut'

const channels = [
  { label: 'DAPI', colour: '3f6fff', window: [100, 4000] as [number, number] },
  { label: 'GFP', colour: '3fff6f', window: [200, 9000] as [number, number] },
]

beforeEach(() => {
  setActivePinia(createPinia())
  vi.useFakeTimers()
})
afterEach(() => vi.useRealTimers())

describe('the LUT store', () => {
  it('starts from each channel of the image: its window and colour, shown', () => {
    const luts = useViewerLuts()
    luts.load(channels)
    expect(luts.applied).toEqual([
      { label: 'DAPI', min: 100, max: 4000, gamma: 1, colour: '3f6fff', visible: true },
      { label: 'GFP', min: 200, max: 9000, gamma: 1, colour: '3fff6f', visible: true },
    ])
  })

  it('applies a mapping once the controls rest, the last of a drag only', () => {
    const luts = useViewerLuts()
    luts.load(channels)
    const before = luts.applied
    for (const max of [5000, 6000, 7000]) {
      luts.set(0, { max })
      vi.advanceTimersByTime(SETTLE_MS - 1)
    }
    expect(luts.channels[0]!.max).toBe(7000)
    expect(luts.applied).toBe(before)
    vi.advanceTimersByTime(1)
    expect(luts.applied[0]!.max).toBe(7000)
    expect(luts.applied[1]).toEqual(before[1])
  })

  it('shows and hides a channel at once', () => {
    const luts = useViewerLuts()
    luts.load(channels)
    luts.set(1, { visible: false })
    expect(luts.applied[1]!.visible).toBe(false)
  })

  it('applies only mappings the routes take: a window that is not empty, a gamma in range', () => {
    const luts = useViewerLuts()
    luts.load(channels)
    luts.set(0, { min: 5000, gamma: 50 })
    vi.advanceTimersByTime(SETTLE_MS)
    expect(luts.applied[0]).toMatchObject({ min: 5000, max: 5001, gamma: 20 })
  })

  it('gives a channel its own mapping again on Reset, keeping whether it is shown', () => {
    const luts = useViewerLuts()
    luts.load(channels)
    luts.set(0, { min: 0, max: 100, colour: 'ff0000', visible: false })
    luts.reset(0)
    expect(luts.applied[0]).toEqual({ ...luts.applied[0], min: 100, max: 4000, colour: '3f6fff' })
    expect(luts.applied[0]!.visible).toBe(false)
    luts.set(5, { min: 1 })
    luts.reset(5)
    expect(luts.channels).toHaveLength(2)
  })
})
