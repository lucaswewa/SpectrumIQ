import { flushPromises, mount, type VueWrapper } from '@vue/test-utils'
import { afterEach, describe, expect, it } from 'vitest'
import { h } from 'vue'

import AppCheckbox from '@/ui/AppCheckbox.vue'
import AppSelect from '@/ui/AppSelect.vue'
import AppSlider from '@/ui/AppSlider.vue'
import AppToggle from '@/ui/AppToggle.vue'
import FormField from '@/ui/FormField.vue'

afterEach(() => {
  document.body.innerHTML = ''
})

for (const [name, component, role] of [
  ['AppCheckbox', AppCheckbox, null],
  ['AppToggle', AppToggle, 'switch'],
] as const) {
  describe(name, () => {
    const mountBound = (modelValue: boolean, attrs = {}) => {
      const wrapper = mount(component, {
        props: {
          modelValue,
          label: 'Save to gallery',
          'onUpdate:modelValue': (value: boolean) => wrapper.setProps({ modelValue: value }),
        },
        attrs,
        attachTo: document.body,
      })
      return wrapper
    }

    it(`is a native checkbox${role ? ` with the ${role} role` : ''}, named by its label`, () => {
      const wrapper = mountBound(true)
      const input = wrapper.get('input')
      expect(input.attributes('type')).toBe('checkbox')
      expect(input.attributes('role')).toBe(role ?? undefined)
      expect(input.element.checked).toBe(true)
      expect(input.element.closest('label')?.textContent?.trim()).toBe('Save to gallery')
    })

    it('updates its model both ways', async () => {
      const wrapper = mountBound(false)
      await wrapper.get('input').setValue(true)
      expect(wrapper.emitted('update:modelValue')).toEqual([[true]])
      await wrapper.setProps({ modelValue: false })
      expect(wrapper.get('input').element.checked).toBe(false)
    })

    it('toggles when its label is clicked', async () => {
      const wrapper = mountBound(false)
      await wrapper.get('label').trigger('click')
      expect(wrapper.emitted('update:modelValue')).toEqual([[true]])
    })

    it('passes attributes such as disabled to the input', () => {
      const input = mountBound(false, { disabled: true, name: 'save' }).get('input')
      expect(input.element.disabled).toBe(true)
      expect(input.attributes('name')).toBe('save')
    })
  })
}

describe('AppSelect', () => {
  const options = [
    { value: 'small', label: 'Small' },
    { value: 'large', label: 'Large' },
    { value: 'video', label: 'Video', disabled: true },
  ]

  function mountSelect(modelValue?: string) {
    const wrapper: VueWrapper = mount(AppSelect, {
      props: {
        options,
        modelValue,
        placeholder: 'Choose',
        'onUpdate:modelValue': (value?: string | number) => wrapper.setProps({ modelValue: value }),
      },
      attachTo: document.body,
    })
    return wrapper
  }

  const listbox = () => document.querySelector('[role="listbox"]')
  const optionsShown = () =>
    [...document.querySelectorAll('[role="option"]')].map((option) => ({
      text: option.textContent?.trim(),
      selected: option.getAttribute('aria-selected'),
      disabled: option.hasAttribute('data-disabled'),
    }))

  it('is a closed combobox showing its placeholder', () => {
    const trigger = mountSelect().get('[role="combobox"]')
    expect(trigger.attributes('aria-expanded')).toBe('false')
    expect(trigger.text()).toBe('Choose')
    expect(listbox()).toBeNull()
  })

  it('shows the chosen option', async () => {
    const wrapper = mountSelect('large')
    await flushPromises()
    expect(wrapper.get('[role="combobox"]').text()).toBe('Large')
  })

  for (const key of ['Enter', ' ', 'ArrowDown']) {
    it(`opens with ${JSON.stringify(key)}, listing the options`, async () => {
      const wrapper = mountSelect('large')
      await wrapper.get('[role="combobox"]').trigger('keydown', { key })
      await flushPromises()
      expect(wrapper.get('[role="combobox"]').attributes('aria-expanded')).toBe('true')
      expect(listbox()).not.toBeNull()
      expect(optionsShown()).toEqual([
        { text: 'Small', selected: 'false', disabled: false },
        { text: 'Large', selected: 'true', disabled: false },
        { text: 'Video', selected: 'false', disabled: true },
      ])
    })
  }

  it('updates its model when an option is chosen, and closes', async () => {
    const wrapper = mountSelect()
    await wrapper.get('[role="combobox"]').trigger('keydown', { key: 'Enter' })
    await flushPromises()
    const small = [...document.querySelectorAll<HTMLElement>('[role="option"]')][0]!
    small.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true }))
    await flushPromises()
    expect(wrapper.emitted('update:modelValue')).toEqual([['small']])
    expect(listbox()).toBeNull()
    expect(wrapper.get('[role="combobox"]').text()).toBe('Small')
  })

  it("opens its list in the theme of the region it's in", async () => {
    const wrapper = mount(
      () => h('div', { 'data-theme': 'dark' }, [h(AppSelect, { options, modelValue: 'small' })]),
      { attachTo: document.body },
    )
    await wrapper.get('[role="combobox"]').trigger('keydown', { key: 'Enter' })
    await flushPromises()
    expect(listbox()?.closest('[data-theme]')?.getAttribute('data-theme')).toBe('dark')
  })

  it('takes its label, description and error from a FormField', () => {
    const wrapper = mount(() =>
      h(FormField, { label: 'Size', error: 'Required' }, () => h(AppSelect, { options })),
    )
    const trigger = wrapper.get('[role="combobox"]')
    expect(wrapper.get('label').attributes('for')).toBe(trigger.attributes('id'))
    expect(trigger.attributes('aria-invalid')).toBe('true')
    expect(trigger.attributes('aria-describedby')).toMatch(/-error$/)
  })
})

describe('AppSlider', () => {
  it('is a named slider that the keys move, within its range', async () => {
    const wrapper = mount(AppSlider, {
      props: {
        modelValue: 100,
        label: 'Brightness',
        max: 200,
        'onUpdate:modelValue': (value: number) => wrapper.setProps({ modelValue: value }),
      },
      attachTo: document.body,
    })
    await flushPromises()
    const thumb = wrapper.get('[role="slider"]')
    expect(thumb.attributes()).toMatchObject({
      'aria-label': 'Brightness',
      'aria-valuemin': '0',
      'aria-valuemax': '200',
      'aria-valuenow': '100',
    })
    for (const [key, value] of [
      ['ArrowRight', 101],
      ['PageUp', 111],
      ['Home', 0],
      ['ArrowLeft', 0],
      ['End', 200],
    ] as const) {
      await thumb.trigger('keydown', { key })
      await flushPromises()
      expect(wrapper.props('modelValue')).toBe(value)
    }
  })
})
