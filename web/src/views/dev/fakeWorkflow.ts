// The fake instrument's workflows: the `workflow` Thing's saved positions, its field of view, and
// tile scans estimated on a grid along the stage's axes, as the server's would be, near enough;
// a scan "runs" at once. For development only (`/#/dev/scan`).
import { motionValues } from './fakeMotion'

type Point = [number, number]
type Region =
  | { shape: 'rect'; x0: number; y0: number; x1: number; y1: number }
  | { shape: 'polygon'; points: Point[] }

/** The 20×'s frame on the fake's 4000 × 3000 sensor, at 0.188 µm a pixel. */
const HALF: Point = [376, 282]

export const workflowValues: Record<string, unknown> = {
  'workflow/state': 'idle',
  'workflow/progress': null,
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
}

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
  const field = workflowValues['workflow/field']
  return {
    tiles,
    frame: field,
    frames,
    seconds: frames * 1.3,
    bytes: frames * 24e6,
    free_bytes: 4e11,
    fits: true,
    warnings,
  }
}

type Position = { name: string; x: number; y: number; z?: number; focus?: number }

/** What a workflow action answers. */
export function workflowAct(action: string, input: Record<string, unknown>) {
  const positions = workflowValues['workflow/positions'] as Position[]
  const keep = (next: Position[]) => (workflowValues['workflow/positions'] = next)
  if (action === 'estimate_tile_scan') return estimate(input as never)
  if (action === 'save_position') {
    const at = motionValues['stage/position'] as { x: number; y: number; z: number }
    const { name, ...given } = input as Partial<Position> & { name: string }
    return keep([...positions, { name, x: at.x, y: at.y, z: at.z, ...given }])
  }
  if (action === 'rename_position') {
    return keep(
      positions.map((p) => (p.name === input.name ? { ...p, name: String(input.to) } : p)),
    )
  }
  if (action === 'remove_position') return keep(positions.filter((p) => p.name !== input.name))
  const { frames } = estimate(input as never)
  return {
    name: input.name,
    dataset: crypto.randomUUID(),
    outcome: 'finished',
    error: null,
    steps: [frames, frames],
    frames,
  }
}
