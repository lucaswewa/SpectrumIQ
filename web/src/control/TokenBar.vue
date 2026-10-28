<script setup lang="ts">
// Who has control of the instrument, above every page (ADR-0068): this session, with Release;
// another, named, with Take over for an admin or the local console; or no one, with Take control
// for an operator. Then who is logged in, and Log out.
import { computed } from 'vue'

import { describeError } from '@/api/wot/errors'
import { useSessionStore } from '@/auth/session'
import AppButton from '@/ui/AppButton.vue'
import { useToast } from '@/ui/overlays'

import { useTokenStore } from './token'

const session = useSessionStore()
const token = useTokenStore()
const toast = useToast()

const role = computed(() => session.me?.role)
const operates = computed(() => role.value === 'operator' || role.value === 'admin')
const takesOver = computed(() => role.value === 'admin' || session.me?.local === true)
const other = computed(() => !token.holds && !!token.holder)
const status = computed(() => {
  if (token.holds) return 'You have control of the instrument.'
  if (token.holder) {
    const since = token.since
      ? new Date(token.since).toLocaleTimeString([], { timeStyle: 'short' })
      : ''
    return `${token.holder} has control since ${since}: you see what they do.`
  }
  return operates.value ? 'No one has control.' : 'View only.'
})

async function act(action: () => Promise<unknown>) {
  try {
    await action()
  } catch (error) {
    toast.error(describeError(error).message ?? 'The server refused.')
  }
}
</script>

<template>
  <div
    v-if="session.enforced && session.me"
    class="token-bar"
    :class="{ 'token-bar--other': other }"
    role="status"
    aria-label="Control of the instrument"
  >
    <span class="token-bar__status">{{ status }}</span>
    <AppButton v-if="token.holds" size="sm" @click="act(token.release)">Release control</AppButton>
    <AppButton
      v-else-if="operates && !token.holder"
      size="sm"
      variant="primary"
      @click="act(token.request)"
      >Take control</AppButton
    >
    <AppButton v-else-if="other && takesOver" size="sm" @click="act(token.takeOver)"
      >Take over</AppButton
    >
    <span class="token-bar__user">{{ session.me.name }}, {{ session.me.role }}</span>
    <AppButton size="sm" variant="ghost" @click="act(session.logout)">Log out</AppButton>
  </div>
</template>

<style scoped>
.token-bar {
  display: flex;
  gap: var(--space-3);
  align-items: center;
  padding: var(--space-1) var(--space-4);
  border-bottom: var(--border-width) solid var(--color-border);
  background: var(--color-surface-panel);
}

.token-bar--other {
  background: var(--color-surface-sunken);
  color: var(--color-warning);
}

.token-bar__status {
  flex: 1;
}

.token-bar__user {
  color: var(--color-text-muted);
}
</style>
