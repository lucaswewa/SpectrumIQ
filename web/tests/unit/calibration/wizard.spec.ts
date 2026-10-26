// The calibration wizard's state machine and the Calibration page (ADR-0051), against a fake
// server: the first-run prompt for the objective in use, Back, Next and Skip, a calibration
// accepted or run again, no second prompt, closing early, and the page's own tasks.
// REQ-CAL-001, REQ-CAL-002.
import { flushPromises, type VueWrapper } from '@vue/test-utils'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { WotClient } from '@/api/wot/client'
import { allNeeded, neededFor, summary } from '@/calibration/tasks'
import { useConnectionStore } from '@/stores/connection'

import { fakeServer, json, type Received } from '../api/fakeServer'
import { BASE, describeThing } from '../api/things'
import { mountApp } from '../mountApp'

const descriptions = {
  calibration: describeThing('calibration', { status: false }, [
    'stage_camera',
    'capture_dark',
    'capture_flat',
  ]),
  objective: describeThing(
    'objective',
    { objectives: false, position: false, current: false, moving: false },
    ['select'],
  ),
  imaging: describeThing('imaging', { channel: false, channels: false }, ['select_channel']),
}

const OBJECTIVES = [
  { id: 1, name: '10x/0.30', magnification: 10, numerical_aperture: 0.3, turret_position: 0 },
  { id: 2, name: '20x/0.75', magnification: 20, numerical_aperture: 0.75, turret_position: 1 },
].map((objective) => ({ ...objective, parfocal_offset: objective.id === 2 ? 12.5 : 0 }))

const STAGE_CAMERA = {
  id: 'c1',
  objective: 1,
  created: '2026-10-25T12:00:00Z',
  affine: [
    [2.667, -0.016],
    [0.018, 2.667],
  ],
  px_size_um: 0.37496,
  rotation_deg: 0.391,
  scale: [2.667, 2.6669],
  shear: 0.0009,
  residual_rms_um: 0.87,
  residual_max_um: 1.34,
  backlash_um: [2.19, 2.55],
  moves: 16,
  method: 'stage_camera',
}
const shading = (kind: 'dark' | 'flat', channel: number | null) => ({
  id: `${kind}-${channel}`,
  kind,
  objective: 1,
  channel,
  created: '2026-10-25T12:00:00Z',
  method: 'capture',
  frames: kind === 'dark' ? 16 : 18,
  exposure_ms: 10,
  binning: { horizontal: 1, vertical: 1 },
  level: 100.1,
  flat: null,
  range: kind === 'flat' ? [0.868, 1.078] : null,
})

let status: {
  objective: number
  name: string
  calibration: typeof STAGE_CAMERA | null
  shading: { channel: number | null; name: string; dark: unknown; flat: unknown }[]
}[]
let received: Received[]
let wrapper: VueWrapper | undefined

/** A finished invocation, as teta-wot answers a POST that completed at once. */
const finished = (output: unknown) =>
  json({ id: 'i', status: 'completed', href: `${BASE}api/v1/action_invocations/i`, output }, 201)

beforeEach(() => {
  vi.useFakeTimers()
  status = [
    {
      objective: 1,
      name: '10x/0.30',
      calibration: null,
      shading: [
        { channel: 1, name: 'Brightfield', dark: null, flat: null },
        { channel: null, name: 'no channel', dark: null, flat: null },
      ],
    },
    {
      objective: 2,
      name: '20x/0.75',
      calibration: { ...STAGE_CAMERA, objective: 2 },
      shading: [{ channel: 1, name: 'Brightfield', dark: null, flat: shading('flat', 1) }],
    },
  ]
  const brightfield = () => status[0]!.shading[0]!
  const server = fakeServer({
    'GET /api/v1/thing_descriptions/': descriptions,
    'GET /api/siq/system/health': { status: 'ok', version: 'dev', uptime_s: 1, camera: 'fake' },
    'GET /api/v1/calibration/status': () => json(status),
    'GET /api/v1/objective/objectives': OBJECTIVES,
    'GET /api/v1/objective/position': 0,
    'GET /api/v1/objective/current': OBJECTIVES[0]!,
    'GET /api/v1/objective/moving': false,
    'GET /api/v1/imaging/channel': 1,
    'GET /api/v1/imaging/channels': [{ id: 1, name: 'Brightfield' }],
    'POST /api/v1/calibration/stage_camera': () => {
      status[0]!.calibration = STAGE_CAMERA
      return finished(STAGE_CAMERA)
    },
    'POST /api/v1/calibration/capture_dark': () =>
      finished((brightfield().dark = shading('dark', 1))),
    'POST /api/v1/calibration/capture_flat': () =>
      finished((brightfield().flat = shading('flat', 1))),
  })
  received = server.received
  vi.stubGlobal('fetch', server.fetch)
})

afterEach(() => {
  if (wrapper) useConnectionStore().stop()
  wrapper?.unmount()
  wrapper = undefined
  vi.unstubAllGlobals()
  vi.useRealTimers()
})

async function settle() {
  await vi.advanceTimersByTimeAsync(0)
  await flushPromises()
}

async function open(path: string) {
  ;({ wrapper } = await mountApp(path))
  const client = new WotClient({ baseUrl: `${BASE}api/v1/`, webSocket: null })
  await useConnectionStore().start(client.baseUrl, client)
  await settle()
}

const wizard = () => document.querySelector<HTMLElement>('[role="dialog"]')
const title = () => wizard()?.querySelector('h2')?.textContent?.trim()
/** The wizard's Back and Next buttons. */
const footer = () =>
  [...wizard()!.querySelectorAll('footer button')].map((b) => b.textContent!.trim())
