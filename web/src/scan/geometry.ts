// The stage map's geometry (ADR-0057): stage µm to map pixels and back. Stage x runs right and
// y down, as the camera's image does under the stage–camera calibration.

/** A point: on the stage in µm, or on the map in pixels. */
export type Point = [number, number]

/** What the map shows: the stage about `centre`, at `scale` pixels a µm, on `size` pixels. */
export interface View {
  centre: Point
  scale: number
  size: Point
}

/** Where stage point `at` is on the map. */
export function toMap(view: View, [x, y]: Point): Point {
  const { centre, scale, size } = view
  return [(x - centre[0]) * scale + size[0] / 2, (y - centre[1]) * scale + size[1] / 2]
}

/** Where map point `at` is on the stage. */
export function toStage(view: View, [px, py]: Point): Point {
  const { centre, scale, size } = view
  return [(px - size[0] / 2) / scale + centre[0], (py - size[1] / 2) / scale + centre[1]]
}

/** The smallest box around `points`: its two corners. */
export function bounds(points: Point[]): [Point, Point] {
  const xs = points.map(([x]) => x)
  const ys = points.map(([, y]) => y)
  return [
    [Math.min(...xs), Math.min(...ys)],
    [Math.max(...xs), Math.max(...ys)],
  ]
}

/** The view of `size` pixels that shows the box `[low, high]` whole, `margin` of it to spare. */
export function fit(size: Point, [low, high]: [Point, Point], margin = 0.05): View {
  const span: Point = [Math.max(high[0] - low[0], 1), Math.max(high[1] - low[1], 1)]
  const scale = Math.min(size[0] / span[0], size[1] / span[1]) / (1 + 2 * margin)
  return { centre: [(low[0] + high[0]) / 2, (low[1] + high[1]) / 2], scale, size }
}

/** The view zoomed by `factor` about map point `at`, which stays where it is. */
export function zoom(view: View, factor: number, at: Point): View {
  const anchor = toStage(view, at)
  const scale = view.scale * factor
  const centre: Point = [
    anchor[0] - (at[0] - view.size[0] / 2) / scale,
    anchor[1] - (at[1] - view.size[1] / 2) / scale,
  ]
  return { ...view, centre, scale }
}

/** The view dragged by `delta` map pixels: the stage under the pointer follows it. */
export function pan(view: View, [dx, dy]: Point): View {
  return { ...view, centre: [view.centre[0] - dx / view.scale, view.centre[1] - dy / view.scale] }
}

/** The corners of `frame`, offsets from a frame's centre, about stage point `at`. */
export function placed(frame: Point[], [x, y]: Point): Point[] {
  return frame.map(([dx, dy]) => [x + dx, y + dy])
}

/** `points` on the map, as an SVG `points` attribute. */
export function svgPoints(view: View, points: Point[]): string {
  return points.map((point) => toMap(view, point).join(',')).join(' ')
}
