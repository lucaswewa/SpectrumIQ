import { ApiError } from './errors'
import { Invocation, sleep, type FollowOptions, type InvocationRecord } from './invocation'
import { byteChunks, multipartBoundary, parseMultipart } from './mjpeg'
import { parseEventStream, textChunks, type SseMessage } from './sse'
import { formUrl, type Form, type ThingDescription } from './td'

export interface ClientOptions {
  /**
   * The server's API root: `/api/v1/` on the server that served the page, or
   * a full URL such as `http://lab-pc:5000/api/v1/`.
   */
  baseUrl: string | URL
  /** Headers for every request, such as an `Authorization` header. Unused in milestone 1. */
  headers?: () => Record<string, string>
  /** Replaces the global `fetch`, for tests. */
  fetch?: typeof fetch
}

export interface RequestOptions {
  signal?: AbortSignal
}

export interface StreamOptions {
  /** Stops the stream, as calling the returned function does. */
  signal?: AbortSignal
  /** Called when the stream fails or drops, before each attempt to reconnect. */
  onError?: (error: ApiError) => void
}

/**
 * A client for a `teta-wot` server (ADR-0014). It finds every URL through
 * the Things' descriptions, and sends JSON with `fetch`, server-sent events
 * included, so a header hook covers every request.
 */
export class WotClient {
  readonly baseUrl: URL

  constructor(private readonly options: ClientOptions) {
    this.baseUrl = new URL(options.baseUrl, globalThis.location?.href)
  }

  /** Every Thing's description, by the Thing's name. */
  async thingDescriptions(options: RequestOptions = {}) {
    const url = new URL('thing_descriptions/', this.baseUrl)
    return (await this.request(url, options)) as Record<string, ThingDescription>
  }

  /** Reads and writes a Thing's affordances, as its description says. */
  consume(td: ThingDescription): ConsumedThing {
    return new ConsumedThing(this, td)
  }

  /**
   * Sends a request and returns its JSON answer (null for none), or throws an
   * ApiError. `body`, if given, is sent as JSON, whatever its value.
   */
  async request(
    url: URL,
    init: { method?: string; body?: unknown; signal?: AbortSignal } = {},
  ): Promise<unknown> {
    const headers: Record<string, string> = { Accept: 'application/json', ...this.headers() }
    if ('body' in init) headers['Content-Type'] = 'application/json'
    const response = await this.fetch(url, {
      method: init.method ?? 'GET',
      headers,
      body: 'body' in init ? JSON.stringify(init.body) : undefined,
      signal: init.signal,
    })
    if (!response.ok) throw await ApiError.fromResponse(response)
    const text = await response.text()
    return text ? JSON.parse(text) : null
  }

  /** Downloads `url`, such as an action's file output, or throws an ApiError. */
  async download(url: URL, init: { signal?: AbortSignal } = {}): Promise<Blob> {
    const response = await this.fetch(url, { headers: this.headers(), signal: init.signal })
    if (!response.ok) throw await ApiError.fromResponse(response)
    return response.blob()
  }

  /**
   * Follows a server-sent event stream, calling `onMessage` with each event,
   * until the returned function is called or `signal` aborts. A stream that
   * drops, or a server that can't be reached or answers with a 5xx, is
   * retried after 1 s, then 2, 4, … up to 30 s; any other error response
   * ends it.
   */
  stream(url: URL, onMessage: (message: SseMessage) => void, options: StreamOptions = {}) {
    return this.follow(url, 'text/event-stream', options, async (body) => {
      for await (const message of parseEventStream(textChunks(body))) onMessage(message)
    })
  }

  /**
   * Follows an MJPEG stream (`multipart/x-mixed-replace`), calling `onFrame`
   * with each frame's bytes, until the returned function is called or
   * `signal` aborts. It reconnects as `stream` does.
   */
  frames(url: URL, onFrame: (jpeg: Uint8Array<ArrayBuffer>) => void, options: StreamOptions = {}) {
    return this.follow(url, 'multipart/x-mixed-replace', options, async (body, response) => {
      const boundary = multipartBoundary(response.headers.get('Content-Type'))
      if (!boundary) {
        throw new ApiError('http', 'The answer is not an MJPEG stream.', response.status)
      }
      for await (const frame of parseMultipart(byteChunks(body), boundary)) onFrame(frame)
    })
  }

  /** Requests `url` and reads its answer with `read`, reconnecting as `stream` describes. */
  private follow(
    url: URL,
    accept: string,
    options: StreamOptions,
    read: (body: ReadableStream<Uint8Array>, response: Response) => Promise<void>,
  ) {
    const controller = new AbortController()
    options.signal?.addEventListener('abort', () => controller.abort(), { once: true })
    const signal = controller.signal

    const follow = async () => {
      let delay = 1000
      while (!signal.aborted) {
        try {
          const response = await this.fetch(url, {
            headers: { Accept: accept, ...this.headers() },
            signal,
          })
          if (!response.ok || !response.body) throw await ApiError.fromResponse(response)
          delay = 1000
          await read(response.body, response)
          throw new ApiError('network', 'The stream ended.')
        } catch (failure) {
          if (signal.aborted) return
          const error = ApiError.fromFailure(failure)
          options.onError?.(error)
          if (error.status !== undefined && error.status < 500) return
        }
        await sleep(delay, signal).catch(() => {})
        delay = Math.min(delay * 2, 30_000)
      }
    }
    void follow()
    return () => controller.abort()
  }

