import type { Value } from '@/stores/thing'

/** The display mapping (`display.lut`). */
export type Lut = Value<'display', 'lut'>
/** The colours of a display mapping. */
export type Colormap = Lut['colormap']

/** A pixel format's bits: 14 for `Mono14`. */
export function formatBits(format: string | undefined): number {
  return Number(/\d+$/.exec(format ?? '')?.[0] ?? 16)
}

/** The largest value of `bits`: 16383 for 14. */
export function fullScale(bits: number): number {
  return 2 ** bits - 1
}

/** The raw value at a share `t` (0 to 1) of the histogram's width: the bins span 0 to full scale. */
export function shareToValue(t: number, bits: number): number {
  return Math.round(Math.min(Math.max(t, 0), 1) * fullScale(bits))
}

/** The share of the histogram's width at a raw value. */
export function valueToShare(value: number, bits: number): number {
  return Math.min(Math.max(value / fullScale(bits), 0), 1)
}

/** The bars' heights, from 0 to 1: the counts against the largest, or their logarithms. */
export function barHeights(counts: readonly number[], log: boolean): number[] {
  const scaled = counts.map((count) => (log ? Math.log10(1 + count) : count))
  const top = Math.max(...scaled, 0)
  return scaled.map((value) => (top > 0 ? value / top : 0))
}

/**
 * The window the previews show now: a manual mapping's own, or, for auto-contrast, the latest
 * histogram's percentiles. Undefined before there is any.
 */
export function shownWindow(
  lut: Lut | undefined,
  measured: readonly [number, number] | undefined,
): [number, number] | undefined {
  if (lut?.contrast.kind === 'manual') return [lut.contrast.min, lut.contrast.max]
  return measured ? [measured[0], measured[1]] : undefined
}

/**
 * The mapping with one end of its window moved: manual from then on, keeping the other end, the
 * gamma (1 from auto) and the colours. The ends stay at least one value apart.
 */
export function withEnd(
  lut: Lut,
  window: readonly [number, number],
  end: 'min' | 'max',
  value: number,
): Lut {
  const gamma = lut.contrast.kind === 'manual' ? lut.contrast.gamma : 1
  const [min, max] =
    end === 'min'
      ? [Math.min(Math.round(value), window[1] - 1), window[1]]
      : [window[0], Math.max(Math.round(value), window[0] + 1)]
  return { contrast: { kind: 'manual', min: Math.max(min, 0), max, gamma }, colormap: lut.colormap }
}

/** The colormap's CSS colour at full scale, for its swatch. */
export function colormapColour(colormap: Colormap): string {
  if (colormap === 'grey') return 'rgb(255 255 255)'
  if (colormap === 'inverted') return 'rgb(0 0 0)'
  const [r, g, b] = colormap.colour
  return `rgb(${r} ${g} ${b})`
}
