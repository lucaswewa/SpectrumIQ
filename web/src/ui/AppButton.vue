<script setup lang="ts">
// A button: `default` (accent outline), `primary` (accent fill), `danger`
// (for destructive actions) or `ghost` (no outline), in two sizes. A native
// <button type="button">; pass `type="submit"` in a form. Other attributes,
// such as `disabled`, go to the button.
import AppIcon from './AppIcon.vue'

withDefaults(
  defineProps<{
    variant?: 'default' | 'primary' | 'danger' | 'ghost'
    size?: 'sm' | 'md'
    /** An icon before the label, as raw SVG text (see AppIcon). */
    icon?: string
  }>(),
  { variant: 'default', size: 'md', icon: undefined },
)
</script>

<template>
  <button
    type="button"
    class="app-button"
    :class="[`app-button--${variant}`, `app-button--${size}`]"
  >
    <AppIcon v-if="icon" :svg="icon" :size="size === 'sm' ? 'var(--icon-size-sm)' : undefined" />
    <slot />
  </button>
</template>

<style scoped>
.app-button {
  display: inline-flex;
  gap: var(--space-1);
  align-items: center;
  justify-content: center;
  height: var(--control-height-md);
  padding: 0 var(--control-padding-x);
  border: var(--border-width) solid var(--button-border, transparent);
  border-radius: var(--radius-sm);
  background: var(--button-bg, transparent);
  color: var(--button-text);
  font: inherit;
  font-weight: var(--font-weight-medium);
  line-height: var(--line-height-tight);
  white-space: nowrap;
  cursor: pointer;
  transition:
    background-color var(--duration-fast) var(--easing-standard),
    border-color var(--duration-fast) var(--easing-standard);
}

.app-button--sm {
  height: var(--control-height-sm);
  padding: 0 var(--space-2);
  font-size: var(--font-size-sm);
}

.app-button--default {
  --button-bg: var(--color-control-bg);
  --button-border: var(--color-accent-text);
  --button-text: var(--color-accent-text);
  --button-hover-bg: var(--color-accent-subtle);
}

.app-button--primary {
  --button-bg: var(--color-accent);
  --button-border: var(--color-accent);
  --button-text: var(--color-on-accent);
  --button-hover-bg: var(--color-accent-hover);
  --button-pressed-bg: var(--color-accent-pressed);
}

.app-button--danger {
  --button-bg: var(--color-control-bg);
  --button-border: var(--color-danger);
  --button-text: var(--color-danger);
  --button-hover-bg: color-mix(in srgb, var(--color-danger) 12%, var(--color-control-bg));
}

.app-button--ghost {
  --button-text: var(--color-text);
  --button-hover-bg: color-mix(in srgb, currentcolor 10%, transparent);
}

.app-button:hover:not(:disabled) {
  background: var(--button-hover-bg);
}

.app-button:active:not(:disabled) {
  background: var(--button-pressed-bg, var(--button-hover-bg));
}

.app-button:disabled {
  border-color: var(--color-border);
  background: transparent;
  color: var(--color-text-disabled);
  cursor: not-allowed;
}

.app-button--ghost:disabled {
  border-color: transparent;
}
</style>
