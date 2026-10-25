// The scale bar: a round length, 1, 2 or 5 × 10ⁿ µm, about as wide as `target` pixels on screen.

/** A scale bar: its length in µm, its width on screen, and its label. */
export interface ScaleBar {
  um: number
  pixels: number
  label: string
}

/** The longest round length within `target` screen pixels, at `umPerPixel` µm a screen pixel. */
export function scaleBar(umPerPixel: number, target = 120): ScaleBar | undefined {
  if (!(umPerPixel > 0) || !Number.isFinite(umPerPixel)) return undefined
  const most = target * umPerPixel
  const power = 10 ** Math.floor(Math.log10(most))
  const um = [5, 2, 1].map((step) => step * power).find((length) => length <= most) ?? power
  const label =
    um >= 1000 ? `${Number((um / 1000).toPrecision(3))} mm` : `${Number(um.toPrecision(3))} µm`
  return { um, pixels: um / umPerPixel, label }
}
