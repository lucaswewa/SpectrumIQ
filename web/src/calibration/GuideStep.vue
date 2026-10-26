<script setup lang="ts">
// A step of the calibration wizard that says what to do, with the live image and the d-pad when
// they help.
import DirectionPad from '@/control/DirectionPad.vue'
import { useJog } from '@/control/useJog'
import MiniStream from '@/live/MiniStream.vue'

const props = defineProps<{
  lines: string[]
  bullets?: string[]
  stream?: boolean
  pad?: boolean
}>()
// The step is made afresh for each step of the wizard, so `pad` doesn't change. The jog
// provides the motion that the d-pad drives; the arrow keys stay the page's.
const jog = props.pad ? useJog({ keys: false }) : undefined
</script>

<template>
  <div class="wizard-step">
    <p v-for="line in lines" :key="line">{{ line }}</p>
    <ul v-if="bullets">
      <li v-for="bullet in bullets" :key="bullet">{{ bullet }}</li>
    </ul>
    <div v-if="stream || jog" class="wizard-step__view">
      <MiniStream v-if="stream" />
      <DirectionPad v-if="jog" />
    </div>
  </div>
</template>

<style>
/* Not scoped: the other steps share it. */
.wizard-step {
  display: flex;
  flex-direction: column;
  gap: var(--space-3);
}

.wizard-step p,
.wizard-step ul,
.wizard-step dl {
  margin: 0;
}

.wizard-step__view,
.wizard-step__actions {
  display: flex;
  gap: var(--space-6);
  align-items: center;
}

.wizard-step__actions {
  gap: var(--space-3);
}

.wizard-step__facts {
  display: grid;
  grid-template-columns: max-content 1fr;
  gap: var(--space-1) var(--space-4);
}

.wizard-step__facts dt {
  color: var(--color-text-muted);
}

.wizard-step__facts dd {
  margin: 0;
  font-variant-numeric: tabular-nums;
}
</style>
