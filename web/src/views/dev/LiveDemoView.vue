<script setup lang="ts">
// The Live page against a fake instrument (`fakeInstrument.ts`), without a server: the example
// of P3.3. It runs in an app of its own, so the fake connection stays here. For development only.
import { createPinia } from 'pinia'
import { createApp, h, onBeforeUnmount, onMounted, useTemplateRef } from 'vue'

import { WotClient } from '@/api/wot/client'
import LivePage from '@/live/LivePage.vue'
import { useConnectionStore } from '@/stores/connection'
import { useThemeStore } from '@/theme/store'

import { fakeFetch } from './fakeInstrument'

const host = useTemplateRef<HTMLElement>('host')
const theme = useThemeStore()
let demo: ReturnType<typeof createApp> | undefined
const pinia = createPinia()

onMounted(() => {
  demo = createApp({ render: () => h(LivePage) })
  demo.use(pinia)
  const client = new WotClient({ baseUrl: '/api/v1/', fetch: fakeFetch, webSocket: null })
  void useConnectionStore(pinia).start(client.baseUrl, client)
  demo.mount(host.value!)
})
onBeforeUnmount(() => {
  // The demo's own connection, not the app's.
  useConnectionStore(pinia).stop()
  demo?.unmount()
})
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
