<script setup lang="ts">
// A slider for a number from `min` to `max` in steps of `step`, on Reka UI's
// Slider (ADR-0013). The arrow keys move it a step, Page Up and Page Down ten,
// and Home and End to its ends. `label` names it.
import { SliderRange, SliderRoot, SliderThumb, SliderTrack } from 'reka-ui'
import { computed } from 'vue'

const model = defineModel<number>({ default: 0 })
withDefaults(
  defineProps<{ label: string; min?: number; max?: number; step?: number; disabled?: boolean }>(),
  { min: 0, max: 100, step: 1, disabled: false },
)

// Reka UI's slider can have several thumbs, so its value is a list.
const values = computed({
  get: () => [model.value],
  set: ([value]) => {
    if (value !== undefined) model.value = value
  },
})
</script>

<template>
  <SliderRoot
    v-model="values"
    :min="min"
    :max="max"
    :step="step"
    :disabled="disabled"
    class="app-slider"
  >
    <SliderTrack class="app-slider__track">
      <SliderRange class="app-slider__range" />
    </SliderTrack>
    <SliderThumb class="app-slider__thumb" :aria-label="label" />
  </SliderRoot>
</template>

<style scoped>
.app-slider {
  position: relative;
  display: flex;
  align-items: center;
  width: 100%;
  height: var(--control-height-sm);
  touch-action: none;
  user-select: none;
}

.app-slider__track {
  position: relative;
  flex: 1;
  height: var(--space-1);
  border-radius: var(--radius-full);
  background: var(--color-control-border);
}

.app-slider__range {
  position: absolute;
  height: 100%;
  border-radius: inherit;
  background: var(--color-accent);
}

.app-slider__thumb {
  display: block;
  width: var(--space-4);
  height: var(--space-4);
  border: var(--border-width) solid var(--color-accent);
  border-radius: var(--radius-full);
  background: var(--color-surface-raised);
  box-shadow: var(--shadow-sm);
  cursor: grab;
}

.app-slider__thumb:focus-visible {
  outline: var(--focus-ring-width) solid var(--color-focus);
  outline-offset: var(--focus-ring-offset);
}

.app-slider[data-disabled] .app-slider__range,
.app-slider[data-disabled] .app-slider__thumb {
  border-color: var(--color-text-disabled);
  background: var(--color-text-disabled);
  cursor: not-allowed;
}
</style>
