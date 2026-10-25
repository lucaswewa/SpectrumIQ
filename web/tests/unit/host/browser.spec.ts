import { afterEach, describe, expect, it, vi } from 'vitest'

import { createBrowserHost, REVOKE_AFTER_MS } from '@/host/browser'

describe('the browser host', () => {
  afterEach(() => {
    vi.restoreAllMocks()
    vi.useRealTimers()
  })

  it('describes the web app', () => {
    const { info, service } = createBrowserHost()
    expect(info.name).toBe('SpectrumIQ')
    expect(info.kind).toBe('browser')
    expect(info.version).toBe(__APP_VERSION__)
    expect(info.version).toMatch(/^\d+\.\d+\.\d+/)
    expect(service).toBeUndefined()
  })

  it('opens links in a new tab that has no access to the app', () => {
    const open = vi.spyOn(window, 'open').mockReturnValue(null)
    createBrowserHost().openLink('https://example.org/')
    expect(open).toHaveBeenCalledWith('https://example.org/', '_blank', 'noopener,noreferrer')
  })

  it('saves files as downloads, and releases them afterwards', async () => {
    vi.useFakeTimers()
    const createObjectURL = vi.spyOn(window.URL, 'createObjectURL').mockReturnValue('blob:capture')
    const revokeObjectURL = vi.spyOn(window.URL, 'revokeObjectURL').mockImplementation(() => {})
    const clicked: HTMLAnchorElement[] = []
    vi.spyOn(HTMLAnchorElement.prototype, 'click').mockImplementation(function (
      this: HTMLAnchorElement,
    ) {
      clicked.push(this)
    })
    const data = new Blob(['pixels'], { type: 'image/jpeg' })

    await createBrowserHost().saveFile('capture.jpeg', data)

    expect(createObjectURL).toHaveBeenCalledWith(data)
    expect(clicked).toHaveLength(1)
    expect(clicked[0]?.download).toBe('capture.jpeg')
    expect(clicked[0]?.href).toBe('blob:capture')
    expect(document.querySelector('a[download]')).toBeNull()

    vi.advanceTimersByTime(REVOKE_AFTER_MS - 1)
    expect(revokeObjectURL).not.toHaveBeenCalled()
    vi.advanceTimersByTime(1)
    expect(revokeObjectURL).toHaveBeenCalledWith('blob:capture')
  })

  it('fills the screen, leaves it, and says when it changes', async () => {
    let element: Element | null = null
    // jsdom has no full screen: the test gives the document one.
    Object.defineProperty(document, 'fullscreenElement', { configurable: true, get: () => element })
    const enter = vi.fn(async () => {
      element = document.documentElement
      document.dispatchEvent(new Event('fullscreenchange'))
    })
    const leave = vi.fn(async () => {
      element = null
      document.dispatchEvent(new Event('fullscreenchange'))
    })
    document.documentElement.requestFullscreen = enter
    document.exitFullscreen = leave
    const { fullscreen } = createBrowserHost()
    const seen: boolean[] = []
    const stop = fullscreen.watch((on) => seen.push(on))

    await fullscreen.set(true)
    await fullscreen.set(true) // already: nothing to do
    expect(fullscreen.active()).toBe(true)
    await fullscreen.set(false)
    stop()
    await fullscreen.set(true)
    expect([enter.mock.calls.length, leave.mock.calls.length, seen]).toEqual([2, 1, [true, false]])
    Reflect.deleteProperty(document, 'fullscreenElement')
  })
})
