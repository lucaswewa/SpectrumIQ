// The fake instrument's stage, focus and peripherals: moves that take time, jogs that run until
// halted or at a limit, and properties followed over server-sent events, as the server's are
// over its socket. It logs every motion command it gets. For development only (`/#/dev/control`).
import { ref } from 'vue'

type Xyz = { x: number; y: number; z: number }

/** The motion commands received, newest first: what the controls sent. */
export const sent = ref<string[]>([])

const SPEED = 2000 // µm/s
const TICK_MS = 50
const TRAVEL = { x: 50_000, y: 50_000, z: 10_000 }
/** Where the fake sample is sharpest. */
export const BEST_FOCUS = 5000

const OBJECTIVES = [
  { id: 1, name: '10x/0.30', magnification: 10, numerical_aperture: 0.3, parfocal_offset: 0 },
  { id: 2, name: '20x/0.75', magnification: 20, numerical_aperture: 0.75, parfocal_offset: 12.5 },
  { id: 3, name: '60x/1.40 oil', magnification: 60, numerical_aperture: 1.4, parfocal_offset: -8 },
].map((objective, index) => ({
  ...objective,
  immersion: index === 2 ? 'oil' : 'air',
  turret_position: index,
}))

/** The properties, which `set` changes and pushes to their followers. */
export const motionValues: Record<string, unknown> = {
  'stage/position': { x: 0, y: 0, z: 2000 },
  'stage/state': 'idle',
  'stage/homed': false,
  'stage/speed': SPEED,
  'stage/limits': { min: { x: -TRAVEL.x, y: -TRAVEL.y, z: 0 }, max: TRAVEL },
  'focus/position': BEST_FOCUS - 40,
  'focus/state': 'idle',
  'focus/homed': true,
  'focus/limits': { min: 0, max: 10_000 },
  'imaging/channel': 1,
  'imaging/channels': [
    { id: 1, name: 'Brightfield', colour: '#ffffff' },
    { id: 2, name: 'DAPI', colour: '#3f6fff' },
  ],
  'illumination/sources': ['white', '365nm', '470nm'],
  'illumination/levels': [1, 0, 0],
  'filter_wheel/filters': ['empty', 'DAPI', 'GFP'],
  'filter_wheel/position': 0,
  'filter_wheel/moving': false,
  'objective/objectives': OBJECTIVES,
  'objective/position': 0,
  'objective/current': OBJECTIVES[0],
  'objective/moving': false,
  'modality/modes': ['widefield', 'confocal', 'structured_illumination'],
  'modality/patterns': { orientations: 3, phases: 3 },
  'modality/modality': { mode: 'widefield', pattern: null },
  'modality/moving': false,
}

const followers = new Map<string, Set<(value: unknown) => void>>()

function set(path: string, value: unknown) {
  motionValues[path] = value
  followers.get(path)?.forEach((follow) => follow(value))
}

/** Property `path` as server-sent events: its value now, then each change. */
export function follow(path: string, signal?: AbortSignal | null): Response {
  const encoder = new TextEncoder()
  const body = new ReadableStream<Uint8Array>({
    start(controller) {
      const push = (value: unknown) =>
        controller.enqueue(encoder.encode(`data: ${JSON.stringify(value)}\n\n`))
      push(motionValues[path])
      const all = followers.get(path) ?? new Set()
      followers.set(path, all.add(push))
      signal?.addEventListener('abort', () => all.delete(push))
    },
  })
  return new Response(body, { headers: { 'Content-Type': 'text/event-stream' } })
}

// ---- Motion: one timer moves the stage and the focus towards their targets, or at the jog's
// velocity, and settles each move's answer when it arrives or is halted.

const stage = () => motionValues['stage/position'] as Xyz
let target: Xyz | undefined
let velocity: Xyz | undefined
let focusTarget: number | undefined
let settle: (() => void)[] = []
let timer: ReturnType<typeof setInterval> | undefined

const clamp = (value: number, axis: keyof Xyz) =>
  Math.min(Math.max(value, axis === 'z' ? 0 : -TRAVEL[axis]), TRAVEL[axis])

