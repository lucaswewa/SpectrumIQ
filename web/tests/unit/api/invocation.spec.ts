// @vitest-environment node
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { WotClient } from '@/api/wot/client'
import { ApiError } from '@/api/wot/errors'
import type { InvocationRecord } from '@/api/wot/invocation'

import { fakeServer, json, stage } from './fakeServer'

const HREF = 'http://microscope.test/api/v1/action_invocations/1'

function record(status: InvocationRecord['status'], extra: Partial<InvocationRecord> = {}) {
  return {
    id: '1',
    status,
    action: '/api/v1/stage/move_to',
    href: HREF,
    timeRequested: '2026-10-09T12:00:00',
    timeStarted: null,
    timeCompleted: null,
    ...extra,
  } satisfies InvocationRecord
}

/** A server whose invocation answers each poll with the next of `polls`. */
function serverWith(...polls: InvocationRecord[]) {
  const queue = [...polls]
  const server = fakeServer({
    'POST /api/v1/stage/move_to': () => json(record('pending'), 201),
    'GET /api/v1/action_invocations/1': () => json(queue.length > 1 ? queue.shift() : queue[0]),
    'DELETE /api/v1/action_invocations/1': null,
  })
  const thing = new WotClient({ baseUrl: 'http://microscope.test/api/v1/', fetch: server.fetch })
  return { thing: thing.consume(stage), received: server.received }
}

beforeEach(() => vi.useFakeTimers())
afterEach(() => vi.useRealTimers())

describe('invokeAction', () => {
  it('posts the input and follows the invocation to its output, slowing its polls', async () => {
    const { thing, received } = serverWith(
      record('running'),
      record('running'),
      record('completed', { output: { x: 1 } }),
    )
    const updates: string[] = []
    const invocation = await thing.invokeAction(
      'move_to',
      { x: 1 },
      {
        onUpdate: (update) => updates.push(update.status),
      },
    )
    expect(received[0]!.body).toBe('{"x":1}')
    expect(invocation.record.status).toBe('pending')

    await vi.advanceTimersByTimeAsync(100)
    expect(updates).toEqual(['running'])
    await vi.advanceTimersByTimeAsync(149) // the next poll is 150 ms later
    expect(updates).toEqual(['running'])
    await vi.advanceTimersByTimeAsync(1)
    expect(updates).toEqual(['running', 'running'])
    await vi.advanceTimersByTimeAsync(225)
    expect(await invocation.output()).toEqual({ x: 1 })
    expect(invocation.finished).toBe(true)
    expect(updates).toEqual(['running', 'running', 'completed'])
  })

  it('sends no body for an action without input', async () => {
    const { thing, received } = serverWith(record('completed'))
    await thing.invokeAction('move_to')
    expect(received[0]!.body).toBeUndefined()
  })

  for (const [why, final, kind, message] of [
    [
      'failed',
      record('error', { error: { title: 'X', detail: 'At its limit.' } }),
      'failed',
      'At its limit.',
    ],
    [
      'found the lock taken',
      record('error', { error: { title: 'GlobalLockBusyError', detail: 'Busy.' } }),
      'lock-busy',
      'Busy.',
    ],
    ['was cancelled', record('cancelled'), 'cancelled', 'The action was cancelled.'],
  ] as const) {
    it(`rejects its output when it ${why}`, async () => {
      const { thing } = serverWith(final)
      const invocation = await thing.invokeAction('move_to')
      const output = invocation.output().catch((error: unknown) => error)
      await vi.advanceTimersByTimeAsync(100)
      expect(await output).toMatchObject({ kind, message })
      expect(await output).toBeInstanceOf(ApiError)
      expect((await invocation.done).status).toBe(final.status)
    })
  }

  it('cancels with DELETE', async () => {
    const { thing, received } = serverWith(record('running'))
    const invocation = await thing.invokeAction('move_to')
    await invocation.cancel()
    expect(received.at(-1)).toMatchObject({ method: 'DELETE', url: new URL(HREF) })
  })

  it('stops following when its signal aborts, leaving the action alone', async () => {
    const { thing, received } = serverWith(record('running'))
    const controller = new AbortController()
    const invocation = await thing.invokeAction('move_to', undefined, { signal: controller.signal })
    await vi.advanceTimersByTimeAsync(100)
    controller.abort()
    await expect(invocation.done).rejects.toMatchObject({ kind: 'cancelled' })
    const polls = received.length
    await vi.advanceTimersByTimeAsync(5000)
    expect(received).toHaveLength(polls)
    expect(received.some((request) => request.method === 'DELETE')).toBe(false)
  })
})

describe('ongoingInvocations', () => {
  it('follows the action’s pending and running invocations', async () => {
    const server = fakeServer({
      'GET /api/v1/stage/move_to': [
        record('completed', { id: '0', href: `${HREF}0` }),
        record('running'),
        record('error', { id: '2', href: `${HREF}2` }),
      ],
      'GET /api/v1/action_invocations/1': record('completed', { output: 5 }),
    })
    const client = new WotClient({ baseUrl: 'http://microscope.test/api/v1/', fetch: server.fetch })
    const ongoing = await client.consume(stage).ongoingInvocations('move_to')
    expect(ongoing.map((invocation) => invocation.record.id)).toEqual(['1'])
    await vi.advanceTimersByTimeAsync(100)
    expect(await ongoing[0]!.output()).toBe(5)
  })
})
