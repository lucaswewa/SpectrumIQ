// @vitest-environment node
// REQ-STG-001: presses of the d-pad, the coarse-Z buttons and the keys, as moves (ADR-0039).
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { JogController, stageWithinTravel, Stepper, withinTravel, type Axes } from '@/control/jog'

beforeEach(() => vi.useFakeTimers())
afterEach(() => vi.useRealTimers())

const UP: Axes = { x: 0, y: -1, z: 0 }
const DOWN: Axes = { x: 0, y: 1, z: 0 }
const RIGHT: Axes = { x: 1, y: 0, z: 0 }

/** A controller with steps of 100, 100 and 50 µm, what it sends, and a way to end a step. */
function controller() {
  const sent: (['step' | 'jog', Axes] | 'stop')[] = []
  const ends: (() => void)[] = []
  const jog = new JogController({
    step: (by) => {
      sent.push(['step', by])
      return new Promise<void>((resolve) => ends.push(resolve))
    },
    jog: (velocity) => sent.push(['jog', velocity]),
    stop: () => sent.push('stop'),
    steps: () => ({ x: 100, y: 100, z: 50 }),
  })
  const endStep = async () => {
    ends.shift()?.()
    await vi.advanceTimersByTimeAsync(0)
  }
  return { jog, sent, endStep }
}

describe('the jog controller', () => {
  it('moves one step for a tap, and lets it finish', async () => {
    const { jog, sent, endStep } = controller()
    jog.press('ArrowUp', UP)
    await vi.advanceTimersByTimeAsync(100)
    jog.release('ArrowUp')
    await vi.advanceTimersByTimeAsync(1000)
    await endStep()
    expect(sent).toEqual([['step', { x: 0, y: -100, z: 0 }]])
  })

  it('jogs at 4 steps a second while held, once its step has ended, and stops on release', async () => {
    const { jog, sent, endStep } = controller()
    jog.press('pointer:z-up', { x: 0, y: 0, z: 1 })
    await vi.advanceTimersByTimeAsync(300)
    // Held, but the step still runs: the stage takes one move at a time.
    expect(sent).toHaveLength(1)
    await endStep()
    jog.release('pointer:z-up')
    expect(sent).toEqual([['step', { x: 0, y: 0, z: 50 }], ['jog', { x: 0, y: 0, z: 200 }], 'stop'])
  })

  it('never jogs for a hold released before its step ended', async () => {
    const { jog, sent, endStep } = controller()
    jog.press('ArrowRight', RIGHT)
    await vi.advanceTimersByTimeAsync(400)
    jog.release('ArrowRight')
    await endStep()
    await vi.advanceTimersByTimeAsync(1000)
    expect(sent).toEqual([['step', { x: 100, y: 0, z: 0 }]])
  })

  it('adds up the taps made while a step runs, and sends them as one step', async () => {
    const { jog, sent, endStep } = controller()
    for (const [key, direction] of [
      ['ArrowUp', UP],
      ['ArrowRight', RIGHT],
      ['ArrowRight', RIGHT],
    ] as const) {
      jog.press(key, direction)
      jog.release(key)
    }
    expect(sent).toEqual([['step', { x: 0, y: -100, z: 0 }]])
    await endStep()
    expect(sent.at(-1)).toEqual(['step', { x: 200, y: 0, z: 0 }])
    await endStep()
    expect(sent).toHaveLength(2)
  })

  it('combines held presses, follows what is still held, and opposite ones cancel out', async () => {
    const { jog, sent, endStep } = controller()
    jog.press('ArrowUp', UP)
    await vi.advanceTimersByTimeAsync(300)
    await endStep()
    jog.press('pointer:right', RIGHT)
    jog.press('ArrowDown', DOWN)
    jog.release('pointer:right')
    jog.release('ArrowUp')
    jog.release('ArrowDown')
    expect(sent.slice(1)).toEqual([
      ['jog', { x: 0, y: -400, z: 0 }],
      ['jog', { x: 400, y: -400, z: 0 }],
      ['jog', { x: 400, y: 0, z: 0 }],
      ['jog', { x: 0, y: 0, z: 0 }],
      ['jog', { x: 0, y: 400, z: 0 }],
      'stop',
    ])
  })

  it('ignores a held key’s repeats, and a release of what was never pressed', async () => {
    const { jog, sent } = controller()
    jog.press('ArrowRight', RIGHT)
    jog.press('ArrowRight', RIGHT)
    jog.release('ArrowDown')
    jog.release('ArrowRight')
    await vi.advanceTimersByTimeAsync(1000)
    expect(sent).toEqual([['step', { x: 100, y: 0, z: 0 }]])
  })

  it('releases everything at once, as when the window loses focus, and stops once', async () => {
    const { jog, sent, endStep } = controller()
    jog.press('ArrowUp', UP)
    jog.press('PageDown', { x: 0, y: 0, z: -1 })
    await vi.advanceTimersByTimeAsync(300)
    await endStep()
    await endStep()
    jog.releaseAll()
    jog.releaseAll()
    expect(sent.at(-1)).toBe('stop')
    expect(sent.filter((each) => each === 'stop')).toHaveLength(1)
  })

  it('forgets the steps still to come on cancel, leaving the stop to STOP', async () => {
    const { jog, sent, endStep } = controller()
    jog.press('ArrowUp', UP)
    jog.release('ArrowUp')
    jog.press('ArrowRight', RIGHT)
    jog.cancel()
    await endStep()
    await vi.advanceTimersByTimeAsync(1000)
    expect(sent).toEqual([['step', { x: 0, y: -100, z: 0 }]])
  })
})

describe('the stepper', () => {
  it('sends one move at a time, skips what adds up to nothing, and survives a failure', async () => {
    const moves: Axes[] = []
    const stepper = new Stepper(async (by) => {
      moves.push(by)
      throw new Error('refused')
    })
    stepper.add({ x: 0, y: 0, z: 2 })
    stepper.add({ x: 0, y: 0, z: 1.5 })
    stepper.add({ x: 0, y: 0, z: -1.5 })
    await stepper.idle()
    expect(moves).toEqual([{ x: 0, y: 0, z: 2 }])
    stepper.add({ x: 1, y: 0, z: 0 })
    await stepper.idle()
    expect(moves.at(-1)).toEqual({ x: 1, y: 0, z: 0 })
  })
})

describe('the travel', () => {
  it('cuts a move to stay within it, and never turns one the other way', () => {
    const travel = { min: -100, max: 0 }
    expect(withinTravel(50, -20, travel)).toBe(20)
    expect(withinTravel(-50, -20, travel)).toBe(-50)
    expect(withinTravel(-500, -20, travel)).toBe(-80)
    // Already beyond it: no further, but back is fine.
    expect([withinTravel(10, 5, travel), withinTravel(-10, 5, travel)]).toEqual([0, -10])
    // Unknown: the move as asked, for the server to judge.
    expect([withinTravel(7, undefined, travel), withinTravel(7, 0)]).toEqual([7, 7])
    const stage = { min: { x: -100, y: -60, z: 0 }, max: { x: 0, y: 0, z: 0 } }
    expect(stageWithinTravel({ x: 50, y: -50, z: 1 }, { x: -20, y: -20, z: 0 }, stage)).toEqual({
      x: 20,
      y: -40,
      z: 0,
    })
  })
})
