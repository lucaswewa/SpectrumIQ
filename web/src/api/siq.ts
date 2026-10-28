import { ApiError } from './wot/errors'

/**
 * A request to the server's own routes, `/api/siq/<path>` beside the page's API, with the session's
 * cookie: its JSON answer (null for none), or an ApiError. `body`, if given, is sent as JSON.
 */
export async function siq(path: string, init: { method?: string; body?: unknown } = {}) {
  const url = new URL(`/api/siq/${path}`, globalThis.location?.href)
  const response = await fetch(url, {
    method: init.method ?? ('body' in init ? 'POST' : 'GET'),
    headers: { Accept: 'application/json', 'Content-Type': 'application/json' },
    body: 'body' in init ? JSON.stringify(init.body) : undefined,
    credentials: 'same-origin',
  })
  if (!response.ok) throw await ApiError.fromResponse(response)
  return response.status === 204 ? null : ((await response.json()) as unknown)
}
