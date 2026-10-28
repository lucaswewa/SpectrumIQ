// REQ-SEC-001, REQ-SEC-003: the admin's page (ADR-0068), against a fake server: the users and a
// page of the audit trail, filtered by who; a user added; a new password asked for in a dialog,
// not in the clear; a removal that asks first; and the page told to be an admin's to others.
import { flushPromises, type VueWrapper } from '@vue/test-utils'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { useSessionStore } from '@/auth/session'

import { fakeServer, json, type Received } from '../api/fakeServer'
import { mountApp } from '../mountApp'

const USERS = [
  { name: 'ada', role: 'admin', created: '2026-10-20T09:00:00Z' },
  { name: 'olga', role: 'operator', created: '2026-10-21T09:00:00Z' },
]
const entry = (seq: number) => ({
  seq,
  time: '2026-10-28T08:00:00Z',
  kind: 'request',
  user: 'olga',
  address: '127.0.0.1',
  detail: { method: 'PUT', path: '/api/v1/camera/exposure_ms', status: 204 },
})

let received: Received[]
let wrapper: VueWrapper | undefined

beforeEach(() => {
  const server = fakeServer({
    'GET /api/siq/users': USERS,
    'POST /api/siq/users': (request) => json(JSON.parse(request.body!), 201),
    'PUT /api/siq/users/olga': (request) => json({ ...USERS[1], ...JSON.parse(request.body!) }),
    'DELETE /api/siq/users/olga': () => new Response(null, { status: 204 }),
    'GET /api/siq/audit': { entries: [entry(2), entry(1)], total: 60 },
  })
  received = server.received
  vi.stubGlobal('fetch', server.fetch)
})

afterEach(() => {
  wrapper?.unmount()
  wrapper = undefined
  vi.unstubAllGlobals()
})

async function open() {
  ;({ wrapper } = await mountApp('/admin'))
  await flushPromises()
}

const sent = (method: string) =>
  received.filter((r) => r.method === method).map((r) => `${r.url.pathname} ${r.body ?? ''}`)
const rows = (label: string) =>
  [...document.querySelectorAll(`table[aria-label="${label}"] tbody tr`)].map((row) =>
    row.querySelector('td')!.textContent!.trim(),
  )
const question = () => document.querySelector<HTMLElement>('[role="alertdialog"]')

/** Presses the button that says `label` within `within`. */
async function press(label: string, within: ParentNode = document) {
  const button = [...within.querySelectorAll('button')].find((b) => b.textContent!.trim() === label)
  if (!button) throw new Error(`No ${label} button`)
  button.click()
  await flushPromises()
}
const row = (user: string) =>
  [...document.querySelectorAll('table[aria-label="Users"] tbody tr')].find(
    (each) => each.querySelector('td')!.textContent!.trim() === user,
  )!

function type(input: HTMLInputElement, value: string) {
  input.value = value
  input.dispatchEvent(new Event('input'))
}

describe('the admin page', () => {
  it('lists the users and a page of the audit trail, filtered by who', async () => {
    await open()
    expect(rows('Users')).toEqual(['ada', 'olga'])
    expect(rows('Audit trail')).toHaveLength(2)
    expect(document.body.textContent).toContain('PUT /api/v1/camera/exposure_ms → 204')
    const audit = () => received.filter((r) => r.url.pathname === '/api/siq/audit').at(-1)!
    expect(audit().url.searchParams.toString()).toBe('offset=0&limit=25')

    const who = document.querySelector<HTMLInputElement>('section[aria-label="Audit trail"] input')!
    type(who, 'olga')
    expect(audit().url.searchParams.get('user')).toBeNull()
    who.dispatchEvent(new Event('change'))
    await flushPromises()
    expect(audit().url.searchParams.get('user')).toBe('olga')
  })

  it('adds a user', async () => {
    await open()
    const form = document.querySelector<HTMLFormElement>('form[aria-label="Add a user"]')!
    const [name, password] = form.querySelectorAll('input')
    type(name!, 'vic')
    type(password!, 'correct horse')
    expect(password!.type).toBe('password')
    await flushPromises()
    form.dispatchEvent(new Event('submit'))
    await flushPromises()
    expect(sent('POST')).toEqual([
      '/api/siq/users {"name":"vic","password":"correct horse","role":"viewer"}',
    ])
  })

  it('asks for a new password in a dialog, not in the clear', async () => {
    await open()
    await press('New password', row('olga'))
    const input = question()!.querySelector('input')!
    expect(input.type).toBe('password')
    type(input, 'battery staple')
    await flushPromises()
    await press('Change', question()!)
    expect(sent('PUT')).toEqual(['/api/siq/users/olga {"password":"battery staple"}'])
  })

  it('removes a user only once asked', async () => {
    await open()
    await press('Remove', row('olga'))
    expect(question()!.textContent).toContain('Remove olga?')
    await press('Cancel', question()!)
    expect(sent('DELETE')).toEqual([])
    await press('Remove', row('olga'))
    await press('Remove', question()!)
    expect(sent('DELETE')).toEqual(['/api/siq/users/olga '])
  })

  it('tells anyone else it is an admin’s', async () => {
    await open()
    useSessionStore().$patch({
      enforced: true,
      me: {
        name: 'olga',
        role: 'operator',
        local: false,
        since: '',
        expires: '',
        holds_token: false,
      },
    })
    await flushPromises()
    expect(document.body.textContent).toContain('Only an admin can do this.')
    expect(document.querySelector('table[aria-label="Users"]')).toBeNull()
  })
})
