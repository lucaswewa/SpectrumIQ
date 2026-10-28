// A fake of the server's own routes for the admin's development page: the users, added, changed
// and removed; an audit trail of 60 entries, filtered and paged as the server does; and the control
// token, which this page's session always gets. For development only (`/#/dev/admin`).
import type { Role } from '@/auth/session'

interface User {
  name: string
  role: Role
  created: string
}

const DAY = 86_400_000
const START = Date.UTC(2026, 9, 28, 8)

export const USERS: User[] = (
  [
    ['ada', 'admin'],
    ['olga', 'operator'],
    ['vic', 'viewer'],
    ['console', 'operator'],
  ] as const
).map(([name, role], index) => ({
  name,
  role,
  created: new Date(START - (10 - index) * DAY).toISOString(),
}))

/** The trail's entries, newest first: requests, refusals, the token's changes, logins, jobs. */
const TRAIL = Array.from({ length: 60 }, (_, index) => {
  const user = ['olga', 'vic', 'ada'][index % 3]!
  const [kind, detail]: [string, Record<string, unknown>] =
    index % 10 === 9
      ? ['job', { kind: 'stitch', state: 'done' }]
      : user === 'vic'
        ? ['refused', { method: 'PUT', path: '/api/v1/camera/exposure_ms', status: 403 }]
        : index % 4 === 0
          ? ['token', { reason: { kind: 'requested' }, holder: user }]
          : ['request', { method: 'POST', path: '/api/v1/stage/move_to', status: 201 }]
  const time = new Date(START - index * 90_000).toISOString()
  return { seq: 60 - index, time, kind, user, address: '127.0.0.1', detail }
})

const json = (body: unknown, status = 200) =>
  new Response(JSON.stringify(body), { status, headers: { 'Content-Type': 'application/json' } })
const nothing = () => new Response(null, { status: 204 })

/**
 * A `fetch` that answers for the users, the audit and the token as the server does, and passes
 * the rest to `real`.
 */
export function adminFetch(real: typeof fetch): typeof fetch {
  return async (input, init = {}) => {
    const url = new URL(String(input), location.href)
    const path = url.pathname.match(/^\/api\/siq\/(.+)$/)?.[1]
    if (!path) return real(input, init)
    const method = init.method ?? 'GET'
    const body = init.body ? (JSON.parse(String(init.body)) as Record<string, string>) : {}
    if (path === 'users' && method === 'GET') return json(USERS)
    if (path === 'users') {
      if (USERS.some((user) => user.name === body.name))
        return json({ detail: 'a user of that name exists' }, 409)
      const user = { name: body.name!, role: body.role as Role, created: new Date().toISOString() }
      USERS.push(user)
      return json(user, 201)
    }
    const named = path.match(/^users\/(.+)$/)?.[1]
    if (named) {
      const at = USERS.findIndex((user) => user.name === decodeURIComponent(named))
      if (at < 0) return json({ detail: 'no such user' }, 404)
      if (method === 'DELETE') return (USERS.splice(at, 1), nothing())
      if (body.role) USERS[at]!.role = body.role as Role
      return json(USERS[at])
    }
    if (path === 'audit') {
      const [user, kind] = [url.searchParams.get('user'), url.searchParams.get('kind')]
      const kept = TRAIL.filter((e) => (!user || e.user === user) && (!kind || e.kind === kind))
      const offset = Number(url.searchParams.get('offset') ?? 0)
      const limit = Number(url.searchParams.get('limit') ?? 100)
      return json({ entries: kept.slice(offset, offset + limit), total: kept.length })
    }
    if (path === 'control/release' || path === 'control/heartbeat') return nothing()
    if (path.startsWith('control/')) return json({ holder: { user: 'you' } })
    return real(input, init)
  }
}
