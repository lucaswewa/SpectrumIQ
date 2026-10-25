import { flushPromises, mount } from '@vue/test-utils'
import { afterEach, describe, expect, it } from 'vitest'
import { defineComponent, h, ref } from 'vue'

import AccordionSection from '@/ui/AccordionSection.vue'
import AppAccordion from '@/ui/AppAccordion.vue'

afterEach(() => {
  document.body.innerHTML = ''
})

describe('AppAccordion', () => {
  const Sections = defineComponent({
    setup() {
      const open = ref(['move'])
      return () =>
        h(
          AppAccordion,
          {
            modelValue: open.value,
            'onUpdate:modelValue': (value: string[]) => (open.value = value),
          },
          () => [
            h(AccordionSection, { value: 'move', title: 'Move' }, () => 'Move content'),
            h(
              AccordionSection,
              { value: 'off', title: 'Off', disabled: true },
              () => 'Off content',
            ),
            h(AccordionSection, { value: 'capture', title: 'Capture' }, () => 'Capture content'),
          ],
        )
    },
  })

  const mountSections = async () => {
    const wrapper = mount(Sections, { attachTo: document.body })
    await flushPromises()
    return { wrapper, triggers: () => wrapper.findAll('button') }
  }

  it('has a heading with a button for each section, which controls its region', async () => {
    const { wrapper, triggers } = await mountSections()
    expect(wrapper.findAll('h3').map((heading) => heading.text())).toEqual([
      'Move',
      'Off',
      'Capture',
    ])
    for (const [trigger, expanded, text] of [
      [triggers()[0]!, 'true', 'Move content'],
      [triggers()[2]!, 'false', ''],
    ] as const) {
      expect(trigger.attributes('aria-expanded')).toBe(expanded)
      const region = wrapper.get(`[id="${trigger.attributes('aria-controls')}"]`)
      expect(region.attributes('role')).toBe('region')
      expect(region.attributes('aria-labelledby')).toBe(trigger.attributes('id'))
      expect(region.text()).toBe(text)
    }
  })

  it('opens and closes sections, any number at once, and reports them in its model', async () => {
    const { wrapper, triggers } = await mountSections()
    await triggers()[2]!.trigger('click')
    expect(wrapper.findComponent(AppAccordion).emitted('update:modelValue')?.at(-1)).toEqual([
      ['move', 'capture'],
    ])
    expect(wrapper.text()).toContain('Capture content')
    await triggers()[0]!.trigger('click')
    expect(triggers()[0]!.attributes('aria-expanded')).toBe('false')
    expect(wrapper.text()).not.toContain('Move content')
  })

  it('does not open a disabled section', async () => {
    const { wrapper, triggers } = await mountSections()
    expect(triggers()[1]!.element.disabled).toBe(true)
    expect(wrapper.text()).not.toContain('Off content')
  })

  it('moves between section titles with ↑, ↓, Home and End, skipping disabled ones', async () => {
    const { triggers } = await mountSections()
    const [move, , capture] = triggers()
    move!.element.focus()
    await move!.trigger('keydown', { key: 'ArrowDown' })
    expect(document.activeElement).toBe(capture!.element)
    await capture!.trigger('keydown', { key: 'ArrowDown' })
    expect(document.activeElement).toBe(move!.element)
    await move!.trigger('keydown', { key: 'End' })
    expect(document.activeElement).toBe(capture!.element)
    await capture!.trigger('keydown', { key: 'Home' })
    expect(document.activeElement).toBe(move!.element)
  })
})
