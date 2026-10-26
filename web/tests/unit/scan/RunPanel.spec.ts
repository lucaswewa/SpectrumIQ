// REQ-WF-005: the run panel through a run's states, wherever it was started: its progress, Pause
// and Resume, Retry and Skip while a failed step waits, Cancel only once confirmed, how it ended,
// and a toast (ADR-0057).
import { flushPromises, mount, type VueWrapper } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import { afterEach, beforeEach, describe, expect, it } from 'vitest'

import { WotClient } from '@/api/wot/client'
import RunPanel from '@/scan/RunPanel.vue'
import { useConnectionStore } from '@/stores/connection'
import { useWorkflowStore } from '@/stores/workflow'

import { fakeServer, json, type Received } from '../api/fakeServer'
import { BASE, describeThing } from '../api/things'
import { inShell } from '../shell'

const ACTIONS = ['pause', 'resume', 'retry', 'skip', 'run', 'tile_scan']
const things = {
  workflow: describeThing('workflow', { state: false, progress: false, problem: false }, [
    ...ACTIONS,
    'z_stack',
    'multi_position',
    'time_lapse',
  ]),
}
const RUNNING = { id: '7', status: 'running', href: `${BASE}api/v1/action_invocations/7` }

let wrapper: VueWrapper | undefined
let requests: string[]

beforeEach(() => {
  setActivePinia(createPinia())
  requests = []
})

afterEach(() => {
  wrapper?.unmount()
  useConnectionStore().stop()
  document.body.innerHTML = ''
})

async function showPanel() {
  const record = (request: Received) => {
    requests.push(`${request.method} ${request.url.pathname.replace('/api/v1/', '')}`)
    return json({ id: '1', status: 'completed', href: `${BASE}api/v1/action_invocations/1` }, 201)
  }
  const server = fakeServer({
    'GET /api/v1/thing_descriptions/': things,
    'GET /api/siq/system/health': { status: 'ok', version: '0.1.0', uptime_s: 1, camera: 'sim' },
    'GET /api/v1/workflow/state': 'idle',
    'GET /api/v1/workflow/progress': null,
    'GET /api/v1/workflow/problem': null,
    // A scan a script started: the one under way.
    'GET /api/v1/workflow/tile_scan': [RUNNING],
    'GET /api/v1/action_invocations/7': RUNNING,
    'DELETE /api/v1/action_invocations/7': (request: Received) => {
      requests.push(`DELETE ${request.url.pathname.replace('/api/v1/', '')}`)
      return new Response(null, { status: 204 })
    },
    ...Object.fromEntries(ACTIONS.map((action) => [`POST /api/v1/workflow/${action}`, record])),
    ...Object.fromEntries(
      ['run', 'z_stack', 'multi_position', 'time_lapse'].map((a) => [
        `GET /api/v1/workflow/${a}`,
        [],
      ]),
    ),
  })
  const client = new WotClient({ baseUrl: `${BASE}api/v1/`, fetch: server.fetch, webSocket: null })
  await useConnectionStore().start(client.baseUrl, client)
  wrapper = mount(inShell(RunPanel), { attachTo: document.body })
  await flushPromises()
  return wrapper
}

const button = (text: string) =>
  [...document.querySelectorAll('button')].find((each) => each.textContent?.trim() === text)

describe('the run panel', () => {
  it('follows a run from running to paused, asking and its end', async () => {
    const panel = await showPanel()
    const workflow = useWorkflowStore()
    expect(panel.find('.run-panel').exists()).toBe(false)

    workflow.state = 'running'
    workflow.progress = { done: 3, total: 12, eta_s: 95, step: 'snap t0 c1 z0 p2' }
    await flushPromises()
    expect(panel.text()).toContain('3 of 12 steps, about 1 min 35 s left')
    expect(panel.text()).toContain('snap t0 c1 z0 p2')
    button('Pause')!.click()
    await flushPromises()

    workflow.state = 'paused'
    await flushPromises()
    expect(button('Pause')).toBeUndefined()
    button('Resume')!.click()
    await flushPromises()

    workflow.state = 'asking'
    workflow.problem = { step: 'move to (1000, 0) µm', attempt: 2, error: 'outside the travel' }
    await flushPromises()
    expect(panel.get('[role="alert"]').text()).toContain('failed, at attempt 2: outside the travel')
    button('Retry')!.click()
    await flushPromises()
    button('Skip')!.click()
    await flushPromises()
    expect(requests).toEqual([
      'POST workflow/pause',
      'POST workflow/resume',
      'POST workflow/retry',
      'POST workflow/skip',
    ])

    workflow.state = 'idle'
    workflow.finished = {
      name: 'scan',
      dataset: '1',
      outcome: 'finished',
      error: null,
      steps: [12, 12],
      frames: 4,
    }
    await flushPromises()
    expect(panel.get('[role="status"]').text()).toBe('scan finished: 4 frames.')
    button('Close')!.click()
    await flushPromises()
    expect(panel.find('.run-panel').exists()).toBe(false)
  })

  it('cancels the run under way only once confirmed', async () => {
    await showPanel()
    const workflow = useWorkflowStore()
    workflow.state = 'running'
    await flushPromises()
    button('Cancel')!.click()
    await flushPromises()
    expect(document.body.textContent).toContain('Cancel the run?')
    button('Keep running')!.click()
    await flushPromises()
    expect(requests).toEqual([])
    button('Cancel')!.click()
    await flushPromises()
    button('Cancel the run')!.click()
    await flushPromises()
    expect(requests).toEqual(['DELETE action_invocations/7'])
  })
})
