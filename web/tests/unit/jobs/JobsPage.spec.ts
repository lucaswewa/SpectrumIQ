// REQ-STI-002, REQ-WF-001: the Jobs page against a fake server (ADR-0043, ADR-0064): the jobs
// running with their progress and those waiting, Cancel on each, and whether a finished tile
// scan is stitched by itself; and the stitch options dialog starting a dataset's stitch.
import { flushPromises, mount, type VueWrapper } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import { afterEach, beforeEach, describe, expect, it } from 'vitest'

import { WotClient } from '@/api/wot/client'
import StitchDialog from '@/datasets/StitchDialog.vue'
import JobsPage from '@/jobs/JobsPage.vue'
import { useConnectionStore } from '@/stores/connection'
import type { DatasetEntry } from '@/stores/datasets'
import { h, type Component } from 'vue'

import { fakeServer, json, type Received } from '../api/fakeServer'
import { BASE, describeThing } from '../api/things'
import { inShell } from '../shell'

const things = {
  jobs: describeThing('jobs', { queue: false, running: false, auto_stitch: false }, [
    'cancel',
    'stitch',
  ]),
}
const job = (id: number, kind: string, state: string, progress: number) => ({
  id,
  kind,
  priority: 'background',
  state,
  progress,
  estimate: { ram: 1e9, vram: 0, gpu: false, threads: 8 },
  exceeded: false,
  dataset: `C:/data/datasets/20261027-120000_scan_${id}`,
  queued: new Date().toISOString(),
  started: state === 'running' ? new Date().toISOString() : null,
  message: state === 'running' ? 'registering 12 pairs of 9 tiles' : null,
})

let wrapper: VueWrapper | undefined
let sent: { method: string; path: string; body: unknown }[]

beforeEach(() => {
  setActivePinia(createPinia())
  sent = []
})

afterEach(() => {
  wrapper?.unmount()
  useConnectionStore().stop()
})

async function show(component: Component) {
  const record = (request: Received) => {
    const path = request.url.pathname.replace('/api/v1/', '')
    sent.push({ method: request.method, path, body: JSON.parse(request.body ?? 'null') })
    if (request.method === 'PUT') return new Response(null, { status: 204 })
    const output = path === 'jobs/stitch' ? 42 : true
    return json(
      { id: '1', status: 'completed', href: `${BASE}api/v1/action_invocations/1`, output },
      201,
    )
  }
  const server = fakeServer({
    'GET /api/v1/thing_descriptions/': things,
    'GET /api/siq/system/health': { status: 'ok', version: '0.1.0', uptime_s: 1, camera: 'sim' },
    'GET /api/v1/jobs/running': [job(7, 'stitch', 'running', 0.4)],
    'GET /api/v1/jobs/queue': [job(8, 'pyramid', 'queued', 0)],
    'GET /api/v1/jobs/auto_stitch': true,
    'PUT /api/v1/jobs/auto_stitch': record,
    'POST /api/v1/jobs/cancel': record,
    'POST /api/v1/jobs/stitch': record,
  })
  const client = new WotClient({ baseUrl: `${BASE}api/v1/`, fetch: server.fetch, webSocket: null })
  await useConnectionStore().start(client.baseUrl, client)
  wrapper = mount(inShell(component), { attachTo: document.body })
  await flushPromises()
  return wrapper
}

const button = (text: string) =>
  [...document.querySelectorAll('button')].find((each) => each.textContent?.trim() === text)

describe('the Jobs page', () => {
  it('shows the jobs running and waiting, and cancels one', async () => {
    const page = await show(JobsPage)
    const [running, waiting] = page.findAll('section[aria-label]')
    expect(running!.text()).toContain('Stitch 7, 20261027-120000_scan_7')
    expect(running!.get('[role="progressbar"]').attributes('aria-valuenow')).toBe('40')
    expect(running!.text()).toContain('registering 12 pairs of 9 tiles')
    expect(waiting!.text()).toContain('Pyramid 8')
    expect(waiting!.find('[role="progressbar"]').exists()).toBe(false)
    await waiting!.get('button').trigger('click')
    await flushPromises()
    expect(sent).toEqual([{ method: 'POST', path: 'jobs/cancel', body: { id: 8 } }])
  })

  it('turns stitching by itself off', async () => {
    const page = await show(JobsPage)
    const toggle = page.get('input[role="switch"]')
    expect((toggle.element as HTMLInputElement).checked).toBe(true)
    await toggle.setValue(false)
    await flushPromises()
    expect(sent).toContainEqual({ method: 'PUT', path: 'jobs/auto_stitch', body: false })
  })
})

describe('the stitch options dialog', () => {
  it("starts the dataset's stitch with the options chosen", async () => {
    const entry = { id: 'd1', name: 'scan', kind: 'tile_scan', frames: 9 } as DatasetEntry
    await show({ render: () => h(StitchDialog, { entry, open: true }) })
    expect(document.body.textContent).toContain('Stitch scan')
    const integer = [...document.querySelectorAll('input[type="checkbox"]')].at(
      -1,
    ) as HTMLInputElement
    integer.click()
    await flushPromises()
    button('Stitch')!.click()
    await flushPromises()
    const asked = sent.find((each) => each.path === 'jobs/stitch')!
    expect(asked.body).toEqual({
      dataset: 'd1',
      image: 'stitched',
      options: {
        pairs: { stage_sigma_um: 5 },
        solve: { stage_sigma_um: 5 },
        composite: { feather_px: 128, integer: true },
      },
    })
    expect(document.body.textContent).toContain('scan is being stitched into stitched: job 42.')
  })
})
