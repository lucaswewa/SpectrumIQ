import { defineStore } from 'pinia'
import { computed, ref, watchEffect } from 'vue'

/** What the user chose: a theme, or to follow the system's. */
export type ThemePreference = 'light' | 'dark' | 'system'

/** The theme that is applied. */
export type Theme = 'light' | 'dark'

/** Where the preference is kept. `index.html`'s bootstrap script reads it too. */
export const THEME_STORAGE_KEY = 'spectrumiq.theme'

const DARK_QUERY = '(prefers-color-scheme: dark)'

export function isThemePreference(value: unknown): value is ThemePreference {
  return value === 'light' || value === 'dark' || value === 'system'
}

/** The theme for a preference, given whether the system is in dark mode. */
export function resolveTheme(preference: ThemePreference, systemDark: boolean): Theme {
  if (preference === 'system') return systemDark ? 'dark' : 'light'
  return preference
}

/** The saved preference, or `system` if there's none, it's invalid, or storage fails. */
export function readPreference(): ThemePreference {
  try {
    const saved = window.localStorage.getItem(THEME_STORAGE_KEY)
    return isThemePreference(saved) ? saved : 'system'
  } catch {
    return 'system'
  }
}

/** Saves the preference. Returns false if storage isn't available. */
export function writePreference(preference: ThemePreference): boolean {
  try {
    window.localStorage.setItem(THEME_STORAGE_KEY, preference)
    return true
  } catch {
    return false
  }
}

/** Applies a theme to the page, through `data-theme` on `<html>` (see tokens.css). */
export function applyTheme(theme: Theme, root: HTMLElement = document.documentElement) {
  root.dataset.theme = theme
}

/**
 * The theme: the user's preference (persisted), the system's dark mode
 * (followed as it changes), and the theme they resolve to, which is applied
 * to the page whenever it changes.
 */
export const useThemeStore = defineStore('theme', () => {
  const preference = ref<ThemePreference>(readPreference())
  const media = window.matchMedia(DARK_QUERY)
  const systemDark = ref(media.matches)
  media.addEventListener('change', (event) => {
    systemDark.value = event.matches
  })
  const theme = computed(() => resolveTheme(preference.value, systemDark.value))

  function setPreference(next: ThemePreference) {
    preference.value = next
    writePreference(next)
  }

  watchEffect(() => applyTheme(theme.value))

  return { preference, systemDark, theme, setPreference }
})
