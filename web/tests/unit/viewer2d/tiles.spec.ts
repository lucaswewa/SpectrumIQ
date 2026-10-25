// REQ-VIS-001: the data routes' URLs the viewer asks for (ADR-0045), and the scale bar's lengths.
import { describe, expect, it } from 'vitest'

import { scaleBar } from '@/viewer2d/scale'
import {
  dataRoot,
  descriptorUrl,
  infoUrl,
  lutQuery,
  manifestUrl,
  thumbnailUrl,
  valueUrl,
} from '@/viewer2d/tiles'

const data = new URL('http://127.0.0.1:5000/api/siq/data/')
const image = { data, dataset: '01a174d2-5ba5-74d1-b01e-93a6f8c947db', image: 'snap' }
const lut = { min: 100, max: 16383, gamma: 1, colour: '3F6FFF' }

describe('the tile URLs', () => {
  it('give a plane its descriptor, with the mapping in the query for its tiles', () => {
    expect(descriptorUrl(image, { c: 1, z: 2, t: 3 }, lut)).toBe(
      `${data}${image.dataset}/snap/1/2/3.dzi?min=100&max=16383&gamma=1&cmap=3f6fff`,
    )
  })

  it('give each tile the mapping, as OpenSeadragon asks for tiles from the descriptor', () => {
    const descriptor = new URL(descriptorUrl(image, { c: 0, z: 0, t: 0 }, lut))
    // OpenSeadragon's DziTileSource: `<name>_files/<level>/<col>_<row>.<format>`, with the query.
    const tile = new URL(`${descriptor.pathname.replace(/\.dzi$/, '_files')}/12/3_4.jpeg`, data)
    tile.search = descriptor.search
    expect(tile.href).toBe(
      `${data}${image.dataset}/snap/0/0/0_files/12/3_4.jpeg?min=100&max=16383&gamma=1&cmap=3f6fff`,
    )
  })

  it('find the data routes beside the Things', () => {
    expect(dataRoot({ baseUrl: new URL('http://127.0.0.1:5000/api/v1/') }).href).toBe(data.href)
  })

  it('show white as grey, and round a drag to six digits', () => {
    const white = lutQuery({
      min: 0.1234567,
      max: 1000.00004,
      gamma: 0.999999949,
      colour: 'FFFFFF',
    })
    expect(Object.fromEntries(white)).toEqual({
      min: '0.123457',
      max: '1000',
      gamma: '1',
      cmap: 'grey',
    })
  })

  it('name the metadata, the probe, the thumbnail and the manifest', () => {
    const base = `${data}${image.dataset}`
    expect(infoUrl(image).href).toBe(`${base}/snap/info`)
    expect(valueUrl(image, { c: 1, z: 0, t: 0 }, 0, [10, 20]).href).toBe(
      `${base}/snap/value?c=1&z=0&t=0&level=0&x=10&y=20`,
    )
    expect(thumbnailUrl(data, image.dataset).href).toBe(`${base}/thumb.jpg`)
    expect(manifestUrl(data, image.dataset).href).toBe(`${base}/manifest`)
    // Names are escaped.
    expect(infoUrl({ ...image, image: 'a b' }).href).toBe(`${base}/a%20b/info`)
  })
})

describe('the scale bar', () => {
  it('is the longest round length within its width', () => {
    // 0.376 µm a pixel, at 1:1: 120 pixels are 45 µm, so 20 µm.
    expect(scaleBar(0.376)).toEqual({ um: 20, pixels: 20 / 0.376, label: '20 µm' })
    expect(scaleBar(10)?.label).toBe('1 mm')
    expect(scaleBar(25)?.label).toBe('2 mm')
    expect(scaleBar(0.004)?.label).toBe('0.2 µm')
    expect(scaleBar(0)).toBeUndefined()
    expect(scaleBar(Number.NaN)).toBeUndefined()
  })
})
