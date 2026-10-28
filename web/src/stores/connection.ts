import { defineStore } from 'pinia'
import { computed, ref, shallowRef } from 'vue'

import { WotClient } from '@/api/wot/client'
import { ApiError } from '@/api/wot/errors'
import { useSessionStore } from '@/auth/session'
import type { ThingDescription } from '@/api/wot/td'

/**
 * Where the connection stands:
 * - `idle`: not started;
 * - `connecting`: reaching the server for the first time;
 * - `connected`: it answers, with its Things' descriptions;
 * - `lost`: it stopped answering, or never did; a new attempt is scheduled;
 * - `reconnecting`: trying again.
 */
export type ConnectionState = 'idle' | 'connecting' | 'connected' | 'lost' | 'reconnecting'

/** What `/api/siq/system/health` says about the server. */
export interface Health {
  status: string
  version: string
  uptime_s: number
  camera: string
}

/** How often a connected app checks that the server is still there. */
export const HEARTBEAT_MS = 5000
/** The first wait before trying again; it doubles each time, up to the maximum. */
export const FIRST_RETRY_MS = 1000
export const MAX_RETRY_MS = 10_000

/**
 * The connection to the server that served the page: its client, its Things' descriptions, and
 * its health. The other stores follow it, so they stop and start again with it.
 */
export const useConnectionStore = defineStore('connection', () => {
  const state = ref<ConnectionState>('idle')
  const client = shallowRef<WotClient>()
  /** The Things' descriptions, kept while the connection is lost. */
  const descriptions = shallowRef<Readonly<Record<string, ThingDescription>>>()
  const health = shallowRef<Health>()
  /** Why the last attempt, or the connection, failed. */
  const error = shallowRef<unknown>()
  const connected = computed(() => state.value === 'connected')

  let attempt = 0
  let timer: ReturnType<typeof setTimeout> | undefined
  let retryDelay = FIRST_RETRY_MS

  function schedule(task: () => void, ms: number) {
    clearTimeout(timer)
    timer = setTimeout(task, ms)
  }

  /** The server's own routes, beside the Things' (`/api/siq/`). */
  function siqUrl(path: string) {
    return new URL(`../siq/${path}`, client.value!.baseUrl)
  }

  /** Connects to the API at `base`: `/api/v1/` of the server that served the page by default. */
  async function start(base: string | URL = '/api/v1/', wot = new WotClient({ baseUrl: base })) {
    clearTimeout(timer)
    attempt += 1
    client.value = wot
    descriptions.value = undefined
    health.value = undefined
    error.value = undefined
    retryDelay = FIRST_RETRY_MS
    state.value = 'connecting'
    await reach(attempt)
  }

  async function reach(current: number) {
    const wot = client.value!
    try {
      const [things, status] = await Promise.all([
        wot.thingDescriptions(),
        wot.request(siqUrl('system/health')),
      ])
      if (current !== attempt) return
      descriptions.value = things
      health.value = status as Health
      error.value = undefined
      retryDelay = FIRST_RETRY_MS
      state.value = 'connected'
      schedule(() => void heartbeat(current), HEARTBEAT_MS)
    } catch (failure) {
      if (current === attempt) lose(failure)
    }
  }

  async function heartbeat(current: number) {
    try {
      health.value = (await client.value!.request(siqUrl('system/health'))) as Health
      if (current === attempt) schedule(() => void heartbeat(current), HEARTBEAT_MS)
    } catch (failure) {
      if (current === attempt) lose(failure)
    }
  }

  function lose(failure: unknown) {
    // The session ended: the login asks again, and the connection waits for it.
    if (failure instanceof ApiError && failure.status === 401) {
      useSessionStore().expire()
      return stop()
    }
    error.value = failure
    state.value = 'lost'
    schedule(() => {
      state.value = 'reconnecting'
      void reach(attempt)
    }, retryDelay)
    retryDelay = Math.min(retryDelay * 2, MAX_RETRY_MS)
  }

  /** Tries again at once, after a failure. */
  function retry() {
    if (state.value !== 'lost') return
    clearTimeout(timer)
    retryDelay = FIRST_RETRY_MS
    state.value = 'reconnecting'
    void reach(attempt)
  }

  /** Stops, as at the end of a test. */
  function stop() {
    clearTimeout(timer)
    attempt += 1
    state.value = 'idle'
  }

  /** Whether the server has a Thing. Before any description is known, every Thing counts. */
  function isAvailable(thing: string): boolean {
    return descriptions.value === undefined || thing in descriptions.value
  }

  return { state, client, descriptions, health, error, connected, start, retry, stop, isAvailable }
})
