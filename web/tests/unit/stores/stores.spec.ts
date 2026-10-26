// REQ-NET-002, REQ-WF-005: the stores, against a fake server and fake observation sockets (ADR-0037).
import { flushPromises } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { watch } from 'vue'

import { WotClient } from '@/api/wot/client'
import { useCameraStore } from '@/stores/camera'
import { useConnectionStore } from '@/stores/connection'
import { useStageStore } from '@/stores/stage'
import { bindToaster } from '@/stores/thing'
import { useWorkflowStore } from '@/stores/workflow'
import type { Toaster } from '@/ui/overlays'

import { fakeServer, json, type Received } from '../api/fakeServer'
import { fakeSockets } from '../api/fakeSocket'
import { BASE, describeThing } from '../api/things'

const stage = describeThing(
  'stage',
  { position: true, state: true, homed: true, speed: true, limits: false },
  ['move_to', 'halt'],
  ['arrived'],
)
const camera = describeThing('camera', { stats: false, exposure_ms: false }, ['start_live'])
const HEALTH = { status: 'ok', version: '0.1.0', uptime_s: 1, camera: 'simulated camera' }

/** A finished invocation, as teta-wot answers a POST that completed or failed at once. */
const finished = (status: 'completed' | 'error', body: Record<string, unknown>) => ({
  id: '1',
  status,
  action: '/api/v1/stage/move_to',
  href: `${BASE}api/v1/action_invocations/1`,
  timeRequested: '2026-10-24T22:00:00Z',
  timeStarted: null,
  timeCompleted: null,
  ...body,
})

async function connect(routes: Record<string, unknown>) {
  const server = fakeServer({
    'GET /api/v1/thing_descriptions/': { stage, camera },
    'GET /api/siq/system/health': HEALTH,
    ...(routes as Record<string, never>),
  })
  const sockets = fakeSockets()
  const client = new WotClient({
    baseUrl: `${BASE}api/v1/`,
    fetch: server.fetch,
    webSocket: sockets.open,
  })
  const connection = useConnectionStore()
  await connection.start(client.baseUrl, client)
  return { connection, server, sockets }
}

const reads = (server: { received: Received[] }, path: string) =>
  server.received.filter((request) => request.method === 'GET' && request.url.pathname === path)
    .length

