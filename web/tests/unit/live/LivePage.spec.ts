// REQ-PRV-001, REQ-PRV-002, REQ-PRV-003: the Live page, against a fake server's streams (ADR-0038).
import { flushPromises, mount, type VueWrapper } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { WotClient } from '@/api/wot/client'
import LivePage from '@/live/LivePage.vue'
import { useConnectionStore } from '@/stores/connection'

import { fakeServer, mjpegStream, type Received } from '../api/fakeServer'
import { BASE, describeThing } from '../api/things'

const camera = {
  ...describeThing('camera', {
    exposure_ms: false,
    region: false,
    binning: false,
    pixel_format: false,
    frame_rate: false,
    sensor: false,
    stats: false,
  }),
  links: ['overview', 'roi'].map((name) => ({
    href: `/api/v1/camera/${name}`,
    type: 'multipart/x-mixed-replace',
  })),
}
const display = describeThing('display', {
  lut: false,
  roi_region: false,
  histogram: false,
  focus_measure: false,
})

let wrapper: VueWrapper | undefined
let puts: { path: string; body: unknown }[]

class Observer {
  constructor(private readonly callback: IntersectionObserverCallback) {}
  observe() {
    queueMicrotask(() =>
      this.callback([{ isIntersecting: true } as IntersectionObserverEntry], {} as never),
    )
  }
  disconnect() {}
}

beforeEach(() => {
  vi.useFakeTimers()
  window.localStorage.clear()
  setActivePinia(createPinia())
  vi.stubGlobal('IntersectionObserver', Observer)
  vi.stubGlobal('ResizeObserver', undefined)
  // The overview's frames: 1440 × 1080, a 4000 × 3000 frame's shape.
  vi.stubGlobal(
    'createImageBitmap',
    vi.fn(async () => ({ width: 1440, height: 1080, close: vi.fn() })),
  )
  puts = []
})

afterEach(() => {
  wrapper?.unmount()
  wrapper = undefined
  useConnectionStore().stop()
  vi.unstubAllGlobals()
  vi.useRealTimers()
})

async function showPage() {
  const put = (request: Received) => {
    puts.push({ path: request.url.pathname, body: JSON.parse(request.body!) })
    return new Response(null, { status: 201 })
  }
  const frames = (request: Received) =>
    mjpegStream(request, [Uint8Array.of(0xff, 0xd8, 0xff, 0xd9)]).response
  const server = fakeServer({
    'GET /api/v1/thing_descriptions/': { camera, display },
    'GET /api/siq/system/health': { status: 'ok', version: '0.1.0', uptime_s: 1, camera: 'sim' },
    'GET /api/v1/camera/region': { offset_x: 0, offset_y: 0, width: 4000, height: 3000 },
    'GET /api/v1/camera/binning': { horizontal: 1, vertical: 1 },
    'GET /api/v1/camera/pixel_format': 'Mono14',
    'GET /api/v1/camera/sensor': { width: 4000, height: 3000, pixel_size: 3.76, pixel_formats: [] },
    'GET /api/v1/camera/stats': {
      mode: 'live',
      fps: 9,
      published: 10,
      lost_no_slot: 0,
      lost_device: 2,
    },
    'GET /api/v1/display/lut': {
      contrast: { kind: 'auto', low_pct: 0.1, high_pct: 99.9 },
      colormap: 'grey',
    },
    'GET /api/v1/display/roi_region': { region: null, zoom: 'native' },
    'GET /api/v1/display/histogram': { frame: 3, bins: [0, 5, 9], window: [1000, 3000], focus: 2 },
    'GET /api/v1/camera/overview': frames,
    'GET /api/v1/camera/roi': frames,
    'PUT /api/v1/display/roi_region': put,
    'PUT /api/v1/display/lut': put,
  })
  const client = new WotClient({ baseUrl: `${BASE}api/v1/`, fetch: server.fetch, webSocket: null })
  await useConnectionStore().start(client.baseUrl, client)
  wrapper = mount(LivePage, { attachTo: document.body })
  await vi.advanceTimersByTimeAsync(100)
  await flushPromises()
  return wrapper
}

describe('the Live page', () => {
  it('shows the overview and the ROI beside it, then in its corner', async () => {
    const page = await showPage()
    const images = () => page.findAll('.live-image').map((image) => image.attributes('data-status'))
    expect(images()).toEqual(['live', 'live'])
    expect(page.find('.live-page__pip').exists()).toBe(false)
    await page.get('button[aria-label="ROI in the corner"]').trigger('click')
    await vi.advanceTimersByTimeAsync(100)
    expect(page.find('.live-page__pip').exists()).toBe(true)
    expect(window.localStorage.getItem('spectrumiq.roi-layout')).toBe('pip')
    // The lost frames show.
    expect(page.get('[data-test="lost"]').text()).toBe('2')
  })

  it('sets the ROI, in sensor pixels, from a rectangle dragged on the overview', async () => {
    const page = await showPage()
    const selector = page.get('.roi-selector')
    // The overview's box: the frames fit it exactly, so each box pixel is 4 frame pixels.
    const box = { left: 0, top: 0, width: 1000, height: 750, right: 1000, bottom: 750 }
    vi.spyOn(selector.element, 'getBoundingClientRect').mockReturnValue(box as DOMRect)
    await selector.trigger('pointerdown', { button: 0, clientX: 600, clientY: 450, pointerId: 1 })
    await selector.trigger('pointermove', { clientX: 500, clientY: 375, pointerId: 1 })
    await selector.trigger('pointerup', { pointerId: 1 })
    await flushPromises()
    expect(puts).toEqual([
      {
        path: '/api/v1/display/roi_region',
        body: { region: { x: 2000, y: 1500, width: 400, height: 300 }, zoom: 'native' },
      },
    ])
    // A click is not a drag.
    await selector.trigger('pointerdown', { button: 0, clientX: 10, clientY: 10, pointerId: 1 })
    await selector.trigger('pointerup', { pointerId: 1 })
    expect(puts).toHaveLength(1)
  })

  it('sets the ROI’s zoom, and moves the window from the histogram', async () => {
    const page = await showPage()
    await page
      .findAll('button')
      .find((button) => button.text() === '2:1')!
      .trigger('click')
    await flushPromises()
    expect(puts.at(-1)).toEqual({
      path: '/api/v1/display/roi_region',
      body: { region: null, zoom: 'half' },
    })
    // Moving the black marker from auto-contrast's window makes the mapping manual.
    await page.get('[aria-label="Black level"]').trigger('keydown', { key: 'ArrowRight' })
    await flushPromises()
    expect(puts.at(-1)).toEqual({
      path: '/api/v1/display/lut',
      body: { contrast: { kind: 'manual', min: 1064, max: 3000, gamma: 1 }, colormap: 'grey' },
    })
  })
})
