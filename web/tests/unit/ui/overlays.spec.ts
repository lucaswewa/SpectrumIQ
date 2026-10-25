import { flushPromises, mount } from '@vue/test-utils'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { defineComponent, h, ref } from 'vue'

import AppDialog from '@/ui/AppDialog.vue'
import OverlayProvider from '@/ui/OverlayProvider.vue'
import { useConfirm, useToast, type Confirm, type Toaster } from '@/ui/overlays'

afterEach(() => {
  document.body.innerHTML = ''
  vi.useRealTimers()
})

const byId = (id: string) => document.getElementById(id)!
const dialog = () => document.querySelector<HTMLElement>('[role="dialog"], [role="alertdialog"]')
const button = (text: string) =>
  [...document.querySelectorAll<HTMLButtonElement>('button')].find(
    (candidate) =>
      candidate.textContent?.trim() === text || candidate.getAttribute('aria-label') === text,
  )!

async function press(key: string) {
  document.activeElement!.dispatchEvent(
    new KeyboardEvent('keydown', { key, bubbles: true, cancelable: true }),
  )
  await flushPromises()
}

async function clickOutside() {
  const event = new PointerEvent('pointerdown', {
    bubbles: true,
    cancelable: true,
    button: 0,
    pointerType: 'mouse',
  })
  byId('opener').dispatchEvent(event)
  await flushPromises()
}

describe('AppDialog', () => {
  async function openDialog(props: Record<string, unknown> = {}) {
    const open = ref(false)
    mount(
      () => [
        h('button', { id: 'opener', onClick: () => (open.value = true) }, 'Open'),
        h(
          AppDialog,
          {
            open: open.value,
            'onUpdate:open': (value: boolean) => (open.value = value),
            title: 'Move to',
            ...props,
          },
          { default: () => h('input', { id: 'field' }), footer: () => h('button', 'Move') },
        ),
      ],
      { attachTo: document.body },
    )
    byId('opener').focus()
    byId('opener').click()
    await flushPromises()
    return open
  }

  it('is named by its title, and starts on its first field', async () => {
    await openDialog()
    const labelledBy = dialog()!.getAttribute('aria-labelledby')!
    expect(byId(labelledBy).textContent).toBe('Move to')
    expect(document.activeElement).toBe(byId('field'))
  })

  it('is described by its description, if it has one', async () => {
    await openDialog()
    expect(dialog()!.hasAttribute('aria-describedby')).toBe(false)
    document.body.innerHTML = ''
    await openDialog({ description: 'In steps' })
    expect(byId(dialog()!.getAttribute('aria-describedby')!).textContent?.trim()).toBe('In steps')
  })

  it('keeps Tab inside itself', async () => {
    await openDialog()
    const focusable = [...dialog()!.querySelectorAll<HTMLElement>('input, button')]
    focusable.at(-1)!.focus()
    await press('Tab')
    expect(dialog()!.contains(document.activeElement)).toBe(true)
  })

  for (const [how, close] of [
    ['Escape', () => press('Escape')],
    ['its close button', async () => button('Close').click()],
    ['a click outside', clickOutside],
  ] as const) {
    it(`closes with ${how}, giving the focus back`, async () => {
      const open = await openDialog()
      await close()
      await flushPromises()
      expect(open.value).toBe(false)
      expect(dialog()).toBeNull()
      expect(document.activeElement).toBe(byId('opener'))
    })
  }

  it('can ignore clicks outside', async () => {
    const open = await openDialog({ closeOnOutsideClick: false })
    await clickOutside()
    expect(open.value).toBe(true)
    await press('Escape')
    expect(open.value).toBe(false)
  })

  it('ignores Escape and clicks outside, and has no close button, when not dismissible', async () => {
    const open = await openDialog({ dismissible: false })
    await press('Escape')
    await clickOutside()
    expect(open.value).toBe(true)
    expect(button('Close')).toBeUndefined()
  })

  it('is an alert dialog when asked', async () => {
    await openDialog({ alert: true })
    expect(dialog()!.getAttribute('role')).toBe('alertdialog')
  })
})

/** Mounts an OverlayProvider with a component inside that uses it. */
function mountProvider() {
  const used: { confirm?: Confirm; toast?: Toaster } = {}
  const User = defineComponent(() => {
    used.confirm = useConfirm()
    used.toast = useToast()
    return () => h('button', { id: 'opener' }, 'Ask')
  })
  mount(OverlayProvider, { slots: { default: () => h(User) }, attachTo: document.body })
  byId('opener').focus()
  return used as Required<typeof used>
}

