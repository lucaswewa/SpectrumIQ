<script setup lang="ts">
// A label above one control (TextField, NumberField or AppSelect), with
// optional help and error texts below it. The control takes its id from the
// field, is described by the texts, and is marked invalid while there is an
// error.
import errorIcon from '@material-symbols/svg-400/outlined/error.svg?raw'
import { computed, provide, useId } from 'vue'

import AppIcon from './AppIcon.vue'
import { fieldKey } from './field'

const props = defineProps<{ label: string; help?: string; error?: string }>()

const id = useId()
provide(fieldKey, {
  id,
  describedBy: computed(
    () =>
      [props.help && `${id}-help`, props.error && `${id}-error`].filter(Boolean).join(' ') ||
      undefined,
  ),
  invalid: computed(() => Boolean(props.error)),
})
</script>

<template>
  <div class="form-field">
    <label :for="id" class="form-field__label">{{ label }}</label>
    <slot />
    <p v-if="help" :id="`${id}-help`" class="form-field__text">{{ help }}</p>
    <p v-if="error" :id="`${id}-error`" class="form-field__text form-field__text--error">
      <AppIcon :svg="errorIcon" size="var(--icon-size-xs)" />
      {{ error }}
    </p>
  </div>
</template>

<style scoped>
.form-field {
  display: flex;
  flex-direction: column;
  gap: var(--space-1);
}

.form-field__label {
  font-weight: var(--font-weight-medium);
}

.form-field__text {
  display: flex;
  gap: var(--space-1);
  align-items: center;
  margin: 0;
  color: var(--color-text-muted);
  font-size: var(--font-size-sm);
}

.form-field__text--error {
  color: var(--color-danger);
}
</style>
