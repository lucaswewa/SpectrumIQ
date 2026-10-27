// The live mosaic's viewport (ADR-0059, ADR-0060): a map's view as the server renders it, and
// where a rendered image sits in a view. A view is the stage map's (`scan/geometry`): the stage
// point at its centre, map pixels a µm, and its size; the server's zoom is view pixels a canvas
// pixel, and the canvas has `px_per_um` pixels a µm.
import { toMap, toStage, type Point, type View } from '@/scan/geometry'
import type { Value } from '@/stores/thing'

/** What the server renders: the stage point at its centre, its zoom, and its size. */
export type Viewport = Value<'mosaic_live', 'viewport'>

/** The largest image the server renders. */
export const MAX_SIZE: Point = [1920, 1080]
/** The zooms it renders, least and most. */
export const ZOOMS: Point = [1 / 1024, 64]

/**
 * The viewport that renders `view` on a canvas of `pxPerUm` pixels a µm: at most 1920 × 1080
 * pixels, the image then stretched over the view, and its zoom within what the server renders.
 */
export function viewportOf(view: View, pxPerUm: number): Viewport {
  const shrink = Math.max(1, view.size[0] / MAX_SIZE[0], view.size[1] / MAX_SIZE[1])
  const [width, height] = view.size.map((side) => Math.max(1, Math.round(side / shrink)))
  const zoom = Math.min(Math.max(view.scale / shrink / pxPerUm, ZOOMS[0]), ZOOMS[1])
  return { centre: [view.centre[0], view.centre[1]], zoom, width: width!, height: height! }
}

/** The stage box an image of `viewport` shows: its two corners. */
export function covered(viewport: Viewport, pxPerUm: number): [Point, Point] {
  const umPerPx = 1 / (viewport.zoom * pxPerUm)
  const [x, y] = viewport.centre as Point
  const [w, h] = [(viewport.width * umPerPx) / 2, (viewport.height * umPerPx) / 2]
  return [
    [x - w, y - h],
    [x + w, y + h],
  ]
}

/** Where an image of `viewport` sits in `view`, in map pixels. */
export function placement(view: View, viewport: Viewport, pxPerUm: number) {
  const [low, high] = covered(viewport, pxPerUm)
  const [left, top] = toMap(view, low)
  const [width, height] = [(high[0] - low[0]) * view.scale, (high[1] - low[1]) * view.scale]
  return { left, top, width, height }
}

/** The stage box `view` shows: its two corners. */
export function shownBy(view: View): [Point, Point] {
  return [toStage(view, [0, 0]), toStage(view, view.size)]
}

/** The smallest box around both `a` and `b`. */
export function union(a: [Point, Point], b: [Point, Point]): [Point, Point] {
  return [
    [Math.min(a[0][0], b[0][0]), Math.min(a[0][1], b[0][1])],
    [Math.max(a[1][0], b[1][0]), Math.max(a[1][1], b[1][1])],
  ]
}
