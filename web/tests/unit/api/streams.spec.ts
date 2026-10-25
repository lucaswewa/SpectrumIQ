// REQ-PRV-001: the MJPEG stream budget (architecture §7.3, ADR-0037).
import { describe, expect, it } from 'vitest'
import { effectScope, nextTick, ref } from 'vue'

import { StreamBudget, useBudgetedStream, type BudgetEnvironment } from '@/api/streams'

/** A page whose visibility, and whose elements' intersection, the test sets. */
function fakePage() {
  const listeners: (() => void)[] = []
  const observers: { callback: IntersectionObserverCallback; target?: Element }[] = []
  const page = {
    visibilityState: 'visible' as DocumentVisibilityState,
    addEventListener: (_: string, listener: () => void) => listeners.push(listener),
  }
  class Observer {
    constructor(callback: IntersectionObserverCallback) {
      observers.push({ callback })
      this.index = observers.length - 1
    }
    index: number
    observe(target: Element) {
      observers[this.index]!.target = target
    }
    disconnect() {
      observers[this.index]!.target = undefined
    }
  }
  const env = { document: page, IntersectionObserver: Observer } as unknown as BudgetEnvironment
  return {
    env,
    setHidden(hidden: boolean) {
      page.visibilityState = hidden ? 'hidden' : 'visible'
      listeners.forEach((listener) => listener())
    },
    setInView(target: Element, inView: boolean) {
      for (const observer of observers.filter((each) => each.target === target)) {
        const change = { isIntersecting: inView, target } as IntersectionObserverEntry
        observer.callback([change], {} as IntersectionObserver)
      }
    },
  }
}

/** A stream that records whether it runs. */
function stream(log: string[], name: string, element?: Element) {
  return {
    element,
    start: () => {
      log.push(`+${name}`)
      return () => log.push(`-${name}`)
    },
  }
}

describe('the stream budget', () => {
  it('runs three streams at most, in the order asked, and the next when one ends', () => {
    const page = fakePage()
    const budget = new StreamBudget(3, page.env)
    const log: string[] = []
    const tickets = ['a', 'b', 'c', 'd'].map((name) => budget.add(stream(log, name)))
    expect(log).toEqual(['+a', '+b', '+c'])
    expect(tickets.map((ticket) => ticket.running.value)).toEqual([true, true, true, false])
    tickets[1]!.release()
    expect(log).toEqual(['+a', '+b', '+c', '-b', '+d'])
    expect(budget.active).toBe(3)
  })

  it('stops every stream while the page is hidden, and starts them again when it shows', () => {
    const page = fakePage()
    const budget = new StreamBudget(3, page.env)
    const log: string[] = []
    budget.add(stream(log, 'overview'))
    budget.add(stream(log, 'roi'))
    page.setHidden(true)
    expect(budget.active).toBe(0)
    page.setHidden(false)
    expect(log).toEqual(['+overview', '+roi', '-overview', '-roi', '+overview', '+roi'])
  })

  it('pauses a stream out of view, giving its place to the next, and resumes it in view', () => {
    const page = fakePage()
    const budget = new StreamBudget(2, page.env)
    const log: string[] = []
    const elements = ['a', 'b', 'c'].map(() => document.createElement('div'))
    elements.forEach((element, index) => budget.add(stream(log, 'abc'[index]!, element)))
    // Nothing runs until the observer says what is in view.
    expect(log).toEqual([])
    elements.forEach((element) => page.setInView(element, true))
    expect(log).toEqual(['+a', '+b'])
    page.setInView(elements[0]!, false)
    expect(log).toEqual(['+a', '+b', '-a', '+c'])
    page.setInView(elements[0]!, true)
    expect(log).toEqual(['+a', '+b', '-a', '+c', '-c', '+a'])
  })

  it('runs a component’s stream while its element exists, and ends it with the component', async () => {
    const page = fakePage()
    const budget = new StreamBudget(3, page.env)
    const log: string[] = []
    const element = ref<Element | null>(null)
    const scope = effectScope()
    const running = scope.run(() => useBudgetedStream(element, stream(log, 'live').start, budget))!
    expect(running.value).toBe(false)
    element.value = document.createElement('img')
    await nextTick()
    page.setInView(element.value, true)
    await nextTick()
    expect([running.value, log]).toEqual([true, ['+live']])
    scope.stop()
    expect(log).toEqual(['+live', '-live'])
    expect(budget.active).toBe(0)
  })
})
