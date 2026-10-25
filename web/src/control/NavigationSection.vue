<script setup lang="ts">
// The navigation preferences: how far a step of the d-pad, the keys or the wheel goes, with
// presets for x and y, and which of x and y move the other way. Kept in this browser.
import AppButton from '@/ui/AppButton.vue'
import AppCheckbox from '@/ui/AppCheckbox.vue'
import FormField from '@/ui/FormField.vue'
import NumberField from '@/ui/NumberField.vue'

import { useNavigationPreferences, XY_PRESETS } from './navigation'

const preferences = useNavigationPreferences()
</script>

<template>
  <div class="navigation-section">
    <div class="navigation-section__presets" role="group" aria-label="XY step">
      <AppButton
        v-for="preset in XY_PRESETS"
        :key="preset"
        size="sm"
        :variant="preferences.steps.xy === preset ? 'primary' : 'default'"
        :aria-pressed="preferences.steps.xy === preset"
        @click="preferences.steps.xy = preset"
      >
        {{ preset }} µm
      </AppButton>
    </div>
    <FormField label="XY step (µm)" help="A press moves a step; holding moves 4 steps a second.">
      <NumberField v-model="preferences.steps.xy" size="sm" :min="0.1" :step="1" />
    </FormField>
    <FormField label="Coarse z step (µm)">
      <NumberField v-model="preferences.steps.z" size="sm" :min="0.1" :step="1" />
    </FormField>
    <FormField label="Focus step (µm)" help="A notch of the wheel, or a press of Focus up or down.">
      <NumberField v-model="preferences.steps.focus" size="sm" :min="0.01" :step="0.1" />
    </FormField>
    <AppCheckbox v-model="preferences.invert.x" label="Invert x" />
    <AppCheckbox v-model="preferences.invert.y" label="Invert y" />
  </div>
</template>

<style scoped>
.navigation-section {
  display: flex;
  flex-direction: column;
  gap: var(--space-2);
}

.navigation-section__presets {
  display: flex;
  gap: var(--space-1);
}
</style>
