import { defineStore } from 'pinia'
import { ref, watch } from 'vue'

import { siq } from '@/api/siq'
import { useSessionStore } from '@/auth/session'
import { useThingMirror } from '@/stores/thing'

/** How often the holder says it is still there: a third of the server's 30 s. */
export const HEARTBEAT_MS = 10_000

/** `POST /api/siq/control/<action>`. */
const post = (action: string) => siq(`control/${action}`, { method: 'POST' })

/**
 * The Hardware Control Token (ADR-0066, ADR-0068): who holds it, from the `control` Thing, and
 * whether this session does; requesting, releasing and taking it over; and the heartbeat that
 * keeps it while this session holds it.
 */
export const useTokenStore = defineStore('token', () => {
  const session = useSessionStore()
  const control = useThingMirror('control')
  const holder = control.property('holder')
  const since = control.property('since')
  const holds = ref(false)
  let beat: ReturnType<typeof setInterval> | undefined

  function keep() {
    holds.value = true
    clearInterval(beat)
    beat = setInterval(() => void post('heartbeat').catch(lose), HEARTBEAT_MS)
  }

  function lose() {
    holds.value = false
    clearInterval(beat)
  }

  // A session that holds it at a reload keeps it; no one's or another's name on it means it was
  // lost, but not a holder not yet known.
  watch(
    () => session.me?.holds_token,
    (held) => held && keep(),
    { immediate: true },
  )
  watch([holder, () => session.me?.name], ([who, me]) => {
    if (holds.value && who !== undefined && who !== me) lose()
  })

  return {
    fitted: control.fitted,
    holder,
    since,
    holds,
    request: () => post('request').then(keep),
    release: () => post('release').finally(lose),
    takeOver: () => post('take_over').then(keep),
    lose,
  }
})
