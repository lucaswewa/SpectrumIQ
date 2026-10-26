<script setup lang="ts">
// A sequence's form (ADR-0057, amended in P6.7), after `microscope`'s Sequence tab: where, in
// which channels, with an autofocus and a z-stack at each place, how often and how many times,
// and what a failed step does.
import { computed } from 'vue'

import { useImagingStore } from '@/stores/imaging'
import { useWorkflowStore } from '@/stores/workflow'
import AppSelect from '@/ui/AppSelect.vue'
import AppToggle from '@/ui/AppToggle.vue'
import FormField from '@/ui/FormField.vue'
import MultiSelect from '@/ui/MultiSelect.vue'
import NumberField from '@/ui/NumberField.vue'
import TextField from '@/ui/TextField.vue'

import type { SequenceForm } from './sequence'

const form = defineModel<SequenceForm>({ required: true })
const imaging = useImagingStore()
const workflow = useWorkflowStore()

const channels = computed(() =>
  (imaging.channels ?? []).map((channel) => ({ value: channel.id, label: channel.name })),
)
const positions = computed(() =>
  (workflow.positions ?? []).map((position) => ({ value: position.name, label: position.name })),
)
const ON_ERROR = [
  { value: 'stop', label: 'End the run' },
  { value: 'skip', label: 'Skip the step' },
  { value: 'ask', label: 'Wait, and ask' },
] as const
</script>

<template>
  <div class="sequence-form">
    <FormField label="Name">
      <TextField v-model="form.name" size="sm" />
    </FormField>
    <FormField label="Positions" help="Saved on the Live page; where the stage is if none.">
      <MultiSelect v-model="form.positions" :options="positions" size="sm" />
    </FormField>
    <FormField label="Channels" help="The one in use if none.">
      <MultiSelect v-model="form.channels" :options="channels" size="sm" />
    </FormField>
    <AppToggle v-model="form.autofocus.on" label="Autofocus at each position" />
    <FormField v-if="form.autofocus.on" label="Autofocus range, µm">
      <NumberField v-model="form.autofocus.rangeUm" :min="1" size="sm" />
    </FormField>
    <AppToggle v-model="form.stack.on" label="A z-stack at each position" />
    <template v-if="form.stack.on">
      <FormField label="Stack range, µm">
        <NumberField v-model="form.stack.rangeUm" :min="0" size="sm" />
      </FormField>
      <FormField label="Step, µm">
        <NumberField v-model="form.stack.stepUm" :min="0.1" :step="0.1" size="sm" />
      </FormField>
    </template>
    <AppToggle v-model="form.lapse.on" label="A time-lapse" />
    <template v-if="form.lapse.on">
      <FormField label="Every, s">
        <NumberField v-model="form.lapse.intervalS" :min="1" size="sm" />
      </FormField>
      <FormField label="Times">
        <NumberField v-model="form.lapse.count" :min="1" size="sm" />
      </FormField>
    </template>
    <FormField label="On a failure">
      <AppSelect v-model="form.onError" :options="ON_ERROR" size="sm" />
    </FormField>
  </div>
</template>

<style scoped>
.sequence-form {
  display: flex;
  flex-direction: column;
  gap: var(--space-3);
}
</style>
