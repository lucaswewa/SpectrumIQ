import type { ThingDescription } from '@/api/wot/td'

/** A request the fake server received. */
export interface Received {
  url: URL
  method: string
  headers: Headers
  body: string | undefined
  signal: AbortSignal | undefined
}

type Route =
  ((request: Received) => Response | Promise<Response>) | object | string | number | boolean | null

/**
 * A `fetch` that answers from `routes`, keyed by method and path
 * (`GET /api/v1/stage/position`), with 404 for anything else. Each route is a
 * JSON value or a function. Every request is recorded.
 */
export function fakeServer(routes: Record<string, Route>) {
  const received: Received[] = []
  const fetch = async (input: RequestInfo | URL, init: RequestInit = {}) => {
    const request: Received = {
      url: new URL(String(input)),
      method: init.method ?? 'GET',
      headers: new Headers(init.headers),
      body: init.body as string | undefined,
      signal: init.signal ?? undefined,
    }
    received.push(request)
    request.signal?.throwIfAborted()
    const route = routes[`${request.method} ${request.url.pathname}`]
    if (route === undefined) return json({ detail: 'Not Found' }, 404)
    return typeof route === 'function' ? route(request) : json(route)
  }
  return { fetch: fetch as typeof globalThis.fetch, received }
}

export const json = (body: unknown, status = 200) =>
  new Response(JSON.stringify(body), { status, headers: { 'Content-Type': 'application/json' } })

/**
 * A `text/event-stream` answer whose body arrives in `chunks`. It then ends,
 * or with `open`, stays open until the request is aborted, as a real stream
 * does.
 */
export function eventStream(chunks: string[], request: Received, { open = false } = {}) {
  const encoder = new TextEncoder()
  const body = new ReadableStream<Uint8Array>({
    start(controller) {
      for (const chunk of chunks) controller.enqueue(encoder.encode(chunk))
      if (!open) controller.close()
      request.signal?.addEventListener('abort', () => controller.error(request.signal!.reason))
    },
  })
  return new Response(body, { headers: { 'Content-Type': 'text/event-stream' } })
}

/** A small JPEG: start-of-image, `n`, a stuffed 0xFF, then end-of-image. */
export const jpeg = (n: number) => Uint8Array.of(0xff, 0xd8, n, 0xff, 0x00, n, 0xff, 0xd9)

/** One frame of an MJPEG stream, as a teta-wot server writes it. */
export function mjpegPart(frame: Uint8Array) {
  const head = new TextEncoder().encode('--frame\r\nContent-Type: image/jpeg\r\n\r\n')
  const part = new Uint8Array(head.length + frame.length + 2)
  part.set(head)
  part.set(frame, head.length)
  part.set([13, 10], head.length + frame.length)
  return part
}

/**
 * An MJPEG answer that sends `frames`, then each frame given to `push`. It
 * stays open until the request is aborted, or without `open`, ends at once.
 */
export function mjpegStream(request: Received, frames: Uint8Array[] = [], { open = true } = {}) {
  let push!: (frame: Uint8Array) => void
  const body = new ReadableStream<Uint8Array>({
    start(controller) {
      push = (frame) => controller.enqueue(mjpegPart(frame))
      frames.forEach(push)
      if (!open) controller.close()
      request.signal?.addEventListener('abort', () => controller.error(request.signal!.reason))
    },
  })
  const headers = { 'Content-Type': 'multipart/x-mixed-replace; boundary=frame' }
  return { response: new Response(body, { headers }), push }
}

/** A stage's description, with the forms a teta-wot server writes. */
export const stage: ThingDescription = {
  title: 'Stage',
  base: 'http://microscope.test/',
  properties: {
    position: {
      type: 'object',
      readOnly: true,
      observable: true,
      forms: [
        { href: '/api/v1/stage/position', op: ['readproperty'] },
        {
          href: '/api/v1/stage/position',
          op: ['observeproperty', 'unobserveproperty'],
          subprotocol: 'sse',
        },
        { href: 'ws://microscope.test/api/v1/stage/ws', op: ['observeproperty'] },
      ],
    },
    step: {
      type: 'integer',
      forms: [{ href: '/api/v1/stage/step', op: ['readproperty', 'writeproperty'] }],
    },
  },
  actions: {
    move_to: {
      forms: [
        { href: '/api/v1/stage/move_to', op: ['invokeaction'] },
        { href: '/api/v1/action_invocations/{id}', op: ['queryaction', 'cancelaction'] },
      ],
    },
  },
  events: {
    arrived: {
      forms: [
        { href: '/api/v1/stage/arrived', op: ['subscribeevent'], subprotocol: 'sse' },
        { href: 'ws://microscope.test/api/v1/stage/ws', op: ['subscribeevent'] },
      ],
    },
  },
  forms: [
    { href: '/api/v1/stage/properties', op: ['readallproperties'] },
    { href: '/api/v1/stage/properties', op: ['observeallproperties'], subprotocol: 'sse' },
  ],
}
