<script setup lang="ts">
// One channel's display in the viewer (ADR-0009, amended): shown or hidden, its colour, its window
// and gamma, and Reset, back to the channel's own. Changes go to the LUT store, which fetches new
// tiles once the controls rest.
import { computed } from 'vue'

import AppButton from '@/ui/AppButton.vue'
import AppCheckbox from '@/ui/AppCheckbox.vue'
import AppSlider from '@/ui/AppSlider.vue'
import FormField from '@/ui/FormField.vue'
import NumberField from '@/ui/NumberField.vue'

import { useViewerLuts, type ChannelView } from './lut'

const props = defineProps<{ c: number }>()
const luts = useViewerLuts()
const channel = computed(() => luts.channels[props.c])

/** The control's value, and its change. */
function bind<K extends 'min' | 'max' | 'gamma' | 'visible'>(key: K, fallback: ChannelView[K]) {
  return computed<ChannelView[K]>({
    get: () => channel.value?.[key] ?? fallback,
    set: (value) => luts.set(props.c, { [key]: value }),
  })
}
const [min, max, gamma] = [bind('min', 0), bind('max', 1), bind('gamma', 1)]
const visible = bind('visible', true)
const colour = computed({
  get: () => `#${channel.value?.colour ?? 'ffffff'}`,
  set: (value: string) => luts.set(props.c, { colour: value.slice(1).toLowerCase() }),
})
</script>

<template>
  <fieldset v-if="channel" class="channel-controls">
    <legend class="channel-controls__title">
      <AppCheckbox v-model="visible" :label="channel.label" />
      <input
        v-model="colour"
        type="color"
        class="channel-controls__colour"
        :aria-label="`${channel.label}'s colour`"
      />
    </legend>
    <div class="channel-controls__window">
      <FormField label="Black at">
        <NumberField v-model="min" size="sm" />
      </FormField>
      <FormField label="White at">
        <NumberField v-model="max" size="sm" />
      </FormField>
    </div>
    <FormField :label="`Gamma ${channel.gamma.toFixed(2)}`">
      <AppSlider
        v-model="gamma"
        :label="`${channel.label}'s gamma`"
        :min="0.2"
        :max="5"
        :step="0.05"
      />
    </FormField>
    <AppButton size="sm" @click="luts.reset(c)">Reset</AppButton>
  </fieldset>
</template>

<style scoped>
.channel-controls {
  display: grid;
  gap: var(--space-2);
  margin: 0;
  padding: var(--space-3);
  border: var(--border-width) solid var(--color-border);
  border-radius: var(--radius-md);
}

.channel-controls__title {
  display: flex;
  gap: var(--space-2);
  align-items: center;
  padding: 0 var(--space-1);
}

.channel-controls__colour {
  inline-size: var(--space-6);
  block-size: var(--space-6);
  padding: 0;
  border: none;
  background: none;
}

.channel-controls__window {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: var(--space-2);
}
</style>
