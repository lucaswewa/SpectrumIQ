import { createPinia, setActivePinia } from 'pinia'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { nextTick } from 'vue'

import { readPreference, THEME_STORAGE_KEY, useThemeStore } from '@/theme/store'

import { fakeSystemDarkMode } from './media'

const appliedTheme = () => document.documentElement.dataset.theme

describe('the theme store', () => {
  beforeEach(() => {
    window.localStorage.clear()
    delete document.documentElement.dataset.theme
    setActivePinia(createPinia())
  })

  afterEach(() => {
    vi.restoreAllMocks()
  })

  it('follows the system theme when nothing is saved', () => {
    fakeSystemDarkMode(true)
    const theme = useThemeStore()
    expect(theme.preference).toBe('system')
    expect(theme.theme).toBe('dark')
    expect(appliedTheme()).toBe('dark')
  })

  it('follows the system theme as it changes', async () => {
    const system = fakeSystemDarkMode(false)
    const theme = useThemeStore()
    expect(appliedTheme()).toBe('light')

    system.set(true)
    await nextTick()
    expect(theme.theme).toBe('dark')
    expect(appliedTheme()).toBe('dark')
  })

  it('applies a chosen theme at once, saves it, and restores it later', async () => {
    fakeSystemDarkMode(true)
    const theme = useThemeStore()
    theme.setPreference('light')
    await nextTick()
    expect(appliedTheme()).toBe('light')
    expect(window.localStorage.getItem(THEME_STORAGE_KEY)).toBe('light')

    // As after a reload: a new store reads the saved preference.
    setActivePinia(createPinia())
    const reloaded = useThemeStore()
    expect(reloaded.preference).toBe('light')
    expect(reloaded.theme).toBe('light')
  })

  it('ignores a saved value it does not understand', () => {
    fakeSystemDarkMode(false)
    window.localStorage.setItem(THEME_STORAGE_KEY, 'purple')
    expect(readPreference()).toBe('system')
    expect(useThemeStore().theme).toBe('light')
  })

  it('falls back to the system theme when storage fails, and still switches', async () => {
    fakeSystemDarkMode(true)
    vi.spyOn(window.localStorage, 'getItem').mockImplementation(() => {
      throw new Error('storage is blocked')
    })
    vi.spyOn(window.localStorage, 'setItem').mockImplementation(() => {
      throw new Error('storage is blocked')
    })

    const theme = useThemeStore()
    expect(theme.preference).toBe('system')
    expect(appliedTheme()).toBe('dark')

    expect(() => theme.setPreference('light')).not.toThrow()
    await nextTick()
    expect(appliedTheme()).toBe('light')
  })
})
