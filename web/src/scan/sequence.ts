// A sequence (ADR-0057, amended in P6.7; ADR-0055): frames at saved positions, or where the
// stage is, in channels, with an autofocus and a z-stack at each, once or as a time-lapse; and the
// plan it asks the server to run.
import type { PlanRequest, Position } from '@/stores/workflow'

/** What a failed step does: ends the run, is skipped, or waits to be told. */
export type OnError = 'stop' | 'skip' | 'ask'

/** What the sequence form asks for. */
export interface SequenceForm {
  name: string
  /** The saved positions visited, by name: where the stage is if none. */
  positions: string[]
  channels: number[]
  autofocus: { on: boolean; rangeUm: number }
  stack: { on: boolean; rangeUm: number; stepUm: number }
  /** Off for once. */
  lapse: { on: boolean; intervalS: number; count: number }
  onError: OnError
}

export function newSequence(): SequenceForm {
  return {
    name: `sequence-${new Date().toISOString().slice(0, 16).replace(/[-:T]/g, '')}`,
    positions: [],
    channels: [],
    autofocus: { on: false, rangeUm: 50 },
    stack: { on: false, rangeUm: 10, stepUm: 2 },
    lapse: { on: true, intervalS: 60, count: 10 },
    onError: 'stop',
  }
}

/** An error policy's form. */
export const policy = (onError: OnError) => ({
  retries: 0,
  skip: onError === 'skip',
  ask: onError === 'ask',
})

/** What is wrong with `form`, before asking the server. */
export function sequenceProblems(form: SequenceForm, saved: readonly Position[]): string[] {
  const found: string[] = []
  if (!form.name.trim()) found.push('The sequence needs a name.')
  const missing = form.positions.filter((name) => !saved.some((p) => p.name === name))
  if (missing.length) found.push(`No position is saved as ${missing.join(', ')}.`)
  if (form.autofocus.on && !(form.autofocus.rangeUm > 0)) {
    found.push("The autofocus's range must be more than 0.")
  }
  const { stack, lapse } = form
  if (
    stack.on &&
    !(stack.stepUm > 0 && stack.rangeUm >= 0 && stack.rangeUm / stack.stepUm < 1000)
  ) {
    found.push('A z-stack takes from 1 to 1000 planes, a step of more than 0 apart.')
  }
  if (lapse.on && !(lapse.intervalS > 0 && Number.isInteger(lapse.count) && lapse.count >= 1)) {
    found.push('A time-lapse needs an interval of more than 0, and a count of 1 or more.')
  }
  return found
}

/** The frames `form` takes. */
export function framesOf(form: SequenceForm): number {
  const planes = form.stack.on ? Math.floor(form.stack.rangeUm / form.stack.stepUm + 1e-9) + 1 : 1
  const points = form.lapse.on ? form.lapse.count : 1
  return points * Math.max(form.positions.length, 1) * planes * Math.max(form.channels.length, 1)
}

/** The plan `form` asks for. */
export function sequencePlan(form: SequenceForm, saved: readonly Position[]): PlanRequest {
  // The steps are opaque in the API's schema (ADR-0056): built as the engine reads them.
  type Steps = Record<string, unknown>[]
  const snaps: Steps = form.channels.length
    ? form.channels.flatMap((id) => [{ step: 'channel', id }, { step: 'snap' }])
    : [{ step: 'snap' }]
  const { stack, lapse, autofocus } = form
  let steps: Steps = stack.on
    ? [
        {
          step: 'z_stack',
          planes: { range_um: stack.rangeUm, step_um: stack.stepUm },
          steps: snaps,
        },
      ]
    : snaps
  const focusing = autofocus.on ? autofocus.rangeUm : null
  const positions = saved.filter((position) => form.positions.includes(position.name))
  if (positions.length) {
    steps = [{ step: 'multi_position', positions, autofocus: focusing, steps }]
  } else if (focusing) {
    steps = [{ step: 'autofocus', range_um: focusing }, ...steps]
  }
  if (lapse.on) {
    steps = [{ step: 'time_lapse', interval_s: lapse.intervalS, count: lapse.count, steps }]
  }
  const plan = steps as PlanRequest['steps']
  return { name: form.name.trim(), steps: plan, on_error: policy(form.onError) }
}
