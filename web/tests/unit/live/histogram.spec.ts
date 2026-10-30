// REQ-PRV-003: the histogram and the display mapping it moves (ADR-0038).
import { mount } from '@vue/test-utils'
import { describe, expect, it } from 'vitest'

import HistogramChart from '@/live/HistogramChart.vue'
import {
  barHeights,
  colormapColour,
  formatBits,
  fullScale,
  shareToValue,
  shownWindow,
  valueToShare,
  withEnd,
  type Lut,
} from '@/live/histogram'

const auto: Lut = { contrast: { kind: 'auto', low_pct: 0.1, high_pct: 99.9 }, colormap: 'grey' }
const manual: Lut = {
  contrast: { kind: 'manual', min: 200, max: 9000, gamma: 1.5 },
  colormap: { colour: [63, 255, 111] },
}

describe('the histogram and the mapping', () => {
  it('spans the pixel format’s whole range', () => {
    expect([formatBits('Mono14'), formatBits('Mono8'), formatBits(undefined)]).toEqual([14, 8, 16])
    expect(fullScale(14)).toBe(16383)
    expect([shareToValue(0, 14), shareToValue(0.5, 14), shareToValue(1.2, 14)]).toEqual([
      0, 8192, 16383,
    ])
    expect(valueToShare(16383, 14)).toBe(1)
    expect(valueToShare(shareToValue(0.25, 12), 12)).toBeCloseTo(0.25, 3)
  })

  it('scales the bars to the tallest, or to its logarithm', () => {
    expect(barHeights([0, 50, 100], false)).toEqual([0, 0.5, 1])
    const log = barHeights([0, 9, 99], true)
    expect(log[1]).toBeCloseTo(0.5)
    expect(barHeights([0, 0], false)).toEqual([0, 0])
  })

  it('shows a manual mapping’s window, or the histogram’s for auto-contrast', () => {
    expect(shownWindow(manual, [10, 20])).toEqual([200, 9000])
    expect(shownWindow(auto, [1828, 8923])).toEqual([1828, 8923])
    expect(shownWindow(auto, undefined)).toBeUndefined()
  })

  it('makes the mapping manual when an end moves, keeping the rest', () => {
    // From auto: the measured window, gamma 1, the colours kept.
    expect(withEnd(auto, [1828, 8923], 'max', 6000.4)).toEqual({
      contrast: { kind: 'manual', min: 1828, max: 6000, gamma: 1 },
      colormap: 'grey',
    })
    expect(withEnd(manual, [200, 9000], 'min', 500)).toEqual({
      contrast: { kind: 'manual', min: 500, max: 9000, gamma: 1.5 },
      colormap: { colour: [63, 255, 111] },
    })
    // The ends stay apart, and black never goes below 0.
    expect(withEnd(manual, [200, 9000], 'min', 12_000).contrast).toMatchObject({ min: 8999 })
    expect(withEnd(manual, [200, 9000], 'max', 100).contrast).toMatchObject({ max: 201 })
    expect(withEnd(manual, [200, 9000], 'min', -50).contrast).toMatchObject({ min: 0 })
    // A colour frame's white balance stays as it was.
    const balanced: Lut = { ...auto, white_balance: { kind: 'fixed', red: 1.4, blue: 1.9 } }
    expect(withEnd(balanced, [1828, 8923], 'min', 2000).white_balance).toEqual({
      kind: 'fixed',
      red: 1.4,
      blue: 1.9,
    })
  })

  it('names each colormap’s colour', () => {
    expect(colormapColour('grey')).toBe('rgb(255 255 255)')
    expect(colormapColour({ colour: [63, 255, 111] })).toBe('rgb(63 255 111)')
  })

  it('has a marker for each end, which the arrow keys move by 1/256 of the range', async () => {
    const chart = mount(HistogramChart, {
      props: { counts: [1, 2, 3], bits: 12, window: [1024, 3072] },
    })
    const markers = chart.findAll('[role="slider"]')
    expect(markers.map((marker) => marker.attributes('aria-valuenow'))).toEqual(['1024', '3072'])
    expect(markers[1]!.attributes('style')).toContain('left: 75.0')
    await markers[0]!.trigger('keydown', { key: 'ArrowRight' })
    await markers[1]!.trigger('keydown', { key: 'ArrowLeft', shiftKey: true })
    expect(chart.emitted('update')).toEqual([
      ['min', 1040],
      ['max', 2816],
    ])
  })
})
