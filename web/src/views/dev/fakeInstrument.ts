// A fake instrument for the development pages: a `fetch` that answers as a SpectrumIQ server
// does, with a camera and a display Thing, MJPEG streams drawn in the browser, the stage, focus
// and peripherals of `fakeMotion.ts`, which the drawing follows, the calibrations of
// `fakeCalibration.ts`, and the workflows of `fakeWorkflow.ts`. For development only
// (`/#/dev/live`, `/#/dev/control`, `/#/dev/calibration`, `/#/dev/scan`).
import type { Pinia } from 'pinia'
import { createApp, defineComponent, h, type Component } from 'vue'

import { WotClient } from '@/api/wot/client'
import type { ThingDescription } from '@/api/wot/td'
import { provideShortcuts } from '@/app/shortcuts'
import LivePage from '@/live/LivePage.vue'
import { useConnectionStore } from '@/stores/connection'
import OverlayProvider from '@/ui/OverlayProvider.vue'

import { calibrate, status } from './fakeCalibration'
import { act, BEST_FOCUS, follow, motionValues } from './fakeMotion'
import { workflowAct, workflowValues } from './fakeWorkflow'

const BASE = `${location.origin}/`

/** A Thing's description: its properties followed over server-sent events if `observable`. */
function describe(
  thing: string,
  properties: string[],
  { actions = [] as string[], links = [] as string[], observable = false } = {},
): ThingDescription {
  const href = (name: string) => `/api/v1/${thing}/${name}`
  const forms = (name: string) => [
    { href: href(name), op: ['readproperty', 'writeproperty'] },
    ...(observable ? [{ href: href(name), op: ['observeproperty'], subprotocol: 'sse' }] : []),
  ]
  return {
    title: thing,
    base: BASE,
    properties: Object.fromEntries(
      properties.map((name) => [name, { observable, forms: forms(name) }]),
    ),
    actions: Object.fromEntries(
      actions.map((name) => [name, { forms: [{ href: href(name), op: ['invokeaction'] }] }]),
    ),
    links: links.map((name) => ({ href: href(name), type: 'multipart/x-mixed-replace' })),
  }
}

/** A Thing of `fakeMotion.ts`, with the properties it has values for. */
function moving(thing: string, actions: string[]) {
  const properties = Object.keys(motionValues)
    .filter((path) => path.startsWith(`${thing}/`))
    .map((path) => path.slice(thing.length + 1))
  return describe(thing, properties, { actions, observable: true })
}

const THINGS = {
  camera: describe(
    'camera',
    ['exposure_ms', 'region', 'binning', 'pixel_format', 'frame_rate', 'sensor', 'stats'],
    { actions: ['start_live', 'stop_live', 'snap'], links: ['overview', 'roi'] },
  ),
  display: describe('display', ['lut', 'roi_region', 'histogram', 'focus_measure'], {
    actions: ['auto_contrast_once'],
  }),
  stage: moving('stage', ['move_to', 'move_by', 'jog', 'home', 'halt']),
  focus: moving('focus', ['move_to', 'move_by', 'halt']),
  imaging: moving('imaging', ['select_channel']),
  illumination: moving('illumination', ['set_level']),
  filter_wheel: moving('filter_wheel', ['select']),
  objective: moving('objective', ['select']),
  modality: moving('modality', ['set_modality']),
  calibration: describe('calibration', ['status'], {
    actions: ['stage_camera', 'capture_dark', 'capture_flat'],
  }),
  workflow: describe('workflow', ['state', 'progress', 'positions', 'field'], {
    actions: [
      'estimate_tile_scan',
      'tile_scan',
      'save_position',
      'rename_position',
      'remove_position',
    ],
  }),
}

/** The fake's state: what reads answer, and what writes change. */
const values: Record<string, unknown> = {
  'camera/exposure_ms': 10,
  'camera/region': { offset_x: 0, offset_y: 0, width: 4000, height: 3000 },
  'camera/binning': { horizontal: 1, vertical: 1 },
  'camera/pixel_format': 'Mono14',
  'camera/frame_rate': 9,
  'camera/sensor': { width: 4000, height: 3000, pixel_size: 3.76, pixel_formats: ['Mono14'] },
  'display/lut': { contrast: { kind: 'auto', low_pct: 0.1, high_pct: 99.9 }, colormap: 'grey' },
  'display/roi_region': { region: null, zoom: 'native' },
  ...workflowValues,
}
let published = 0
let live = true

const json = (body: unknown, status = 200) =>
  new Response(JSON.stringify(body), { status, headers: { 'Content-Type': 'application/json' } })

/** A bell-shaped histogram that drifts, as a live one does. */
function histogram() {
  const centre = 120 + 20 * Math.sin(Date.now() / 3000)
  const bins = Array.from({ length: 256 }, (_, i) =>
    Math.round(5000 * Math.exp(-(((i - centre) / 30) ** 2))),
  )
  return {
    frame: published,
    bins,
    window: [(centre - 60) * 64, (centre + 60) * 64],
    focus: 3e6 + 2e5 * Math.sin(Date.now() / 1500),
  }
}

