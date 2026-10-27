// REQ-STI-001: the live mosaic's viewport maths (ADR-0060): a map's view as the server renders
// it, where its image sits in a view that has moved since, and a call throttled to once an
// interval with the last always made.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { throttle } from '@/mosaic/throttle'
import { covered, placement, shownBy, union, viewportOf, ZOOMS } from '@/mosaic/viewport'
import { pan, zoom, type View } from '@/scan/geometry'

/** 1.33 canvas pixels a µm: the simulator's 10× at a canvas scale of ½. */
const PX_PER_UM = 1.33
const view: View = { centre: [-20_000, -15_000], scale: 0.5, size: [800, 600] }

describe('the viewport', () => {
  it("is the view's centre and size, its scale in view pixels a canvas pixel", () => {
    const viewport = viewportOf(view, PX_PER_UM)
    expect(viewport).toEqual({
      centre: [-20_000, -15_000],
      zoom: 0.5 / PX_PER_UM,
      width: 800,
      height: 600,
    })
    // It shows the stage the view shows.
    const [low, high] = covered(viewport, PX_PER_UM)
    const [shownLow, shownHigh] = shownBy(view)
    for (const [a, b] of [
      [low, shownLow],
      [high, shownHigh],
    ]) {
      expect(a![0]).toBeCloseTo(b![0], 9)
      expect(a![1]).toBeCloseTo(b![1], 9)
    }
  })

  it('is at most 1920 × 1080, the zoom shrunk with it, and within the zooms rendered', () => {
    const wide = viewportOf({ ...view, size: [3840, 1600] }, PX_PER_UM)
    expect([wide.width, wide.height]).toEqual([1920, 800])
    expect(wide.zoom).toBeCloseTo(0.25 / PX_PER_UM, 12)
    // Still the same stage, the image stretched over the view.
    const placed = placement({ ...view, size: [3840, 1600] }, wide, PX_PER_UM)
    expect(placed.left).toBeCloseTo(0, 9)
    expect(placed.width).toBeCloseTo(3840, 9)
    expect(viewportOf({ ...view, scale: 1e-9 }, PX_PER_UM).zoom).toBe(ZOOMS[0])
    expect(viewportOf({ ...view, scale: 1e6 }, PX_PER_UM).zoom).toBe(ZOOMS[1])
  })

  it('places an image where its viewport was, in a view panned and zoomed since', () => {
    const viewport = viewportOf(view, PX_PER_UM)
    expect(placement(view, viewport, PX_PER_UM)).toEqual({
      left: 0,
      top: 0,
      width: 800,
      height: 600,
    })
    // Dragged 30 pixels right and 10 down: the image follows the drag.
    const dragged = placement(pan(view, [30, 10]), viewport, PX_PER_UM)
    expect(dragged.left).toBeCloseTo(30, 9)
    expect(dragged.top).toBeCloseTo(10, 9)
    expect(dragged.width).toBeCloseTo(800, 9)
    // Zoomed in twice about the view's corner: twice as large, the corner where it was.
    const zoomed = placement(zoom(view, 2, [0, 0]), viewport, PX_PER_UM)
    expect(zoomed.left).toBeCloseTo(0, 9)
    expect(zoomed.width).toBeCloseTo(1600, 9)
    expect(zoomed.height).toBeCloseTo(1200, 9)
  })

  it('makes the box of two boxes', () => {
    expect(
      union(
        [
          [0, 0],
          [10, 5],
        ],
        [
          [-3, 2],
          [4, 9],
        ],
      ),
    ).toEqual([
      [-3, 0],
      [10, 9],
    ])
  })
})

describe('the throttle', () => {
  beforeEach(() => vi.useFakeTimers())
  afterEach(() => vi.useRealTimers())

  it('calls at once, then at most once an interval, the latest call last', () => {
    const calls: number[] = []
    const throttled = throttle((n: number) => calls.push(n), 100)
    throttled(1)
    expect(calls).toEqual([1])
    for (let n = 2; n <= 10; n++) {
      vi.advanceTimersByTime(10)
      throttled(n)
    }
    // 90 ms on: only the first so far; the latest goes at 100 ms.
    expect(calls).toEqual([1])
    vi.advanceTimersByTime(10)
    expect(calls).toEqual([1, 10])
    // Nothing waits, so nothing more; after a quiet interval, the next goes at once.
    vi.advanceTimersByTime(500)
    expect(calls).toEqual([1, 10])
    throttled(11)
    expect(calls).toEqual([1, 10, 11])
    throttled(12)
    throttled.cancel()
    vi.advanceTimersByTime(500)
    expect(calls).toEqual([1, 10, 11])
  })
})
