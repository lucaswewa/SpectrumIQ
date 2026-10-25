import { flushPromises, mount, type VueWrapper } from '@vue/test-utils'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { h } from 'vue'

import AppMenu, { type MenuItems } from '@/ui/AppMenu.vue'
import AppPagination from '@/ui/AppPagination.vue'
import ButtonMenu from '@/ui/ButtonMenu.vue'
import FormField from '@/ui/FormField.vue'
import MultiSelect from '@/ui/MultiSelect.vue'

afterEach(() => {
  document.body.innerHTML = ''
})

const all = (selector: string) => [...document.querySelectorAll<HTMLElement>(selector)]
const key = async (target: Element, key: string) => {
  target.dispatchEvent(new KeyboardEvent('keydown', { key, bubbles: true, cancelable: true }))
  await flushPromises()
}

describe('AppMenu and ButtonMenu', () => {
  function mountMenu() {
    const run = { jpeg: vi.fn(), tiff: vi.fn(), zip: vi.fn() }
    const items: MenuItems = [
      { label: 'JPEG', run: run.jpeg },
      { label: 'TIFF', run: run.tiff },
      'separator',
      { label: 'ZIP', disabled: true, danger: true, run: run.zip },
    ]
    mount(() => h('div', { 'data-theme': 'dark' }, [h(ButtonMenu, { label: 'Download', items })]), {
      attachTo: document.body,
    })
    return { run, trigger: document.querySelector('button')! }
  }

  it('is a button with a menu, which opens with the keyboard', async () => {
    const { trigger } = mountMenu()
    expect(trigger.getAttribute('aria-haspopup')).toBe('menu')
    expect(trigger.getAttribute('aria-expanded')).toBe('false')
    trigger.focus()
    await key(trigger, 'Enter')
    expect(trigger.getAttribute('aria-expanded')).toBe('true')
    const menu = document.querySelector('[role="menu"]')!
    expect(menu.closest('[data-theme]')?.getAttribute('data-theme')).toBe('dark')
    expect(all('[role="menuitem"]').map((item) => item.textContent?.trim())).toEqual([
      'JPEG',
      'TIFF',
      'ZIP',
    ])
    expect(all('[role="separator"]')).toHaveLength(1)
    expect(all('[role="menuitem"]')[2]!.getAttribute('aria-disabled')).toBe('true')
  })

  it('runs the chosen action and closes', async () => {
    const { run, trigger } = mountMenu()
    trigger.focus()
    await key(trigger, 'Enter')
    await key(all('[role="menuitem"]')[1]!, 'Enter')
    expect(run.tiff).toHaveBeenCalledOnce()
    expect(run.jpeg).not.toHaveBeenCalled()
    expect(document.querySelector('[role="menu"]')).toBeNull()
  })

  it('closes on Escape without running anything', async () => {
    const { run, trigger } = mountMenu()
    trigger.focus()
    await key(trigger, 'Enter')
    await key(all('[role="menuitem"]')[0]!, 'Escape')
    expect(document.querySelector('[role="menu"]')).toBeNull()
    expect(Object.values(run).some((action) => action.mock.calls.length > 0)).toBe(false)
  })

  it('ticks the choices that are on, and turns one over without closing', async () => {
    const toggle = vi.fn()
    const items: MenuItems = [
      { label: 'Outlines', checked: true, toggle },
      { label: 'Scale', checked: false, toggle },
    ]
    mount(() => h(ButtonMenu, { label: 'Layers', items }), { attachTo: document.body })
    const trigger = document.querySelector('button')!
    trigger.focus()
    await key(trigger, 'Enter')
    const choices = all('[role="menuitemcheckbox"]')
    expect(choices.map((choice) => choice.getAttribute('aria-checked'))).toEqual(['true', 'false'])
    await key(choices[1]!, 'Enter')
    expect(toggle).toHaveBeenCalledExactlyOnceWith(true)
    expect(document.querySelector('[role="menu"]')).not.toBeNull()
  })

  it('opens from any trigger it is given', async () => {
    mount(AppMenu, {
      props: { items: [{ label: 'Delete', run: () => {} }] },
      slots: { default: () => h('button', { id: 'more' }, '⋮') },
      attachTo: document.body,
    })
    const trigger = document.getElementById('more')!
    expect(trigger.getAttribute('aria-haspopup')).toBe('menu')
  })
})

