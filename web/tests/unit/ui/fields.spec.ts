import { mount } from '@vue/test-utils'
import { afterEach, describe, expect, it } from 'vitest'
import { h } from 'vue'

import FormField from '@/ui/FormField.vue'
import NumberField from '@/ui/NumberField.vue'
import TextField from '@/ui/TextField.vue'

afterEach(() => {
  document.body.innerHTML = ''
})

/** A NumberField whose model is bound as with `v-model`. */
function mountNumber(props: { modelValue: number; min?: number; max?: number; step?: number }) {
  const wrapper = mount(NumberField, {
    props: {
      ...props,
      'onUpdate:modelValue': (value: number) => wrapper.setProps({ modelValue: value }),
    },
    attachTo: document.body,
  })
  return wrapper
}

describe('TextField', () => {
  it('updates its model on every keystroke and submits on Enter', async () => {
    const wrapper = mount(TextField, {
      props: {
        modelValue: '',
        'onUpdate:modelValue': (value: string) => wrapper.setProps({ modelValue: value }),
      },
    })
    await wrapper.get('input').setValue('slide 1')
    expect(wrapper.emitted('update:modelValue')).toEqual([['slide 1']])
    await wrapper.get('input').trigger('keydown', { key: 'Enter' })
    expect(wrapper.emitted('submit')).toEqual([['slide 1']])
  })

  it('shows a value it is given and passes attributes to the input', () => {
    const input = mount(TextField, {
      props: { modelValue: 'a' },
      attrs: { placeholder: 'Name', disabled: true },
    }).get('input')
    expect(input.element.value).toBe('a')
    expect(input.attributes('placeholder')).toBe('Name')
    expect(input.element.disabled).toBe(true)
  })

  it('is marked invalid when told to be', () => {
    const input = mount(TextField, { props: { invalid: true } }).get('input')
    expect(input.attributes('aria-invalid')).toBe('true')
  })
})

