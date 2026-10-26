<script setup lang="ts">
// The Calibration page and its wizard against a fake instrument (`fakeInstrument.ts`), without a
// server: the example of P5.4. The 10× in place lacks its calibrations, so the wizard opens for
// it. It runs in an app of its own, so the fake connection stays here. For development only.
import { createPinia } from 'pinia'
import { defineComponent, h, onBeforeUnmount, onMounted, useTemplateRef } from 'vue'

import CalibrationPage from '@/calibration/CalibrationPage.vue'
import CalibrationWizard from '@/calibration/CalibrationWizard.vue'
import { useThemeStore } from '@/theme/store'

import { mountDemo } from './fakeInstrument'

const host = useTemplateRef<HTMLElement>('host')
const theme = useThemeStore()
const page = defineComponent(() => () => [h(CalibrationPage), h(CalibrationWizard)])
let unmount: (() => void) | undefined

onMounted(() => (unmount = mountDemo(host.value!, createPinia(), page)))
onBeforeUnmount(() => unmount?.())
</script>

<template>
  <section class="calibration-demo" :data-theme="theme.theme">
    <p class="calibration-demo__note">
      The Calibration page and its wizard against a fake instrument: no server is needed. Each
      calibration takes a second or two, and finds plausible values.
    </p>
    <div ref="host" class="calibration-demo__page" />
  </section>
</template>

<style scoped>
.calibration-demo {
  display: flex;
  flex: 1;
  flex-direction: column;
  min-height: 0;
}

.calibration-demo__note {
  margin: 0;
  padding: var(--space-2) var(--space-4);
  border-bottom: var(--border-width) solid var(--color-border);
  color: var(--color-text-muted);
}

.calibration-demo__page {
  display: flex;
  flex: 1;
  min-height: 0;
}
</style>
