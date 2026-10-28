<script setup lang="ts">
// The keyboard shortcuts' registry is created here, above the shell, so the
// shell and every page can register with it. The login stands in front of
// the shell while the server asks for one (ADR-0068).
import AppShell from './app/AppShell.vue'
import { provideShortcuts } from './app/shortcuts'
import LoginPage from './auth/LoginPage.vue'
import { useSessionStore } from './auth/session'
import OverlayProvider from './ui/OverlayProvider.vue'

provideShortcuts()
const session = useSessionStore()
</script>

<template>
  <OverlayProvider>
    <AppShell v-if="session.admitted" />
    <LoginPage v-else-if="session.checked" />
  </OverlayProvider>
</template>
