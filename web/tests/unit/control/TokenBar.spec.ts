// REQ-SEC-002, REQ-VIS-005: the control token's bar (ADR-0068): View only for a viewer; Take
// control for an operator while it is free, then Release; another's name and since when while
// another holds it, with Take over for an admin and not for an operator; and a control pane
// disabled, with the reason, until this session holds the token.
import { flushPromises, mount, type VueWrapper } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import { h } from 'vue'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { WotClient } from '@/api/wot/client'
import { useSessionStore, type Role } from '@/auth/session'
import TokenBar from '@/control/TokenBar.vue'
import { useTokenStore } from '@/control/token'
import { useConnectionStore } from '@/stores/connection'
import ControlPane from '@/ui/ControlPane.vue'

import { fakeServer, json } from '../api/fakeServer'
import { BASE, describeThing } from '../api/things'
import { inShell } from '../shell'

let wrapper: VueWrapper | undefined
let posted: string[]

beforeEach(() => {
  setActivePinia(createPinia())
  posted = []
})

afterEach(() => {
  wrapper?.unmount()
  useConnectionStore().stop()
  vi.unstubAllGlobals()
})

/** The bar and a control pane, for `role`, while `holder` holds the token. */
async function show(role: Role, holder: string | null) {
  const server = fakeServer({
    'GET /api/v1/thing_descriptions/': {
      control: describeThing('control', { holder: false, since: false }),
    },
    'GET /api/siq/system/health': { status: 'ok', version: '0.1.0', uptime_s: 1 },
    'GET /api/v1/control/holder': holder,
    'GET /api/v1/control/since': holder ? '2026-10-28T08:15:00Z' : null,
    'POST /api/siq/control/request': () => {
      posted.push('request')
      return json({ holder: { user: 'olga' } })
    },
    'POST /api/siq/control/release': () => {
      posted.push('release')
      return new Response(null, { status: 204 })
    },
    'POST /api/siq/control/take_over': () => {
      posted.push('take_over')
      return json({ holder: { user: 'ada' } })
    },
  })
  vi.stubGlobal('fetch', server.fetch)
  const me = { name: 'olga', role, local: false, since: '', expires: '', holds_token: false }
  useSessionStore().$patch({ enforced: true, checked: true, me })
  const client = new WotClient({ baseUrl: `${BASE}api/v1/`, fetch: server.fetch, webSocket: null })
  await useConnectionStore().start(client.baseUrl, client)
  const page = () => [
    h(TokenBar),
    h(ControlPane, { need: 'operate' }, () => h('button', { type: 'button' }, 'Move')),
  ]
  wrapper = mount(inShell({ render: page }), { attachTo: document.body })
  await flushPromises()
  return wrapper
}

const bar = () => document.querySelector('[aria-label="Control of the instrument"]')!
const button = (text: string) =>
  [...bar().querySelectorAll('button')].find((each) => each.textContent?.trim() === text)
const move = () => [...document.querySelectorAll('button')].find((b) => b.textContent === 'Move')!

describe('the control token bar', () => {
  it('says View only to a viewer, and keeps the controls disabled', async () => {
    await show('viewer', null)
    expect(bar().textContent).toContain('View only.')
    expect(button('Take control')).toBeUndefined()
    expect(move().closest('fieldset')!.disabled).toBe(true)
    expect(document.body.textContent).toContain('View only: your role is viewer.')
  })

  it('lets an operator take control while it is free, then release it', async () => {
    await show('operator', null)
    expect(bar().textContent).toContain('No one has control.')
    expect(move().closest('fieldset')!.disabled).toBe(true)
    button('Take control')!.click()
    await flushPromises()
    expect(posted).toEqual(['request'])
    expect(useTokenStore().holds).toBe(true)
    expect(bar().textContent).toContain('You have control of the instrument.')
    expect(move().closest('fieldset')!.disabled).toBe(false)
    button('Release control')!.click()
    await flushPromises()
    expect(posted).toEqual(['request', 'release'])
    expect(move().closest('fieldset')!.disabled).toBe(true)
  })

  it('names another holder, and offers an admin alone to take over', async () => {
    await show('operator', 'otto')
    expect(bar().textContent).toMatch(/otto has control since .+: you see what they do\./)
    expect(button('Take over')).toBeUndefined()
    wrapper?.unmount()
    useConnectionStore().stop()

    setActivePinia(createPinia())
    await show('admin', 'otto')
    button('Take over')!.click()
    await flushPromises()
    expect(posted).toEqual(['take_over'])
  })
})
