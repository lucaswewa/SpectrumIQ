// The fake instrument's live mosaic: the `mosaic_live` Thing, a field painted wherever the fake
// stage stands still while it paints, and the viewport rendered as an MJPEG stream of those fields'
// cells, as the server renders it: unpainted stage black. For development only (`/#/dev/mosaic`,
// and the stage map of `/#/dev/scan`).
import { motionValues } from './fakeMotion'

type Viewport = { centre: number[]; zoom: number; width: number; height: number }

/** Canvas pixels a µm: the simulator's 10× at a canvas scale of ½. */
const PX_PER_UM = 1.33
/** Half the field of view, as the fake `workflow/field`. */
const HALF = [376, 282] as const
/** The fake specimen's cells: one in each square of this side, in µm. */
const SPACING = 40
const BASE = `${location.origin}/`

let enabled = true
let viewport: Viewport = { centre: [0, 0], zoom: 0.25, width: 1280, height: 720 }
let fields: [number, number][] = []
let painted = 0

/** Paints a field where the stage stands still, once. */
function paint() {
  if (!enabled || motionValues['stage/state'] !== 'idle') return
  const { x, y } = motionValues['stage/position'] as { x: number; y: number }
  painted += 1
  if (!fields.some(([fx, fy]) => fx === x && fy === y)) fields.push([x, y])
}

function bounds() {
  // It paints whether or not anything looks: as often as the bounds are asked for, or shown.
  paint()
  if (!fields.length) return null
  const xs = fields.map(([x]) => x)
  const ys = fields.map(([, y]) => y)
  return {
    x0: Math.min(...xs) - HALF[0],
    y0: Math.min(...ys) - HALF[1],
    x1: Math.max(...xs) + HALF[0],
    y1: Math.max(...ys) + HALF[1],
  }
}

/** A number from 0 to 1 for each pair of integers, the same each time. */
function hash(i: number, j: number) {
  let h = Math.imul(i, 374761393) + Math.imul(j, 668265263)
  h = Math.imul(h ^ (h >>> 13), 1274126177)
  return ((h ^ (h >>> 16)) >>> 0) / 4294967296
}

/** The viewport rendered, ten times a second, as an MJPEG stream. */
function view(signal?: AbortSignal | null) {
  const canvas = new OffscreenCanvas(viewport.width, viewport.height)
  const head = new TextEncoder().encode('--frame\r\nContent-Type: image/jpeg\r\n\r\n')
  const body = new ReadableStream<Uint8Array>({
    start(controller) {
      const timer = setInterval(async () => {
        paint()
        const { centre, zoom, width, height } = viewport
        if (canvas.width !== width || canvas.height !== height) {
          ;[canvas.width, canvas.height] = [width, height]
        }
        const k = zoom * PX_PER_UM
        const at = (x: number, y: number) => [
          (x - centre[0]!) * k + width / 2,
          (y - centre[1]!) * k + height / 2,
        ]
        const context = canvas.getContext('2d')!
        context.fillStyle = '#000'
        context.fillRect(0, 0, width, height)
        for (const [fx, fy] of fields) {
          const [left, top] = at(fx - HALF[0], fy - HALF[1])
          context.save()
          context.beginPath()
          context.rect(left!, top!, 2 * HALF[0] * k, 2 * HALF[1] * k)
          context.clip()
          context.fillStyle = '#eeeeee'
          context.fillRect(left!, top!, 2 * HALF[0] * k, 2 * HALF[1] * k)
          context.fillStyle = '#4a4a4a'
          const squares = (from: number, to: number) => [
            Math.floor(from / SPACING),
            Math.ceil(to / SPACING),
          ]
          const [i0, i1] = squares(fx - HALF[0], fx + HALF[0])
          const [j0, j1] = squares(fy - HALF[1], fy + HALF[1])
          for (let i = i0!; i <= i1!; i++) {
            for (let j = j0!; j <= j1!; j++) {
              const [x, y] = at((i + hash(i, j)) * SPACING, (j + hash(j, i)) * SPACING)
              context.beginPath()
              context.arc(x!, y!, Math.max((5 + 8 * hash(i + 7, j)) * k, 0.5), 0, 2 * Math.PI)
              context.fill()
            }
          }
          context.restore()
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

const json = (body: unknown, status = 200) =>
  new Response(JSON.stringify(body), { status, headers: { 'Content-Type': 'application/json' } })
const completed = (output: unknown) =>
  json({ id: '1', status: 'completed', href: `${BASE}api/v1/action_invocations/1`, output }, 201)

/** The answer to a request of the `mosaic_live` Thing, or undefined for another's. */
export function mosaicFetch(
  method: string,
  path: string,
  body: string | undefined,
  signal?: AbortSignal | null,
): Response | undefined {
  if (!path.startsWith('mosaic_live/')) return undefined
  const name = path.slice('mosaic_live/'.length)
  if (method === 'PUT') {
    const value = JSON.parse(body ?? 'null')
    if (name === 'enabled') enabled = value
    if (name === 'viewport') viewport = value
    return new Response(null, { status: 204 })
  }
  if (method === 'POST' && name === 'clear') {
    fields = []
    return completed(null)
  }
  if (method === 'POST' && name === 'save_snapshot') {
    const created = new Date().toISOString()
    const dataset = { id: '1', kind: 'mosaic', name: 'live mosaic', created, user: 'dev' }
    return completed({ dataset: { ...dataset, frames: 0, images: ['mosaic'] }, image: 'mosaic' })
  }
  const pages = Math.min(fields.length, 32)
  const answers: Record<string, () => unknown> = {
    view: () => view(signal),
    enabled: () => enabled,
    viewport: () => viewport,
    bounds,
    px_per_um: () => (fields.length ? PX_PER_UM : null),
    canvas_scale: () => 0.5,
    max_uncertainty_px: () => 2,
    stats: () => ({
      painted,
      passed_over: 0,
      last_paint_ms: 1.8,
      views: 0,
      pages_on_gpu: pages,
      pages_in_ram: 0,
      vram_bytes: pages * 64 * 2 ** 20,
      vram_budget_bytes: 2e9,
    }),
  }
  const answer = answers[name]
  if (!answer) return json({ detail: 'Not Found' }, 404)
  const value = answer()
  return value instanceof Response ? value : json(value)
}
