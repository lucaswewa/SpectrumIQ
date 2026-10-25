import { readFileSync } from 'node:fs'
import { dirname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

import { afterEach, describe, expect, it, vi } from 'vitest'

import { readPreference, resolveTheme, THEME_STORAGE_KEY } from '@/theme/store'

import { fakeSystemDarkMode } from './media'

/** The inline script in index.html that applies the theme before the app loads. */
// Paths, not URLs: in the happy-dom environment URL is happy-dom's, which Node's fs refuses.
const here = dirname(fileURLToPath(import.meta.url))
const indexHtml = readFileSync(resolve(here, '../../../index.html'), 'utf8')
const bootstrap = /<script>([\s\S]*?)<\/script>/.exec(indexHtml)?.[1] ?? ''

type Saved = string | null | 'storage fails'
const SAVED: Saved[] = [null, 'light', 'dark', 'system', 'purple', 'storage fails']

describe("index.html's theme bootstrap", () => {
  afterEach(() => {
    vi.restoreAllMocks()
    window.localStorage.clear()
    delete document.documentElement.dataset.theme
  })

  it('is there', () => {
    expect(bootstrap).toContain(THEME_STORAGE_KEY)
  })

  for (const saved of SAVED) {
    for (const systemDark of [false, true]) {
      it(`agrees with the store: saved ${saved}, system ${systemDark ? 'dark' : 'light'}`, () => {
        fakeSystemDarkMode(systemDark)
        if (saved === 'storage fails') {
          vi.spyOn(window.localStorage, 'getItem').mockImplementation(() => {
            throw new Error('storage is blocked')
          })
        } else if (saved !== null) {
          window.localStorage.setItem(THEME_STORAGE_KEY, saved)
        }

        new Function(bootstrap)()

        const expected = resolveTheme(readPreference(), systemDark)
        expect(document.documentElement.dataset.theme).toBe(expected)
        // And the expected rule: dark if saved so, or following a dark system.
        const followsSystem = saved !== 'light' && saved !== 'dark'
        expect(expected).toBe(saved === 'dark' || (followsSystem && systemDark) ? 'dark' : 'light')
      })
    }
  }
})
