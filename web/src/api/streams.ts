import { onScopeDispose, readonly, ref, watch, type Ref } from 'vue'

/**
 * The most MJPEG streams open at once. A browser opens 6 HTTP/1.1 connections per server, and
 * each stream holds one: the rest are kept for requests (architecture §7.3, ADR-0037).
 */
export const MAX_STREAMS = 3

/** A stream the budget runs: `start` opens it, and returns the function that closes it. */
export interface BudgetedStream {
  start: () => () => void
  /** Where it is shown: the stream pauses while that is out of view. */
  element?: Element
}

/** A stream's place in the budget. */
export interface StreamTicket {
  /** Whether it runs now. */
  readonly running: Readonly<Ref<boolean>>
  /** Gives up its place, closing it if it runs. */
  release(): void
}

/** What the budget watches: the page's visibility, and the elements' intersection with it. */
export interface BudgetEnvironment {
  document?: Pick<Document, 'visibilityState' | 'addEventListener'>
  IntersectionObserver?: typeof IntersectionObserver
}

interface Entry {
  stream: BudgetedStream
  inView: boolean
  running: Ref<boolean>
  stop?: () => void
  observer?: IntersectionObserver
}

/**
 * Runs at most `limit` streams. Each stream wants to run while the page is visible and, if it
 * has an element, while that is in view. The first `limit` that want to run do, in the order
 * they were added; the others wait, and start by themselves when a place frees up.
 */
export class StreamBudget {
  private readonly entries: Entry[] = []

  constructor(
    readonly limit = MAX_STREAMS,
    private readonly env: BudgetEnvironment = globalThis,
  ) {
    env.document?.addEventListener('visibilitychange', () => this.update())
  }

  add(stream: BudgetedStream): StreamTicket {
    const Observer = this.env.IntersectionObserver
    const entry: Entry = { stream, inView: !stream.element || !Observer, running: ref(false) }
    if (stream.element && Observer) {
      // The first callback, soon after observing, says whether it is in view.
      entry.observer = new Observer((changes) => {
        entry.inView = changes.at(-1)?.isIntersecting ?? entry.inView
        this.update()
      })
      entry.observer.observe(stream.element)
    }
    this.entries.push(entry)
    this.update()
    return {
      running: readonly(entry.running),
      release: () => {
        const index = this.entries.indexOf(entry)
        if (index === -1) return
        this.entries.splice(index, 1)
        entry.observer?.disconnect()
        entry.stop?.()
        entry.running.value = false
        this.update()
      },
    }
  }

  /** How many streams run now. */
  get active(): number {
    return this.entries.filter((entry) => entry.running.value).length
  }

  /** Stops the streams that lost their place first, so that no more than `limit` ever run. */
  private update() {
    const shown = this.env.document?.visibilityState !== 'hidden'
    const wanted = this.entries.filter((entry) => shown && entry.inView).slice(0, this.limit)
    for (const entry of this.entries) {
      if (!wanted.includes(entry) && entry.stop) {
        entry.stop()
        entry.stop = undefined
        entry.running.value = false
      }
    }
    for (const entry of wanted) {
      if (!entry.stop) {
        entry.stop = entry.stream.start()
        entry.running.value = true
      }
    }
  }
}

/** The app's budget: every MJPEG stream goes through it. */
export const streamBudget = new StreamBudget()

/**
 * Runs `start` through the budget while `element` exists, for the calling component: it pauses
 * while the element is out of view or the page is hidden. Returns whether it runs.
 */
export function useBudgetedStream(
  element: Readonly<Ref<Element | null | undefined>>,
  start: () => () => void,
  budget: StreamBudget = streamBudget,
): Readonly<Ref<boolean>> {
  const running = ref(false)
  let ticket: StreamTicket | undefined
  let unwatch: (() => void) | undefined
  const stopWatch = watch(
    element,
    (shownIn) => {
      unwatch?.()
      ticket?.release()
      ticket = shownIn ? budget.add({ start, element: shownIn }) : undefined
      running.value = false
      unwatch = ticket && watch(ticket.running, (now) => (running.value = now), { immediate: true })
    },
    { immediate: true },
  )
  onScopeDispose(() => {
    stopWatch()
    unwatch?.()
    ticket?.release()
  })
  return readonly(running)
}
