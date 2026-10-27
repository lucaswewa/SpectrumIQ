// REQ-STI-001, REQ-WF-001: the Mosaic page against a fake server (ADR-0060): what was painted shown
// whole, the camera's field placed where the stage is, the viewport written as the view zooms, no
// more than once an interval and the last one always, a double-click moving the stage there,
// Clear only once confirmed, and Save; and the Scan page's stage map over the live mosaic.
import { flushPromises, mount, type VueWrapper } from '@vue/test-utils'
import { h, type Component } from 'vue'
import { createPinia, setActivePinia } from 'pinia'
import { afterEach, beforeEach, describe, expect, it } from 'vitest'

import { WotClient } from '@/api/wot/client'
import MosaicPage from '@/mosaic/MosaicPage.vue'
import StageMap from '@/scan/StageMap.vue'
import { useConnectionStore } from '@/stores/connection'
import { bindToaster } from '@/stores/thing'
import type { Toaster } from '@/ui/overlays'

import { fakeServer, json, type Received } from '../api/fakeServer'
import { BASE, describeThing } from '../api/things'
import { inShell } from '../shell'

const things = {
  stage: describeThing('stage', { position: false, limits: false }, ['move_to']),
  workflow: describeThing('workflow', { field: false }),
  mosaic_live: describeThing(
    'mosaic_live',
    { enabled: false, viewport: false, bounds: false, px_per_um: false, stats: false },
    ['clear', 'save_snapshot'],
  ),
}
const PX_PER_UM = 1.33
const SNAPSHOT = {
  dataset: { id: '9', kind: 'mosaic', name: 'live mosaic', created: '', user: 'me', frames: 0 },
  image: 'mosaic',
  pyramid: 3,
}

let wrapper: VueWrapper | undefined
let writes: { path: string; body: unknown }[]
let toasts: string[]

beforeEach(() => {
  setActivePinia(createPinia())
  writes = []
  toasts = []
  const toast = (message: string) => void toasts.push(message)
  bindToaster({ success: toast, info: toast, error: toast } as Toaster)
})

afterEach(() => {
  wrapper?.unmount()
  useConnectionStore().stop()
  bindToaster(undefined)
})

async function showPage(page: Component = MosaicPage) {
  const record = (request: Received) => {
    const path = request.url.pathname.replace('/api/v1/', '')
    writes.push({ path, body: JSON.parse(request.body ?? 'null') })
    if (request.method === 'PUT') return new Response(null, { status: 204 })
    const output = path.endsWith('save_snapshot') ? SNAPSHOT : null
    return json(
      { id: '1', status: 'completed', href: `${BASE}api/v1/action_invocations/1`, output },
      201,
    )
  }
  const server = fakeServer({
    'GET /api/v1/thing_descriptions/': things,
    'GET /api/siq/system/health': { status: 'ok', version: '0.1.0', uptime_s: 1, camera: 'sim' },
    'GET /api/v1/stage/position': { x: 400, y: 300, z: 1500 },
    'GET /api/v1/stage/limits': { min: { x: 0, y: 0, z: 0 }, max: { x: 50_000, y: 50_000, z: 0 } },
    'GET /api/v1/workflow/field': [
      [-50, -40],
      [50, -40],
      [50, 40],
      [-50, 40],
    ],
    'GET /api/v1/mosaic_live/enabled': true,
    'GET /api/v1/mosaic_live/viewport': { centre: [0, 0], zoom: 0.25, width: 1280, height: 720 },
    'GET /api/v1/mosaic_live/bounds': { x0: 0, y0: 0, x1: 800, y1: 600 },
    'GET /api/v1/mosaic_live/px_per_um': PX_PER_UM,
    'GET /api/v1/mosaic_live/stats': {
      painted: 12,
      passed_over: 3,
      last_paint_ms: 1.8,
      views: 40,
      pages_on_gpu: 2,
      pages_in_ram: 0,
      vram_bytes: 134_217_728,
      vram_budget_bytes: 2e9,
    },
    'PUT /api/v1/mosaic_live/viewport': record,
    'POST /api/v1/stage/move_to': record,
    'POST /api/v1/mosaic_live/clear': record,
    'POST /api/v1/mosaic_live/save_snapshot': record,
  })
  const client = new WotClient({ baseUrl: `${BASE}api/v1/`, fetch: server.fetch, webSocket: null })
  await useConnectionStore().start(client.baseUrl, client)
  wrapper = mount(inShell(page), { attachTo: document.body })
  await flushPromises()
  return wrapper
}

