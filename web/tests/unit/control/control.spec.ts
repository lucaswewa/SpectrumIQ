// REQ-SEC-004, REQ-STG-001, REQ-DEV-001, REQ-SEC-002: the stage, the focus and the peripherals on
// the Live page, and STOP in every state (ADR-0039), for anyone but a viewer (ADR-0068).
import { flushPromises, mount, type VueWrapper } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { WotClient } from '@/api/wot/client'
import { useSessionStore } from '@/auth/session'
import LivePage from '@/live/LivePage.vue'
import { useConnectionStore } from '@/stores/connection'

import { fakeServer, json, type Received } from '../api/fakeServer'
import { BASE, describeThing } from '../api/things'
import { inShell } from '../shell'

const ACTIONS = ['stage/move_to', 'stage/move_by', 'stage/jog', 'stage/home', 'stage/halt']
const things = {
  stage: describeThing(
    'stage',
    { position: false, state: false, homed: false, speed: false, limits: false },
    ACTIONS.map((path) => path.split('/')[1]!),
  ),
  focus: describeThing('focus', { position: false, state: false }, ['move_to', 'move_by', 'halt']),
  objective: describeThing(
    'objective',
    { objectives: false, position: false, moving: false, current: false },
    ['select'],
  ),
  workflow: describeThing('workflow', { positions: false }, ['save_position']),
}
const OBJECTIVES = [
  { id: 1, name: '10x/0.30', magnification: 10, numerical_aperture: 0.3, turret_position: 0 },
  {
    id: 3,
    name: '60x/1.40 oil',
    magnification: 60,
    numerical_aperture: 1.4,
    immersion: 'oil',
    turret_position: 2,
    parfocal_offset: -8,
  },
]

let wrapper: VueWrapper | undefined
let posts: { path: string; body: unknown }[]
let state: unknown

class Observer {
  observe() {}
  disconnect() {}
}

beforeEach(() => {
  window.localStorage.clear()
  setActivePinia(createPinia())
  vi.stubGlobal('IntersectionObserver', Observer)
  vi.stubGlobal('ResizeObserver', undefined)
  posts = []
  state = 'idle'
})

afterEach(() => {
  wrapper?.unmount()
  wrapper = undefined
  useConnectionStore().stop()
  vi.unstubAllGlobals()
})

/** The Live page, with a stage, a focus and an objective turret: unless `connect` is false. */
async function showPage({ connect = true } = {}) {
  const post = (request: Received) => {
    posts.push({
      path: request.url.pathname.replace('/api/v1/', ''),
      body: JSON.parse(request.body!),
    })
    const record = { id: '1', status: 'completed', href: `${BASE}api/v1/action_invocations/1` }
    // The server saves where the stage and focus are.
    const saved = { x: 10, y: 20, z: 0, focus: 5 }
    const output = request.url.pathname.includes('move')
      ? { state: 'idle' }
      : request.url.pathname.endsWith('save_position')
        ? [{ name: JSON.parse(request.body!).name, ...saved }]
        : null
    return json({ ...record, output }, 201)
  }
  const server = fakeServer({
    'GET /api/v1/thing_descriptions/': things,
    'GET /api/siq/system/health': { status: 'ok', version: '0.1.0', uptime_s: 1, camera: 'sim' },
    'GET /api/v1/stage/position': { x: 10, y: 20, z: 0 },
    'GET /api/v1/stage/state': () => json(state),
    'GET /api/v1/stage/homed': false,
    'GET /api/v1/stage/limits': {
      min: { x: -1000, y: -1000, z: -100 },
      max: { x: 10, y: 1000, z: 100 },
    },
    'GET /api/v1/focus/position': 5,
    'GET /api/v1/objective/objectives': OBJECTIVES,
    'GET /api/v1/objective/position': 0,
    'GET /api/v1/objective/current': OBJECTIVES[0]!,
    'GET /api/v1/workflow/positions': [],
    ...Object.fromEntries(
      [
        ...ACTIONS,
        'focus/move_to',
        'focus/move_by',
        'focus/halt',
        'objective/select',
        'workflow/save_position',
      ].map((path) => [`POST /api/v1/${path}`, post]),
    ),
  })
  if (connect) {
    const client = new WotClient({
      baseUrl: `${BASE}api/v1/`,
      fetch: server.fetch,
      webSocket: null,
    })
    await useConnectionStore().start(client.baseUrl, client)
  }
  wrapper = mount(inShell(LivePage), { attachTo: document.body })
  await flushPromises()
  return wrapper
}

const halts = () => posts.filter((post) => post.path.endsWith('/halt')).map((post) => post.path)
const press = (key: string, type: 'keydown' | 'keyup' = 'keydown') =>
  document.body.dispatchEvent(new KeyboardEvent(type, { key, bubbles: true, cancelable: true }))
const button = (page: VueWrapper, text: string) =>
  page.findAll('button').find((each) => each.text() === text)!

