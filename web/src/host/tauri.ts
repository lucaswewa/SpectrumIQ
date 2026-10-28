import { invoke } from '@tauri-apps/api/core'
import { getCurrentWindow } from '@tauri-apps/api/window'

import type { HostAdapter, ServiceState } from './types'

/** What the shell says about itself: its `app_info` command. */
interface ShellInfo {
  version: string
  server: string | null
  managesService: boolean
}

/**
 * The host inside the Tauri desktop shell (ADR-0040), through the shell's commands, which its
 * remote capability grants to the local service's origin only. Links open in the system's
 * browser, files save through a dialog, and full screen is the window's. In development builds
 * the shell manages the simulated instrument, which `service` controls.
 */
export async function createTauriHost(): Promise<HostAdapter> {
  const shell = await invoke<ShellInfo>('app_info')
  const shown = getCurrentWindow()
  let fullscreen = await shown.isFullscreen()

  return {
    info: { name: 'SpectrumIQ', version: __APP_VERSION__, kind: 'tauri' },

    openLink(url) {
      void invoke('open_link', { url })
    },

    localSecret: () => invoke<string>('local_secret'),

    async saveFile(name, data) {
      // The bytes go as the request's raw body, not as JSON.
      const bytes = new Uint8Array(await data.arrayBuffer())
      await invoke('save_file', bytes, { headers: { 'x-file-name': encodeURIComponent(name) } })
    },

    fullscreen: {
      active: () => fullscreen,
      async set(on) {
        await shown.setFullscreen(on)
        fullscreen = on
      },
      watch(onChange) {
        // Filling the screen resizes the window: each resize reads it again.
        const listening = shown.onResized(async () => {
          const now = await shown.isFullscreen()
          if (now === fullscreen) return
          fullscreen = now
          onChange(now)
        })
        return () => void listening.then((stop) => stop())
      },
    },

    service: shell.managesService
      ? {
          state: () => invoke<ServiceState>('service_state'),
          start: () => invoke<void>('service_start'),
          stop: () => invoke<void>('service_stop'),
          restart: () => invoke<void>('service_restart'),
        }
      : undefined,
  }
}