  private headers(): Record<string, string> {
    return this.options.headers?.() ?? {}
  }

  private async fetch(url: URL, init: RequestInit): Promise<Response> {
    try {
      // Called without a receiver: the global fetch refuses any other `this`.
      return await (this.options.fetch ?? globalThis.fetch)(url, init)
    } catch (error) {
      throw ApiError.fromFailure(error)
    }
  }
}

/** One Thing on the server, used through its description. */
export class ConsumedThing {
  constructor(
    private readonly client: WotClient,
    readonly td: ThingDescription,
  ) {}

  async readProperty<T = unknown>(name: string, options: RequestOptions = {}): Promise<T> {
    return (await this.client.request(this.propertyUrl(name, 'readproperty'), options)) as T
  }

  async writeProperty(name: string, value: unknown, options: RequestOptions = {}) {
    const url = this.propertyUrl(name, 'writeproperty')
    await this.client.request(url, { method: 'PUT', body: value, ...options })
  }

  /** Resets a writable property to its default. */
  async resetProperty(name: string, options: RequestOptions = {}) {
    const url = this.propertyUrl(name, 'writeproperty')
    url.pathname += '/reset'
    await this.client.request(url, { method: 'POST', ...options })
  }

  /** Starts an action, and returns its invocation, which follows it to the end. */
  async invokeAction(name: string, input?: unknown, options: FollowOptions = {}) {
    const url = this.actionUrl(name)
    const body = input === undefined ? {} : { body: input }
    const record = await this.client.request(url, {
      method: 'POST',
      signal: options.signal,
      ...body,
    })
    return this.follow(record as InvocationRecord, options)
  }

  /** The action's invocations that are still pending or running, followed. */
  async ongoingInvocations(name: string, options: FollowOptions = {}) {
    const records = (await this.client.request(this.actionUrl(name), options)) as InvocationRecord[]
    return records
      .filter((record) => record.status === 'pending' || record.status === 'running')
      .map((record) => this.follow(record, options))
  }

  /** Calls `onValue` with each new value of an observable property. Returns a function that stops. */
  observeProperty(name: string, onValue: (value: unknown) => void, options?: StreamOptions) {
    const url = this.url(this.td.properties?.[name]?.forms, 'observeproperty', `property "${name}"`)
    return this.client.stream(url, (message) => onValue(JSON.parse(message.data)), options)
  }

  /** Calls `onChange` with the name and new value of any observable property that changes. */
  observeProperties(onChange: (name: string, value: unknown) => void, options?: StreamOptions) {
    const url = this.url(this.td.forms, 'observeallproperties', 'its properties')
    return this.client.stream(
      url,
      (message) => onChange(message.event, JSON.parse(message.data)),
      options,
    )
  }

  /** The URL of the Thing's MJPEG stream `name`, from its description's links. */
  streamUrl(name: string): URL {
    for (const link of this.td.links ?? []) {
      if (link.type?.split(';')[0]?.trim() !== 'multipart/x-mixed-replace') continue
      const url = new URL(link.href, this.td.base ?? this.client.baseUrl)
      if (url.pathname.split('/').pop() === name) return url
    }
    throw new Error(`${this.td.title} has no stream "${name}".`)
  }

  /** Calls `onEvent` with the data of each of the event's occurrences. */
  subscribeEvent(name: string, onEvent: (data: unknown) => void, options?: StreamOptions) {
    const url = this.url(this.td.events?.[name]?.forms, 'subscribeevent', `event "${name}"`)
    return this.client.stream(url, (message) => onEvent(JSON.parse(message.data)), options)
  }

  private follow(record: InvocationRecord, options: FollowOptions) {
    const request = (url: URL, method: 'GET' | 'DELETE', signal?: AbortSignal) =>
      this.client.request(url, { method, signal })
    return new Invocation(record, request, options)
  }

  private propertyUrl(name: string, op: string) {
    return this.url(this.td.properties?.[name]?.forms, op, `property "${name}"`, false)
  }

  private actionUrl(name: string) {
    return this.url(this.td.actions?.[name]?.forms, 'invokeaction', `action "${name}"`, false)
  }

  /** The URL of the form for `op`, an SSE form unless `sse` is false. */
  private url(forms: Form[] | undefined, op: string, what: string, sse = true): URL {
    const url = formUrl(forms, op, this.td.base ?? this.client.baseUrl, { sse })
    if (!url) throw new Error(`${this.td.title} has no ${op} form for ${what}.`)
    return url
  }
}