/** An MJPEG stream of frames drawn here: drifting cells, at 10 frames a second. */
function frames(name: string, signal?: AbortSignal | null) {
  const [width, height] = name === 'roi' ? [960, 540] : [1440, 1080]
  // The cells are drawn sharp, then blurred into the frame at once.
  const [sharp, canvas] = [new OffscreenCanvas(width, height), new OffscreenCanvas(width, height)]
  const context = sharp.getContext('2d')!
  const head = new TextEncoder().encode('--frame\r\nContent-Type: image/jpeg\r\n\r\n')
  const body = new ReadableStream<Uint8Array>({
    start(controller) {
      const timer = setInterval(async () => {
        if (!live) return
        published += 1
        context.fillStyle = '#f4f4f4'
        context.fillRect(0, 0, width, height)
        const scale = name === 'roi' ? 4 : 1
        // The sample moves with the stage, by the overview's 1504 µm across 1440 pixels, and
        // blurs away from focus.
        const stage = motionValues['stage/position'] as { x: number; y: number }
        const [dx, dy] = [stage.x, stage.y].map((um) => (um * scale * 1440) / 1504)
        const wrap = (value: number, size: number) => ((value % size) + size) % size
        for (let i = 0; i < 400; i++) {
          const x =
            wrap(((i * 7919) % width) - dx!, width) + Math.sin(published / 20 + i) * 3 * scale
          const y =
            wrap(((i * 104729) % height) - dy!, height) + Math.cos(published / 20 + i) * 3 * scale
          context.fillStyle = `rgb(${40 + (i % 60)} ${40 + (i % 60)} ${40 + (i % 60)})`
          context.beginPath()
          context.arc(x, y, (3 + (i % 5)) * scale, 0, 2 * Math.PI)
          context.fill()
        }
        const frame = canvas.getContext('2d')!
        const defocus = Math.abs((motionValues['focus/position'] as number) - BEST_FOCUS)
        frame.filter = `blur(${Math.min(defocus / 5, 12)}px)`
        frame.drawImage(sharp, 0, 0)
        const blob = await canvas.convertToBlob({ type: 'image/jpeg', quality: 0.8 })
        const jpeg = new Uint8Array(await blob.arrayBuffer())
        try {
          controller.enqueue(head)
          controller.enqueue(jpeg)
          controller.enqueue(new TextEncoder().encode('\r\n'))
        } catch {
          clearInterval(timer)
        }
      }, 100)
      signal?.addEventListener('abort', () => {
        clearInterval(timer)
        controller.error(signal.reason)
      })
    },
  })
  return new Response(body, {
    headers: { 'Content-Type': 'multipart/x-mixed-replace; boundary=frame' },
  })
}

/** Answers as the server would. */
export const fakeFetch: typeof fetch = async (input, init = {}) => {
  const url = new URL(String(input))
  const method = init.method ?? 'GET'
  const path = url.pathname.replace('/api/v1/', '')
  if (path === 'thing_descriptions/') return json(THINGS)
  if (url.pathname === '/api/siq/system/health') {
    return json({ status: 'ok', version: 'dev', uptime_s: 1, camera: 'the fake camera' })
  }
  if (path === 'camera/overview' || path === 'camera/roi') {
    return frames(path.split('/')[1]!, init.signal)
  }
  if (path === 'camera/stats') {
    return json({
      mode: live ? 'live' : 'stopped',
      fps: live ? 10 : 0,
      published,
      lost_no_slot: 0,
      lost_device: 0,
    })
  }
  if (path === 'calibration/status') return json(status())
  if (method === 'POST' && path.startsWith('calibration/')) {
    const output = await calibrate(path.split('/')[1]!)
    return json(
      { id: '1', status: 'completed', href: `${BASE}api/v1/action_invocations/1`, output },
      201,
    )
  }
  if (method === 'POST' && path.startsWith('workflow/')) {
    const output = workflowAct(path.split('/')[1]!, JSON.parse(String(init.body ?? '{}')))
    Object.assign(values, workflowValues)
    return json(
      { id: '1', status: 'completed', href: `${BASE}api/v1/action_invocations/1`, output },
      201,
    )
  }
  if (path === 'display/histogram') return json(histogram())
  if (path === 'display/focus_measure') return json(histogram().focus)
  if (path in motionValues) {
    const accept = (init.headers as Record<string, string> | undefined)?.Accept
    return accept === 'text/event-stream' ? follow(path, init.signal) : json(motionValues[path])
  }
  if (method === 'POST' && path.split('/')[0]! in THINGS && !path.startsWith('camera/')) {
    const output = await act(path, JSON.parse(String(init.body ?? '{}')))
    return json(
      { id: '1', status: 'completed', href: `${BASE}api/v1/action_invocations/1`, output },
      201,
    )
  }
  if (method === 'PUT' && path in values) {
    values[path] = JSON.parse(String(init.body))
    return new Response(null, { status: 201 })
  }
  if (method === 'GET' && path in values) return json(values[path])
  if (method === 'POST') {
    if (path === 'camera/start_live') live = true
    if (path === 'camera/stop_live') live = false
    const output = path === 'camera/snap' ? { href: `${BASE}favicon.svg` } : null
    return json(
      { id: '1', status: 'completed', href: `${BASE}api/v1/action_invocations/1`, output },
      201,
    )
  }
  return json({ detail: 'Not Found' }, 404)
}

/**
 * Mounts `page` (the Live page by default) against the fake, in an app of its own so that the
 * fake connection stays there, with what the app shell gives a page: shortcuts, confirmations and
 * toasts. Returns the function that unmounts it.
 */
export function mountDemo(host: HTMLElement, pinia: Pinia, page: Component = LivePage) {
  const demo = createApp(
    defineComponent(() => {
      provideShortcuts()
      return () => h(OverlayProvider, null, { default: () => h(page) })
    }),
  )
  demo.use(pinia)
  const client = new WotClient({ baseUrl: '/api/v1/', fetch: fakeFetch, webSocket: null })
  void useConnectionStore(pinia).start(client.baseUrl, client)
  demo.mount(host)
  return () => {
    // The demo's own connection, not the app's.
    useConnectionStore(pinia).stop()
    demo.unmount()
  }
}