const posted = (action: string) =>
  received.filter((r) => r.method === 'POST' && r.url.pathname.endsWith(`/${action}`)).length

/** Presses the button that says `label`, or failing that, is labelled it. */
async function press(label: string, within: HTMLElement | null = wizard()) {
  const buttons = [...within!.querySelectorAll('button')]
  const button =
    buttons.find((b) => b.textContent!.trim() === label) ??
    buttons.find((b) => b.getAttribute('aria-label') === label)
  if (!button) throw new Error(`No ${label} button`)
  button.click()
  await settle()
}

describe('the calibration tasks', () => {
  it("are what an objective lacks: its stage–camera calibration, then its channels' flats", () => {
    const titles = (entry: (typeof status)[number]) =>
      neededFor(entry as never).map((task) => task.title)
    expect(titles(status[0]!)).toEqual([
      'Stage–camera, 10x/0.30',
      'Dark and flat, 10x/0.30, Brightfield',
    ])
    expect(titles(status[1]!)).toEqual([])
    // Without channels configured, the frames taken with none.
    const bare = { ...status[0]!, shading: [status[0]!.shading[1]!] }
    expect(titles(bare)).toContain('Dark and flat, 10x/0.30, no channel')
    expect(allNeeded(status as never)).toHaveLength(2)
  })

  it('summarise what a calibration found', () => {
    expect(summary('stage_camera', STAGE_CAMERA as never)).toContainEqual([
      'Residuals',
      '0.87 µm RMS, 1.34 µm at most',
    ])
    expect(summary('flat', shading('flat', 1) as never)).toEqual([
      ['Light', '87 % to 108 % of the mean, from 18 frames'],
    ])
  })
})

describe('the calibration wizard', () => {
  it('opens for the objective in use, and goes through what it lacks', async () => {
    await open('/live')
    expect(title()).toBe('Welcome to SpectrumIQ')
    expect(footer()).toEqual(['Next'])

    await press('Next')
    expect(title()).toBe('Calibration: Stage–camera, 10x/0.30')
    expect(wizard()!.textContent).toContain('10x/0.30 is in place.')
    expect(footer()).toEqual(['Back', 'Next'])
    await press('Next')
    await press('Next')
    // Until it has run, Next skips it.
    expect(footer()).toEqual(['Back', 'Skip'])
    await press('Calibrate')
    expect(posted('stage_camera')).toBe(1)
    expect(wizard()!.textContent).toContain('0.3750 µm')
    expect(footer()).toEqual(['Back', 'Next'])
    // Run again, then accept.
    await press('Again')
    expect(posted('stage_camera')).toBe(2)

    await press('Next')
    expect(title()).toBe('Calibration: Dark and flat, 10x/0.30, Brightfield')
    expect(wizard()!.textContent).toContain('Brightfield is in use.')
    await press('Next')
    await press('Take the dark')
    expect(wizard()!.textContent).toContain('100.1 ADU, from 16 frames')
    await press('Next')
    await press('Next')
    expect(footer()).toEqual(['Back', 'Skip'])
    await press('Skip')
    expect(title()).toBe('Calibration')
    expect(footer()).toEqual(['Back', 'Finish'])
    await press('Back')
    await press('Take the flat')
    await press('Next')
    await press('Finish')
    expect(wizard()).toBeNull()
    expect([posted('capture_dark'), posted('capture_flat')]).toEqual([1, 1])
  })

  it("doesn't open with nothing missing, nor again for the same objective", async () => {
    status[0]!.calibration = STAGE_CAMERA
    status[0]!.shading[0]!.flat = shading('flat', 1)
    await open('/live')
    expect(wizard()).toBeNull()

    status[0]!.calibration = null
    await vi.advanceTimersByTimeAsync(10_000)
    await settle()
    expect(wizard()).toBeNull()
  })

  it('asks before closing early', async () => {
    await open('/live')
    wizard()!.dispatchEvent(
      new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true }),
    )
    await settle()
    const question = () => document.querySelector<HTMLElement>('[role="alertdialog"]')
    expect(question()!.textContent).toContain('Close the calibration wizard?')
    await press('Keep calibrating', question())
    expect(title()).toBe('Welcome to SpectrumIQ')
    await press('Close')
    await press('Close the wizard', question())
    expect(wizard()).toBeNull()
  })
})

describe('the Calibration page', () => {
  it('shows each objective and channel, and opens a task for any of them', async () => {
    await open('/calibration')
    await press('Close')
    await press('Close the wizard', document.querySelector<HTMLElement>('[role="alertdialog"]'))
    const page = wrapper!.element as HTMLElement
    const rows = [...page.querySelectorAll('tbody tr')].map((row) => row.textContent!)
    expect(rows[0]).toContain('Not calibrated')
    expect(rows[0]).toContain('Blocked')
    expect(rows[1]).toContain('12.5 µm')
    expect(rows[1]).toContain('Ready')
    expect(page.textContent).toContain('10x/0.30 has no stage–camera calibration')
    expect(rows.at(-1)).toContain('87 % to 108 % of the mean')

    await press('Calibrate again', page)
    expect(title()).toBe('Calibration: Stage–camera, 20x/0.75')
    // 20x/0.75 isn't at the turret: the step turns it there.
    await press('Turn to 20x/0.75')
    const turned = received.find((r) => r.url.pathname.endsWith('/objective/select'))
    expect(JSON.parse(turned!.body!)).toEqual({ position: 1 })
  })
})