describe('STOP', () => {
  const STATES = ['idle', 'moving', 'jogging', 'homing', 'holding', { alarm: 9 }, 'disconnected']
  for (const now of STATES) {
    it(`halts the stage and the focus while the stage is ${JSON.stringify(now)}`, async () => {
      state = now
      const page = await showPage()
      const stop = page.get('.live-page__toolbar .stop-button')
      expect(stop.attributes('disabled')).toBeUndefined()
      await stop.trigger('click')
      await flushPromises()
      expect(halts()).toEqual(['stage/halt', 'focus/halt'])
    })
  }

  it('is there, and enabled, without a server', async () => {
    const page = await showPage({ connect: false })
    const stop = page.get('.stop-button')
    expect(stop.attributes('disabled')).toBeUndefined()
    await stop.trigger('click')
    expect(posts).toEqual([])
  })

  it('is disabled for a viewer, with why, and Esc with it', async () => {
    const page = await showPage()
    const me = { name: 'vic', role: 'viewer', local: false, since: '', expires: '' } as const
    useSessionStore().$patch({ enforced: true, checked: true, me: { ...me, holds_token: false } })
    await flushPromises()
    const stop = page.get('.stop-button')
    expect(stop.attributes('disabled')).toBeDefined()
    expect(stop.attributes('title')).toBe('View only: your role is viewer.')
    press('Escape')
    await flushPromises()
    expect(halts()).toEqual([])
  })

  it('is Esc, even while a confirmation is open, and Home is only after one', async () => {
    const page = await showPage()
    expect(page.get('[role="status"]').text()).toBe('Still, not homed')
    await button(page, 'Home…').trigger('click')
    await flushPromises()
    const dialog = document.querySelector<HTMLElement>('[role="alertdialog"], [role="dialog"]')!
    expect(dialog.textContent).toContain('Home the stage?')
    dialog.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }))
    await flushPromises()
    expect(halts()).toEqual(['stage/halt', 'focus/halt'])
    expect(posts.some((post) => post.path === 'stage/home')).toBe(false)

    await button(page, 'Home…').trigger('click')
    await flushPromises()
    const confirm = [...document.querySelectorAll<HTMLElement>('button')].find(
      (each) => each.textContent?.trim() === 'Home',
    )!
    confirm.click()
    await flushPromises()
    expect(posts.at(-1)).toEqual({ path: 'stage/home', body: {} })
  })
})

describe('the stage and the focus', () => {
  it('step with the arrow keys and the d-pad, and focus with the wheel', async () => {
    const page = await showPage()
    expect(page.get('[data-test="position-y"]').text()).toBe('20.0 µm')
    expect(page.get('[data-test="position-focus"]').text()).toBe('5.0 µm')
    press('ArrowUp')
    press('ArrowUp', 'keyup')
    await flushPromises()
    // At the edge of x's travel: nothing to send.
    press('ArrowRight')
    press('ArrowRight', 'keyup')
    await flushPromises()
    await page.get('button[aria-label="Coarse Z down"]').trigger('click', { detail: 0 })
    await flushPromises()
    page
      .get('.live-page__images')
      .element.dispatchEvent(
        new WheelEvent('wheel', { deltaY: -200, bubbles: true, cancelable: true }),
      )
    await flushPromises()
    expect(posts).toEqual([
      { path: 'stage/move_by', body: { dx: 0, dy: -100, dz: 0 } },
      { path: 'stage/move_by', body: { dx: 0, dy: 0, dz: -50 } },
      { path: 'focus/move_by', body: { dz: 2, unidirectional: false } },
    ])
  })

  it('save a position on the server, then go back to it, the stage first', async () => {
    const page = await showPage()
    await button(page, 'Saved positions').trigger('click')
    await flushPromises()
    await button(page, 'Save this position').trigger('click')
    await flushPromises()
    await button(page, 'Go').trigger('click')
    await flushPromises()
    expect(posts).toEqual([
      { path: 'workflow/save_position', body: { name: 'Position 1' } },
      { path: 'stage/move_to', body: { x: 10, y: 20, z: 0 } },
      { path: 'focus/move_to', body: { z: 5, unidirectional: false } },
    ])
  })
})

describe('the objective', () => {
  it('changes only after a confirmation, which gives the parfocal offset', async () => {
    const page = await showPage()
    await button(page, 'Peripherals').trigger('click')
    await flushPromises()
    const choose = async () => {
      await page.get('[role="combobox"]').trigger('keydown', { key: 'Enter' })
      await flushPromises()
      const oil = [...document.querySelectorAll<HTMLElement>('[role="option"]')].find((option) =>
        option.textContent?.includes('60x'),
      )!
      oil.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true }))
      await flushPromises()
      return [...document.querySelectorAll<HTMLElement>('[role="alertdialog"], [role="dialog"]')]
        .map((dialog) => dialog.textContent ?? '')
        .join()
    }
    const asked = await choose()
    expect(asked).toContain('Change to 60x/1.40 oil?')
    expect(asked).toContain('8 µm below')
    expect(asked).toContain('It takes oil.')
    const answer = (text: string) =>
      [...document.querySelectorAll<HTMLElement>('button')]
        .find((each) => each.textContent?.trim() === text)!
        .click()
    answer('Cancel')
    await flushPromises()
    expect(posts).toEqual([])
    await choose()
    answer('Change')
    await flushPromises()
    expect(posts).toEqual([{ path: 'objective/select', body: { position: 2 } }])
  })
})
