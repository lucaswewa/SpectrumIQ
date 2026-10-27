// Overlay layers (ADR-0009, amended): information drawn over an image, such as where
// a mosaic's images lie, in layers that can each be shown or hidden. A
// layer's shapes are in its own pixels, those of the image it describes,
// and are drawn wherever that image is on screen, at any zoom.

/** A shape's colour, as a meaning the theme colours: a stitch's residual, say. */
export type Tone = 'success' | 'warning' | 'danger' | 'muted'

/** A shape, in its layer's pixels: x to the right and y down, from the top left. */
export type Shape = { title?: string; tone?: Tone } & (
  | {
      kind: 'rect'
      x: number
      y: number
      width: number
      height: number
      dashed?: boolean
      /** A label inside its top-left corner. */
      label?: string
    }
  | { kind: 'line'; points: [number, number][]; dashed?: boolean }
  | { kind: 'point'; x: number; y: number }
  | { kind: 'label'; x: number; y: number; text: string }
)

/** A layer of shapes over an image. */
export interface OverlayLayer {
  /** What its choice to show it is kept under. */
  id: string
  /** What the Layers menu calls it. */
  label: string
  /** The size its shapes are measured in, in pixels: width and height. */
  size: [number, number]
  shapes: Shape[]
}

/** Where an image is drawn, in the pixels of the element over it. */
export interface Placement {
  x: number
  y: number
  width: number
  height: number
}

/** A point in `size`'s pixels, placed on screen by `placement`. */
export function toScreen(
  [x, y]: [number, number],
  size: [number, number],
  placement: Placement,
): [number, number] {
  return [
    placement.x + (x * placement.width) / size[0],
    placement.y + (y * placement.height) / size[1],
  ]
}