describe('useConfirm', () => {
  it('resolves true when confirmed', async () => {
    const { confirm } = mountProvider()
    const answer = confirm({ title: 'Move home?' })
    await flushPromises()
    expect(dialog()!.getAttribute('role')).toBe('alertdialog')
    button('Confirm').click()
    expect(await answer).toBe(true)
    await flushPromises()
    expect(dialog()).toBeNull()
  })

  for (const [how, answer] of [
    ['cancelled', async () => button('Cancel').click()],
    ['closed', async () => button('Close').click()],
    ['dismissed with Escape', () => press('Escape')],
  ] as const) {
    it(`resolves false when ${how}`, async () => {
      const { confirm } = mountProvider()
      const result = confirm({ title: 'Move home?' })
      await flushPromises()
      await answer()
      expect(await result).toBe(false)
    })
  }

  it('ignores clicks outside', async () => {
    const { confirm } = mountProvider()
    void confirm({ title: 'Move home?' })
    await flushPromises()
    await clickOutside()
    expect(dialog()).not.toBeNull()
  })

  it('shows its message, its content and its labels, starting on Cancel', async () => {
    const { confirm } = mountProvider()
    void confirm({
      title: 'Delete 2 captures?',
      message: "This can't be undone.",
      content: () => h('ul', [h('li', 'a.jpg'), h('li', 'b.jpg')]),
      confirmLabel: 'Delete',
      cancelLabel: 'Keep',
      danger: true,
    })
    await flushPromises()
    expect(dialog()!.textContent).toContain("This can't be undone.")
    expect([...dialog()!.querySelectorAll('li')].map((item) => item.textContent)).toEqual([
      'a.jpg',
      'b.jpg',
    ])
    expect(button('Delete').classList).toContain('app-button--danger')
    expect(document.activeElement).toBe(button('Keep'))
  })

  it('asks one question at a time, in order', async () => {
    const { confirm } = mountProvider()
    const first = confirm({ title: 'First?' })
    const second = confirm({ title: 'Second?' })
    await flushPromises()
    expect(dialog()!.textContent).toContain('First?')
    button('Confirm').click()
    await flushPromises()
    expect(dialog()!.textContent).toContain('Second?')
    button('Cancel').click()
    expect([await first, await second]).toEqual([true, false])
  })

  it('needs an OverlayProvider', () => {
    const Lonely = defineComponent(() => {
      useConfirm()
      return () => null
    })
    expect(() => mount(Lonely)).toThrow('useConfirm() needs an OverlayProvider')
  })
})

describe('useToast', () => {
  const toasts = () =>
    [...document.querySelectorAll<HTMLElement>('.overlay-toast')].map((toast) => ({
      kind: [...toast.classList].find((name) => name.startsWith('overlay-toast--')),
      text: toast.querySelector('.overlay-toast__text')?.textContent?.trim(),
    }))

  it('shows each kind of toast in the notifications region', async () => {
    const { toast } = mountProvider()
    toast.success('Saved')
    toast.info('Calibrating')
    toast.error('Failed')
    await flushPromises()
    expect(toasts()).toEqual([
      { kind: 'overlay-toast--success', text: 'Saved' },
      { kind: 'overlay-toast--info', text: 'Calibrating' },
      { kind: 'overlay-toast--error', text: 'Failed' },
    ])
    const region = document.querySelector('[role="region"]')!
    expect(region.getAttribute('aria-label')).toMatch(/^Notification/)
  })

  it('folds its details, which may be an error from the server', async () => {
    const { toast } = mountProvider()
    toast.error('Not moved', { details: { detail: [{ loc: ['body', 'x'], msg: 'Too far' }] } })
    await flushPromises()
    const details = document.querySelector('.overlay-toast details')!
    expect(details.hasAttribute('open')).toBe(false)
    expect(details.querySelector('summary')!.textContent).toBe('Details')
    expect(details.textContent).toContain('x Too far')
  })

  it('removes success and info after 5 s, and keeps errors until closed', async () => {
    vi.useFakeTimers()
    const { toast } = mountProvider()
    toast.success('Saved')
    toast.error('Failed')
    toast.info('Slow', { duration: 10_000 })
    await flushPromises()
    vi.advanceTimersByTime(5_000)
    await flushPromises()
    expect(toasts().map((shown) => shown.text)).toEqual(['Failed', 'Slow'])
    vi.advanceTimersByTime(60_000)
    await flushPromises()
    expect(toasts().map((shown) => shown.text)).toEqual(['Failed'])
    button('Dismiss').click()
    await flushPromises()
    expect(toasts()).toEqual([])
  })
})
