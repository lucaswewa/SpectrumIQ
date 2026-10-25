import { ApiError } from './errors'

export type InvocationStatus = 'pending' | 'running' | 'completed' | 'error' | 'cancelled'

/** One record of an invocation's log. */
export interface InvocationLogRecord {
  message: string
  levelname: string
  levelno: number
  filename: string
  lineno: number
  created: string
  exception_type: string | null
  traceback: string | null
}

/** An invocation as a `teta-wot` server reports it, in its default wire profile. */
export interface InvocationRecord {
  id: string
  status: InvocationStatus
  /** The action's path, such as `/api/v1/stage/move_to`. */
  action: string
  href: string
  timeRequested: string
  timeStarted: string | null
  timeCompleted: string | null
  /** These four are in a single invocation's record, not in lists. */
  input?: unknown
  output?: unknown
  log?: InvocationLogRecord[]
  error?: unknown
}

export interface FollowOptions {
  /** Stops following (it doesn't cancel the action): `done` rejects as cancelled. */
  signal?: AbortSignal
  /** Called with each new record while it is followed. */
  onUpdate?: (record: InvocationRecord) => void
}

/** Sends a request to an invocation's URL and returns the JSON answer, or throws an ApiError. */
export type InvocationRequest = (
  url: URL,
  method: 'GET' | 'DELETE',
  signal?: AbortSignal,
) => Promise<unknown>

const FINISHED: readonly InvocationStatus[] = ['completed', 'error', 'cancelled']

/**
 * An action running on the server. It is followed by polling, 100 ms after
 * the last answer at first and slowing by half each time to every second,
 * until it finishes.
 */
export class Invocation {
  /** The latest record. */
  record: InvocationRecord
  /** The final record. Rejects if following fails or is stopped. */
  readonly done: Promise<InvocationRecord>

  constructor(
    record: InvocationRecord,
    private readonly request: InvocationRequest,
    options: FollowOptions = {},
  ) {
    this.record = record
    this.done = this.follow(options)
    // `done` is often never awaited; don't report its rejection as unhandled.
    this.done.catch(() => {})
  }

  get finished(): boolean {
    return FINISHED.includes(this.record.status)
  }

  private async follow({ signal, onUpdate }: FollowOptions): Promise<InvocationRecord> {
    let delay = 100
    try {
      while (!this.finished) {
        await sleep(delay, signal)
        delay = Math.min(delay * 1.5, 1000)
        this.record = (await this.request(
          new URL(this.record.href),
          'GET',
          signal,
        )) as InvocationRecord
        onUpdate?.(this.record)
      }
      return this.record
    } catch (error) {
      throw ApiError.fromFailure(error)
    }
  }

  /**
   * The output, once it has completed. Rejects with an ApiError if it didn't:
   * `cancelled`, or `failed` (`lock-busy` if the global lock was taken).
   */
  async output<T = unknown>(): Promise<T> {
    const record = await this.done
    if (record.status === 'completed') return record.output as T
    if (record.status === 'cancelled') throw new ApiError('cancelled', 'The action was cancelled.')
    throw ApiError.fromBody(record.error, undefined, 'The action failed.', 'failed')
  }

  /** Asks the server to cancel it. Actions stop where they check, so `done` says when it ends. */
  async cancel(): Promise<void> {
    await this.request(new URL(this.record.href), 'DELETE')
  }
}

/** Waits `ms` milliseconds, or rejects with the signal's reason if it aborts first. */
export function sleep(ms: number, signal?: AbortSignal): Promise<void> {
  return new Promise((resolve, reject) => {
    if (signal?.aborted) return reject(signal.reason)
    const onAbort = () => {
      clearTimeout(timer)
      reject(signal!.reason)
    }
    const timer = setTimeout(() => {
      signal?.removeEventListener('abort', onAbort)
      resolve()
    }, ms)
    signal?.addEventListener('abort', onAbort, { once: true })
  })
}
