// @vitest-environment node
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { WotClient } from '@/api/wot/client'
import type { ApiError } from '@/api/wot/errors'
import type { ThingDescription } from '@/api/wot/td'

import { fakeServer, jpeg, json, mjpegStream, type Received } from './fakeServer'

beforeEach(() => vi.useFakeTimers())
afterEach(() => vi.useRealTimers())

/** A camera's description, with the links a teta-wot server writes. */
const camera: ThingDescription = {
  title: 'Camera',
  base: 'http://microscope.test/',
  links: [
    { href: '/api/v1/camera/mjpeg_stream', type: 'multipart/x-mixed-replace', rel: 'alternate' },
    { href: '/api/v1/camera/mjpeg_stream/viewer', type: 'text/html', rel: 'alternate' },
    { href: '/api/v1/camera/lores_mjpeg_stream', type: 'multipart/x-mixed-replace' },
  ],
}

function connect(routes: Parameters<typeof fakeServer>[0]) {
  const server = fakeServer(routes)
  const client = new WotClient({
    baseUrl: 'http://microscope.test/api/v1/',
    fetch: server.fetch,
    headers: () => ({ Authorization: 'Bearer test' }),
  })
  return { client, thing: client.consume(camera), received: server.received }
}

describe('streams of frames', () => {
  it('finds a stream through the description’s links', () => {
    const { thing } = connect({})
    expect(thing.streamUrl('mjpeg_stream').href).toBe(
      'http://microscope.test/api/v1/camera/mjpeg_stream',
    )
    expect(thing.streamUrl('lores_mjpeg_stream').pathname).toBe('/api/v1/camera/lores_mjpeg_stream')
    expect(() => thing.streamUrl('viewer')).toThrow('Camera has no stream "viewer".')
  })

  it('delivers each frame, with the client’s headers, until told to stop', async () => {
    const { client, thing, received } = connect({
      'GET /api/v1/camera/mjpeg_stream': (request: Received) =>
        mjpegStream(request, [jpeg(1), jpeg(2)]).response,
    })
    const frames: number[] = []
    const stop = client.frames(thing.streamUrl('mjpeg_stream'), (frame) => frames.push(frame[2]!))
    await vi.advanceTimersByTimeAsync(0)
    expect(frames).toEqual([1, 2])
    expect(received[0]!.headers.get('Accept')).toBe('multipart/x-mixed-replace')
    expect(received[0]!.headers.get('Authorization')).toBe('Bearer test')
    stop()
    expect(received[0]!.signal?.aborted).toBe(true)
  })

  it('reconnects after the stream ends, but not after an answer that isn’t a stream', async () => {
    let attempt = 0
    const { client, thing, received } = connect({
      'GET /api/v1/camera/mjpeg_stream': (request: Received) => {
        attempt += 1
        if (attempt === 3) return json({ not: 'a stream' })
        return mjpegStream(request, [jpeg(attempt)], { open: false }).response
      },
    })
    const frames: number[] = []
    const errors: string[] = []
    client.frames(thing.streamUrl('mjpeg_stream'), (frame) => frames.push(frame[2]!), {
      onError: (error: ApiError) => errors.push(error.message),
    })
    await vi.advanceTimersByTimeAsync(0)
    await vi.advanceTimersByTimeAsync(1000) // attempt 2
    await vi.advanceTimersByTimeAsync(1000) // attempt 3: JSON
    await vi.advanceTimersByTimeAsync(60_000)
    expect(frames).toEqual([1, 2])
    expect(received).toHaveLength(3)
    expect(errors).toEqual([
      'The stream ended.',
      'The stream ended.',
      'The answer is not an MJPEG stream.',
    ])
  })
})
