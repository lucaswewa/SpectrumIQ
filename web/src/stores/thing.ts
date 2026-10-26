import { computed, shallowRef, watch, type ShallowRef } from 'vue'

import type { components } from '@/api/generated/schema'
import { ApiError } from '@/api/wot/errors'
import type { Toaster } from '@/ui/overlays'

import { useConnectionStore } from './connection'

type Schemas = components['schemas']

/** The affordances of Thing `T` whose schemas are named `${T}_{name}_${Kind}` (the OpenAPI document). */
type Names<T extends string, Kind extends string> = keyof Schemas extends infer Key
  ? Key extends `${T}_${infer Name}_${Kind}`
    ? Name
    : never
  : never

/** The schema named `K`. A conditional type, so that it stays deferred in generic code. */
type Schema<K extends string> = K extends keyof Schemas ? Schemas[K] : never

/** The type of Thing `T`'s property `P`, from the OpenAPI document. */
export type Value<T extends string, P extends string> = Schema<`${T}_${P}_value`>
/** The input and output of Thing `T`'s action `A`. */
export type Input<T extends string, A extends string> = Schema<`${T}_${A}_input`>
export type Output<T extends string, A extends string> = Schema<`${T}_${A}_output`>
/** The data of Thing `T`'s event `E`. */
export type EventData<T extends string, E extends string> = Schema<`${T}_${E}_data`>

let toaster: Toaster | undefined

/** Where the stores' failures are shown: the app shell binds its toasts (ADR-0037). */
export function bindToaster(next: Toaster | undefined) {
  toaster = next
}

/** Shows `message` as a toast. */
export function notify(kind: 'success' | 'info' | 'error', message: string) {
  toaster?.[kind](message)
}

/** Shows an action's or a write's failure, with the server's error folded under it. */
export function notifyFailure(what: string, failure: unknown) {
  const error = ApiError.fromFailure(failure)
  if (error.kind === 'cancelled') return
  toaster?.error(`${what}: ${error.message}`, { details: error.body ?? error.message })
}

/**
 * Thing `name` of the connected server, for a store: its properties mirrored, and its actions
 * and writes, whose failures are shown as toasts.
 */
export function useThingMirror<T extends string>(name: T) {
  const connection = useConnectionStore()
  /** The Thing, while connected and the server has it. */
  const thing = computed(() => {
    const td = connection.connected ? connection.descriptions?.[name] : undefined
    return td && connection.client ? connection.client.consume(td) : undefined
  })
  const fitted = computed(() => thing.value !== undefined)

  /**
   * Property `p`, mirrored: read when the Thing is there, then followed if it is observable, or
   * read every `pollMs` if given. A value equal to the last is ignored.
   */
  function property<P extends Names<T, 'value'>>(p: P, { pollMs }: { pollMs?: number } = {}) {
    const value: ShallowRef<Value<T, P> | undefined> = shallowRef()
    const set = (next: unknown) => {
      if (JSON.stringify(next) !== JSON.stringify(value.value)) value.value = next as Value<T, P>
    }
    watch(
      thing,
      (consumed, _, onCleanup) => {
        if (!consumed) return
        const read = () => consumed.readProperty(p).then(set, () => {})
        void read()
        let stop: (() => void) | undefined
        let timer: ReturnType<typeof setInterval> | undefined
        if (consumed.td.properties?.[p]?.observable) stop = consumed.observeProperty(p, set)
        else if (pollMs) timer = setInterval(read, pollMs)
        onCleanup(() => {
          stop?.()
          clearInterval(timer)
        })
      },
      { immediate: true },
    )
    return value
  }

  /** Reads property `p` into `mirror` now, as after an action that changes it. */
  async function read<P extends Names<T, 'value'>>(p: P, mirror: { value: unknown }) {
    const consumed = thing.value
    if (consumed) mirror.value = await consumed.readProperty(p).catch(() => mirror.value)
  }

  /** Writes property `p`, then reads it back: whether the server took it. */
  async function write<P extends Names<T, 'value'>>(
    p: P,
    next: Value<T, P>,
    mirror?: { value: unknown },
  ): Promise<boolean> {
    const consumed = thing.value
    if (!consumed) return false
    try {
      await consumed.writeProperty(p, next)
      if (mirror) mirror.value = await consumed.readProperty(p)
      return true
    } catch (failure) {
      notifyFailure(`${consumed.td.title}: ${p} wasn't set`, failure)
      return false
    }
  }

  /** Runs action `a` to its end: its output, or undefined if it failed (shown as a toast). */
  async function run<A extends Names<T, 'input'>>(
    a: A,
    input?: Partial<Input<T, A>>,
  ): Promise<Output<T, A> | undefined> {
    const consumed = thing.value
    if (!consumed) return undefined
    try {
      const invocation = await consumed.invokeAction(a, input ?? {})
      return await invocation.output<Output<T, A>>()
    } catch (failure) {
      notifyFailure(`${consumed.td.title}: ${a} failed`, failure)
      return undefined
    }
  }

  /** Calls `handler` with each occurrence of event `e`, while the Thing is there. */
  function on<E extends Names<T, 'data'>>(e: E, handler: (data: EventData<T, E>) => void) {
    watch(
      thing,
      (consumed, _, onCleanup) => {
        // A Thing without the event (another version of the server) is left alone.
        if (!consumed?.td.events?.[e]) return
        onCleanup(consumed.subscribeEvent(e, (data) => handler(data as EventData<T, E>)))
      },
      { immediate: true },
    )
  }

  return { thing, fitted, property, read, write, run, on }
}
