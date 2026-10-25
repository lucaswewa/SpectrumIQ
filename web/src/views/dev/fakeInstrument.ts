// A fake instrument for the development pages: a `fetch` that answers as a SpectrumIQ server
// does, with a camera and a display Thing, and MJPEG streams drawn in the browser. For
// development only (`/#/dev/live`).
import type { ThingDescription } from '@/api/wot/td'

const BASE = `${location.origin}/`

function describe(thing: string, properties: string[], links: string[] = []): ThingDescription {
  const forms = (name: string) => [
    { href: `/api/v1/${thing}/${name}`, op: ['readproperty', 'writeproperty'] },
  ]
  return {
    title: thing,
    base: BASE,
    properties: Object.fromEntries(properties.map((name) => [name, { forms: forms(name) }])),
    actions: Object.fromEntries(
      ['start_live', 'stop_live', 'snap', 'auto_contrast_once'].map((name) => [
        name,
        { forms: [{ href: `/api/v1/${thing}/${name}`, op: ['invokeaction'] }] },
      ]),
    ),
    links: links.map((name) => ({
      href: `/api/v1/${thing}/${name}`,
      type: 'multipart/x-mixed-replace',
    })),
  }
}

const THINGS = {
  camera: describe(
    'camera',
    ['exposure_ms', 'region', 'binning', 'pixel_format', 'frame_rate', 'sensor', 'stats'],
    ['overview', 'roi'],
  ),
  display: describe('display', ['lut', 'roi_region', 'histogram', 'focus_measure']),
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
  const canvas = new OffscreenCanvas(width, height)
  const context = canvas.getContext('2d')!
  const head = new TextEncoder().encode('--frame\r\nContent-Type: image/jpeg\r\n\r\n')
  const body = new ReadableStream<Uint8Array>({
    start(controller) {
      const timer = setInterval(async () => {
        if (!live) return
        published += 1
        context.fillStyle = '#f4f4f4'
        context.fillRect(0, 0, width, height)
        const scale = name === 'roi' ? 4 : 1
        for (let i = 0; i < 400; i++) {
          const x = ((i * 7919) % width) + Math.sin(published / 20 + i) * 3 * scale
          const y = ((i * 104729) % height) + Math.cos(published / 20 + i) * 3 * scale
          context.fillStyle = `rgb(${40 + (i % 60)} ${40 + (i % 60)} ${40 + (i % 60)})`
          context.beginPath()
          context.arc(x, y, (3 + (i % 5)) * scale, 0, 2 * Math.PI)
          context.fill()
        }
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
  if (path === 'display/histogram') return json(histogram())
  if (path === 'display/focus_measure') return json(histogram().focus)
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
