// @vitest-environment node
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { WotClient } from '@/api/wot/client'
import { watchStream, type StreamState, type StreamViewer } from '@/live/streams'

import { fakeServer, jpeg, json, mjpegStream, type Received } from '../api/fakeServer'

beforeEach(() => vi.useFakeTimers())
afterEach(() => vi.useRealTimers())

const URL_ = new URL('http://microscope.test/api/v1/camera/mjpeg_stream')

/** A camera whose stream sends what `send` is given, and the requests it got. */
function camera(answer?: (attempt: number) => Response | undefined) {
  const requests: Received[] = []
  let push: (frame: Uint8Array) => void = () => {}
  const server = fakeServer({
    'GET /api/v1/camera/mjpeg_stream': (request: Received) => {
      requests.push(request)
      const stream = mjpegStream(request)
      push = stream.push
      return answer?.(requests.length) ?? stream.response
    },
  })
  const client = new WotClient({ baseUrl: 'http://microscope.test/api/v1/', fetch: server.fetch })
  return { client, requests, send: (n: number) => push(jpeg(n)) }
}

/** A stand-in for createImageBitmap: a "bitmap" that remembers its frame's number. */
type Bitmap = ImageBitmap & { n: number; close: ReturnType<typeof vi.fn> }
const decoded: number[] = []
async function decode(frame: Uint8Array) {
  decoded.push(frame[2]!)
  return { n: frame[2]!, close: vi.fn() } as unknown as Bitmap
}

/** A viewer that records what it sees. */
function viewer() {
  const seen = { frames: [] as Bitmap[], states: [] as StreamState[] }
  const watcher: StreamViewer = {
    onFrame: (frame) => {
      const bitmap = frame as Bitmap
      expect(bitmap.close).not.toHaveBeenCalled()
      seen.frames.push(bitmap)
    },
    onState: (state) => seen.states.push(state),
  }
  return { watcher, seen, numbers: () => seen.frames.map((frame) => frame.n) }
}

beforeEach(() => (decoded.length = 0))

describe('shared streams', () => {
  it('viewers of one URL share one request, and each frame is decoded once', async () => {
    const { client, requests, send } = camera()
    const [a, b] = [viewer(), viewer()]
    watchStream(client, URL_, a.watcher, { decode })
    watchStream(client, URL_, b.watcher, { decode })
    await vi.advanceTimersByTimeAsync(0)
    send(1)
    await vi.advanceTimersByTimeAsync(0)
    expect(requests).toHaveLength(1)
    expect(decoded).toEqual([1])
    expect(a.numbers()).toEqual([1])
    expect(b.seen.frames[0]).toBe(a.seen.frames[0])
    // Drawn, then closed.
    expect(a.seen.frames[0]!.close).toHaveBeenCalledOnce()
    expect(a.seen.states).toEqual(['connecting', 'live'])
  })

  it('stops the request once the last viewer has been gone a while, unless one returns', async () => {
    const { client, requests } = camera()
    const leaveA = watchStream(client, URL_, viewer().watcher, { decode })
    await vi.advanceTimersByTimeAsync(0)
    leaveA()
    leaveA() // Leaving twice is harmless.
    await vi.advanceTimersByTimeAsync(900)
    const leaveB = watchStream(client, URL_, viewer().watcher, { decode })
    await vi.advanceTimersByTimeAsync(5000)
    expect(requests).toHaveLength(1)
    expect(requests[0]!.signal?.aborted).toBe(false)

    leaveB()
    await vi.advanceTimersByTimeAsync(1000)
    expect(requests[0]!.signal?.aborted).toBe(true)
    watchStream(client, URL_, viewer().watcher, { decode })
    await vi.advanceTimersByTimeAsync(0)
    expect(requests).toHaveLength(2)
  })

  it('keeps only the newest frame while one decodes', async () => {
    const { client, send } = camera()
    const pending: (() => void)[] = []
    const slowDecode = (frame: Uint8Array) =>
      new Promise<ImageBitmap>((resolve) => {
        decoded.push(frame[2]!)
        pending.push(() => resolve({ n: frame[2]!, close: vi.fn() } as unknown as Bitmap))
      })
    const a = viewer()
    watchStream(client, URL_, a.watcher, { decode: slowDecode, maxFps: 1000 })
    await vi.advanceTimersByTimeAsync(0)
    for (const n of [1, 2, 3, 4]) send(n)
    await vi.advanceTimersByTimeAsync(0)
    expect(decoded).toEqual([1])
    pending.shift()!()
    await vi.advanceTimersByTimeAsync(5)
    pending.shift()!()
    await vi.advanceTimersByTimeAsync(5)
    expect(decoded).toEqual([1, 4])
    expect(a.numbers()).toEqual([1, 4])
  })

  it('caps the frame rate, always showing the newest frame', async () => {
    const { client, send } = camera()
    const a = viewer()
    watchStream(client, URL_, a.watcher, { decode, maxFps: 10 })
    await vi.advanceTimersByTimeAsync(0)
    // 50 frames a second for a second.
    for (let n = 1; n <= 50; n++) {
      send(n)
      await vi.advanceTimersByTimeAsync(20)
    }
    await vi.advanceTimersByTimeAsync(100)
    expect(a.seen.frames.length).toBeGreaterThanOrEqual(9)
    expect(a.seen.frames.length).toBeLessThanOrEqual(11)
    expect(a.numbers().at(-1)).toBe(50)
  })

  it('skips a frame that won’t decode', async () => {
    const { client, send } = camera()
    const a = viewer()
    // Frame 2 arrives while frame 1 is still failing to decode.
    const brittle = async (frame: Uint8Array) => {
      if (frame[2] !== 1) return decode(frame)
      await new Promise((resolve) => setTimeout(resolve, 5))
      throw new Error('Corrupt JPEG')
    }
    watchStream(client, URL_, a.watcher, { decode: brittle, maxFps: 1000 })
    await vi.advanceTimersByTimeAsync(0)
    send(1)
    send(2)
    await vi.advanceTimersByTimeAsync(20)
    expect(a.numbers()).toEqual([2])
  })

  it('tells viewers when it fails and comes back, and late viewers where it stands', async () => {
    const { client, send } = camera((attempt) =>
      attempt === 1 ? json({ detail: 'Starting' }, 503) : undefined,
    )
    const a = viewer()
    watchStream(client, URL_, a.watcher, { decode })
    await vi.advanceTimersByTimeAsync(0)
    expect(a.seen.states).toEqual(['connecting', 'error'])
    const late = viewer()
    watchStream(client, URL_, late.watcher, { decode })
    expect(late.seen.states).toEqual(['error'])
    await vi.advanceTimersByTimeAsync(1000) // reconnected
    send(7)
    await vi.advanceTimersByTimeAsync(0)
    expect(a.seen.states).toEqual(['connecting', 'error', 'live'])
    expect(late.numbers()).toEqual([7])
  })
})
