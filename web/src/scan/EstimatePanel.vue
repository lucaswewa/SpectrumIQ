<script setup lang="ts">
// What a scan would take (ADR-0057): first what stops it, then the server's estimate, its tiles,
// frames, time, data and the disk's room, or why it couldn't be made; then what may go wrong.
import { computed } from 'vue'

import type { ScanEstimate } from '@/stores/workflow'

import { duration, size } from './plan'

const props = defineProps<{
  estimate?: ScanEstimate | string
  blockers: string[]
  warnings: string[]
}>()

const known = computed(() => (typeof props.estimate === 'object' ? props.estimate : undefined))
const disk = computed(() => {
  const free = known.value?.free_bytes
  if (free === undefined || free === null) return 'unknown'
  return `${size(free)} free${known.value!.fits ? '' : ': not enough'}`
})
const cautions = computed(() => [...props.warnings, ...(known.value?.warnings ?? [])])
</script>

<template>
  <section class="estimate" aria-label="The estimate">
    <ul v-if="blockers.length" class="estimate__list estimate__list--blocking">
      <li v-for="blocker in blockers" :key="blocker">{{ blocker }}</li>
    </ul>
    <p v-else-if="typeof estimate === 'string'" class="estimate__list--blocking">{{ estimate }}</p>
    <p v-else-if="!estimate" class="estimate__pending">Estimating…</p>
    <dl v-if="known && !blockers.length" class="estimate__facts">
      <dt>Tiles</dt>
      <dd>{{ known.tiles.length }}</dd>
      <dt>Frames</dt>
      <dd>{{ known.frames }}</dd>
      <dt>Time</dt>
      <dd>about {{ duration(known.seconds) }}</dd>
      <dt>Data</dt>
      <dd>{{ size(known.bytes) }}</dd>
      <dt>Disk</dt>
      <dd :class="{ 'estimate__list--blocking': !known.fits }">{{ disk }}</dd>
      <dt>Memory</dt>
      <dd>the frame pool's: it doesn't grow with the scan</dd>
    </dl>
    <ul v-if="cautions.length" class="estimate__list estimate__list--warning">
      <li v-for="caution in cautions" :key="caution">{{ caution }}</li>
    </ul>
  </section>
</template>

<style scoped>
.estimate {
  display: flex;
  flex-direction: column;
  gap: var(--space-2);
  font-size: var(--font-size-sm);
}

.estimate__facts {
  display: grid;
  grid-template-columns: auto 1fr;
  gap: var(--space-1) var(--space-3);
  margin: 0;
}

.estimate__facts dt {
  color: var(--color-text-muted);
}

.estimate__facts dd {
  margin: 0;
}

.estimate__list {
  margin: 0;
  padding-left: var(--space-4);
}

.estimate__list--blocking {
  color: var(--color-danger);
}

.estimate__list--warning {
  color: var(--color-warning);
}

.estimate__pending {
  margin: 0;
  color: var(--color-text-muted);
}
</style>
