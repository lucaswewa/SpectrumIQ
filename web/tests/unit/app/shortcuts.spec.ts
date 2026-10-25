import { flushPromises, mount, type VueWrapper } from '@vue/test-utils'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { defineComponent, h, ref } from 'vue'

import {
  keysOf,
  provideShortcuts,
  useShortcut,
  type Shortcut,
  type ShortcutRegistry,
} from '@/app/shortcuts'

import { mountApp } from '../mountApp'

const press = (key: string, options: KeyboardEventInit = {}, target: EventTarget = window) => {
  const event = new KeyboardEvent('keydown', { key, bubbles: true, cancelable: true, ...options })
  target.dispatchEvent(event)
  return event
}

// Each mounted app listens on the window until it is unmounted.
const mounted: VueWrapper[] = []
afterEach(() => {
  mounted.splice(0).forEach((wrapper) => wrapper.unmount())
  document.body.innerHTML = ''
})

describe('keysOf', () => {
  it('names a key press as shortcuts write it', () => {
    const keys = (key: string, options: KeyboardEventInit = {}) =>
      keysOf(new KeyboardEvent('keydown', { key, ...options }))
    expect(keys('a')).toBe('a')
    expect(keys('ArrowDown', { shiftKey: true })).toBe('Shift+ArrowDown')
    expect(keys('s', { ctrlKey: true, altKey: true, shiftKey: true, metaKey: true })).toBe(
      'Ctrl+Alt+Meta+s',
    )
    // A printable character includes Shift already.
    expect(keys('?', { shiftKey: true })).toBe('?')
    expect(keys('A', { shiftKey: true })).toBe('A')
  })
})

describe('the shortcut registry', () => {
  /** An app with a registry, and a component that registers `shortcuts` while `shown` is true. */
  function mountRegistry(...shortcuts: Shortcut[]) {
    const shown = ref(true)
    let registry!: ShortcutRegistry
    const User = defineComponent(() => {
      useShortcut(...shortcuts)
      return () => h('input', { id: 'field' })
    })
    const Root = defineComponent(() => {
      registry = provideShortcuts()
      return () => [
        shown.value && h(User),
        h('div', { role: 'dialog' }, [h('button', { id: 'in-dialog' })]),
      ]
    })
    const wrapper: VueWrapper = mount(Root, { attachTo: document.body })
    mounted.push(wrapper)
    return { shown, registry }
  }

  const shortcut = (keys: string, extra: Partial<Shortcut> = {}): Shortcut => ({
    keys,
    description: `Do ${keys}`,
    group: 'Test',
    run: vi.fn(),
    ...extra,
  })

  it('runs a shortcut for its keys, and claims the key press', () => {
    const capture = shortcut('c')
    const next = shortcut('Shift+ArrowDown')
    mountRegistry(capture, next)
    const event = press('c')
    expect(capture.run).toHaveBeenCalledWith(event)
    expect(event.defaultPrevented).toBe(true)
    press('ArrowDown')
    press('ArrowDown', { shiftKey: true, ctrlKey: true })
    expect(next.run).not.toHaveBeenCalled()
    press('ArrowDown', { shiftKey: true })
    expect(next.run).toHaveBeenCalledOnce()
    expect(press('x').defaultPrevented).toBe(false)
  })

  it('leaves typing alone, unless a shortcut asks otherwise', () => {
    const capture = shortcut('c')
    const save = shortcut('Ctrl+s', { inFields: true })
    mountRegistry(capture, save)
    const field = document.getElementById('field')!
    press('c', {}, field)
    expect(capture.run).not.toHaveBeenCalled()
    press('s', { ctrlKey: true }, field)
    expect(save.run).toHaveBeenCalledOnce()
  })

  it('leaves key presses alone in a dialog, and those already handled', () => {
    const capture = shortcut('c')
    mountRegistry(capture)
    press('c', {}, document.getElementById('in-dialog')!)
    const handled = new KeyboardEvent('keydown', { key: 'c', bubbles: true, cancelable: true })
    handled.preventDefault()
    window.dispatchEvent(handled)
    expect(capture.run).not.toHaveBeenCalled()
  })

  it('refuses a second shortcut for the same keys', () => {
    const { registry } = mountRegistry(shortcut('c'))
    expect(() => registry.add(shortcut('c', { description: 'Clear' }))).toThrow(
      'c is already the shortcut for "Do c".',
    )
  })

  it('removes a component’s shortcuts when it goes', async () => {
    const capture = shortcut('c')
    const { shown } = mountRegistry(capture)
    shown.value = false
    await flushPromises()
    press('c')
    expect(capture.run).not.toHaveBeenCalled()
    // The keys are free again.
    shown.value = true
    await flushPromises()
    press('c')
    expect(capture.run).toHaveBeenCalledOnce()
  })
})

describe('the ? dialog', () => {
  it('lists every shortcut by group', async () => {
    const app = await mountApp('/view')
    mounted.push(app.wrapper)
    press('?', { shiftKey: true })
    await flushPromises()
    const dialog = document.querySelector('[role="dialog"]')!
    expect(dialog.querySelector('h2')?.textContent).toBe('Keyboard shortcuts')
    const groups = [...dialog.querySelectorAll('section')].map((section) => ({
      group: section.querySelector('h3')?.textContent,
      shortcuts: [...section.querySelectorAll('dt')].map(
        (keys) =>
          `${[...keys.querySelectorAll('kbd')].map((k) => k.textContent).join('+')} ${keys.nextElementSibling?.textContent}`,
      ),
    }))
    expect(groups).toEqual([
      { group: 'Navigation', shortcuts: ['Shift+↓ Next tab', 'Shift+↑ Previous tab'] },
      { group: 'Help', shortcuts: ['? Show the keyboard shortcuts'] },
    ])
  })
})
