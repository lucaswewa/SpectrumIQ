<script setup lang="ts">
// A checkbox and its label: `label`, or the default slot. A native checkbox,
// so the label, Space and forms work as usual. Other attributes, such as
// `disabled` or `name`, go to the <input>.
import check from '@material-symbols/svg-400/outlined/check.svg?raw'

import AppIcon from './AppIcon.vue'

defineOptions({ inheritAttrs: false })
defineProps<{ label?: string }>()
const model = defineModel<boolean>({ default: false })
</script>

<template>
  <label class="app-checkbox">
    <span class="app-checkbox__box">
      <input v-model="model" type="checkbox" class="app-checkbox__input" v-bind="$attrs" />
      <AppIcon :svg="check" size="var(--icon-size-xs)" class="app-checkbox__mark" />
    </span>
    <slot>{{ label }}</slot>
  </label>
</template>

<style scoped>
.app-checkbox {
  display: inline-flex;
  gap: var(--space-2);
  align-items: center;
  cursor: pointer;
}

.app-checkbox:has(:disabled) {
  color: var(--color-text-disabled);
  cursor: not-allowed;
}

.app-checkbox__box {
  position: relative;
  display: grid;
}

.app-checkbox__input {
  width: var(--icon-size-xs);
  height: var(--icon-size-xs);
  margin: 0;
  border: var(--border-width) solid var(--color-control-border);
  border-radius: var(--radius-sm);
  background: var(--color-control-bg);
  cursor: inherit;
  appearance: none;
}

.app-checkbox__input:hover:not(:disabled) {
  border-color: var(--color-control-border-hover);
}

.app-checkbox__input:checked {
  border-color: var(--color-accent);
  background: var(--color-accent);
}

.app-checkbox__input:disabled {
  border-color: var(--color-border);
  background: var(--color-surface-sunken);
}

.app-checkbox__mark {
  position: absolute;
  color: var(--color-on-accent);
  visibility: hidden;
  pointer-events: none;
}

.app-checkbox__input:checked + .app-checkbox__mark {
  visibility: visible;
}

.app-checkbox__input:checked:disabled + .app-checkbox__mark {
  color: var(--color-text-disabled);
}
</style>
