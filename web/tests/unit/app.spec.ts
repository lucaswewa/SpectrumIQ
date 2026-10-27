import { mount, type VueWrapper } from '@vue/test-utils'
import { afterEach, describe, expect, it } from 'vitest'
import { defineComponent } from 'vue'

import { DESTINATIONS } from '@/app/navigation'
import { useHost } from '@/host'

import { mountApp } from './mountApp'

describe('the app', () => {
  let wrapper: VueWrapper | undefined

  afterEach(() => {
    wrapper?.unmount()
    wrapper = undefined
  })

  it('opens on the Live page', async () => {
    const app = await mountApp('/')
    wrapper = app.wrapper
    expect(app.router.currentRoute.value.name).toBe('live')
    expect(wrapper.get('h1').text()).toBe('Live')
  })

  it('sends unknown addresses to the Live page', async () => {
    const app = await mountApp('/no/such/page')
    wrapper = app.wrapper
    expect(app.router.currentRoute.value.name).toBe('live')
  })

  for (const destination of DESTINATIONS) {
    it(`opens the ${destination.label} page`, async () => {
      const app = await mountApp(`/${destination.id}`)
      wrapper = app.wrapper
      expect(wrapper.get('h1').text()).toBe(destination.label)
    })
  }

  it('says which milestone builds a page that is not built yet', async () => {
    const app = await mountApp('/jobs')
    wrapper = app.wrapper
    expect(wrapper.get('.planned-page__note').text()).toContain('milestone M8')
  })

  it('tells what the app is on the About page', async () => {
    const app = await mountApp('/about')
    wrapper = app.wrapper
    const facts = wrapper.findAll('.about-page__facts dd').map((fact) => fact.text())
    expect(facts).toEqual(['SpectrumIQ', __APP_VERSION__, 'A browser', 'MIT'])
  })

  it('has a gallery of the UI components in both themes, in development', async () => {
    const app = await mountApp('/dev/components')
    wrapper = app.wrapper
    expect(app.router.currentRoute.value.name).toBe('dev-components')
    const panels = wrapper.findAll('section[data-theme]')
    expect(panels.map((panel) => panel.attributes('data-theme'))).toEqual(['light', 'dark'])
  })

  it('says so when no host is provided', () => {
    const NeedsHost = defineComponent({
      setup() {
        useHost()
        return () => null
      },
    })
    expect(() => mount(NeedsHost)).toThrow('No host is provided')
  })
})
