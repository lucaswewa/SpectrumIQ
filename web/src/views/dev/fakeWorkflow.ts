// The fake instrument's workflows: the `workflow` Thing's saved positions, its field of view, tile
// scans estimated on a grid along the stage's axes, as the server's would be, near enough, and
// runs that take a step every half second. A run whose policy asks fails once, at its fourth
// step, and waits for Retry or Skip. Its properties are followed as `fakeMotion.ts`'s are. For
// development only (`/#/dev/scan`, `/#/dev/run`).
import { motionValues, set } from './fakeMotion'

type Point = [number, number]
type Region =
  | { shape: 'rect'; x0: number; y0: number; x1: number; y1: number }
  | { shape: 'polygon'; points: Point[] }
type Position = { name: string; x: number; y: number; z?: number; focus?: number }

/** The 20×'s frame on the fake's 4000 × 3000 sensor, at 0.188 µm a pixel. */
const HALF: Point = [376, 282]
const BASE = `${location.origin}/`

Object.assign(motionValues, {
  'workflow/state': 'idle',
  'workflow/progress': null,
  'workflow/problem': null,
  'workflow/field': [
    [-HALF[0], -HALF[1]],
    [HALF[0], -HALF[1]],
    [HALF[0], HALF[1]],
    [-HALF[0], HALF[1]],
  ],
  'workflow/positions': [
    { name: 'Slide corner', x: -20_000, y: -15_000, z: 2000, focus: 4000 },
    { name: 'Section A', x: -14_000, y: -12_000, z: 2000, focus: 4000 },
  ],
})

const json = (body: unknown, status = 200) =>
  new Response(JSON.stringify(body), { status, headers: { 'Content-Type': 'application/json' } })
const record = (id: string, status: string, output: unknown = null) => ({
  id,
  status,
  href: `${BASE}api/v1/action_invocations/${id}`,
  output,
})

/** A snake of tiles a frame less the overlap apart, over the region's bounding box. */
function estimate(request: { region: Region; overlap: number; channels: number[] }) {
  const { region, overlap } = request
  const points: Point[] =
    region.shape === 'rect'
      ? [
          [region.x0, region.y0],
          [region.x1, region.y1],
        ]
      : region.points
  const low = [0, 1].map((axis) => Math.min(...points.map((p) => p[axis]!)))
  const high = [0, 1].map((axis) => Math.max(...points.map((p) => p[axis]!)))
  const step = HALF.map((half) => 2 * half * (1 - overlap))
  const counts = [0, 1].map((a) =>
    Math.max(1, Math.ceil((high[a]! - low[a]! - 2 * HALF[a]!) / step[a]!) + 1),
  )
  const tiles = Array.from({ length: counts[1]! }, (_, row) =>
    Array.from({ length: counts[0]! }, (_, column) => {
      const c = row % 2 ? counts[0]! - 1 - column : column
      const x = (low[0]! + high[0]!) / 2 + (c - (counts[0]! - 1) / 2) * step[0]!
      const y = (low[1]! + high[1]!) / 2 + (row - (counts[1]! - 1) / 2) * step[1]!
      return { column: c, row, x, y }
    }),
  ).flat()
  const frames = tiles.length * Math.max(1, request.channels.length)
  const warnings = tiles.length > 400 ? ['A long scan: check the estimate before starting.'] : []
  return {
    tiles,
    frame: motionValues['workflow/field'],
    frames,
    seconds: frames * 1.3,
    bytes: frames * 24e6,
    free_bytes: 4e11,
    fits: true,
    warnings,
  }
}

/** The run under way, and the record of the last. */
let run: { name: string; done: number; total: number; ask: boolean; timer?: number } | undefined
let last = record('run', 'completed')

function begin(name: string, total: number, ask: boolean) {
  run = { name, done: 0, total, ask }
  last = record('run', 'running')
  set('workflow/state', 'running')
  tick()
}

function tick() {
  if (!run) return
  if (motionValues['workflow/state'] === 'running') {
    if (run.ask && run.done === 3 && !motionValues['workflow/problem']) {
      const problem = { step: 'move to (1000, 0) µm', attempt: 1, error: 'outside the travel' }
      set('workflow/problem', problem)
      set('workflow/state', 'asking')
    } else {
      run.done += 1
      const { done, total } = run
      const step = `snap t0 c1 z0 p${done - 1}`
      set('workflow/progress', { done, total, eta_s: (total - done) * 0.5, step })
      if (done >= total) return end('completed')
    }
  }
  run.timer = window.setTimeout(tick, 500)
}

function end(status: 'completed' | 'cancelled') {
  const { name, done, total, timer } = run!
  clearTimeout(timer)
  const output = { name, dataset: crypto.randomUUID(), outcome: 'finished', error: null }
  last = record('run', status, { ...output, steps: [done, total], frames: done })
  run = undefined
  set('workflow/state', 'idle')
  set('workflow/problem', null)
}

/** Answers a request of the `workflow` Thing, or of its run's invocation: none if it isn't one. */
export function workflowFetch(method: string, path: string, body: Record<string, unknown>) {
  if (path === 'action_invocations/run') {
    if (method !== 'DELETE') return json(last)
    if (run) end('cancelled')
    return new Response(null, { status: 204 })
  }
  if (method !== 'POST' || !path.startsWith('workflow/')) return undefined
  const action = path.split('/')[1]!
  const answer = (output: unknown = null) => json(record('1', 'completed', output), 201)
  const positions = motionValues['workflow/positions'] as Position[]
  const keep = (next: Position[]) => (set('workflow/positions', next), answer(next))
  switch (action) {
    case 'estimate_tile_scan':
      return answer(estimate(body as never))
    case 'tile_scan':
    case 'run': {
      const total = action === 'run' ? 12 : estimate(body as never).frames
      begin(
        String(body.name),
        total,
        Boolean((body.on_error as { ask?: boolean } | undefined)?.ask),
      )
      return json(last, 201)
    }
    case 'pause':
    case 'resume':
      set('workflow/state', action === 'pause' ? 'paused' : 'running')
      return answer()
    case 'retry':
    case 'skip':
      if (action === 'skip' && run) run.done += 1
      set('workflow/problem', null)
      set('workflow/state', 'running')
      return answer()
    case 'save_position': {
      const at = motionValues['stage/position'] as { x: number; y: number; z: number }
      const { name, ...given } = body as unknown as Partial<Position> & { name: string }
      return keep([...positions, { name, x: at.x, y: at.y, z: at.z, ...given }])
    }
    case 'rename_position':
      return keep(
        positions.map((p) => (p.name === body.name ? { ...p, name: String(body.to) } : p)),
      )
    case 'remove_position':
      return keep(positions.filter((p) => p.name !== body.name))
  }
  return json({ detail: 'Not Found' }, 404)
}
