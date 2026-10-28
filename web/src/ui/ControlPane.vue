<script setup lang="ts">
// The fixed-width column of controls beside a page's main view: narrow
// (Settings) or wide (Control, Slide Scan, Sequence). It scrolls when its
// controls are taller than it, so they keep their heights even where a page
// lays them out in a flex column. With `need`, its controls are disabled, with
// the reason, while the session may not do that (ADR-0068).
import type { Need } from '@/auth/session'

import GatedFields from './GatedFields.vue'

withDefaults(defineProps<{ width?: 'narrow' | 'wide'; need?: Need }>(), {
  width: 'narrow',
  need: undefined,
})
</script>

<template>
  <aside class="control-pane" :class="`control-pane--${width}`">
    <GatedFields :need="need">
      <slot />
    </GatedFields>
  </aside>
</template>

<style scoped>
.control-pane {
  flex: none;
  height: 100%;
  padding: var(--space-3);
  overflow: hidden auto;
  border-right: var(--border-width) solid var(--color-border);
  background: var(--color-surface-panel);
  scrollbar-gutter: stable;
}

/* A flex column's children would otherwise shrink to fit, buttons too. */
.control-pane :deep(.gated-fields > *) {
  flex-shrink: 0;
}

.control-pane--narrow {
  width: var(--pane-width-narrow);
}

.control-pane--wide {
  width: var(--pane-width-wide);
}
</style>
