/** A width and a height, in any unit. */
export interface Size {
  width: number
  height: number
}

/** A point, or a displacement: rightwards and downwards. */
export interface Point {
  x: number
  y: number
}

/**
 * Where `point`, from the top left of a box of size `box`, falls on an image
 * of size `image` drawn contain-fit in the box: centred, as large as fits,
 * and letterboxed. It's measured from the image's centre, as shares of its
 * width and height, so the image's edges are at ±0.5. Undefined on the
 * letterboxing.
 */
export function containPoint(point: Point, box: Size, image: Size): Point | undefined {
  const scale = Math.min(box.width / image.width, box.height / image.height)
  const x = (point.x - box.width / 2) / (image.width * scale)
  const y = (point.y - box.height / 2) / (image.height * scale)
  return Math.abs(x) <= 0.5 && Math.abs(y) <= 0.5 ? { x, y } : undefined
}