const button = (text: string, within: ParentNode = document) =>
  [...within.querySelectorAll('button')].find((each) => each.textContent?.trim() === text)
const viewports = () =>
  writes.filter((write) => write.path === 'mosaic_live/viewport').map((write) => write.body)
const settle = async (ms: number) => {
  await new Promise((resolve) => setTimeout(resolve, ms))
  await flushPromises()
}

describe('the Mosaic page', () => {
  it('shows what was painted, and the camera field where the stage is', async () => {
    const page = await showPage()
    // The painted 800 × 600 µm whole in the 800 × 600 view, 5 % to spare each side.
    const scale = 1 / 1.1
    const points = page
      .get('polygon.mosaic-page__field')
      .attributes('points')!
      .split(' ')
      .map((pair) => pair.split(',').map(Number))
    const expected = [
      [-50, -40],
      [50, -40],
      [50, 40],
      [-50, 40],
    ].map(([dx, dy]) => [400 + dx! * scale, 300 + dy! * scale])
    points.forEach(([x, y], i) => {
      expect(x).toBeCloseTo(expected[i]![0]!, 6)
      expect(y).toBeCloseTo(expected[i]![1]!, 6)
    })
    // The viewport written for it, and the minimap and the status shown.
    await settle(150)
    expect(viewports().at(-1)).toEqual({
      centre: [400, 300],
      zoom: scale / PX_PER_UM,
      width: 800,
      height: 600,
    })
    expect(page.find('svg.mosaic-page__minimap').exists()).toBe(true)
    expect(page.text()).toContain('12 frames painted; 2 pages on the GPU, 134 of 2000 MB.')
  })

  it('writes the viewport as the view zooms, at most once an interval, the last always', async () => {
    const page = await showPage()
    await settle(150)
    const before = viewports().length
    const map = page.get('svg.mosaic-page__svg')
    for (let i = 0; i < 8; i++) {
      await map.trigger('wheel', { deltaY: -100, clientX: 400, clientY: 300 })
      await flushPromises()
    }
    expect(viewports().length - before).toBe(1)
    await settle(150)
    expect(viewports().length - before).toBe(2)
    // Eight notches in about the centre: the zoom e^(8 × 0.2) times the first's.
    const last = viewports().at(-1) as { zoom: number; centre: number[] }
    expect(last.zoom).toBeCloseTo(Math.exp(1.6) / 1.1 / PX_PER_UM, 9)
    expect(last.centre[0]).toBeCloseTo(400, 9)
  })

  it('moves the stage to a point double-clicked', async () => {
    const page = await showPage()
    await page.get('svg.mosaic-page__svg').trigger('dblclick', { clientX: 510, clientY: 245 })
    await flushPromises()
    const moved = writes.find((write) => write.path === 'stage/move_to')!.body as Record<
      string,
      number
    >
    expect(moved.x).toBeCloseTo(400 + 110 * 1.1, 9)
    expect(moved.y).toBeCloseTo(300 - 55 * 1.1, 9)
    expect(moved.z).toBe(1500)
  })

  it('clears only once confirmed, and saves', async () => {
    await showPage()
    const asked = () => writes.filter((write) => write.path.startsWith('mosaic_live/c'))
    button('Clear')!.click()
    await flushPromises()
    expect(document.body.textContent).toContain('Clear the mosaic?')
    button('Cancel')!.click()
    await flushPromises()
    expect(asked()).toEqual([])
    button('Clear')!.click()
    await flushPromises()
    button('Clear', document.querySelector('[role="alertdialog"]')!)!.click()
    await flushPromises()
    expect(asked().map((write) => write.path)).toEqual(['mosaic_live/clear'])
    button('Save')!.click()
    await flushPromises()
    expect(writes.at(-1)!.path).toBe('mosaic_live/save_snapshot')
    expect(toasts).toEqual(['The mosaic was saved as the dataset "live mosaic".'])
  })
})

describe('the stage map', () => {
  it('is drawn over the live mosaic once something was painted, its view the viewport', async () => {
    const map = await showPage({ render: () => h(StageMap, { tool: 'pan' }) })
    expect(map.find('.stage-map--mosaic').exists()).toBe(true)
    expect(map.find('.mosaic-backdrop').exists()).toBe(true)
    await settle(150)
    // The whole travel, 50 mm a side, in the 800 × 600 map, 5 % to spare each side.
    const scale = 600 / 50_000 / 1.1
    const last = viewports().at(-1) as { zoom: number; centre: number[] }
    expect(last.centre).toEqual([25_000, 25_000])
    expect(last.zoom).toBeCloseTo(scale / PX_PER_UM, 12)
  })
})
