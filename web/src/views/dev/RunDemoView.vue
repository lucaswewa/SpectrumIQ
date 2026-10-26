<script setup lang="ts">
// The run panel and the sequence setup, on the Scan page against a fake instrument
// (`fakeInstrument.ts`), without a server: the example of P6.7. A run takes a step every half
// second; one whose policy asks fails once, at its fourth step. For development only.
import { createPinia } from 'pinia'
import { onBeforeUnmount, onMounted, useTemplateRef } from 'vue'

import ScanPage from '@/scan/ScanPage.vue'
import { useThemeStore } from '@/theme/store'

import { mountDemo } from './fakeInstrument'
import { motionValues } from './fakeMotion'

const host = useTemplateRef<HTMLElement>('host')
const theme = useThemeStore()
let unmount: (() => void) | undefined

onMounted(() => {
  const objectives = motionValues['objective/objectives'] as unknown[]
  Object.assign(motionValues, { 'objective/position': 1, 'objective/current': objectives[1] })
  unmount = mountDemo(host.value!, createPinia(), ScanPage)
})
onBeforeUnmount(() => unmount?.())
</script>

<template>
  <section class="run-demo" :data-theme="theme.theme">
    <p class="run-demo__note">
      Runs against a fake instrument: no server is needed. Choose Sequence, set On a failure to
      "Wait, and ask", and Start: the run panel follows it, pauses and resumes it, waits at its
      fourth step for Retry or Skip, and cancels it once confirmed.
    </p>
    <div ref="host" class="run-demo__page" />
  </section>
</template>

<style scoped>
.run-demo {
  display: flex;
  flex: 1;
  flex-direction: column;
  min-height: 0;
}

.run-demo__note {
  margin: 0;
  padding: var(--space-2) var(--space-4);
  border-bottom: var(--border-width) solid var(--color-border);
  color: var(--color-text-muted);
}

.run-demo__page {
  display: flex;
  flex: 1;
  min-height: 0;
}
</style>
