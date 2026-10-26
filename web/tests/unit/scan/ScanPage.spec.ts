// REQ-WF-001, REQ-WF-003: the Scan page against a fake server: a region drawn on the stage map,
// its estimate shown, and Start posting the tile scan; and Start blocked without a calibration.
import { flushPromises, mount, type VueWrapper } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import { afterEach, beforeEach, describe, expect, it } from 'vitest'

import { WotClient } from '@/api/wot/client'
import ScanPage from '@/scan/ScanPage.vue'
import { useConnectionStore } from '@/stores/connection'

import { fakeServer, json, type Received } from '../api/fakeServer'
import { BASE, describeThing } from '../api/things'
import { inShell } from '../shell'

const things = {
  stage: describeThing('stage', { position: false, limits: false }),
  calibration: describeThing('calibration', { status: false }),
  imaging: describeThing('imaging', { channel: false, channels: false }),
  workflow: describeThing(
    'workflow',
    { state: false, progress: false, positions: false, field: false },
    ['estimate_tile_scan', 'tile_scan'],
  ),
}
const calibrated = (yes: boolean) => [
  {
    objective: 1,
    name: '10x/0.30',
    calibration: yes ? { created: new Date().toISOString() } : null,
    shading: [],
  },
]
const FRAME = [
  [-50, -40],
  [50, -40],
  [50, 40],
  [-50, 40],
]
const ESTIMATE = {
  tiles: [0, 1, 2, 3].map((i) => ({ column: i % 2, row: i >> 1, x: -100 + i, y: -100 })),
  frame: FRAME,
  frames: 4,
  seconds: 754,
  bytes: 3.2e9,
  free_bytes: 5e11,
  fits: true,
  warnings: ['under 5 % overlap leaves stitching little to register'],
}

let wrapper: VueWrapper | undefined
let posts: { path: string; body: Record<string, unknown> }[]

beforeEach(() => {
  setActivePinia(createPinia())
  posts = []
})

afterEach(() => {
  wrapper?.unmount()
  useConnectionStore().stop()
})

async function showPage(calibration: boolean) {
  const post = (request: Received) => {
    const path = request.url.pathname.replace('/api/v1/', '')
    posts.push({ path, body: JSON.parse(request.body!) })
    const output = path.endsWith('estimate_tile_scan') ? ESTIMATE : null
    const record = { id: '1', status: 'completed', href: `${BASE}api/v1/action_invocations/1` }
    return json({ ...record, output }, 201)
  }
  const server = fakeServer({
    'GET /api/v1/thing_descriptions/': things,
    'GET /api/siq/system/health': { status: 'ok', version: '0.1.0', uptime_s: 1, camera: 'sim' },
    'GET /api/v1/stage/position': { x: -50_000, y: -37_500, z: 0 },
    'GET /api/v1/stage/limits': {
      min: { x: -100_000, y: -75_000, z: -20_000 },
      max: { x: 0, y: 0, z: 0 },
    },
    'GET /api/v1/calibration/status': calibrated(calibration),
    'GET /api/v1/imaging/channel': 1,
    'GET /api/v1/imaging/channels': [{ id: 1, name: 'Brightfield' }],
    'GET /api/v1/workflow/state': 'idle',
    'GET /api/v1/workflow/progress': null,
    'GET /api/v1/workflow/positions': [{ name: 'Section A', x: -40_000, y: -30_000 }],
    'GET /api/v1/workflow/field': FRAME,
    'POST /api/v1/workflow/estimate_tile_scan': post,
    'POST /api/v1/workflow/tile_scan': post,
  })
  const client = new WotClient({ baseUrl: `${BASE}api/v1/`, fetch: server.fetch, webSocket: null })
  await useConnectionStore().start(client.baseUrl, client)
  wrapper = mount(inShell(ScanPage), { attachTo: document.body })
  await flushPromises()
  return wrapper
}

/** Drags a rectangle on the map, from one point to another, in map pixels. */
async function drag(page: VueWrapper, [x0, y0]: number[], [x1, y1]: number[]) {
  const map = page.get('svg.stage-map__svg')
  await map.trigger('pointerdown', { clientX: x0, clientY: y0, button: 0 })
  await map.trigger('pointermove', { clientX: x1, clientY: y1 })
  await map.trigger('pointerup', { clientX: x1, clientY: y1 })
  await new Promise((resolve) => setTimeout(resolve, 350))
  await flushPromises()
}

const start = (page: VueWrapper) => page.findAll('button').find((b) => b.text() === 'Start')!

describe('the Scan page', () => {
  it('estimates the region drawn, shows its estimate, and starts it', async () => {
    const page = await showPage(true)
    // The travel, the saved position and the field of view are drawn.
    expect(page.findAll('.stage-map__position')).toHaveLength(1)
    expect(page.find('polygon.stage-map__field').exists()).toBe(true)
    expect(start(page).attributes('disabled')).toBeDefined()
    await drag(page, [300, 200], [500, 400])
    const asked = posts.find((post) => post.path === 'workflow/estimate_tile_scan')!.body
    expect(asked.region).toMatchObject({ shape: 'rect' })
    const region = asked.region as { x0: number; x1: number; y0: number; y1: number }
    expect(region.x0).toBeLessThan(region.x1)
    expect(region.y0).toBeLessThan(region.y1)
    // Its estimate, its tiles on the map, and what may go wrong.
    const values = page.findAll('.estimate__facts dd').map((dd) => dd.text())
    const facts = page.findAll('.estimate__facts dt').map((dt, i) => [dt.text(), values[i]])
    expect(Object.fromEntries(facts)).toMatchObject({
      Tiles: '4',
      Frames: '4',
      Time: 'about 12 min 34 s',
      Data: '3.2 GB',
      Disk: '500 GB free',
    })
    expect(page.get('.estimate').text()).toContain('under 5 % overlap')
    expect(page.findAll('.stage-map__tile')).toHaveLength(4)
    await start(page).trigger('click')
    await flushPromises()
    const started = posts.find((post) => post.path === 'workflow/tile_scan')!.body
    expect(started).toEqual(asked)
  })

  it("says why a scan can't start without the stage–camera calibration", async () => {
    const page = await showPage(false)
    await drag(page, [300, 200], [500, 400])
    expect(page.get('.estimate').text()).toContain('has no stage–camera calibration')
    expect(start(page).attributes('disabled')).toBeDefined()
    expect(posts.filter((post) => post.path === 'workflow/estimate_tile_scan')).toHaveLength(1)
  })
})
