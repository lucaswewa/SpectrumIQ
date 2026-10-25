import { inject, type InjectionKey } from 'vue'

import type { HostAdapter } from './types'

export type { AppInfo, FullscreenControl, HostAdapter, ServiceControl, ServiceState } from './types'

/** The key under which the app's host is provided (`main.ts` provides the browser's). */
export const hostKey: InjectionKey<HostAdapter> = Symbol('host')

/** The host the app runs in. Call it from a component's `setup`. */
export function useHost(): HostAdapter {
  const host = inject(hostKey)
  if (!host) {
    throw new Error('No host is provided: call app.provide(hostKey, host) before mounting.')
  }
  return host
}
