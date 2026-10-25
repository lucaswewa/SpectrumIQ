// REQ-VIS-001: the deep-zoom view on OpenSeadragon, mocked: a tiled image per layer, composited
// additively; a new URL shown only once its tiles are loaded, and a stale one dropped; visibility
// at once; the pointer's pixel and the zoom told; and the viewer destroyed when it unmounts.
import { flushPromises, mount } from '@vue/test-utils'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import DeepZoom from '@/viewer2d/DeepZoom.vue'

/** OpenSeadragon's viewer, as the view uses it. */
const fake = vi.hoisted(() => {
  class Item {
    loaded?: () => void
    constructor(
      readonly url: string,
      public opacity: number,
    ) {}
    setOpacity(opacity: number) {
      this.opacity = opacity
    }
    whenFullyLoaded(callback: () => void) {
      this.loaded = callback
    }
    getBounds() {
      return { x: 0, y: 0, width: 1, height: 0.75 }
    }
    viewportToImageZoom(zoom: number) {
      return zoom / 4
    }
    viewerElementToImageCoordinates(point: { x: number; y: number }) {
      return { x: point.x * 2, y: point.y * 2 }
    }
    getContentSize() {
      return { x: 1000, y: 750 }
    }
  }
  const state = {
    viewer: undefined as ReturnType<typeof viewer> | undefined,
    added: [] as Item[],
  }
  function viewer() {
    const shown: Item[] = []
    return {
      shown,
      handlers: {} as Record<string, () => void>,
      addTiledImage: vi.fn(
        (options: { tileSource: string; opacity: number; success: (e: unknown) => void }) => {
          const item = new Item(options.tileSource, options.opacity)
          state.added.push(item)
          shown.push(item)
          options.success({ item })
        },
      ),
      world: { removeItem: vi.fn((item: Item) => shown.splice(shown.indexOf(item), 1)) },
      viewport: {
        viewportToViewerElementRectangle: (rect: unknown) => rect,
        getZoom: () => 2,
        zoomBy: vi.fn(),
        applyConstraints: vi.fn(),
        goHome: vi.fn(),
      },
      addHandler: vi.fn(function (this: void, name: string, handler: () => void) {
        state.viewer!.handlers[name] = handler
      }),
      destroy: vi.fn(),
    }
  }
  const openSeadragon = Object.assign(
    vi.fn(() => (state.viewer = viewer())),
    {
      Point: class {
        constructor(
          readonly x: number,
          readonly y: number,
        ) {}
      },
    },
  )
  return { state, openSeadragon }
})
vi.mock('openseadragon', () => ({ default: fake.openSeadragon }))

beforeEach(() => {
  fake.state.added = []
  fake.openSeadragon.mockClear()
})

async function mounted(
  layers = [
    { url: 'dapi.dzi', visible: true },
    { url: 'gfp.dzi', visible: false },
  ],
) {
  const view = mount(DeepZoom, { props: { layers } })
  await flushPromises()
  return { view, viewer: fake.state.viewer! }
}

describe('the deep-zoom view', () => {
  it('composites a tiled image per layer, shown once loaded', async () => {
    const { view, viewer } = await mounted()
    expect(fake.openSeadragon).toHaveBeenCalledOnce()
    expect(viewer.addTiledImage.mock.calls.map(([options]) => options)).toEqual([
      expect.objectContaining({
        tileSource: 'dapi.dzi',
        compositeOperation: 'lighter',
        opacity: 0,
      }),
      expect.objectContaining({ tileSource: 'gfp.dzi', compositeOperation: 'lighter', opacity: 0 }),
    ])
    expect(view.attributes('data-status')).toBe('loading')
    fake.state.added.forEach((item) => item.loaded!())
    await flushPromises()
    expect(fake.state.added.map((item) => item.opacity)).toEqual([1, 0])
    expect(view.attributes('data-status')).toBe('shown')
    // Zoom 2 in the viewport is 0.5 screen pixels per image pixel, here.
    expect(view.emitted('zoom')?.at(-1)).toEqual([0.5])
  })

  it('swaps a layer for its new URL only once that is loaded, and drops a stale one', async () => {
    const { view, viewer } = await mounted()
    fake.state.added.forEach((item) => item.loaded!())
    const [dapi] = fake.state.added
    await view.setProps({
      layers: [
        { url: 'dapi2.dzi', visible: true },
        { url: 'gfp.dzi', visible: false },
      ],
    })
    await view.setProps({
      layers: [
        { url: 'dapi3.dzi', visible: true },
        { url: 'gfp.dzi', visible: false },
      ],
    })
    const [, , second, third] = fake.state.added
    expect([second!.url, third!.url]).toEqual(['dapi2.dzi', 'dapi3.dzi'])
    // The old one stays until the newest is loaded; the one in between is dropped.
    second!.loaded!()
    expect(viewer.world.removeItem).toHaveBeenLastCalledWith(second)
    expect(viewer.shown).toContain(dapi)
    third!.loaded!()
    expect(viewer.world.removeItem).toHaveBeenLastCalledWith(dapi)
    expect(third!.opacity).toBe(1)
  })

  it('shows and hides a layer at once', async () => {
    const { view } = await mounted()
    fake.state.added.forEach((item) => item.loaded!())
    await view.setProps({
      layers: [
        { url: 'dapi.dzi', visible: false },
        { url: 'gfp.dzi', visible: true },
      ],
    })
    expect(fake.state.added.map((item) => item.opacity)).toEqual([0, 1])
    await view.setProps({ layers: [{ url: 'dapi.dzi', visible: false }] })
    expect(fake.state.viewer!.shown.map((item) => item.url)).toEqual(['dapi.dzi'])
  })

  it("tells the image's pixel under the pointer, and nothing off the image", async () => {
    const { view } = await mounted()
    fake.state.added.forEach((item) => item.loaded!())
    await view.trigger('pointermove', { clientX: 10.4, clientY: 20.7 })
    await view.trigger('pointermove', { clientX: 600, clientY: 20 })
    await view.trigger('pointerleave')
    expect(view.emitted('pointer')).toEqual([[[20, 41]], [undefined], [undefined]])
  })

  it('destroys the viewer when it unmounts, and shows nothing that finishes loading after', async () => {
    const { view, viewer } = await mounted()
    const [dapi] = fake.state.added
    view.unmount()
    expect(viewer.destroy).toHaveBeenCalledOnce()
    expect(() => dapi!.loaded!()).not.toThrow()
    expect(dapi!.opacity).toBe(0)
  })

  it("isn't made at all if it unmounts before OpenSeadragon has loaded", async () => {
    mount(DeepZoom, { props: { layers: [] } }).unmount()
    await flushPromises()
    expect(fake.openSeadragon).not.toHaveBeenCalled()
  })
})
