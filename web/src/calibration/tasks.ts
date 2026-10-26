// The calibration wizard's tasks (ADR-0051), each a series of steps for one objective, or one
// objective and channel: the stage–camera calibration (ADR-0049), and the dark and flat
// (ADR-0050). What is missing for an objective is offered when it comes into use.
import { markRaw, type Component } from 'vue'

import type {
  CalibrationStatus,
  ObjectiveStatus,
  ShadingRecord,
  StageCameraRecord,
} from '@/stores/calibration'

import CalibrateStep from './CalibrateStep.vue'
import GuideStep from './GuideStep.vue'
import PlaceStep from './PlaceStep.vue'

export interface WizardStep {
  component: Component
  props: Record<string, unknown>
}

export interface WizardTask {
  title?: string
  steps: WizardStep[]
}

/** What a task calibrates: an objective, and a channel (`null` for frames taken with none). */
export interface Objective {
  id: number
  name: string
}
export interface Channel {
  id: number | null
  name: string
}

/** A step that says what to do, with the live image and the d-pad if it helps. */
const guide = (props: {
  lines: string[]
  bullets?: string[]
  stream?: boolean
  pad?: boolean
}) => ({
  component: markRaw(GuideStep),
  props,
})

/** A step that puts the objective, and the channel, in use. */
const place = (props: { objective: Objective; channel?: Channel }) => ({
  component: markRaw(PlaceStep),
  props,
})

/** A step that runs a calibration, shows what it found, and can run it again. */
const calibrate = (props: { kind: CalibrationKind; lines: string[]; stream?: boolean }) => ({
  component: markRaw(CalibrateStep),
  props,
})

export type CalibrationKind = 'stage_camera' | 'dark' | 'flat'

export const WELCOME: WizardTask = {
  steps: [
    guide({
      lines: [
        "The objective in place isn't fully calibrated. Without its stage–camera calibration, scans can't place their tiles; without a flat, a mosaic shows its seams.",
        'Press Next to calibrate it, or Skip any step to do it later from the Calibration page.',
      ],
    }),
  ],
}

export const FINAL: WizardTask = {
  steps: [
    guide({
      lines: [
        "That's the calibration done.",
        'The Calibration page shows what is in force for each objective and channel, and calibrates any of them again.',
      ],
    }),
  ],
}

/** The stage–camera calibration of `objective`. */
export const stageCameraTask = (objective: Objective): WizardTask => ({
  title: `Stage–camera, ${objective.name}`,
  steps: [
    place({ objective }),
    guide({
      lines: [
        'The calibration moves the stage about where it is and follows the image, so it needs a sample with detail across the field.',
      ],
      bullets: ['Put one in.', 'Bring it into focus.'],
      stream: true,
      pad: true,
    }),
    calibrate({
      kind: 'stage_camera',
      lines: [
        'With the sample in focus, calibrate: 16 moves, out to 0.4 of the field, then back.',
        'Accept it with Next, or run it again.',
      ],
      stream: true,
    }),
  ],
})

/** The dark and the flat of `objective` in `channel`. */
export const shadingTask = (objective: Objective, channel: Channel): WizardTask => ({
  title: `Dark and flat, ${objective.name}, ${channel.name}`,
  steps: [
    place({ objective, channel }),
    calibrate({
      kind: 'dark',
      lines: ['First the dark: every light turns off while frames are taken, then back on.'],
    }),
    guide({
      lines: ['Now show the camera an empty part of a slide, under its coverslip.'],
      bullets: ['Nothing should be in view: if specks of dust are, focus away from them a little.'],
      stream: true,
      pad: true,
    }),
    calibrate({
      kind: 'flat',
      lines: [
        'When the view is empty and evenly lit, take the flat: frames at nine places a field apart, less the dark.',
      ],
      stream: true,
    }),
  ],
})

/** An objective's channels with a dark and flat: the configured ones, or none if there are none. */
export function channelsOf(entry: ObjectiveStatus) {
  const configured = entry.shading.filter((shading) => shading.channel != null)
  return configured.length > 0 ? configured : entry.shading
}

/** What `entry`'s objective lacks: its stage–camera calibration, then each channel's flat. */
export function neededFor(entry: ObjectiveStatus): WizardTask[] {
  const objective = { id: entry.objective, name: entry.name }
  const flats = channelsOf(entry)
    .filter((shading) => !shading.flat)
    .map((shading) => shadingTask(objective, { id: shading.channel ?? null, name: shading.name }))
  return [...(entry.calibration ? [] : [stageCameraTask(objective)]), ...flats]
}

/** What every objective lacks. */
export const allNeeded = (status: CalibrationStatus) => status.flatMap(neededFor)

const fixed = (value: number, digits: number) => value.toFixed(digits)

/** What a calibration found, as label and value: for its step, and the Calibration page. */
export function summary(
  kind: CalibrationKind,
  record: StageCameraRecord | ShadingRecord,
): [string, string][] {
  if (kind === 'stage_camera') {
    const found = record as StageCameraRecord
    const backlash = found.backlash_um
    return [
      ['Pixel size', `${fixed(found.px_size_um, 4)} µm`],
      ['Rotation', `${fixed(found.rotation_deg, 3)}°`],
      ['Scale', found.scale.map((s) => `${fixed(s, 4)} px/µm`).join(', ')],
      [
        'Residuals',
        `${fixed(found.residual_rms_um, 2)} µm RMS, ${fixed(found.residual_max_um, 2)} µm at most`,
      ],
      ['Backlash', backlash ? `${backlash.map((b) => fixed(b, 1)).join(', ')} µm` : 'not measured'],
    ]
  }
  const shading = record as ShadingRecord
  if (kind === 'dark') {
    return [['Level', `${fixed(shading.level, 1)} ADU, from ${shading.frames} frames`]]
  }
  const [low, high] = (shading.range ?? [1, 1]).map((light) => `${Math.round(light * 100)} %`)
  return [['Light', `${low} to ${high} of the mean, from ${shading.frames} frames`]]
}
