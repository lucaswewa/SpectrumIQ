import { computed, type ComputedRef } from 'vue'

import { useTokenStore } from '@/control/token'

import { useSessionStore, type Need } from './session'

/**
 * Why a control that needs `need` is disabled for this session, or `undefined` while it may be
 * used (ADR-0068): controls are disabled with the reason, not hidden.
 */
export function useGate(need: () => Need | undefined): ComputedRef<string | undefined> {
  const session = useSessionStore()
  const token = useTokenStore()
  return computed(() => {
    const needed = need()
    return needed === undefined ? undefined : session.gate(needed, token.holds)
  })
}
