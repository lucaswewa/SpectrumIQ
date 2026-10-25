import { flushPromises, mount, type VueWrapper } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { Component } from 'vue'

import { WotClient } from '@/api/wot/client'
import type { ThingDescription } from '@/api/wot/td'
import { flashLiveImage } from '@/live/flash'
import LiveImage from '@/live/LiveImage.vue'
import { STREAM_DISABLED_KEY } from '@/live/preferences'
import { useConnectionStore } from '@/stores/connection'

import { fakeServer, jpeg, json, mjpegStream, type Received } from '../api/fakeServer'

const camera: ThingDescription = {
  title: 'SimulatedCamera',
  base: 'http://microscope.test/',
  links: [{ href: '/api/v1/camera/overview', type: 'multipart/x-mixed-replace' }],
}

/**
 * Stands in for IntersectionObserver: tests say whether the images are on screen. As a browser's
 * does, it reports each image in view as soon as it is observed.
 */
let onScreen: (visible: boolean) => void = () => {}
const observers = new Set<FakeObserver>()
class FakeObserver {
  constructor(private readonly callback: IntersectionObserverCallback) {
    observers.add(this)
    onScreen = (visible) => observers.forEach((observer) => observer.report(visible))
  }
  report(visible: boolean) {
    this.callback(
      [{ isIntersecting: visible } as IntersectionObserverEntry],
      this as unknown as IntersectionObserver,
    )
  }
  observe() {
    queueMicrotask(() => this.report(true))
  }
  disconnect() {
    observers.delete(this)
  }
}

let wrapper: VueWrapper | undefined
let requests: Received[]
let send: (n: number) => void
let failures: number

beforeEach(() => {
  vi.useFakeTimers()
  window.localStorage.clear()
  setActivePinia(createPinia())
  vi.stubGlobal('IntersectionObserver', FakeObserver)
  vi.stubGlobal(
    'createImageBitmap',
    vi.fn(async () => ({ width: 820, height: 616, close: vi.fn() })),
  )
  requests = []
  failures = 0
})

afterEach(() => {
  wrapper?.unmount()
  wrapper = undefined
  vi.unstubAllGlobals()
  vi.useRealTimers()
})

/** Connects the store to a camera, which fails `failures` times before streaming. */
function connect() {
  const server = fakeServer({
    'GET /api/v1/camera/overview': (request: Received) => {
      requests.push(request)
      if (requests.length <= failures) return json({ detail: 'The camera is starting.' }, 503)
      const stream = mjpegStream(request)
      send = (n) => stream.push(jpeg(n))
      return stream.response
    },
  })
  const connection = useConnectionStore()
  connection.client = new WotClient({
    baseUrl: 'http://microscope.test/api/v1/',
    fetch: server.fetch,
    webSocket: null,
  })
  connection.descriptions = { camera }
  connection.state = 'connected'
}

async function show(component: Component = LiveImage) {
  wrapper = mount(component, { attachTo: document.body })
  await vi.advanceTimersByTimeAsync(0)
  await flushPromises()
  return wrapper
}

const status = () => wrapper!.find('.live-image').attributes('data-status')
const message = () => wrapper!.find('[role="status"]').text()

describe('LiveImage', () => {
  it('draws the stream’s frames, and says why while there are none', async () => {
    await show()
    expect(status()).toBe('offline')
    expect(message()).toContain('No connection')
    connect()
    await vi.advanceTimersByTimeAsync(0)
    expect(status()).toBe('connecting')
    expect(message()).toContain('Connecting')
    send(1)
    send(2)
    await vi.advanceTimersByTimeAsync(200)
    expect(status()).toBe('live')
    expect(wrapper!.find('[role="status"]').exists()).toBe(false)
    const canvas = wrapper!.find('canvas')
    expect(canvas.attributes('data-frames')).toBe('2')
    expect([canvas.element.width, canvas.element.height]).toEqual([820, 616])
  })

  it('shows an error while the stream fails, then the frames', async () => {
    failures = 1
    connect()
    await show()
    expect(status()).toBe('error')
    expect(message()).toContain('The camera is starting.')
    await vi.advanceTimersByTimeAsync(1000)
    send(1)
    await vi.advanceTimersByTimeAsync(0)
    expect(status()).toBe('live')
  })

  it('pauses its stream while off screen or while the page is hidden', async () => {
    connect()
    await show()
    expect(requests).toHaveLength(1)
    onScreen(false)
    await vi.advanceTimersByTimeAsync(1000)
    expect(requests[0]!.signal?.aborted).toBe(true)
    onScreen(true)
    await vi.advanceTimersByTimeAsync(0)
    expect(requests).toHaveLength(2)

    const visibility = vi.spyOn(document, 'visibilityState', 'get').mockReturnValue('hidden')
    document.dispatchEvent(new Event('visibilitychange'))
    await vi.advanceTimersByTimeAsync(1000)
    expect(requests[1]!.signal?.aborted).toBe(true)
    visibility.mockReturnValue('visible')
    document.dispatchEvent(new Event('visibilitychange'))
    await vi.advanceTimersByTimeAsync(0)
    expect(requests).toHaveLength(3)
  })

  it('says where its image is double-clicked, once it has one', async () => {
    connect()
    await show()
    const canvas = wrapper!.find('canvas')
    const box = { left: 10, top: 20, width: 1000, height: 1000 }
    vi.spyOn(canvas.element, 'getBoundingClientRect').mockReturnValue(box as DOMRect)
    // 820 × 616 frames are drawn 1000 × 751 here: 124 px of letterboxing above.
    const at = (x: number, y: number) => canvas.trigger('dblclick', { clientX: x, clientY: y })
    await at(760, 520)
    send(1)
    await vi.advanceTimersByTimeAsync(0)
    await at(760, 520)
    await at(760, 140)
    expect(wrapper!.emitted('doubleClick')).toEqual([[{ x: 0.25, y: 0 }]])
  })

  it('flashes when a capture is taken', async () => {
    await show()
    expect(wrapper!.find('.live-image__flash').exists()).toBe(false)
    flashLiveImage()
    await flushPromises()
    expect(wrapper!.find('.live-image__flash').exists()).toBe(true)
  })
})

describe('the live image’s preference', () => {
  it('starts turned off if it was, and turns on, remembering it', async () => {
    window.localStorage.setItem(STREAM_DISABLED_KEY, 'true')
    connect()
    await show()
    expect(status()).toBe('disabled')
    expect(requests).toHaveLength(0)
    await wrapper!.get('.live-image button').trigger('click')
    await vi.advanceTimersByTimeAsync(0)
    expect(requests).toHaveLength(1)
    expect(window.localStorage.getItem(STREAM_DISABLED_KEY)).toBe('false')
  })
})

describe('the stream budget', () => {
  it('lets three live images stream, and the fourth waits for a place', async () => {
    connect()
    const images = [0, 1, 2, 3].map(() => mount(LiveImage, { attachTo: document.body }))
    await vi.advanceTimersByTimeAsync(0)
    await flushPromises()
    const statuses = () =>
      images.map((image) => image.find('.live-image').attributes('data-status'))
    expect(statuses()).toEqual(['connecting', 'connecting', 'connecting', 'paused'])
    expect(images[3]!.find('[role="status"]').text()).toContain('Paused')
    images[0]!.unmount()
    await vi.advanceTimersByTimeAsync(0)
    expect(statuses().slice(1)).toEqual(['connecting', 'connecting', 'connecting'])
    images.slice(1).forEach((image) => image.unmount())
  })
})
