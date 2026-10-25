<script setup lang="ts">
// The Live page against a fake instrument (`fakeInstrument.ts`), without a server: the example
// of P3.3. It runs in an app of its own, so the fake connection stays here. For development only.
import { createPinia } from 'pinia'
import { onBeforeUnmount, onMounted, useTemplateRef } from 'vue'

import { useThemeStore } from '@/theme/store'

import { mountDemo } from './fakeInstrument'

const host = useTemplateRef<HTMLElement>('host')
const theme = useThemeStore()
let unmount: (() => void) | undefined

onMounted(() => (unmount = mountDemo(host.value!, createPinia())))
onBeforeUnmount(() => unmount?.())
</script>

<template>
  <section class="live-demo" :data-theme="theme.theme">
    <p class="live-demo__note">
      The Live page against a fake camera, drawn in the browser: no server is needed. Drag on the
      overview to choose the ROI, and move the histogram's markers.
    </p>
    <div ref="host" class="live-demo__page" />
  </section>
</template>

<style scoped>
.live-demo {
  display: flex;
  flex: 1;
  flex-direction: column;
  min-height: 0;
}

.live-demo__note {
  margin: 0;
  padding: var(--space-2) var(--space-4);
  border-bottom: var(--border-width) solid var(--color-border);
  color: var(--color-text-muted);
}

.live-demo__page {
  display: flex;
  flex: 1;
  min-height: 0;
}
</style>
