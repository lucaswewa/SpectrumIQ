import { vi } from 'vitest'

/** A stand-in for the system's dark-mode media query, which tests can flip. */
export interface FakeDarkQuery {
  matches: boolean
  /** Changes the system theme, telling listeners as the browser would. */
  set(dark: boolean): void
}

/** Makes `window.matchMedia` report the system's dark mode as `dark`. */
export function fakeSystemDarkMode(dark: boolean): FakeDarkQuery {
  const listeners = new Set<(event: MediaQueryListEvent) => void>()
  const query = {
    matches: dark,
    media: '(prefers-color-scheme: dark)',
    addEventListener: (_type: string, listener: (event: MediaQueryListEvent) => void) => {
      listeners.add(listener)
    },
    removeEventListener: (_type: string, listener: (event: MediaQueryListEvent) => void) => {
      listeners.delete(listener)
    },
    set(next: boolean) {
      query.matches = next
      for (const listener of listeners) listener({ matches: next } as MediaQueryListEvent)
    },
  }
  vi.spyOn(window, 'matchMedia').mockReturnValue(query as unknown as MediaQueryList)
  return query
}
