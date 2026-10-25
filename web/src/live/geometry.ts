import type { Point, Size } from './contain'

/** A rectangle in pixels: its top left corner and its size. */
export interface Rect {
  x: number
  y: number
  width: number
  height: number
}

/** How the camera reads its sensor out: the region, in sensor pixels, and the binning. */
export interface Readout {
  region: { offset_x: number; offset_y: number; width: number; height: number }
  binning: { horizontal: number; vertical: number }
}

/** The ROI stream's largest size, in preview pixels: `siq-preview`'s default `roi_size`. */
export const ROI_MAX: Size = { width: 1920, height: 1080 }

/** The frame's size, in frame pixels: the region, binned. */
export function frameSize({ region, binning }: Readout): Size {
  return {
    width: Math.floor(region.width / binning.horizontal),
    height: Math.floor(region.height / binning.vertical),
  }
}

/** The part of `box` that an image of size `image` covers, drawn contain-fit (centred, letterboxed). */
export function shownArea(box: Size, image: Size): Rect {
  const scale = Math.min(box.width / image.width, box.height / image.height)
  const [width, height] = [image.width * scale, image.height * scale]
  return { x: (box.width - width) / 2, y: (box.height - height) / 2, width, height }
}

/** Where a point of `box` falls in a frame of size `frame`, shown as `image` contain-fit: clamped to the frame. */
export function boxToFrame(point: Point, box: Size, image: Size, frame: Size): Point {
  const shown = shownArea(box, image)
  const share = (at: number, start: number, length: number) =>
    Math.min(Math.max((at - start) / length, 0), 1)
  return {
    x: share(point.x, shown.x, shown.width) * frame.width,
    y: share(point.y, shown.y, shown.height) * frame.height,
  }
}

/** A rectangle of the frame, as it is drawn in `box`. */
export function frameToBox(rect: Rect, box: Size, image: Size, frame: Size): Rect {
  const shown = shownArea(box, image)
  const [sx, sy] = [shown.width / frame.width, shown.height / frame.height]
  return {
    x: shown.x + rect.x * sx,
    y: shown.y + rect.y * sy,
    width: rect.width * sx,
    height: rect.height * sy,
  }
}

/** A rectangle of the frame in sensor pixels, as `display.roi_region` takes it. */
export function frameToSensor(rect: Rect, { region, binning }: Readout): Rect {
  return {
    x: region.offset_x + Math.round(rect.x) * binning.horizontal,
    y: region.offset_y + Math.round(rect.y) * binning.vertical,
    width: Math.round(rect.width) * binning.horizontal,
    height: Math.round(rect.height) * binning.vertical,
  }
}

/** A rectangle in sensor pixels, in the frame, clamped to it: as `siq-preview` maps it. */
export function sensorToFrame(rect: Rect, readout: Readout): Rect {
  const frame = frameSize(readout)
  const { region, binning } = readout
  const map = (sensor: number, offset: number, bin: number, end: number) =>
    Math.min(Math.max(Math.floor((sensor - offset) / bin), 0), end)
  const x0 = map(rect.x, region.offset_x, binning.horizontal, frame.width)
  const y0 = map(rect.y, region.offset_y, binning.vertical, frame.height)
  const x1 = map(rect.x + rect.width, region.offset_x, binning.horizontal, frame.width)
  const y1 = map(rect.y + rect.height, region.offset_y, binning.vertical, frame.height)
  return { x: x0, y: y0, width: x1 - x0, height: y1 - y0 }
}

/** The frame rectangle between two points of `box`, dragged on the frame shown there. */
export function dragged(a: Point, b: Point, box: Size, image: Size, frame: Size): Rect {
  const [p, q] = [boxToFrame(a, box, image, frame), boxToFrame(b, box, image, frame)]
  const [x, y] = [Math.min(p.x, q.x), Math.min(p.y, q.y)]
  return { x, y, width: Math.abs(p.x - q.x), height: Math.abs(p.y - q.y) }
}

/**
 * The part of the frame the ROI stream shows (`siq-preview`'s `roi_region`): the asked region,
 * or the whole frame, cut to `max` preview pixels at `zoom` about its middle, in whole steps of
 * the zoom.
 */
export function roiArea(frame: Size, asked: Rect | undefined, zoom: 1 | 2, max = ROI_MAX): Rect {
  const area = asked && asked.width > 0 && asked.height > 0 ? asked : undefined
  const { x, y, width, height } = area ?? { x: 0, y: 0, ...frame }
  const cut = (n: number, most: number, end: number) =>
    Math.min(Math.max(Math.floor(Math.min(n, most * zoom) / zoom) * zoom, zoom), end)
  const [cw, ch] = [cut(width, max.width, frame.width), cut(height, max.height, frame.height)]
  const start = (at: number, n: number, length: number, end: number) =>
    Math.min(
      Math.floor(Math.max(at + Math.floor(n / 2) - Math.floor(length / 2), 0) / zoom) * zoom,
      end - length,
    )
  return {
    x: start(x, width, cw, frame.width),
    y: start(y, height, ch, frame.height),
    width: cw,
    height: ch,
  }
}
