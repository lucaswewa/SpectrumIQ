<script setup lang="ts">
// The stage, focus and peripheral controls against a fake instrument (`fakeMotion.ts`), without
// a server: the example of P3.4. Beside the Live page, the commands the controls sent, so that the
// input mapping shows (ADR-0039). For development only.
import { createPinia } from 'pinia'
import { onBeforeUnmount, onMounted, useTemplateRef } from 'vue'

import { useThemeStore } from '@/theme/store'

import { mountDemo } from './fakeInstrument'
import { sent } from './fakeMotion'

const host = useTemplateRef<HTMLElement>('host')
const theme = useThemeStore()
let unmount: (() => void) | undefined

onMounted(() => (unmount = mountDemo(host.value!, createPinia())))
onBeforeUnmount(() => unmount?.())
</script>

<template>
  <section class="control-demo" :data-theme="theme.theme">
    <p class="control-demo__note">
      The controls against a fake stage, focus and peripherals: no server is needed. Tap or hold the
      arrow keys, PgUp and PgDn, or the pad; turn the wheel on the image; double-click the overview;
      press Esc or STOP. The sample moves with the stage and blurs out of focus.
    </p>
    <div class="control-demo__body">
      <div ref="host" class="control-demo__page" />
      <aside class="control-demo__sent" aria-labelledby="control-demo-sent">
        <h2 id="control-demo-sent">Sent, newest first</h2>
        <ol>
          <li v-for="(command, index) in sent" :key="sent.length - index">{{ command }}</li>
        </ol>
      </aside>
    </div>
  </section>
</template>

<style scoped>
.control-demo {
  display: flex;
  flex: 1;
  flex-direction: column;
  min-height: 0;
}

.control-demo__note {
  margin: 0;
  padding: var(--space-2) var(--space-4);
  border-bottom: var(--border-width) solid var(--color-border);
  color: var(--color-text-muted);
}

.control-demo__body {
  display: flex;
  flex: 1;
  min-height: 0;
}

.control-demo__page {
  display: flex;
  flex: 1;
  min-width: 0;
}

.control-demo__sent {
  width: 18rem;
  padding: var(--space-3);
  overflow: auto;
  border-left: var(--border-width) solid var(--color-border);
  font-family: var(--font-family-mono);
  font-size: var(--font-size-sm);
}

.control-demo__sent h2 {
  margin: 0 0 var(--space-2);
  font-size: var(--font-size-sm);
}

.control-demo__sent ol {
  margin: 0;
  padding-left: var(--space-4);
}
</style>
