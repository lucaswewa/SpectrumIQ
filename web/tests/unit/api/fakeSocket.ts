import type { WebSocketLike } from '@/api/wot/ws'

/** A WebSocket that a test drives: it opens, receives and drops when told to. */
export class FakeSocket implements WebSocketLike {
  readyState = 0
  onopen: ((event: Event) => void) | null = null
  onmessage: ((event: MessageEvent) => void) | null = null
  onclose: ((event: CloseEvent) => void) | null = null
  onerror: ((event: Event) => void) | null = null
  /** What the app sent, as JSON. */
  readonly sent: unknown[] = []
  closed = false

  constructor(readonly url: string) {}

  send(data: string) {
    this.sent.push(JSON.parse(data))
  }

  close() {
    this.closed = true
    this.readyState = 3
  }

  /** The server accepts the connection. */
  open() {
    this.readyState = 1
    this.onopen?.(new Event('open'))
  }

  /** The server sends `message`. */
  receive(message: unknown) {
    this.onmessage?.(new MessageEvent('message', { data: JSON.stringify(message) }))
  }

  /** The connection drops. */
  drop() {
    this.readyState = 3
    this.onclose?.(new CloseEvent('close'))
  }
}

/** A socket factory that keeps every socket it opens. */
export function fakeSockets() {
  const opened: FakeSocket[] = []
  return {
    opened,
    open: (url: string) => {
      const socket = new FakeSocket(url)
      opened.push(socket)
      return socket
    },
  }
}
