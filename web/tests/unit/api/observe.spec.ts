// @vitest-environment node
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { WotClient } from '@/api/wot/client'
import type { ApiError } from '@/api/wot/errors'

import { eventStream, fakeServer, json, stage, type Received } from './fakeServer'

beforeEach(() => vi.useFakeTimers())
afterEach(() => vi.useRealTimers())

function connect(routes: Parameters<typeof fakeServer>[0]) {
  const server = fakeServer(routes)
  const client = new WotClient({ baseUrl: 'http://microscope.test/api/v1/', fetch: server.fetch })
  return { thing: client.consume(stage), received: server.received }
}

describe('observation', () => {
  it('delivers each value of a property, even split between chunks', async () => {
    const { thing, received } = connect({
      'GET /api/v1/stage/position': (request: Received) =>
        eventStream(['data: {"x":', ' 1}\n\nda', 'ta: {"x": 2}\n\n'], request, { open: true }),
    })
    const values: unknown[] = []
    const stop = thing.observeProperty('position', (value) => values.push(value))
    await vi.advanceTimersByTimeAsync(0)
    expect(values).toEqual([{ x: 1 }, { x: 2 }])
    expect(received[0]!.headers.get('Accept')).toBe('text/event-stream')
    stop()
    expect(received[0]!.signal?.aborted).toBe(true)
  })

  it('delivers every property by name, and an event’s data', async () => {
    const { thing } = connect({
      'GET /api/v1/stage/properties': (request: Received) =>
        eventStream(['event: position\ndata: 1\nid: a\n\nevent: step\ndata: 5\n\n'], request, {
          open: true,
        }),
      'GET /api/v1/stage/arrived': (request: Received) =>
        eventStream(['data: {"x": 3}\n\n'], request, { open: true }),
    })
    const changes: [string, unknown][] = []
    const events: unknown[] = []
    const signal = new AbortController()
    thing.observeProperties((name, value) => changes.push([name, value]), { signal: signal.signal })
    thing.subscribeEvent('arrived', (data) => events.push(data), { signal: signal.signal })
    await vi.advanceTimersByTimeAsync(0)
    expect(changes).toEqual([
      ['position', 1],
      ['step', 5],
    ])
    expect(events).toEqual([{ x: 3 }])
    signal.abort()
  })

  it('reconnects after the stream drops or the server fails, waiting longer each time', async () => {
    let attempt = 0
    const { thing, received } = connect({
      'GET /api/v1/stage/position': (request: Received) => {
        attempt += 1
        if (attempt === 2) return json({ detail: 'Restarting' }, 503)
        if (attempt === 3) throw new TypeError('Failed to fetch')
        return eventStream([`data: ${attempt}\n\n`], request, { open: attempt === 4 })
      },
    })
    const values: unknown[] = []
    const errors: string[] = []
    const stop = thing.observeProperty('position', (value) => values.push(value), {
      onError: (error: ApiError) => errors.push(error.kind),
    })
    await vi.advanceTimersByTimeAsync(0) // attempt 1: a value, then the stream ends
    expect(values).toEqual([1])
    await vi.advanceTimersByTimeAsync(1000) // attempt 2: 503
    await vi.advanceTimersByTimeAsync(1999)
    expect(received).toHaveLength(2)
    await vi.advanceTimersByTimeAsync(1) // attempt 3, 2 s later: no answer
    await vi.advanceTimersByTimeAsync(4000) // attempt 4, 4 s later: connected
    expect(values).toEqual([1, 4])
    expect(errors).toEqual(['network', 'http', 'network'])
    stop()
  })

  it('gives up on an error that retrying won’t fix', async () => {
    const { thing, received } = connect({
      'GET /api/v1/stage/position': () => json({ detail: 'Not observable' }, 403),
    })
    const errors: number[] = []
    thing.observeProperty('position', () => {}, { onError: (error) => errors.push(error.status!) })
    await vi.advanceTimersByTimeAsync(60_000)
    expect(received).toHaveLength(1)
    expect(errors).toEqual([403])
  })
})
