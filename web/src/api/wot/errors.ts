/** One invalid input in a request, from a 422 response. */
export interface ErrorIssue {
  /** Where in the input, such as `x` or `rows[2]`; empty for the input as a whole. */
  field: string
  message: string
}

/** What an error says, ready to show. */
export interface DescribedError {
  /** A heading: the problem's name, or how many inputs were invalid. */
  title?: string
  message?: string
  issues: ErrorIssue[]
  /** The error as JSON, when it has none of the shapes below. */
  json?: string
}

/**
 * Reads an error in any of the shapes the app meets. The shapes are text, an
 * `Error`, and the response bodies of a `teta-wot` server in its default
 * profile:
 * - `{"detail": "…"}`;
 * - a 422's `{"detail": [{"loc": [...], "msg": "…", "type": "…", …}]}`;
 * - a problem, `{"title", "detail", "status", "type", "instance"}`.
 */
export function describeError(error: unknown): DescribedError {
  if (typeof error === 'string') return { message: error, issues: [] }
  if (error instanceof Error) return { message: error.message, issues: [] }
  if (isRecord(error)) {
    const { detail, title } = error
    if (Array.isArray(detail)) {
      const issues = detail.filter(isRecord).map(toIssue)
      const plural = issues.length === 1 ? '' : 's'
      return { title: `${issues.length} invalid input${plural}`, issues }
    }
    if (typeof detail === 'string' || typeof title === 'string') {
      return {
        title: typeof title === 'string' ? title : undefined,
        message: typeof detail === 'string' ? detail : undefined,
        issues: [],
      }
    }
  }
  return { issues: [], json: JSON.stringify(error, null, 2) ?? String(error) }
}

function toIssue(issue: Record<string, unknown>): ErrorIssue {
  const loc: unknown[] = Array.isArray(issue.loc) ? issue.loc : []
  // The first step says which part of the request: the body, the path or the query.
  const path = ['body', 'path', 'query'].includes(loc[0] as string) ? loc.slice(1) : loc
  const field = path.reduce<string>(
    (text, step) =>
      typeof step === 'number' ? `${text}[${step}]` : text ? `${text}.${step}` : String(step),
    '',
  )
  const message = typeof issue.msg === 'string' ? issue.msg : JSON.stringify(issue)
  return { field, message }
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}

/**
 * What kind of failure an `ApiError` is:
 * - `invalid`: the server refused the input (422, with its validation list);
 * - `lock-busy`: another action holds the microscope's global lock;
 * - `http`: any other error response;
 * - `failed`: an action ran and ended in an error;
 * - `network`: no answer at all (offline, refused, or blocked by CORS);
 * - `cancelled`: the request was aborted, or the action was cancelled.
 */
export type ApiErrorKind = 'invalid' | 'lock-busy' | 'http' | 'failed' | 'network' | 'cancelled'

/** Any failure of the WoT client. `body` is the server's error, for ErrorDetails. */
export class ApiError extends Error {
  override name = 'ApiError'

  constructor(
    readonly kind: ApiErrorKind,
    message: string,
    readonly status?: number,
    readonly body?: unknown,
    options?: ErrorOptions,
  ) {
    super(message, options)
  }

  /** The error for an error response, from its JSON body if it has one. */
  static async fromResponse(response: Response): Promise<ApiError> {
    const text = await response.text().catch(() => '')
    let body: unknown = text || undefined
    try {
      body = JSON.parse(text)
    } catch {
      // Not JSON: keep the text.
    }
    return ApiError.fromBody(body, response.status, response.statusText)
  }

  /**
   * The error for a server's error body, such as an invocation's `error`: of
   * kind `invalid` or `lock-busy` if the body says so, otherwise `kind`.
   */
  static fromBody(body: unknown, status?: number, fallback = 'Error', kind: ApiErrorKind = 'http') {
    const described = describeError(body)
    const message = described.message ?? described.title ?? fallback
    return new ApiError(specificKind(body, status) ?? kind, message, status, body)
  }

  /** The error for a `fetch` that threw: aborted, or no answer. */
  static fromFailure(error: unknown): ApiError {
    if (error instanceof ApiError) return error
    const aborted = error instanceof DOMException && error.name === 'AbortError'
    return aborted
      ? new ApiError('cancelled', 'The request was cancelled.', undefined, undefined, {
          cause: error,
        })
      : new ApiError('network', 'The microscope could not be reached.', undefined, undefined, {
          cause: error,
        })
  }
}

function specificKind(body: unknown, status?: number): ApiErrorKind | undefined {
  if (!isRecord(body)) return undefined
  if (status === 422 && Array.isArray(body.detail)) return 'invalid'
  if (body.title === 'GlobalLockBusyError') return 'lock-busy'
  return undefined
}
