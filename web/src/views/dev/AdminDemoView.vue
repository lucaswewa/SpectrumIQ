<script setup lang="ts">
// The admin page, the control token bar and the Live page's controls as each user sees them,
// against fakes (`fakeAdmin.ts`, `fakeInstrument.ts`), without a server: the example of P9.4.
// Choose who is logged in, and the page; take control, and the Live page's controls come on
// (ADR-0068). For development only.
import { createPinia } from 'pinia'
import { defineComponent, h, onBeforeUnmount, onMounted, ref, useTemplateRef, watch } from 'vue'

import AdminPage from '@/admin/AdminPage.vue'
import { useSessionStore } from '@/auth/session'
import TokenBar from '@/control/TokenBar.vue'
import { useTokenStore } from '@/control/token'
import LivePage from '@/live/LivePage.vue'
import { useThemeStore } from '@/theme/store'
import AppSelect from '@/ui/AppSelect.vue'
import FormField from '@/ui/FormField.vue'

import { adminFetch, USERS } from './fakeAdmin'
import { mountDemo } from './fakeInstrument'

const WHO = USERS.slice(0, 3).map((user) => ({ value: user.name, label: user.name }))
const PAGES = [
  { value: 'admin', label: 'Admin' },
  { value: 'live', label: 'Live' },
]

const host = useTemplateRef<HTMLElement>('host')
const theme = useThemeStore()
const who = ref('ada')
const page = ref('admin')
const pinia = createPinia()
const shown = defineComponent(() => () => [
  h(TokenBar),
  h(page.value === 'admin' ? AdminPage : LivePage),
])
const real = globalThis.fetch
let unmount: (() => void) | undefined

/** Logs `name` in to the demo, without the token. */
function logIn(name: string) {
  const { role } = USERS.find((user) => user.name === name)!
  const since = new Date().toISOString()
  const me = { name, role, local: false, since, expires: since, holds_token: false }
  useTokenStore(pinia).lose()
  useSessionStore(pinia).$patch({ enforced: true, checked: true, me })
}

onMounted(() => {
  globalThis.fetch = adminFetch(real)
  unmount = mountDemo(host.value!, pinia, shown)
  logIn(who.value)
})
watch(who, logIn)
onBeforeUnmount(() => {
  unmount?.()
  globalThis.fetch = real
})
</script>

<template>
  <section class="admin-demo" :data-theme="theme.theme">
    <div class="admin-demo__note">
      <p>
        The users, the audit trail and the control token against a fake server: no server is needed.
        Log in as each to see what they may do, and why the rest is disabled.
      </p>
      <FormField label="Logged in as">
        <AppSelect v-model="who" :options="WHO" size="sm" />
      </FormField>
      <FormField label="Page">
        <AppSelect v-model="page" :options="PAGES" size="sm" />
      </FormField>
    </div>
    <div ref="host" class="admin-demo__page" />
  </section>
</template>

<style scoped>
.admin-demo {
  display: flex;
  flex: 1;
  flex-direction: column;
  min-height: 0;
}

.admin-demo__note {
  display: flex;
  gap: var(--space-4);
  align-items: end;
  padding: var(--space-2) var(--space-4);
  border-bottom: var(--border-width) solid var(--color-border);
  color: var(--color-text-muted);
}

.admin-demo__note p {
  flex: 1;
  margin: 0;
}

.admin-demo__page {
  display: flex;
  flex: 1;
  flex-direction: column;
  min-height: 0;
}
</style>
