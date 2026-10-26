import { defineStore } from 'pinia'

import { useThingMirror, type Output, type Value } from './thing'

/** Each objective, with its calibrations in force (ADR-0049, ADR-0050). */
export type CalibrationStatus = Value<'calibration', 'status'>
/** One objective's. */
export type ObjectiveStatus = CalibrationStatus[number]
/** A stage–camera calibration, as the API gives it. */
export type StageCameraRecord = Output<'calibration', 'stage_camera'>
/** A dark or a flat, as the API gives it. */
export type ShadingRecord = Output<'calibration', 'capture_flat'>

/**
 * The calibrations: what is in force for each objective and channel, read when the Thing is
 * there, every 10 s for a flat a job estimates, and again after each calibration; and the
 * calibrations to run, each of the objective and channel in use.
 */
export const useCalibrationStore = defineStore('calibration', () => {
  const calibration = useThingMirror('calibration')
  const status = calibration.property('status', { pollMs: 10_000 })

  /** Runs `output`, then reads the status again: the output, or undefined if it failed. */
  async function then<T>(output: Promise<T | undefined>) {
    const result = await output
    await calibration.read('status', status)
    return result
  }

  return {
    fitted: calibration.fitted,
    status,
    /** Calibrates the objective in use against the stage. */
    stageCamera: () => then(calibration.run('stage_camera')),
    /** Takes the dark of the objective and channel in use, with every light off. */
    captureDark: () => then(calibration.run('capture_dark', {})),
    /** Takes their flat, `defocusUm` out of focus (0 for a blank slide). */
    captureFlat: (defocusUm = 0) =>
      then(calibration.run('capture_flat', { defocus_um: defocusUm })),
  }
})
