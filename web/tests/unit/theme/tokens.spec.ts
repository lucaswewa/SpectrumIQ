import { readFileSync } from 'node:fs'
import { dirname, resolve as resolvePath } from 'node:path'
import { fileURLToPath } from 'node:url'

import { describe, expect, it } from 'vitest'

import { THEMED_GROUPS, TOKEN_GROUPS } from '@/theme/tokens'

// Read from disk, by path: Vitest doesn't load stylesheets, even with ?raw,
// and in the happy-dom environment URL is happy-dom's, which Node's fs refuses.
const here = dirname(fileURLToPath(import.meta.url))
const css = readFileSync(resolvePath(here, '../../../src/theme/tokens.css'), 'utf8')

/** The custom properties declared in each top-level block of tokens.css. */
function blocks(source: string): Map<string, Map<string, string>> {
  const text = source.replace(/\/\*[\s\S]*?\*\//g, '')
  const result = new Map<string, Map<string, string>>()
  let depth = 0
  let start = 0
  let selector = ''
  for (let i = 0; i < text.length; i++) {
    if (text[i] === '{') {
      if (depth === 0) {
        selector = text.slice(start, i).trim().replace(/\s+/g, ' ')
        start = i + 1
      }
      depth++
    } else if (text[i] === '}') {
      depth--
      if (depth === 0) {
        const declarations = new Map<string, string>()
        for (const [, name, value] of text.slice(start, i).matchAll(/(--[\w-]+)\s*:\s*([^;]+);/g)) {
          declarations.set(name!, value!.trim().replace(/\s+/g, ' '))
        }
        result.set(selector, declarations)
        start = i + 1
      }
    }
  }
  return result
}

const parsed = blocks(css)
const root = parsed.get(':root')!
const themes = {
  light: parsed.get(":root, [data-theme='light']")!,
  dark: parsed.get("[data-theme='dark']")!,
}

/** A token's value in a theme, following var() references to a colour. */
function resolve(theme: keyof typeof themes, token: string): string {
  const value = themes[theme].get(token) ?? root.get(token)
  if (value === undefined) throw new Error(`${token} isn't defined`)
  const reference = /^var\((--[\w-]+)\)$/.exec(value)
  return reference ? resolve(theme, reference[1]!) : value
}

function luminance(hex: string): number {
  const [r, g, b] = [1, 3, 5].map((i) => parseInt(hex.slice(i, i + 2), 16) / 255) as [
    number,
    number,
    number,
  ]
  const linear = (c: number) => (c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4)
  return 0.2126 * linear(r) + 0.7152 * linear(g) + 0.0722 * linear(b)
}

/** WCAG 2 contrast ratio between two #rrggbb colours. */
function contrast(a: string, b: string): number {
  const [high, low] = [luminance(a), luminance(b)].sort((x, y) => y - x) as [number, number]
  return (high + 0.05) / (low + 0.05)
}

// [foreground, background, minimum ratio]: 4.5 for text, 3 for other things
// people must see (WCAG 2.2 AA, 1.4.3 and 1.4.11).
const PAIRS: [string, string, number][] = [
  ['--color-text', '--color-bg', 4.5],
  ['--color-text', '--color-surface-panel', 4.5],
  ['--color-text', '--color-surface-raised', 4.5],
  ['--color-text-muted', '--color-bg', 4.5],
  ['--color-text-muted', '--color-surface-panel', 4.5],
  ['--color-text-heading', '--color-bg', 4.5],
  ['--color-text-heading', '--color-surface-panel', 4.5],
  ['--color-accent-text', '--color-bg', 4.5],
  ['--color-accent-text', '--color-surface-panel', 4.5],
  ['--color-accent-text', '--color-surface-rail', 4.5],
  ['--color-accent-text', '--color-surface-raised', 4.5],
  ['--color-text', '--color-accent-subtle', 4.5],
  ['--color-accent-text', '--color-accent-subtle', 4.5],
  ['--color-on-accent', '--color-accent', 4.5],
  ['--color-on-accent', '--color-accent-hover', 4.5],
  ['--color-on-accent', '--color-accent-pressed', 4.5],
  ['--color-control-text', '--color-control-bg', 4.5],
  ['--color-control-border', '--color-control-bg', 3],
  ['--color-control-border', '--color-bg', 3],
  ['--color-control-border', '--color-surface-panel', 3],
  ['--color-rail-text', '--color-surface-rail', 4.5],
  ['--color-rail-text', '--color-rail-hover-bg', 4.5],
  ['--color-rail-active-text', '--color-rail-active-bg', 4.5],
  ['--color-focus', '--color-bg', 3],
  ['--color-focus', '--color-surface-panel', 3],
  ['--color-danger', '--color-bg', 4.5],
  ['--color-warning', '--color-bg', 4.5],
  ['--color-success', '--color-bg', 4.5],
  ['--color-info', '--color-bg', 4.5],
]

describe('the design tokens', () => {
  it('lists exactly the tokens that tokens.css defines', () => {
    const defined = new Set([...root.keys(), ...themes.light.keys(), ...themes.dark.keys()])
    const listed = new Set(Object.values(TOKEN_GROUPS).flat())
    expect([...defined].sort()).toEqual([...listed].sort())
  })

  it('defines every themed token in both themes, and only those', () => {
    const themed = THEMED_GROUPS.flatMap((group) => TOKEN_GROUPS[group]).sort()
    expect([...themes.light.keys()].sort()).toEqual(themed)
    expect([...themes.dark.keys()].sort()).toEqual(themed)
  })

  it('turns motion off for people who ask for reduced motion', () => {
    const reduced = blocks(
      /@media \(prefers-reduced-motion: reduce\) \{([\s\S]*?\})\s*\}/.exec(css)![1]!,
    )
    expect(reduced.get(':root')?.get('--duration-fast')).toBe('0ms')
    expect(reduced.get(':root')?.get('--duration-normal')).toBe('0ms')
  })

  for (const theme of ['light', 'dark'] as const) {
    describe(`in the ${theme} theme`, () => {
      for (const [foreground, background, minimum] of PAIRS) {
        it(`${foreground} on ${background} has a contrast of at least ${minimum}:1`, () => {
          const ratio = contrast(resolve(theme, foreground), resolve(theme, background))
          expect(ratio).toBeGreaterThanOrEqual(minimum)
        })
      }
    })
  }
})
