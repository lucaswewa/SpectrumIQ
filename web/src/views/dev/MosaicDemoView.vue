<script setup lang="ts">
// The Mosaic page against a fake instrument (`fakeInstrument.ts`), without a server: the example
// of P7.4. The fake paints a field wherever its stage stands still. For development only.
import { createPinia } from 'pinia'
import { onBeforeUnmount, onMounted, useTemplateRef } from 'vue'

import MosaicPage from '@/mosaic/MosaicPage.vue'
import { useThemeStore } from '@/theme/store'

import { mountDemo } from './fakeInstrument'

const host = useTemplateRef<HTMLElement>('host')
const theme = useThemeStore()
let unmount: (() => void) | undefined

onMounted(() => (unmount = mountDemo(host.value!, createPinia(), MosaicPage)))
onBeforeUnmount(() => unmount?.())
</script>

<template>
  <section class="mosaic-demo" :data-theme="theme.theme">
    <p class="mosaic-demo__note">
      The Mosaic page against a fake instrument: no server is needed. Double-click to move the
      stage; each field it stops on is painted. Drag and zoom to look around.
    </p>
    <div ref="host" class="mosaic-demo__page" />
  </section>
</template>

<style scoped>
.mosaic-demo {
  display: flex;
  flex: 1;
  flex-direction: column;
  min-height: 0;
}

.mosaic-demo__note {
  margin: 0;
  padding: var(--space-2) var(--space-4);
  border-bottom: var(--border-width) solid var(--color-border);
  color: var(--color-text-muted);
}

.mosaic-demo__page {
  display: flex;
  flex: 1;
  min-height: 0;
}
</style>
