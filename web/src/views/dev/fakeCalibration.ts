// The fake instrument's calibrations: the `calibration` Thing's status, for the objectives and
// channels of `fakeMotion.ts`, and its calibrations, of the objective and channel in use, which
// take a second or two and find plausible values. For development only (`/#/dev/calibration`).
import { motionValues } from './fakeMotion'

type Objective = { id: number; name: string }
type Channel = { id: number; name: string }

const objectives = () => motionValues['objective/objectives'] as Objective[]
const channels = () => motionValues['imaging/channels'] as Channel[]

/** What is in force, by `objective/channel/kind`: the 20× already calibrated, the rest not. */
const records = new Map<string, Record<string, unknown>>()

const created = () => new Date().toISOString()
const delay = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms))

function stageCamera(objective: Objective) {
  const pixel = 3.76 / ({ 1: 10, 2: 20, 3: 60 }[objective.id] ?? 10)
  const scale = (1 / pixel) * 1.0025
  return {
    id: crypto.randomUUID(),
    objective: objective.id,
    created: created(),
    affine: [
      [scale, -0.0157],
      [0.0182, scale],
    ],
    px_size_um: pixel / 1.0025,
    rotation_deg: 0.391 + Math.random() * 0.01,
    scale: [scale, scale * 0.9999],
    shear: 0.0009,
    residual_rms_um: 0.8 + Math.random() * 0.2,
    residual_max_um: 1.3 + Math.random() * 0.3,
    backlash_um: [2.2, 2.5],
    moves: 16,
    method: 'stage_camera',
  }
}

function shading(kind: 'dark' | 'flat', objective: number, channel: number | null) {
  return {
    id: crypto.randomUUID(),
    kind,
    objective,
    channel,
    created: created(),
    method: 'capture',
    frames: kind === 'dark' ? 16 : 18,
    exposure_ms: 10,
    binning: { horizontal: 1, vertical: 1 },
    level: 100 + Math.random(),
    flat: null,
    range: kind === 'flat' ? [0.862, 1.082] : null,
  }
}
records.set('2/stage_camera', stageCamera({ id: 2, name: '20x/0.75' }))

/** The `status` property. */
export function status() {
  const entries = [...channels(), { id: null, name: 'no channel' }]
  return objectives().map((objective) => ({
    objective: objective.id,
    name: objective.name,
    calibration: records.get(`${objective.id}/stage_camera`) ?? null,
    shading: entries.map((channel) => ({
      channel: channel.id,
      name: channel.name,
      dark: records.get(`${objective.id}/${channel.id}/dark`) ?? null,
      flat: records.get(`${objective.id}/${channel.id}/flat`) ?? null,
    })),
  }))
}

/** Runs action `action` for the objective and channel in use: what it found. */
export async function calibrate(action: string) {
  const objective = motionValues['objective/current'] as Objective
  const channel = motionValues['imaging/channel'] as number | null
  await delay(action === 'stage_camera' ? 2500 : 1200)
  const [key, record] =
    action === 'stage_camera'
      ? [`${objective.id}/stage_camera`, stageCamera(objective)]
      : (() => {
          const kind = action === 'capture_dark' ? 'dark' : 'flat'
          return [`${objective.id}/${channel}/${kind}`, shading(kind, objective.id, channel)]
        })()
  records.set(key, record)
  return record
}
