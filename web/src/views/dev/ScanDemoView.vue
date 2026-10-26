<script setup lang="ts">
// The Scan page against a fake instrument (`fakeInstrument.ts`), without a server: the example of
// P6.6. The 20×, which is calibrated, is in place, so a scan may start. For development only.
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
  <section class="scan-demo" :data-theme="theme.theme">
    <p class="scan-demo__note">
      The Scan page against a fake instrument: no server is needed. Draw a region with Rectangle or
      Polygon; the estimate follows. A scan "runs" at once.
    </p>
    <div ref="host" class="scan-demo__page" />
  </section>
</template>

<style scoped>
.scan-demo {
  display: flex;
  flex: 1;
  flex-direction: column;
  min-height: 0;
}

.scan-demo__note {
  margin: 0;
  padding: var(--space-2) var(--space-4);
  border-bottom: var(--border-width) solid var(--color-border);
  color: var(--color-text-muted);
}

.scan-demo__page {
  display: flex;
  flex: 1;
  min-height: 0;
}
</style>
