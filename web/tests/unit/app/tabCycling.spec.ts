import { flushPromises, type VueWrapper } from '@vue/test-utils'
import { afterEach, describe, expect, it } from 'vitest'

import { thingAvailabilityKey } from '@/app/availability'

import { mountApp } from '../mountApp'

const press = async (
  key: string,
  options: KeyboardEventInit = {},
  target: EventTarget = window,
) => {
  target.dispatchEvent(new KeyboardEvent('keydown', { key, bubbles: true, ...options }))
  await flushPromises()
}

describe('switching tabs from the keyboard', () => {
  let wrapper: VueWrapper | undefined

  afterEach(() => {
    wrapper?.unmount()
    wrapper = undefined
    document.body.innerHTML = ''
  })

  it('goes to the next and previous destination with Shift+↓ and Shift+↑', async () => {
    const app = await mountApp('/live')
    wrapper = app.wrapper
    await press('ArrowDown', { shiftKey: true })
    expect(app.router.currentRoute.value.name).toBe('mosaic')
    await press('ArrowUp', { shiftKey: true })
    expect(app.router.currentRoute.value.name).toBe('live')
  })

  it('wraps around at both ends', async () => {
    const app = await mountApp('/live')
    wrapper = app.wrapper
    await press('ArrowUp', { shiftKey: true })
    expect(app.router.currentRoute.value.name).toBe('about')
    await press('ArrowDown', { shiftKey: true })
    expect(app.router.currentRoute.value.name).toBe('live')
  })

  it('skips hidden destinations', async () => {
    const app = await mountApp('/live', { [thingAvailabilityKey]: () => false })
    wrapper = app.wrapper
    await press('ArrowDown', { shiftKey: true })
    expect(app.router.currentRoute.value.name).toBe('settings')
  })

  it('leaves arrows alone without Shift, with other modifiers, or while typing', async () => {
    const app = await mountApp('/live')
    wrapper = app.wrapper
    await press('ArrowDown')
    await press('ArrowDown', { shiftKey: true, ctrlKey: true })
    const input = document.createElement('input')
    document.body.append(input)
    await press('ArrowDown', { shiftKey: true }, input)
    expect(app.router.currentRoute.value.name).toBe('live')
  })
})
