<script setup lang="ts">
// An on/off switch and its label: `label`, or the default slot. A native
// checkbox with the `switch` role, so it is announced as on or off. Other
// attributes, such as `disabled`, go to the <input>.
defineOptions({ inheritAttrs: false })
defineProps<{ label?: string }>()
const model = defineModel<boolean>({ default: false })
</script>

<template>
  <label class="app-toggle">
    <input
      v-model="model"
      type="checkbox"
      role="switch"
      class="app-toggle__input"
      v-bind="$attrs"
    />
    <slot>{{ label }}</slot>
  </label>
</template>

<style scoped>
.app-toggle {
  display: inline-flex;
  gap: var(--space-2);
  align-items: center;
  cursor: pointer;
}

.app-toggle:has(:disabled) {
  color: var(--color-text-disabled);
  cursor: not-allowed;
}

/* A 36 × 20 px track with a 14 px knob, which slides right and turns the track accent when on. */
.app-toggle__input {
  --knob: 14px;

  display: grid;
  align-items: center;
  width: 36px;
  height: 20px;
  margin: 0;
  padding: 0 2px;
  border: var(--border-width) solid var(--color-control-border);
  border-radius: var(--radius-full);
  background: var(--color-control-bg);
  cursor: inherit;
  appearance: none;
  transition: background-color var(--duration-fast) var(--easing-standard);
}

.app-toggle__input::before {
  width: var(--knob);
  height: var(--knob);
  border-radius: var(--radius-full);
  background: var(--color-control-border);
  content: '';
  transition: transform var(--duration-normal) var(--easing-standard);
}

.app-toggle__input:hover:not(:disabled) {
  border-color: var(--color-control-border-hover);
}

.app-toggle__input:checked {
  border-color: var(--color-accent);
  background: var(--color-accent);
}

.app-toggle__input:checked::before {
  background: var(--color-on-accent);
  transform: translateX(calc(36px - 2 * 2px - 2 * var(--border-width) - var(--knob)));
}

.app-toggle__input:disabled {
  border-color: var(--color-border);
  background: var(--color-surface-sunken);
}

.app-toggle__input:disabled::before {
  background: var(--color-text-disabled);
}
</style>
