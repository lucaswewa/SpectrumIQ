// REQ-VIS-005, REQ-NET-001: the web app's host inside the Tauri desktop shell, with
// `@tauri-apps/api` mocked (ADR-0040).
import { invoke } from '@tauri-apps/api/core'
import { getCurrentWindow } from '@tauri-apps/api/window'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { createHost } from '@/host'
import { createTauriHost } from '@/host/tauri'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))
vi.mock('@tauri-apps/api/window', () => ({ getCurrentWindow: vi.fn() }))

/** The shell, as its commands answer: managing the service or not. */
function shell({ managesService = false, fullscreen = false } = {}) {
  const state = { fullscreen, resized: undefined as (() => Promise<void>) | undefined }
  const unlisten = vi.fn()
  const shown = {
    isFullscreen: vi.fn(async () => state.fullscreen),
    setFullscreen: vi.fn(async (on: boolean) => void (state.fullscreen = on)),
    onResized: vi.fn(async (handler: () => Promise<void>) => {
      state.resized = handler
      return unlisten
    }),
  }
  vi.mocked(getCurrentWindow).mockReturnValue(shown as never)
  vi.mocked(invoke).mockImplementation(async (command) => {
    if (command === 'app_info') {
      return { version: '0.1.0', server: 'http://127.0.0.1:5000/', managesService }
    }
    return command === 'service_state' ? 'running' : undefined
  })
  return { state, shown, unlisten }
}

// A block, since a function returned from beforeEach is run after the test.
beforeEach(() => {
  vi.mocked(invoke).mockReset()
})
afterEach(() => vi.unstubAllGlobals())

describe('the Tauri host', () => {
  it('describes the web app in the shell, and opens links through it', async () => {
    shell()
    const host = await createTauriHost()
    expect(host.info).toEqual({ name: 'SpectrumIQ', version: __APP_VERSION__, kind: 'tauri' })
    host.openLink('https://example.org/')
    expect(invoke).toHaveBeenLastCalledWith('open_link', { url: 'https://example.org/' })
  })

  it('saves a file through the shell, as raw bytes with its name in a header', async () => {
    shell()
    const host = await createTauriHost()
    await host.saveFile('snap 1.jpeg', new Blob([Uint8Array.of(0xff, 0xd8, 0xff, 0xd9)]))
    expect(invoke).toHaveBeenLastCalledWith('save_file', Uint8Array.of(0xff, 0xd8, 0xff, 0xd9), {
      headers: { 'x-file-name': 'snap%201.jpeg' },
    })
  })

  it('fills the screen with the window, and follows it', async () => {
    const { state, shown, unlisten } = shell({ fullscreen: true })
    const host = await createTauriHost()
    expect(host.fullscreen.active()).toBe(true)
    await host.fullscreen.set(false)
    expect(shown.setFullscreen).toHaveBeenCalledWith(false)
    expect(host.fullscreen.active()).toBe(false)

    const changes: boolean[] = []
    const stop = host.fullscreen.watch((on) => changes.push(on))
    await vi.waitFor(() => expect(state.resized).toBeDefined())
    await state.resized!() // a resize that changes nothing
    state.fullscreen = true // F11, or the system's own control
    await state.resized!()
    expect(changes).toEqual([true])
    expect(host.fullscreen.active()).toBe(true)
    stop()
    await vi.waitFor(() => expect(unlisten).toHaveBeenCalled())
  })

  it('controls the service only where the shell manages one', async () => {
    shell()
    expect((await createTauriHost()).service).toBeUndefined()
    shell({ managesService: true })
    const service = (await createTauriHost()).service!
    expect(await service.state()).toBe('running')
    await service.start()
    await service.restart()
    await service.stop()
    const commands = vi.mocked(invoke).mock.calls.map(([command]) => command)
    expect(commands.slice(-4)).toEqual([
      'service_state',
      'service_start',
      'service_restart',
      'service_stop',
    ])
  })

  it('is the host chosen inside the shell, and the browser’s elsewhere', async () => {
    shell()
    expect((await createHost()).info.kind).toBe('browser')
    vi.stubGlobal('__TAURI_INTERNALS__', {})
    expect((await createHost()).info.kind).toBe('tauri')
  })

  it('runs the app as in a browser if the shell refuses its commands', async () => {
    // As for a page the shell's capability doesn't cover.
    vi.stubGlobal('__TAURI_INTERNALS__', {})
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {})
    vi.mocked(invoke).mockImplementation(async () => {
      throw new Error('app_info not allowed')
    })
    expect((await createHost()).info.kind).toBe('browser')
    expect(warn).toHaveBeenCalledOnce()
  })
})
