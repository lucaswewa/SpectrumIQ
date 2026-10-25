<script setup lang="ts">
// Every store's mirrored values, live from the connected server: the example of P3.2. For
// development only: the route exists only in development builds (src/router/index.ts).
import { computed } from 'vue'

import { streamBudget } from '@/api/streams'
import { useCameraStore } from '@/stores/camera'
import { useConnectionStore } from '@/stores/connection'
import { useDisplayStore } from '@/stores/display'
import { useFocusStore } from '@/stores/focus'
import { useImagingStore } from '@/stores/imaging'
import { usePeripheralsStore } from '@/stores/peripherals'
import { useStageStore } from '@/stores/stage'
import AppButton from '@/ui/AppButton.vue'
import AppCard from '@/ui/AppCard.vue'
import MainView from '@/ui/MainView.vue'
import SectionHeading from '@/ui/SectionHeading.vue'

const connection = useConnectionStore()
const camera = useCameraStore()
const display = useDisplayStore()
const stage = useStageStore()
const focus = useFocusStore()
const imaging = useImagingStore()
const peripherals = usePeripheralsStore()

const sockets = computed(() => {
  const things = Object.values(connection.descriptions ?? {})
  return things.filter((td) => JSON.stringify(td).includes('/ws"')).length
})

const sections = computed(() => [
  {
    title: 'Connection',
    values: {
      state: connection.state,
      health: connection.health,
      things: Object.keys(connection.descriptions ?? {}),
      'things with a socket': sockets.value,
      'streams running': streamBudget.active,
    },
  },
  {
    title: 'Camera',
    values: { stats: camera.stats, exposure_ms: camera.exposureMs, region: camera.region },
  },
  {
    title: 'Stage',
    values: { position: stage.position, state: stage.state, arrived: stage.arrived },
  },
  { title: 'Focus', values: { position: focus.position, state: focus.state } },
  {
    title: 'Display',
    values: { lut: display.lut, focus_measure: display.focusMeasure },
  },
  {
    title: 'Imaging',
    values: { channel: imaging.channel, current: imaging.current?.name },
  },
  {
    title: 'Peripherals',
    values: {
      levels: peripherals.lights.levels,
      filter: peripherals.filters.position,
      objective: peripherals.objectives.current?.name,
      modality: peripherals.modes.modality,
    },
  },
])
</script>

<template>
  <MainView class="observe">
    <h1>Observed values</h1>
    <p class="observe__note">
      Each value comes from a store, which follows its Thing over the Thing's observation socket, or
      polls what isn't observable. Move the stage to watch its position change.
    </p>
    <div class="observe__actions">
      <AppButton size="sm" @click="stage.moveBy({ dx: -500 })">Stage −500 µm in x</AppButton>
      <AppButton size="sm" @click="stage.moveBy({ dx: 500 })">Stage +500 µm in x</AppButton>
      <AppButton size="sm" @click="imaging.selectChannel(2)">Channel 2</AppButton>
      <AppButton size="sm" @click="imaging.selectChannel(1)">Channel 1</AppButton>
    </div>
    <div class="observe__grid">
      <AppCard v-for="section in sections" :key="section.title">
        <SectionHeading :level="3">{{ section.title }}</SectionHeading>
        <dl class="observe__values">
          <template v-for="(value, key) in section.values" :key="key">
            <dt>{{ key }}</dt>
            <dd>
              <code>{{ value === undefined ? '…' : JSON.stringify(value) }}</code>
            </dd>
          </template>
        </dl>
      </AppCard>
    </div>
  </MainView>
</template>

<style scoped>
.observe h1 {
  margin: 0 0 var(--space-2);
  font-size: var(--font-size-xl);
}

.observe__note {
  margin: 0 0 var(--space-3);
  color: var(--color-text-muted);
}

.observe__actions {
  display: flex;
  flex-wrap: wrap;
  gap: var(--space-2);
  margin-bottom: var(--space-4);
}

.observe__grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(360px, 1fr));
  gap: var(--space-4);
}

.observe__values {
  display: grid;
  grid-template-columns: max-content 1fr;
  gap: var(--space-1) var(--space-3);
  margin: 0;
}

.observe__values dt {
  color: var(--color-text-muted);
}

.observe__values dd {
  margin: 0;
  overflow-wrap: anywhere;
}

.observe__values code {
  font-family: var(--font-family-mono);
  font-size: var(--font-size-sm);
}
</style>
