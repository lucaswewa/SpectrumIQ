import { defineStore } from 'pinia'
import { computed, ref } from 'vue'

import { siq } from '@/api/siq'
import { ApiError } from '@/api/wot/errors'
import type { HostAdapter } from '@/host/types'

/** What a user may do (ADR-0065): each role may do all that the one before it may. */
export type Role = 'viewer' | 'operator' | 'admin'

/** Who the session is: `/api/siq/auth/me`. */
export interface Me {
  name: string
  role: Role
  local: boolean
  since: string
  expires: string
  holds_token: boolean
}

/** What a control needs (ADR-0066): to process or stop, to change the instrument, or an admin. */
export type Need = 'process' | 'operate' | 'admin'

const RANK: Record<Role, number> = { viewer: 0, operator: 1, admin: 2 }

/** `/api/siq/auth/<path>`: a POST of `body`, or a GET without one. */
const send = (path: string, body?: unknown) =>
  siq(`auth/${path}`, body === undefined ? {} : { body })

/**
 * The session (ADR-0065, ADR-0068): who is logged in, whether the server asks for a login at all,
 * and what the user may do. A 401 anywhere ends it, and the app asks again.
 */
export const useSessionStore = defineStore('session', () => {
  const me = ref<Me>()
  /**
   * Whether the server enforces who may do what: off, nothing is asked or gated. Off until the
   * server says, as in tests and the development pages: the shell waits for `checked` anyway.
   */
  const enforced = ref(false)
  const checked = ref(false)

  /** Whether the app may go on: logged in, or the server doesn't ask. */
  const admitted = computed(() => checked.value && (me.value !== undefined || !enforced.value))

  /** Asks the server who the session is. */
  async function check() {
    try {
      me.value = (await send('me')) as Me
    } catch (error) {
      me.value = undefined
      const body = error instanceof ApiError ? (error.body as { enforced?: boolean }) : undefined
      enforced.value = body?.enforced !== false
    } finally {
      checked.value = true
    }
  }

  /** Checks the session; on the instrument's own computer, logs in by the local secret. */
  async function start(host?: HostAdapter) {
    await check()
    if (me.value || !enforced.value || !host?.localSecret) return
    try {
      me.value = (await send('local', { secret: await host.localSecret() })) as Me
    } catch {
      // No local console here: the login page asks.
    }
  }

  async function login(name: string, password: string) {
    me.value = (await send('login', { name, password })) as Me
    checked.value = true
  }

  async function logout() {
    await send('logout', {}).catch(() => undefined)
    me.value = undefined
  }

  /** The session ended on the server: a 401. */
  function expire() {
    if (enforced.value) me.value = undefined
  }

  /**
   * Whether the user may do what `need` asks, holding the token if `holds`, and if not, why: a
   * control disabled shows it.
   */
  function gate(need: Need, holds: boolean): string | undefined {
    if (!enforced.value) return undefined
    const role = me.value?.role
    if (!role) return 'Log in first.'
    if (need === 'admin' && role !== 'admin') return 'Only an admin can do this.'
    if (RANK[role] < RANK.operator) return 'View only: your role is viewer.'
    if (need === 'operate' && !holds) return 'Take control to change the instrument.'
    return undefined
  }

  return { me, enforced, checked, admitted, check, start, login, logout, expire, gate }
})