describe('the stores', () => {
  beforeEach(() => setActivePinia(createPinia()))
  afterEach(() => {
    useConnectionStore().stop()
    bindToaster(undefined)
    vi.useRealTimers()
  })

  it('connect to the server, with its Things and its health', async () => {
    const { connection } = await connect({})
    expect(connection.state).toBe('connected')
    expect(connection.health?.camera).toBe('simulated camera')
    expect([connection.isAvailable('stage'), connection.isAvailable('mosaic_live')]).toEqual([
      true,
      false,
    ])
  })

  it('lose the connection when the server stops answering, and get it back', async () => {
    vi.useFakeTimers()
    let up = true
    const { connection } = await connect({
      'GET /api/siq/system/health': () => (up ? json(HEALTH) : json({ detail: 'down' }, 503)),
    })
    up = false
    await vi.advanceTimersByTimeAsync(5000)
    expect(connection.state).toBe('lost')
    up = true
    await vi.advanceTimersByTimeAsync(1000)
    expect(connection.state).toBe('connected')
  })

  it('read each property, then follow the observable ones over the Thing’s socket', async () => {
    const { server, sockets } = await connect({
      'GET /api/v1/stage/position': { x: 0, y: 0, z: 0 },
      'GET /api/v1/stage/state': 'idle',
      'GET /api/v1/stage/limits': { min: { x: -100, y: -75, z: -20 }, max: { x: 0, y: 0, z: 0 } },
    })
    const store = useStageStore()
    await flushPromises()
    expect(store.position).toEqual({ x: 0, y: 0, z: 0 })
    expect(store.limits?.min.x).toBe(-100)
    expect(sockets.opened).toHaveLength(1)
    const wire = sockets.opened[0]!
    wire.open()
    expect(wire.sent).toEqual([
      {
        messageType: 'addPropertyObservation',
        data: { position: true, state: true, homed: true, speed: true },
      },
      { messageType: 'addEventSubscription', data: { arrived: true } },
    ])
    let changes = 0
    watch(
      () => store.position,
      () => (changes += 1),
    )
    // Twice the same value: the second changes nothing.
    wire.receive({ messageType: 'propertyStatus', data: { position: { x: -200, y: 0, z: 0 } } })
    wire.receive({ messageType: 'propertyStatus', data: { position: { x: -200, y: 0, z: 0 } } })
    await flushPromises()
    expect([store.position, changes]).toEqual([{ x: -200, y: 0, z: 0 }, 1])
    wire.receive({ messageType: 'event', data: { arrived: { data: { x: -200, y: 0, z: 0 } } } })
    expect(store.arrived).toEqual({ x: -200, y: 0, z: 0 })
    expect(reads(server, '/api/v1/stage/position')).toBe(1)
  })

  it('poll what isn’t observable', async () => {
    vi.useFakeTimers()
    const { server } = await connect({ 'GET /api/v1/camera/stats': { mode: 'stopped', fps: 0 } })
    const store = useCameraStore()
    await vi.advanceTimersByTimeAsync(0)
    expect(store.stats?.mode).toBe('stopped')
    await vi.advanceTimersByTimeAsync(3000)
    expect(reads(server, '/api/v1/camera/stats')).toBe(4)
  })

  it('run an action to its end, and show its failure as a toast', async () => {
    const toaster = { success: vi.fn(), info: vi.fn(), error: vi.fn() } satisfies Toaster
    bindToaster(toaster)
    let fail = false
    const { server } = await connect({
      'POST /api/v1/stage/move_to': () =>
        json(
          fail
            ? finished('error', { error: { title: 'GlobalLockBusyError', detail: 'Busy.' } })
            : finished('completed', { output: { state: 'idle' } }),
          201,
        ),
    })
    const store = useStageStore()
    await flushPromises()
    expect(await store.moveTo({ x: -10, y: 0, z: 0 })).toEqual({ state: 'idle' })
    const posted = server.received.find((request) => request.method === 'POST')!
    expect(JSON.parse(posted.body!)).toEqual({ x: -10, y: 0, z: 0 })
    expect(toaster.error).not.toHaveBeenCalled()

    fail = true
    expect(await store.moveTo({ x: -10, y: 0, z: 0 })).toBeUndefined()
    expect(toaster.error).toHaveBeenCalledExactlyOnceWith('Stage: move_to failed: Busy.', {
      details: { title: 'GlobalLockBusyError', detail: 'Busy.' },
    })
  })

  it('write a setting, and read it back', async () => {
    let exposure = 10
    const { server } = await connect({
      'GET /api/v1/camera/exposure_ms': () => json(exposure),
      'PUT /api/v1/camera/exposure_ms': (request: Received) => {
        exposure = JSON.parse(request.body!)
        return new Response(null, { status: 201 })
      },
    })
    const store = useCameraStore()
    await flushPromises()
    expect(store.exposureMs).toBe(10)
    expect(await store.setExposure(25)).toBe(true)
    expect(store.exposureMs).toBe(25)
    expect(server.received.some((request) => request.method === 'PUT')).toBe(true)
  })
})

describe('the workflow store', () => {
  it('toasts each run as it ends, wherever it was started', async () => {
    const toaster = { success: vi.fn(), info: vi.fn(), error: vi.fn() } satisfies Toaster
    bindToaster(toaster)
    const workflow = describeThing('workflow', { state: true }, [], ['finished'])
    const { sockets } = await connect({
      'GET /api/v1/thing_descriptions/': { stage, camera, workflow },
      'GET /api/v1/workflow/state': 'idle',
    })
    const store = useWorkflowStore()
    await flushPromises()
    const wire = sockets.opened.find((socket) => socket.url.includes('/workflow/'))!
    wire.open()
    const end = (outcome: string, error: string | null = null) => {
      const data = { name: 'scan', dataset: '1', outcome, error, steps: [3, 3], frames: 3 }
      wire.receive({ messageType: 'event', data: { finished: { data } } })
    }
    end('finished')
    end('cancelled')
    end('failed', 'the stage stopped')
    expect(toaster.success).toHaveBeenCalledExactlyOnceWith('scan finished: 3 frames.')
    expect(toaster.info).toHaveBeenCalledExactlyOnceWith('scan was cancelled: 3 frames kept.')
    expect(toaster.error).toHaveBeenCalledExactlyOnceWith('scan failed: the stage stopped.')
    expect(store.finished?.outcome).toBe('failed')
  })
})
