// P10.3: the Settings page lists this computer's serial ports, each with the device the
// configuration puts on it, and says which configured port the computer doesn't have.
import { flushPromises, type VueWrapper } from '@vue/test-utils'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { fakeServer } from '../api/fakeServer'
import { mountApp } from '../mountApp'

let wrapper: VueWrapper | undefined

beforeEach(() => {
  const server = fakeServer({
    'GET /api/siq/system/ports': {
      ports: [
        { name: 'COM3', description: 'PCI', device: null },
        { name: 'COM7', description: 'USB 0403:6001 FTDI USB Serial Port', device: 'stage' },
      ],
      missing: [{ name: 'COM9', device: 'focus' }],
      error: null,
    },
  })
  vi.stubGlobal('fetch', server.fetch)
})

afterEach(() => {
  wrapper?.unmount()
  wrapper = undefined
  vi.unstubAllGlobals()
})

describe('the Settings page', () => {
  it('lists the serial ports, and the configured ones missing', async () => {
    const app = await mountApp('/settings')
    wrapper = app.wrapper
    await flushPromises()
    const rows = wrapper
      .findAll('.settings-page__ports tbody tr')
      .map((row) => row.findAll('td').map((cell) => cell.text()))
    expect(rows).toEqual([
      ['COM3', 'PCI', '—'],
      ['COM7', 'USB 0403:6001 FTDI USB Serial Port', 'stage'],
    ])
    expect(wrapper.get('[role="alert"]').text()).toBe(
      "The focus is configured on COM9, which this computer doesn't have.",
    )
  })
})
