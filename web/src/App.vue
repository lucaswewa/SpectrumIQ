<script setup lang="ts">
// The keyboard shortcuts' registry is created here, above the shell, so the
// shell and every page can register with it.
import AppShell from './app/AppShell.vue'
import { provideShortcuts } from './app/shortcuts'
import OverlayProvider from './ui/OverlayProvider.vue'

provideShortcuts()
</script>

<template>
  <OverlayProvider>
    <AppShell />
  </OverlayProvider>
</template>
