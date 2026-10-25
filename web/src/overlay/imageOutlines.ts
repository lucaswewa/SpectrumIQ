// The Image outlines layer: where each image of a scan lies in its stitched
// mosaic, from the layout stitching records. Its input is microscope's tile
// layout, until stitching (M8) records SpectrumIQ's.
import type { OverlayLayer } from './types'

/** Where each image of a mosaic went, as stitching records it, in the mosaic's pixels. */
export interface TileLayout {
  size: [number, number]
  tiles: {
    /** The image's file name, in the scan's `images/`. */
    image: string
    /** Its number in the scan: 1 for the first taken. */
    number: number
    x: number
    y: number
    width: number
    height: number
  }[]
}

/** A dashed rectangle around each image, labelled with its number, its file name on hover. */
export function imageOutlines(layout: TileLayout): OverlayLayer {
  return {
    id: 'image-outlines',
    label: 'Image outlines',
    size: layout.size,
    shapes: layout.tiles.map(({ image, number, x, y, width, height }) => ({
      kind: 'rect',
      x,
      y,
      width,
      height,
      dashed: true,
      label: String(number),
      title: `${number}: ${image}`,
    })),
  }
}
