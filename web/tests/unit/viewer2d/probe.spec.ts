// REQ-VIS-001: the pixel probe asks for the values under the pointer once it rests, cancels the
// request for a position it has left, and shows nothing once the pointer leaves the image.
import { effectScope } from 'vue'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { PROBE_MS, useProbe, type Pixel } from '@/viewer2d/probe'

beforeEach(() => vi.useFakeTimers())
afterEach(() => vi.useRealTimers())

/** A probe whose reads answer when told, with the pixel's x. */
function probe() {
  const asked: { at: Pixel; signal: AbortSignal; answer: (value: number) => void }[] = []
  const scope = effectScope()
  const it = scope.run(() =>
    useProbe((at, signal) => new Promise<number>((answer) => asked.push({ at, signal, answer }))),
  )!
  return { probe: it, asked, scope }
}

describe('the probe', () => {
  it('asks once the pointer rests, for where it rests', async () => {
    const { probe: p, asked } = probe()
    p.move([1, 1])
    vi.advanceTimersByTime(PROBE_MS / 2)
    p.move([2, 2])
    vi.advanceTimersByTime(PROBE_MS - 1)
    expect(asked).toHaveLength(0)
    vi.advanceTimersByTime(1)
    expect(asked.map((ask) => ask.at)).toEqual([[2, 2]])
    expect(p.at.value).toEqual([2, 2])
    asked[0]!.answer(2)
    await vi.runAllTimersAsync()
    expect(p.value.value).toBe(2)
  })

  it('cancels the request for a position it has left, and drops that answer', async () => {
    const { probe: p, asked } = probe()
    p.move([1, 1])
    vi.advanceTimersByTime(PROBE_MS)
    p.move([3, 3])
    expect(asked[0]!.signal.aborted).toBe(true)
    asked[0]!.answer(1)
    vi.advanceTimersByTime(PROBE_MS)
    asked[1]!.answer(3)
    await vi.runAllTimersAsync()
    expect(p.value.value).toBe(3)
  })

  it('shows nothing once the pointer leaves, and stops with its scope', async () => {
    const { probe: p, asked, scope } = probe()
    p.move([1, 1])
    vi.advanceTimersByTime(PROBE_MS)
    asked[0]!.answer(1)
    await vi.runAllTimersAsync()
    p.move(undefined)
    expect([p.at.value, p.value.value]).toEqual([undefined, undefined])
    p.move([4, 4])
    scope.stop()
    vi.advanceTimersByTime(PROBE_MS)
    expect(asked).toHaveLength(1)
  })
})
