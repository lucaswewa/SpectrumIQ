import { inject, type InjectionKey } from 'vue'

import { createBrowserHost } from './browser'
import type { HostAdapter } from './types'

export type { AppInfo, FullscreenControl, HostAdapter, ServiceControl, ServiceState } from './types'

/** The key under which the app's host is provided (`main.ts` provides the browser's). */
export const hostKey: InjectionKey<HostAdapter> = Symbol('host')

/**
 * The host for where the app runs: the Tauri shell's inside it (ADR-0040), else the browser's.
 * The Tauri host is loaded only there, so browsers don't download it. If the shell refuses its
 * commands, as for a page outside its capability, the app still runs, as in a browser.
 */
export async function createHost(win: Window & typeof globalThis = window): Promise<HostAdapter> {
  if ('__TAURI_INTERNALS__' in win) {
    try {
      return await (await import('./tauri')).createTauriHost()
    } catch (refused) {
      console.warn('The desktop shell refused its host commands:', refused)
    }
  }
  return createBrowserHost(win)
}

/** The host the app runs in. Call it from a component's `setup`. */
export function useHost(): HostAdapter {
  const host = inject(hostKey)
  if (!host) {
    throw new Error('No host is provided: call app.provide(hostKey, host) before mounting.')
  }
  return host
}
