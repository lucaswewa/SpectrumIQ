import { ApiError } from './errors'

/** The part of the browser's `WebSocket` that a `ThingSocket` uses. */
export interface WebSocketLike {
  readonly readyState: number
  onopen: ((event: Event) => void) | null
  onmessage: ((event: MessageEvent) => void) | null
  onclose: ((event: CloseEvent) => void) | null
  onerror: ((event: Event) => void) | null
  send(data: string): void
  close(code?: number, reason?: string): void
}

/** Opens a socket to `url`: the browser's `WebSocket`, or a test's. */
export type SocketFactory = (url: string) => WebSocketLike

/** What a socket subscription follows: a property's values, or an event's occurrences. */
export type Kind = 'property' | 'event'

/** One subscriber to a name: its callback, and where its errors go. */
interface Listener {
  onValue: (value: unknown) => void
  onError?: (error: ApiError) => void
}

const OPEN = 1
const FIRST_RETRY_MS = 1000
const MAX_RETRY_MS = 30_000

/** The `messageType` that subscribes to a kind of name (teta-wot's `{thing}/ws`). */
const SUBSCRIBE: Record<Kind, string> = {
  property: 'addPropertyObservation',
  event: 'addEventSubscription',
}

/**
 * One Thing's observation socket (ADR-0037): every property observation and event
 * subscription of the Thing shares it, so one connection serves them all.
 *
 * - It opens with the first subscriber, and closes with the last.
 * - Each (re)connection subscribes to every name with subscribers.
 * - A socket that closes or fails is reopened after 1 s, then 2, 4, … up to 30 s, as the
 *   fetch streams are (`WotClient.stream`).
 * - The server answers a name it can't follow with an error response, which goes to that
 *   name's subscribers.
 *
 * The protocol has no unsubscribe: values for a name without subscribers are dropped.
 */
export class ThingSocket {
  private readonly listeners: Record<Kind, Map<string, Set<Listener>>> = {
    property: new Map(),
    event: new Map(),
  }
  private socket: WebSocketLike | undefined
  private retry: ReturnType<typeof setTimeout> | undefined
  private delay = FIRST_RETRY_MS

  constructor(
    readonly url: string,
    private readonly open: SocketFactory,
  ) {}

  /** Follows `name`, calling `onValue` with each value or event's data. Returns a function that stops. */
  subscribe(kind: Kind, name: string, listener: Listener): () => void {
    const named = this.listeners[kind]
    const isNew = !named.has(name)
    const set = named.get(name) ?? new Set<Listener>()
    set.add(listener)
    named.set(name, set)
    if (this.socket === undefined && this.retry === undefined) this.connect()
    else if (isNew && this.socket?.readyState === OPEN) this.send(kind, [name])
    return () => {
      set.delete(listener)
      if (set.size === 0) named.delete(name)
      if (this.listeners.property.size + this.listeners.event.size === 0) this.stop()
    }
  }

  /** Whether the socket is open now. */
  get connected(): boolean {
    return this.socket?.readyState === OPEN
  }

  private connect() {
    this.retry = undefined
    const socket = this.open(this.url)
    this.socket = socket
    socket.onopen = () => {
      this.delay = FIRST_RETRY_MS
      for (const kind of ['property', 'event'] as const) {
        const names = [...this.listeners[kind].keys()]
        if (names.length > 0) this.send(kind, names)
      }
    }
    socket.onmessage = (event) => this.receive(String(event.data))
    socket.onerror = () => {}
    socket.onclose = () => {
      if (this.socket !== socket) return
      this.socket = undefined
      this.fail(new ApiError('network', `The observation socket ${this.url} closed.`))
      this.retry = setTimeout(() => this.connect(), this.delay)
      this.delay = Math.min(this.delay * 2, MAX_RETRY_MS)
    }
  }

  private send(kind: Kind, names: string[]) {
    const data = Object.fromEntries(names.map((name) => [name, true]))
    this.socket?.send(JSON.stringify({ messageType: SUBSCRIBE[kind], data }))
  }

  private receive(text: string) {
    let message: {
      messageType?: string
      data?: Record<string, unknown>
      name?: string
      error?: unknown
    }
    try {
      message = JSON.parse(text)
    } catch {
      return
    }
    if (message.messageType === 'propertyStatus' || message.messageType === 'event') {
      const kind: Kind = message.messageType === 'event' ? 'event' : 'property'
      for (const [name, value] of Object.entries(message.data ?? {})) {
        // An event's data comes wrapped with its time: `{data, timestamp}`.
        const payload = kind === 'event' ? (value as { data?: unknown })?.data : value
        for (const listener of this.listeners[kind].get(name) ?? []) listener.onValue(payload)
      }
    } else if (message.messageType === 'response' && message.name !== undefined) {
      const problem = (message.error ?? {}) as { status?: string; detail?: string }
      const status = Number(problem.status) || undefined
      const error = new ApiError('http', problem.detail ?? 'Refused.', status, message.error)
      for (const kind of ['property', 'event'] as const) {
        for (const listener of this.listeners[kind].get(message.name) ?? [])
          listener.onError?.(error)
      }
    }
  }

  private fail(error: ApiError) {
    for (const named of Object.values(this.listeners)) {
      for (const set of named.values()) for (const listener of set) listener.onError?.(error)
    }
  }

  private stop() {
    clearTimeout(this.retry)
    this.retry = undefined
    const socket = this.socket
    this.socket = undefined
    socket?.close(1000, 'no more observations')
  }
}
