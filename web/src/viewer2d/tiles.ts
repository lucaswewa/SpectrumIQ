// The data routes' URLs (ADR-0041, ADR-0045): an image's metadata, a plane's DZI descriptor with
// the channel's mapping in its query, which OpenSeadragon repeats on every tile, and the value
// probe. The image is named by its dataset's id and its name in the dataset.

/** A plane of an image: its channel, focus plane and time point. */
export interface Plane {
  c: number
  z: number
  t: number
}

/** How a channel's values are shown: the window, the gamma, and the colour, as `rrggbb`. */
export interface ChannelLut {
  min: number
  max: number
  gamma: number
  colour: string
}

/** An image: under `data`, the data routes' root (`…/api/siq/data/`). */
export interface ImageRef {
  data: URL
  dataset: string
  image: string
}

/** A channel of an image's metadata. */
export interface ImageChannel {
  label: string
  colour: string
  window: [number, number]
}

/** An image's metadata (`…/info`): its size, levels, pixel size in µm (`z`, `y`, `x`) and channels. */
export interface ImageInfo {
  name: string
  sample: 'u16' | 'f32'
  size: { t: number; c: number; z: number; y: number; x: number }
  levels: number
  pixel_size: [number, number, number]
  origin: [number, number, number]
  channels: ImageChannel[]
}

const segment = encodeURIComponent

/** The data routes' root, beside the Things' API (`…/api/siq/data/`). */
export const dataRoot = (client: { baseUrl: URL }) => new URL('../siq/data/', client.baseUrl)

function imageUrl(image: ImageRef, path: string) {
  return new URL(`${segment(image.dataset)}/${segment(image.image)}/${path}`, image.data)
}

/** A mapping as a query: six significant digits, so that a drag's tiny steps share their tiles. */
export function lutQuery(lut: ChannelLut): URLSearchParams {
  const number = (value: number) => String(Number(value.toPrecision(6)))
  const colour = lut.colour.toLowerCase()
  return new URLSearchParams({
    min: number(lut.min),
    max: number(lut.max),
    gamma: number(lut.gamma),
    cmap: colour === 'ffffff' ? 'grey' : colour,
  })
}

/** The DZI descriptor of `plane`, shown through `lut`. */
export function descriptorUrl(image: ImageRef, { c, z, t }: Plane, lut: ChannelLut): string {
  const url = imageUrl(image, `${c}/${z}/${t}.dzi`)
  url.search = lutQuery(lut).toString()
  return url.href
}

/** The image's metadata. */
export function infoUrl(image: ImageRef): URL {
  return imageUrl(image, 'info')
}

/** The true value at (`x`, `y`) of Zarr level `level`'s `plane`. */
export function valueUrl(
  image: ImageRef,
  { c, z, t }: Plane,
  level: number,
  [x, y]: [number, number],
): URL {
  const url = imageUrl(image, 'value')
  url.search = new URLSearchParams({
    c: String(c),
    z: String(z),
    t: String(t),
    level: String(level),
    x: String(x),
    y: String(y),
  }).toString()
  return url
}

/** A dataset's thumbnail, and its manifest. */
export const thumbnailUrl = (data: URL, dataset: string) =>
  new URL(`${segment(dataset)}/thumb.jpg`, data)
export const manifestUrl = (data: URL, dataset: string) =>
  new URL(`${segment(dataset)}/manifest`, data)
