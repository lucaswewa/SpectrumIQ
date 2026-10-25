import { mount } from '@vue/test-utils'
import { describe, expect, it } from 'vitest'

import AppCard from '@/ui/AppCard.vue'
import AppSpinner from '@/ui/AppSpinner.vue'
import ProgressBar from '@/ui/ProgressBar.vue'
import SectionHeading from '@/ui/SectionHeading.vue'

describe('ProgressBar', () => {
  it('reports its value out of its maximum', () => {
    const bar = mount(ProgressBar, { props: { value: 3, max: 12, label: 'Scan' } })
    expect(bar.attributes()).toMatchObject({
      role: 'progressbar',
      'aria-label': 'Scan',
      'aria-valuemin': '0',
      'aria-valuemax': '12',
      'aria-valuenow': '3',
    })
    expect(bar.get('.progress-bar__fill').attributes('style')).toContain('width: 25%')
  })

  it('keeps its value within range', () => {
    const over = mount(ProgressBar, { props: { value: 140, label: 'Scan' } })
    expect(over.attributes('aria-valuenow')).toBe('100')
    const under = mount(ProgressBar, { props: { value: -5, label: 'Scan' } })
    expect(under.attributes('aria-valuenow')).toBe('0')
  })

  it('is indeterminate without a value', () => {
    for (const value of [undefined, null]) {
      const bar = mount(ProgressBar, { props: { label: 'Focusing', value } })
      expect(bar.attributes('aria-valuenow')).toBeUndefined()
      expect(bar.classes()).toContain('progress-bar--indeterminate')
    }
  })
})

describe('AppSpinner', () => {
  it('is a named busy indicator with a label, and decoration without one', () => {
    const named = mount(AppSpinner, { props: { label: 'Loading' } })
    expect(named.attributes('role')).toBe('progressbar')
    expect(named.attributes('aria-label')).toBe('Loading')
    expect(named.attributes('aria-hidden')).toBeUndefined()
    const decoration = mount(AppSpinner)
    expect(decoration.attributes('role')).toBeUndefined()
    expect(decoration.attributes('aria-hidden')).toBe('true')
  })
})

describe('SectionHeading', () => {
  it('is a heading at the level it is given, with actions beside it', () => {
    const wrapper = mount(SectionHeading, {
      props: { level: 3 },
      slots: { default: 'Capture', actions: '<button>Refresh</button>' },
    })
    expect(wrapper.get('h3').text()).toBe('Capture')
    expect(wrapper.get('.section-heading__actions button').text()).toBe('Refresh')
    expect(mount(SectionHeading).find('h2').exists()).toBe(true)
    expect(mount(SectionHeading).find('.section-heading__actions').exists()).toBe(false)
  })
})

describe('AppCard', () => {
  it('holds its content', () => {
    expect(mount(AppCard, { slots: { default: 'Inside' } }).text()).toBe('Inside')
  })
})