function towards(from: number, to: number) {
  const reach = (SPEED * TICK_MS) / 1000
  return Math.abs(to - from) <= reach ? to : from + Math.sign(to - from) * reach
}

function tick() {
  const at = { ...stage() }
  for (const axis of ['x', 'y', 'z'] as const) {
    if (target) at[axis] = towards(at[axis], target[axis])
    else if (velocity) at[axis] = clamp(at[axis] + (velocity[axis] * TICK_MS) / 1000, axis)
  }
  set('stage/position', at)
  if (target && (['x', 'y', 'z'] as const).every((axis) => at[axis] === target![axis])) {
    target = undefined
  }
  if (focusTarget !== undefined) {
    const focus = towards(motionValues['focus/position'] as number, focusTarget)
    set('focus/position', focus)
    if (focus === focusTarget) focusTarget = undefined
  }
  const still = !target && !velocity && focusTarget === undefined
  set('stage/state', target ? 'moving' : velocity ? 'jogging' : 'idle')
  set('focus/state', focusTarget === undefined ? 'idle' : 'moving')
  if (!target && focusTarget === undefined) {
    settle.forEach((done) => done())
    settle = []
  }
  if (still) timer = void clearInterval(timer)
}

/** Starts the timer, and resolves when the moves under way arrive, or are halted. */
function moving(): Promise<void> {
  timer ??= setInterval(tick, TICK_MS)
  return new Promise((resolve) => settle.push(resolve))
}

function halt() {
  target = velocity = focusTarget = undefined
  tick()
}

/** Runs motion or peripheral action `path`: what it outputs. */
export async function act(path: string, input: Record<string, number>): Promise<unknown> {
  sent.value = [`${path} ${JSON.stringify(input)}`, ...sent.value].slice(0, 14)
  const here = stage()
  const clamped = (to: Xyz) => ({ x: clamp(to.x, 'x'), y: clamp(to.y, 'y'), z: clamp(to.z, 'z') })
  if (path === 'stage/halt' || path === 'focus/halt') halt()
  if (path === 'stage/jog') {
    target = undefined
    const { vx = 0, vy = 0, vz = 0 } = input
    velocity = vx || vy || vz ? { x: vx, y: vy, z: vz } : undefined
    void moving()
    return null
  }
  if (path === 'stage/move_to') target = clamped(input as unknown as Xyz)
  if (path === 'stage/move_by') {
    const { dx = 0, dy = 0, dz = 0 } = input
    target = clamped({ x: here.x + dx, y: here.y + dy, z: here.z + dz })
  }
  if (path === 'stage/home') {
    target = { x: 0, y: 0, z: 0 }
    set('stage/homed', true)
  }
  const focus = motionValues['focus/position'] as number
  if (path === 'focus/move_to') focusTarget = Math.min(Math.max(input.z!, 0), 10_000)
  if (path === 'focus/move_by') focusTarget = Math.min(Math.max(focus + input.dz!, 0), 10_000)
  if (target || focusTarget !== undefined) {
    velocity = undefined
    await moving()
    return { position: path.startsWith('focus') ? motionValues['focus/position'] : stage() }
  }
  return peripheral(path, input)
}

function peripheral(path: string, input: Record<string, number>): unknown {
  if (path === 'imaging/select_channel') set('imaging/channel', input.id)
  if (path === 'illumination/set_level') {
    const sources = motionValues['illumination/sources'] as string[]
    const levels = [...(motionValues['illumination/levels'] as number[])]
    levels[sources.indexOf(input.source as unknown as string)] = input.level!
    set('illumination/levels', levels)
  }
  if (path === 'filter_wheel/select') set('filter_wheel/position', input.position)
  if (path === 'objective/select') {
    set('objective/position', input.position)
    set(
      'objective/current',
      OBJECTIVES.find((each) => each.turret_position === input.position),
    )
  }
  if (path === 'modality/set_modality') set('modality/modality', { pattern: null, ...input })
  return null
}
