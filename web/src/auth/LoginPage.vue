<script setup lang="ts">
// The login, in front of the whole app while the server asks for one (ADR-0068): a name and a
// password, and the server's refusal if it refuses. The desktop on the instrument's computer
// logs in by itself, by the local secret, and never shows it.
import { ref } from 'vue'

import { describeError } from '@/api/wot/errors'
import AppButton from '@/ui/AppButton.vue'
import FormField from '@/ui/FormField.vue'
import TextField from '@/ui/TextField.vue'

import { useSessionStore } from './session'

const session = useSessionStore()
const name = ref('')
const password = ref('')
const error = ref<string>()
const busy = ref(false)

async function submit() {
  busy.value = true
  error.value = undefined
  try {
    await session.login(name.value.trim(), password.value)
  } catch (failure) {
    error.value = describeError(failure).message ?? 'The login failed.'
  } finally {
    password.value = ''
    busy.value = false
  }
}
</script>

<template>
  <main class="login">
    <form class="login__form" aria-label="Log in" @submit.prevent="submit">
      <h1 class="login__title">SpectrumIQ</h1>
      <FormField label="Name">
        <TextField v-model="name" autocomplete="username" />
      </FormField>
      <FormField label="Password" :error="error">
        <TextField v-model="password" type="password" autocomplete="current-password" />
      </FormField>
      <AppButton type="submit" variant="primary" :disabled="busy || !name || !password">
        Log in
      </AppButton>
    </form>
  </main>
</template>

<style scoped>
.login {
  display: grid;
  place-items: center;
  height: 100%;
  background: var(--color-surface-sunken);
}

.login__form {
  display: flex;
  flex-direction: column;
  gap: var(--space-3);
  width: 20rem;
  padding: var(--space-5);
  border: var(--border-width) solid var(--color-border);
  border-radius: var(--radius-md);
  background: var(--color-surface-panel);
}

.login__title {
  margin: 0;
  font-size: var(--font-size-lg);
}
</style>
