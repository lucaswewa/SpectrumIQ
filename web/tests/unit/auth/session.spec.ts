// REQ-SEC-001, REQ-SEC-002: the session (ADR-0068): a login asked for only where the server
// enforces one; the local console's secret logging the desktop in; and what each role may do,
// with the control token or without, as the reason a control is disabled.
import { createPinia, setActivePinia } from 'pinia'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { useSessionStore, type Role } from '@/auth/session'
import { createBrowserHost } from '@/host/browser'

import { fakeServer, json, type Received } from '../api/fakeServer'

const me = (name: string, role: Role, local = false) => ({
  name,
  role,
  local,
  since: '2026-10-28T08:00:00Z',
  expires: '2026-10-28T09:00:00Z',
  holds_token: false,
})
let sent: Received[]

function serve(enforced: boolean) {
  const server = fakeServer({
    'GET /api/siq/auth/me': () => json({ detail: 'not logged in', enforced }, 401),
    'POST /api/siq/auth/login': (request) => {
      const { name, password } = JSON.parse(request.body!) as { name: string; password: string }
      return password === 'correct horse'
        ? json(me(name, name === 'ada' ? 'admin' : name === 'vic' ? 'viewer' : 'operator'))
        : json({ detail: 'the name or the password is wrong' }, 401)
    },
    'POST /api/siq/auth/local': () => json(me('console', 'operator', true)),
    'POST /api/siq/auth/logout': () => new Response(null, { status: 204 }),
  })
  sent = server.received
  vi.stubGlobal('fetch', server.fetch)
}

beforeEach(() => setActivePinia(createPinia()))
afterEach(() => vi.unstubAllGlobals())

describe('the session', () => {
  it('asks for a login only where the server enforces one', async () => {
    serve(true)
    const session = useSessionStore()
    await session.start(createBrowserHost())
    expect([session.checked, session.admitted]).toEqual([true, false])
    expect(session.gate('operate', false)).toBe('Log in first.')

    serve(false)
    await session.check()
    expect(session.admitted).toBe(true)
    expect(session.gate('admin', false)).toBeUndefined()
  })

  it('logs in, refuses a wrong password, and logs out', async () => {
    serve(true)
    const session = useSessionStore()
    await session.check()
    await expect(session.login('olga', 'wrong')).rejects.toThrow(
      'the name or the password is wrong',
    )
    await session.login('olga', 'correct horse')
    expect([session.me?.role, session.admitted]).toEqual(['operator', true])
    await session.logout()
    expect(session.admitted).toBe(false)
    expect(sent.map((r) => `${r.method} ${r.url.pathname}`)).toContain('POST /api/siq/auth/logout')
    // A 401 elsewhere ends it as well.
    await session.login('olga', 'correct horse')
    session.expire()
    expect(session.me).toBeUndefined()
  })

  it('logs the desktop in by the local secret', async () => {
    serve(true)
    const session = useSessionStore()
    const host = { ...createBrowserHost(), localSecret: () => Promise.resolve('ab'.repeat(32)) }
    await session.start(host)
    expect([session.me?.name, session.me?.local]).toEqual(['console', true])
    const asked = sent.find((r) => r.url.pathname === '/api/siq/auth/local')!
    expect(JSON.parse(asked.body!)).toEqual({ secret: 'ab'.repeat(32) })
  })

  it('says why a control is disabled, by role and token', async () => {
    serve(true)
    const session = useSessionStore()
    await session.check()
    await session.login('vic', 'correct horse')
    expect(session.gate('process', false)).toBe('View only: your role is viewer.')
    await session.login('olga', 'correct horse')
    expect(session.gate('process', false)).toBeUndefined()
    expect(session.gate('operate', false)).toBe('Take control to change the instrument.')
    expect(session.gate('operate', true)).toBeUndefined()
    expect(session.gate('admin', true)).toBe('Only an admin can do this.')
    await session.login('ada', 'correct horse')
    expect(session.gate('admin', false)).toBeUndefined()
    expect(session.gate('operate', false)).toBe('Take control to change the instrument.')
  })
})
