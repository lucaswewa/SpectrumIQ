import type { HostAdapter } from './types'

/**
 * How long a saved file's object URL is kept after the download starts.
 * Revoking it at once can cancel the download in some browsers.
 */
export const REVOKE_AFTER_MS = 10_000

/** The host for a browser tab. It doesn't manage the SpectrumIQ service. */
export function createBrowserHost(win: Window & typeof globalThis = window): HostAdapter {
  return {
    info: { name: 'SpectrumIQ', version: __APP_VERSION__, kind: 'browser' },

    openLink(url) {
      win.open(url, '_blank', 'noopener,noreferrer')
    },

    async saveFile(name, data) {
      const url = win.URL.createObjectURL(data)
      const link = win.document.createElement('a')
      link.href = url
      link.download = name
      link.style.display = 'none'
      win.document.body.append(link)
      link.click()
      link.remove()
      win.setTimeout(() => win.URL.revokeObjectURL(url), REVOKE_AFTER_MS)
    },

    fullscreen: {
      active: () => win.document.fullscreenElement !== null,
      async set(on) {
        if (on && !win.document.fullscreenElement) {
          await win.document.documentElement.requestFullscreen()
        } else if (!on && win.document.fullscreenElement) {
          await win.document.exitFullscreen()
        }
      },
      watch(onChange) {
        const changed = () => onChange(win.document.fullscreenElement !== null)
        win.document.addEventListener('fullscreenchange', changed)
        return () => win.document.removeEventListener('fullscreenchange', changed)
      },
    },
  }
}
