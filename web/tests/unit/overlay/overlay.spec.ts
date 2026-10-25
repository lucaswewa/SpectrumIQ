import { flushPromises, mount } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import { beforeEach, describe, expect, it } from 'vitest'

import { imageOutlines } from '@/overlay/imageOutlines'
import { useOverlayLayers } from '@/overlay/layers'
import OverlaidImage from '@/overlay/OverlaidImage.vue'
import OverlayLayers from '@/overlay/OverlayLayers.vue'
import type { OverlayLayer } from '@/overlay/types'

/** Two images side by side in a 300 × 100 mosaic, overlapping by 100 pixels. */
const outlines = imageOutlines({
  size: [300, 100],
  tiles: [
    { image: 'img_0_0_5.jpeg', number: 1, x: 0, y: 0, width: 200, height: 100 },
    { image: 'img_9_0_5.jpeg', number: 2, x: 100, y: 0, width: 200, height: 100 },
  ],
})

/** A rect's position and size, as numbers. */
const box = (rect: Element) =>
  ['x', 'y', 'width', 'height'].map((name) => Number(rect.getAttribute(name)))

beforeEach(() => {
  window.localStorage.clear()
  setActivePinia(createPinia())
})

describe('overlay layers', () => {
  it('draws each shape where its layer says, on the image as it is placed', () => {
    const marks: OverlayLayer = {
      id: 'marks',
      label: 'Marks',
      size: [300, 100],
      shapes: [
        {
          kind: 'line',
          points: [
            [0, 0],
            [150, 50],
          ],
          title: 'A line',
        },
        { kind: 'point', x: 300, y: 100 },
        { kind: 'label', x: 150, y: 0, text: 'Middle' },
      ],
    }
    // Drawn at twice the size, 10 pixels in and 20 down.
    const placement = { x: 10, y: 20, width: 600, height: 200 }
    const wrapper = mount(OverlayLayers, { props: { layers: [outlines, marks], placement } })
    const outline = wrapper.findAll('[data-layer="image-outlines"] > g')
    expect(outline).toHaveLength(2)
    // A dashed stroke over a halo, its number in its corner, its file name on hover.
    const second = outline[1]!
    expect(second.findAll('rect').map((rect) => box(rect.element))).toEqual([
      [210, 20, 400, 200],
      [210, 20, 400, 200],
    ])
    expect(second.find('.overlay-layers__stroke').classes()).toContain('overlay-layers__dashed')
    expect(second.find('text').text()).toBe('2')
    expect(second.find('title').text()).toBe('2: img_9_0_5.jpeg')

    const [line, point, label] = wrapper.findAll('[data-layer="marks"] > g')
    expect(line!.find('polyline').attributes('points')).toBe('10,20 310,120')
    expect(line!.find('title').text()).toBe('A line')
    const circle = point!.find('circle')
    expect([circle.attributes('cx'), circle.attributes('cy')]).toEqual(['610', '220'])
    expect([label!.text(), label!.find('text').attributes('x')]).toEqual(['Middle', '310'])
  })

  it('draws nothing until the image is placed', () => {
    const wrapper = mount(OverlayLayers, { props: { layers: [outlines], placement: undefined } })
    expect(wrapper.find('svg').exists()).toBe(false)
  })

  it('places them on an image fitted in its box, once it has loaded', async () => {
    const wrapper = mount(OverlaidImage, {
      props: { src: 'mosaic.jpeg', alt: 'A mosaic', layers: [outlines] },
    })
    expect(wrapper.find('svg').exists()).toBe(false)
    // A 600 × 100 image in a 400 × 400 box: 400 × 66.7, 166.7 down.
    const image = wrapper.get('img').element
    for (const [name, value] of Object.entries({
      naturalWidth: 600,
      naturalHeight: 100,
      clientWidth: 400,
      clientHeight: 400,
      offsetLeft: 0,
      offsetTop: 0,
    })) {
      Object.defineProperty(image, name, { value })
    }
    await wrapper.get('img').trigger('load')
    const [x, y, width, height] = box(wrapper.get('.overlay-layers__stroke').element)
    expect([x, Math.round(y), Math.round(width), Math.round(height)]).toEqual([0, 167, 267, 67])
  })

  it('keeps which layers are shown in this browser, every layer shown at first', async () => {
    const layers = useOverlayLayers()
    expect(layers.visible([outlines]).map((layer) => layer.id)).toEqual(['image-outlines'])
    layers.show('image-outlines', false)
    await flushPromises()
    expect(layers.visible([outlines])).toEqual([])
    expect(JSON.parse(window.localStorage.getItem('spectrumiq.overlay-layers')!)).toEqual({
      'image-outlines': false,
    })
    // Read back by the next page; a choice that isn't true or false is left out.
    window.localStorage.setItem(
      'spectrumiq.overlay-layers',
      '{"image-outlines": false, "scale": "yes"}',
    )
    setActivePinia(createPinia())
    const again = useOverlayLayers()
    expect([again.shown('image-outlines'), again.shown('scale')]).toEqual([false, true])
  })
})
