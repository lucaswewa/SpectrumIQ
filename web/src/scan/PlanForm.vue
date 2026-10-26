<script setup lang="ts">
// A tile scan's plan (ADR-0057): its name, channels, overlap and order, how its tiles are focused,
// and a z-stack at each. The objective is the one in use, chosen on the Live page.
import { computed } from 'vue'

import { useImagingStore } from '@/stores/imaging'
import AppSelect from '@/ui/AppSelect.vue'
import AppToggle from '@/ui/AppToggle.vue'
import FormField from '@/ui/FormField.vue'
import MultiSelect from '@/ui/MultiSelect.vue'
import NumberField from '@/ui/NumberField.vue'
import TextField from '@/ui/TextField.vue'

import type { ScanForm } from './plan'

defineProps<{ objective?: string }>()
const form = defineModel<ScanForm>({ required: true })
const imaging = useImagingStore()

const channels = computed(() =>
  (imaging.channels ?? []).map((channel) => ({ value: channel.id, label: channel.name })),
)
const ORDERS = [
  { value: 'snake', label: 'Snake' },
  { value: 'raster', label: 'Raster' },
  { value: 'nearest', label: 'Nearest first' },
] as const
const FOCUS = [
  { value: 'none', label: 'As it is' },
  { value: 'fixed', label: 'At one focus' },
  { value: 'map', label: 'On a focus map' },
  { value: 'autofocus', label: 'Autofocus at each tile' },
] as const
const DIRECTIONS = [
  { value: 'up', label: 'Up' },
  { value: 'down', label: 'Down' },
] as const
const overlap = computed({
  get: () => Math.round(form.value.overlap * 100),
  set: (percent: number) => (form.value.overlap = percent / 100),
})
</script>

<template>
  <div class="plan-form">
    <p class="plan-form__note">Objective: {{ objective ?? 'unknown' }}</p>
    <FormField label="Name">
      <TextField v-model="form.name" size="sm" />
    </FormField>
    <FormField label="Channels" help="The one in use if none.">
      <MultiSelect v-model="form.channels" :options="channels" size="sm" />
    </FormField>
    <FormField label="Overlap, %">
      <NumberField v-model="overlap" :min="0" :max="90" size="sm" />
    </FormField>
    <FormField label="Order">
      <AppSelect v-model="form.order" :options="ORDERS" size="sm" />
    </FormField>
    <FormField label="Focus">
      <AppSelect v-model="form.focus" :options="FOCUS" size="sm" />
    </FormField>
    <FormField v-if="form.focus === 'fixed'" label="Focus, µm">
      <NumberField v-model="form.focusZ" size="sm" />
    </FormField>
    <p v-if="form.focus === 'map'" class="plan-form__note">
      {{ form.places.length }} places picked: pick three or more on the map with Places.
    </p>
    <FormField
      v-if="form.focus === 'map' || form.focus === 'autofocus'"
      label="Autofocus range, µm"
    >
      <NumberField v-model="form.rangeUm" :min="1" size="sm" />
    </FormField>
    <AppToggle v-model="form.stack.on" label="A z-stack at each tile" />
    <template v-if="form.stack.on">
      <FormField label="Stack range, µm">
        <NumberField v-model="form.stack.rangeUm" :min="0" size="sm" />
      </FormField>
      <FormField label="Step, µm">
        <NumberField v-model="form.stack.stepUm" :min="0.1" :step="0.1" size="sm" />
      </FormField>
      <FormField label="Direction">
        <AppSelect v-model="form.stack.direction" :options="DIRECTIONS" size="sm" />
      </FormField>
    </template>
  </div>
</template>

<style scoped>
.plan-form {
  display: flex;
  flex-direction: column;
  gap: var(--space-3);
}

.plan-form__note {
  margin: 0;
  color: var(--color-text-muted);
  font-size: var(--font-size-sm);
}
</style>
