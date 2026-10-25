import { mount } from '@vue/test-utils'
import { describe, expect, it } from 'vitest'

import settings from '@material-symbols/svg-400/outlined/settings.svg?raw'
import AppIcon from '@/ui/AppIcon.vue'

describe('AppIcon', () => {
  it('draws the icon in the current text colour, hidden from assistive technology', () => {
    const svg = mount(AppIcon, { props: { svg: settings } }).get('svg')
    expect(svg.attributes('viewBox')).toBe('0 -960 960 960')
    expect(svg.attributes('fill')).toBe('currentColor')
    expect(svg.attributes('aria-hidden')).toBe('true')
    expect(svg.findAll('path')).toHaveLength(1)
    expect(svg.get('path').attributes('d')).toMatch(/^m388-80/)
  })

  it('takes its size from a token unless told otherwise', () => {
    const icon = mount(AppIcon, { props: { svg: settings } })
    expect(icon.get('svg').attributes('style')).toContain('var(--icon-size-md)')
    const small = mount(AppIcon, { props: { svg: settings, size: '16px' } })
    expect(small.get('svg').attributes('style')).toContain('16px')
  })

  it('draws every path of an icon that has several', () => {
    const svg = '<svg viewBox="0 0 24 24"><path d="M1 1h2"/><path d="M5 5h2"/></svg>'
    const paths = mount(AppIcon, { props: { svg } }).findAll('path')
    expect(paths.map((path) => path.attributes('d'))).toEqual(['M1 1h2', 'M5 5h2'])
  })
})
