// The pixel probe (ADR-0045): the true values under the pointer, asked for once it rests for
// PROBE_MS. A newer position cancels the request for an older one, so the values shown are always
// the last position's.
import { onScopeDispose, shallowRef } from 'vue'

/** A pixel of the image: x to the right and y down, from the top left. */
export type Pixel = [number, number]

/** How long the pointer rests before its values are asked for. */
export const PROBE_MS = 120

/**
 * The probe: `move` with each position, or `undefined` when the pointer leaves, and `value`, what
 * `read` gave for the last position it rested at.
 */
export function useProbe<T>(
  read: (at: Pixel, signal: AbortSignal) => Promise<T>,
  delay = PROBE_MS,
) {
  const at = shallowRef<Pixel>()
  const value = shallowRef<T>()
  let timer: ReturnType<typeof setTimeout> | undefined
  let asking: AbortController | undefined

  function stop() {
    clearTimeout(timer)
    asking?.abort()
  }

  function move(next: Pixel | undefined) {
    stop()
    at.value = next
    if (!next) {
      value.value = undefined
      return
    }
    timer = setTimeout(() => {
      const current = (asking = new AbortController())
      read(next, current.signal).then(
        (found) => {
          if (!current.signal.aborted) value.value = found
        },
        () => {
          if (!current.signal.aborted) value.value = undefined
        },
      )
    }, delay)
  }

  onScopeDispose(stop)
  return { at, value, move, stop }
}