describe('AppPagination', () => {
  function mountPages(page: number, total = 250) {
    const wrapper: VueWrapper = mount(AppPagination, {
      props: {
        page,
        total,
        perPage: 18,
        'onUpdate:page': (value: number) => wrapper.setProps({ page: value }),
      },
      attachTo: document.body,
    })
    return wrapper
  }
  const labels = (wrapper: VueWrapper) =>
    wrapper.findAll('button').map((button) => button.attributes('aria-label'))

  it('is a navigation region of page buttons, marking the current page', () => {
    const wrapper = mountPages(4)
    expect(wrapper.attributes('aria-label')).toBe('Pages')
    expect(wrapper.element.tagName).toBe('NAV')
    expect(labels(wrapper)).toEqual([
      'Previous page',
      'Page 1',
      'Page 2',
      'Page 3',
      'Page 4',
      'Page 5',
      'Page 14',
      'Next page',
    ])
    expect(wrapper.get('[aria-current="page"]').text()).toBe('4')
    expect(wrapper.text()).toContain('…')
  })

  it('changes page from a page button and from previous and next', async () => {
    const wrapper = mountPages(4)
    await wrapper.get('[aria-label="Page 14"]').trigger('click')
    expect(wrapper.emitted('update:page')).toEqual([[14]])
    await wrapper.get('[aria-label="Previous page"]').trigger('click')
    expect(wrapper.emitted('update:page')).toEqual([[14], [13]])
  })

  it('disables previous on the first page and next on the last', () => {
    const first = mountPages(1)
    expect(first.get('[aria-label="Previous page"]').attributes('disabled')).toBeDefined()
    const last = mountPages(14)
    expect(last.get('[aria-label="Next page"]').attributes('disabled')).toBeDefined()
  })
})

describe('MultiSelect', () => {
  const options = [
    { value: 'DEBUG', label: 'Debug' },
    { value: 'INFO', label: 'Info' },
    { value: 'ERROR', label: 'Error', disabled: true },
  ]

  function mountLevels(modelValue: string[]) {
    const wrapper: VueWrapper = mount(MultiSelect, {
      props: {
        options,
        modelValue,
        placeholder: 'All levels',
        'onUpdate:modelValue': (value: (string | number)[]) =>
          wrapper.setProps({ modelValue: value }),
      },
      attachTo: document.body,
    })
    return wrapper
  }

  it('shows the chosen labels, or its placeholder', () => {
    expect(mountLevels(['INFO', 'DEBUG']).get('[role="combobox"]').text()).toBe('Debug, Info')
    expect(mountLevels([]).get('[role="combobox"]').text()).toBe('All levels')
  })

  it('lists its options as a multiple-choice list, which stays open while choosing', async () => {
    const wrapper = mountLevels(['DEBUG'])
    await wrapper.get('[role="combobox"]').trigger('keydown', { key: 'Enter' })
    await flushPromises()
    const listbox = document.querySelector('[role="listbox"]')!
    expect(listbox.getAttribute('aria-multiselectable')).toBe('true')
    expect(all('[role="option"]').map((option) => option.getAttribute('aria-selected'))).toEqual([
      'true',
      'false',
      'false',
    ])
    await key(all('[role="option"]')[1]!, 'Enter')
    expect(wrapper.emitted('update:modelValue')).toEqual([[['DEBUG', 'INFO']]])
    expect(document.querySelector('[role="listbox"]')).not.toBeNull()
    await key(all('[role="option"]')[0]!, 'Enter')
    expect(wrapper.emitted('update:modelValue')?.at(-1)).toEqual([['INFO']])
    expect(wrapper.get('[role="combobox"]').text()).toBe('Info')
  })

  it('takes its label and error from a FormField', () => {
    const wrapper = mount(() =>
      h(FormField, { label: 'Levels', error: 'Choose one' }, () => h(MultiSelect, { options })),
    )
    const trigger = wrapper.get('[role="combobox"]')
    expect(wrapper.get('label').attributes('for')).toBe(trigger.attributes('id'))
    expect(trigger.attributes('aria-invalid')).toBe('true')
  })
})
