// REQ-WF-001: the stage map's geometry, stage µm to map pixels and back (ADR-0057).
import { describe, expect, it } from 'vitest'

import {
  bounds,
  fit,
  pan,
  placed,
  toMap,
  toStage,
  zoom,
  type Point,
  type View,
} from '@/scan/geometry'

const view: View = { centre: [-20_000, -15_000], scale: 0.05, size: [800, 600] }
const close = (a: Point, b: Point) => {
  expect(a[0]).toBeCloseTo(b[0], 6)
  expect(a[1]).toBeCloseTo(b[1], 6)
}

describe('the stage map', () => {
  it('puts the centre in the middle, x right and y down, and maps back', () => {
    expect(toMap(view, [-20_000, -15_000])).toEqual([400, 300])
    expect(toMap(view, [-19_000, -14_000])).toEqual([450, 350])
    for (const point of [
      [0, 0],
      [-35_123.5, 7.25],
      [-100_000, -75_000],
    ] as Point[]) {
      close(toStage(view, toMap(view, point)), point)
    }
  })

  it('fits a box whole, centred, with its margin', () => {
    const box: [Point, Point] = [
      [-100_000, -75_000],
      [0, 0],
    ]
    const fitted = fit([800, 600], box)
    expect(fitted.centre).toEqual([-50_000, -37_500])
    const [low, high] = box.map((corner) => toMap(fitted, corner))
    expect(low![0]).toBeGreaterThan(0)
    expect(high![0]).toBeLessThan(800)
    expect(high![1] - low![1]).toBeCloseTo(600 / 1.1, 6)
  })

  it('zooms about the pointer, which stays over the same place, and pans with it', () => {
    const under = toStage(view, [100, 500])
    const zoomed = zoom(view, 2.5, [100, 500])
    expect(zoomed.scale).toBeCloseTo(0.125)
    close(toMap(zoomed, under), [100, 500])
    const panned = pan(view, [40, -20])
    close(toMap(panned, toStage(view, [400, 300])), [440, 280])
  })

  it('places a frame about a tile, and boxes points', () => {
    const frame: Point[] = [
      [-50, -40],
      [50, -40],
      [50, 40],
      [-50, 40],
    ]
    expect(placed(frame, [100, 200])).toEqual([
      [50, 160],
      [150, 160],
      [150, 240],
      [50, 240],
    ])
    expect(bounds(placed(frame, [0, 0]))).toEqual([
      [-50, -40],
      [50, 40],
    ])
  })
})
