import { flushPromises, type VueWrapper } from '@vue/test-utils'
import { afterEach, describe, expect, it } from 'vitest'
import { ref } from 'vue'

import { thingAvailabilityKey } from '@/app/availability'

import { mountApp } from '../mountApp'

const labels = (wrapper: VueWrapper, group: 'top' | 'bottom') =>
  wrapper.findAll(`.rail__group--${group} .rail__label`).map((label) => label.text())

describe('the navigation rail', () => {
  let wrapper: VueWrapper | undefined

  afterEach(() => {
    wrapper?.unmount()
    wrapper = undefined
  })

  it('lists the workflows at the top and the rest at the bottom, in order', async () => {
    wrapper = (await mountApp('/live')).wrapper
    expect(labels(wrapper, 'top')).toEqual([
      'Live',
      'Mosaic',
      'Scan',
      'Datasets',
      'Viewer',
      '3D',
      'Jobs',
    ])
    expect(labels(wrapper, 'bottom')).toEqual([
      'Calibration',
      'Settings',
      'Admin',
      'Logging',
      'About',
    ])
    expect(wrapper.get('nav').attributes('aria-label')).toBe('Main')
    expect(wrapper.findAll('.rail__item svg[aria-hidden="true"]')).toHaveLength(12)
  })

  it('marks the current destination', async () => {
    wrapper = (await mountApp('/datasets')).wrapper
    const active = wrapper.findAll('.rail__item--active')
    expect(active.map((item) => item.text())).toEqual(['Datasets'])
    expect(active[0]?.attributes('aria-current')).toBe('page')
  })

  it('hides destinations whose Things the instrument lacks', async () => {
    const offered = new Set(['camera', 'stage', 'datasets'])
    wrapper = (
      await mountApp('/live', { [thingAvailabilityKey]: (thing: string) => offered.has(thing) })
    ).wrapper
    expect(labels(wrapper, 'top')).toEqual(['Live', 'Datasets', 'Viewer', '3D'])
    expect(labels(wrapper, 'bottom')).toEqual(['Settings', 'Admin', 'Logging', 'About'])
  })
  it('updates as the offered Things change', async () => {
    const offered = ref(new Set(['camera']))
    wrapper = (
      await mountApp('/live', {
        [thingAvailabilityKey]: (thing: string) => offered.value.has(thing),
      })
    ).wrapper
    expect(labels(wrapper, 'top')).toEqual(['Live'])

    offered.value = new Set(['camera', 'workflow', 'jobs'])
    await flushPromises()
    expect(labels(wrapper, 'top')).toEqual(['Live', 'Scan', 'Jobs'])
  })
})
