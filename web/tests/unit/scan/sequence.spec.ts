// REQ-WF-003, REQ-WF-004: a sequence's plan, at the saved positions or where the stage is, its
// frames counted, and its checks (ADR-0057, ADR-0055).
import { describe, expect, it } from 'vitest'

import { framesOf, newSequence, sequencePlan, sequenceProblems } from '@/scan/sequence'

const SAVED = [
  { name: 'A', x: -20_000, y: -15_000, z: 0, focus: 2000 },
  { name: 'B', x: -19_000, y: -15_000, z: 0, focus: 2001 },
]

describe('a sequence', () => {
  it('is a time-lapse of the positions, each autofocused and stacked in its channels', () => {
    const form = {
      ...newSequence(),
      name: ' lapse ',
      positions: ['B', 'A'],
      channels: [1, 2],
      autofocus: { on: true, rangeUm: 30 },
      stack: { on: true, rangeUm: 4, stepUm: 2 },
      lapse: { on: true, intervalS: 30, count: 5 },
      onError: 'ask' as const,
    }
    expect(sequenceProblems(form, SAVED)).toEqual([])
    const snaps = [
      { step: 'channel', id: 1 },
      { step: 'snap' },
      { step: 'channel', id: 2 },
      { step: 'snap' },
    ]
    const stack = { step: 'z_stack', planes: { range_um: 4, step_um: 2 }, steps: snaps }
    expect(sequencePlan(form, SAVED)).toEqual({
      name: 'lapse',
      steps: [
        {
          step: 'time_lapse',
          interval_s: 30,
          count: 5,
          steps: [{ step: 'multi_position', positions: SAVED, autofocus: 30, steps: [stack] }],
        },
      ],
      on_error: { retries: 0, skip: false, ask: true },
    })
    // 5 points × 2 positions × 3 planes × 2 channels.
    expect(framesOf(form)).toBe(60)
  })

  it('is a frame where the stage is, once, after an autofocus if asked', () => {
    const once = { ...newSequence(), lapse: { on: false, intervalS: 60, count: 10 } }
    expect(sequencePlan(once, SAVED).steps).toEqual([{ step: 'snap' }])
    const focused = { ...once, autofocus: { on: true, rangeUm: 50 }, onError: 'skip' as const }
    const plan = sequencePlan(focused, SAVED)
    expect(plan.steps).toEqual([{ step: 'autofocus', range_um: 50 }, { step: 'snap' }])
    expect(plan.on_error).toEqual({ retries: 0, skip: true, ask: false })
    expect(framesOf(once)).toBe(1)
  })

  it('says what is wrong before the server is asked', () => {
    const wrong = {
      ...newSequence(),
      name: '',
      positions: ['gone'],
      lapse: { on: true, intervalS: 0, count: 2 },
    }
    expect(sequenceProblems(wrong, SAVED)).toEqual([
      'The sequence needs a name.',
      'No position is saved as gone.',
      'A time-lapse needs an interval of more than 0, and a count of 1 or more.',
    ])
  })
})
