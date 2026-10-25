import type { WotClient } from '@/api/wot/client'
import type { ApiError } from '@/api/wot/errors'

/** Where a shared stream stands: before its first frame, showing frames, or failing (and trying again). */
export type StreamState = 'connecting' | 'live' | 'error'

/** A viewer of a shared stream. */
export interface StreamViewer {
  /** A new frame, decoded. It is closed when this returns, so draw it now. */
  onFrame(frame: ImageBitmap): void
  /** The stream's state: when the viewer joins, and whenever it changes. */
  onState?(state: StreamState, error?: ApiError): void
}

export interface ShareOptions {
  /** The most frames a second to decode and show. */
  maxFps?: number
  /** How long a stream nobody watches stays open, in ms, in case a viewer comes straight back. */
  linger?: number
  /** Decodes a frame. By default, `createImageBitmap`. */
  decode?: (jpeg: Uint8Array<ArrayBuffer>) => Promise<ImageBitmap>
}

/** The default frame-rate cap: above the simulated camera's 10 frames a second. */
export const MAX_FPS = 15
/** The default linger: long enough to switch between two tabs that both show the stream. */
export const LINGER_MS = 1000

const shared = new WeakMap<WotClient, Map<string, SharedStream>>()

/**
 * Watches the MJPEG stream at `url` through `client`. Viewers of the same URL
 * share one request, and each frame is decoded once. The request stops when
 * the last viewer has been gone for `linger`. A viewer that is hidden should
 * leave, and watch again when it shows. The first viewer's options apply to
 * the shared stream. Returns the function that leaves.
 */
export function watchStream(
  client: WotClient,
  url: URL,
  viewer: StreamViewer,
  options: ShareOptions = {},
): () => void {
  let streams = shared.get(client)
  if (!streams) shared.set(client, (streams = new Map()))
  const key = url.href
  let stream = streams.get(key)
  if (!stream) {
    stream = new SharedStream(client, url, options, () => streams.delete(key))
    streams.set(key, stream)
  }
  stream.add(viewer)
  let left = false
  return () => {
    if (!left) stream.remove(viewer)
    left = true
  }
}

async function decodeJpeg(jpeg: Uint8Array<ArrayBuffer>) {
  return createImageBitmap(new Blob([jpeg], { type: 'image/jpeg' }))
}

/** One request, and its viewers. */
class SharedStream {
  private readonly viewers = new Set<StreamViewer>()
  private state: StreamState = 'connecting'
  private error?: ApiError
  private readonly stop: () => void
  private closing?: ReturnType<typeof setTimeout>
  private closed = false
  /** The newest frame not yet shown: older ones are dropped. */
  private latest?: Uint8Array<ArrayBuffer>
  private showing = false
  private shownAt = -Infinity

  constructor(
    client: WotClient,
    url: URL,
    private readonly options: ShareOptions,
    private readonly onClosed: () => void,
  ) {
    this.stop = client.frames(url, (jpeg) => this.receive(jpeg), {
      onError: (error) => this.setState('error', error),
    })
  }

  add(viewer: StreamViewer) {
    clearTimeout(this.closing)
    this.viewers.add(viewer)
    viewer.onState?.(this.state, this.error)
  }

  remove(viewer: StreamViewer) {
    this.viewers.delete(viewer)
    if (this.viewers.size > 0) return
    this.closing = setTimeout(() => {
      this.closed = true
      this.stop()
      this.onClosed()
    }, this.options.linger ?? LINGER_MS)
  }

  private setState(state: StreamState, error?: ApiError) {
    this.state = state
    this.error = error
    for (const viewer of this.viewers) viewer.onState?.(state, error)
  }

  private receive(jpeg: Uint8Array<ArrayBuffer>) {
    if (this.state !== 'live') this.setState('live')
    this.latest = jpeg
    void this.show()
  }

  /** Shows the newest frame, then any newer one, no faster than the cap. */
  private async show() {
    if (this.showing) return
    this.showing = true
    const interval = 1000 / (this.options.maxFps ?? MAX_FPS)
    const decode = this.options.decode ?? decodeJpeg
    try {
      while (this.latest && !this.closed) {
        const wait = this.shownAt + interval - Date.now()
        if (wait > 0) await new Promise((resolve) => setTimeout(resolve, wait))
        const jpeg = this.latest
        this.latest = undefined
        if (this.closed) break
        let frame: ImageBitmap
        try {
          frame = await decode(jpeg)
        } catch {
          continue // A broken frame is skipped.
        }
        if (this.closed) {
          frame.close()
          break
        }
        this.shownAt = Date.now()
        for (const viewer of this.viewers) viewer.onFrame(frame)
        frame.close()
      }
    } finally {
      this.showing = false
    }
  }
}