describe('NumberField', () => {
  it('is a spin button with its range', () => {
    const input = mountNumber({ modelValue: 5, min: 0, max: 10 }).get('input')
    expect(input.attributes()).toMatchObject({
      role: 'spinbutton',
      'aria-valuenow': '5',
      'aria-valuemin': '0',
      'aria-valuemax': '10',
    })
    expect(input.attributes('aria-invalid')).toBeUndefined()
  })

  it('commits on Enter, not while typing, and always submits', async () => {
    const wrapper = mountNumber({ modelValue: 5 })
    const input = wrapper.get('input')
    await input.setValue('12')
    expect(wrapper.emitted('update:modelValue')).toBeUndefined()
    expect(input.attributes('aria-valuenow')).toBe('12')
    await input.trigger('keydown', { key: 'Enter' })
    expect(wrapper.emitted('update:modelValue')).toEqual([[12]])
    expect(wrapper.emitted('submit')).toEqual([[12]])
    await input.trigger('keydown', { key: 'Enter' })
    expect(wrapper.emitted('update:modelValue')).toHaveLength(1)
    expect(wrapper.emitted('submit')).toEqual([[12], [12]])
  })

  it('commits when it loses focus, without submitting', async () => {
    const wrapper = mountNumber({ modelValue: 5 })
    await wrapper.get('input').setValue('7.5')
    await wrapper.get('input').trigger('blur')
    expect(wrapper.emitted('update:modelValue')).toEqual([[7.5]])
    expect(wrapper.emitted('submit')).toBeUndefined()
  })

  for (const [text, why] of [
    ['', 'empty'],
    ['12a', 'not a number'],
    ['0x10', 'hexadecimal'],
    ['11', 'above the maximum'],
    ['-1', 'below the minimum'],
    ['2.25', 'off the step grid'],
  ]) {
    it(`never commits text that is ${why}`, async () => {
      const wrapper = mountNumber({ modelValue: 5, min: 0, max: 10, step: 0.5 })
      const input = wrapper.get('input')
      await input.setValue(text!)
      expect(input.attributes('aria-invalid')).toBe('true')
      expect(input.attributes('aria-valuenow')).toBeUndefined()
      await input.trigger('keydown', { key: 'Enter' })
      expect(wrapper.emitted('submit')).toBeUndefined()
      await input.trigger('blur')
      expect(wrapper.emitted('update:modelValue')).toBeUndefined()
      expect(input.element.value).toBe('5')
      expect(input.attributes('aria-invalid')).toBeUndefined()
    })
  }

  it('accepts signs, decimals and exponents', async () => {
    const wrapper = mountNumber({ modelValue: 0 })
    for (const [text, value] of [
      ['-3', -3],
      ['.5', 0.5],
      ['1e3', 1000],
      [' 2 ', 2],
    ] as const) {
      await wrapper.get('input').setValue(text)
      await wrapper.get('input').trigger('keydown', { key: 'Enter' })
      expect(wrapper.props('modelValue')).toBe(value)
    }
  })

  it('steps on the grid with ↑ and ↓, within the range', async () => {
    const wrapper = mountNumber({ modelValue: 0.2, min: 0, max: 0.5, step: 0.1 })
    const input = wrapper.get('input')
    const press = async (key: string, times = 1) => {
      for (let i = 0; i < times; i++) await input.trigger('keydown', { key })
    }
    await press('ArrowUp')
    expect(input.element.value).toBe('0.3')
    await press('ArrowUp', 5)
    expect(input.element.value).toBe('0.5')
    await press('ArrowDown', 9)
    expect(input.element.value).toBe('0')
    expect(wrapper.emitted('update:modelValue')).toBeUndefined()
    await input.trigger('blur')
    expect(wrapper.emitted('update:modelValue')).toEqual([[0]])
  })

  it('steps by 1 without a step, and from an off-grid value to the next one', async () => {
    const free = mountNumber({ modelValue: 0.5 })
    await free.get('input').trigger('keydown', { key: 'ArrowUp' })
    expect(free.get('input').element.value).toBe('1.5')
    const grid = mountNumber({ modelValue: 2.6, step: 1 })
    await grid.get('input').trigger('keydown', { key: 'ArrowUp' })
    expect(grid.get('input').element.value).toBe('3')
    await grid.get('input').trigger('keydown', { key: 'ArrowDown' })
    expect(grid.get('input').element.value).toBe('2')
  })

  it('abandons an edit on Escape', async () => {
    const wrapper = mountNumber({ modelValue: 5 })
    await wrapper.get('input').setValue('8')
    await wrapper.get('input').trigger('keydown', { key: 'Escape' })
    expect(wrapper.get('input').element.value).toBe('5')
    await wrapper.get('input').trigger('blur')
    expect(wrapper.emitted('update:modelValue')).toBeUndefined()
  })

  it('follows the model unless an edit is pending', async () => {
    const wrapper = mountNumber({ modelValue: 5 })
    await wrapper.setProps({ modelValue: 6 })
    expect(wrapper.get('input').element.value).toBe('6')
    await wrapper.get('input').setValue('9')
    await wrapper.setProps({ modelValue: 7 })
    expect(wrapper.get('input').element.value).toBe('9')
    await wrapper.get('input').trigger('keydown', { key: 'Enter' })
    await wrapper.setProps({ modelValue: 10 })
    expect(wrapper.get('input').element.value).toBe('10')
  })
})

describe('FormField', () => {
  /** A field around a NumberField. */
  const field = (props: { help?: string; error?: string } = {}) =>
    h(FormField, { label: 'Exposure', ...props }, () => h(NumberField, { modelValue: 1 }))

  it('labels its control', () => {
    const wrapper = mount(() => field())
    const input = wrapper.get('input')
    expect(input.attributes('id')).toBeTruthy()
    expect(wrapper.get('label').attributes('for')).toBe(input.attributes('id'))
    expect(input.attributes('aria-describedby')).toBeUndefined()
  })

  it('describes its control with its help and error, and marks it invalid', () => {
    const wrapper = mount(() => field({ help: 'In µs', error: 'Too long' }))
    const input = wrapper.get('input')
    const described = input.attributes('aria-describedby')!.split(' ')
    expect(described.map((id) => wrapper.get(`[id="${id}"]`).text())).toEqual(['In µs', 'Too long'])
    expect(input.attributes('aria-invalid')).toBe('true')
  })

  it('gives each field its own id', () => {
    const wrapper = mount(() => [field(), field()])
    const [a, b] = wrapper.findAll('input').map((input) => input.attributes('id'))
    expect(a).not.toBe(b)
  })
})
