<script setup lang="ts">
// The dialog that `?` opens, listing every registered shortcut by group.
import { computed, ref } from 'vue'

import AppButton from '@/ui/AppButton.vue'
import AppDialog from '@/ui/AppDialog.vue'

import { useShortcut, useShortcuts, type Shortcut } from './shortcuts'

const open = ref(false)
useShortcut({
  keys: '?',
  description: 'Show the keyboard shortcuts',
  group: 'Help',
  run: () => (open.value = true),
})

const { shortcuts } = useShortcuts()
const groups = computed(() => {
  const byGroup = new Map<string, Shortcut[]>()
  for (const shortcut of shortcuts.value) {
    byGroup.set(shortcut.group, [...(byGroup.get(shortcut.group) ?? []), shortcut])
  }
  return [...byGroup]
})

const NAMES: Record<string, string> = {
  ArrowUp: '↑',
  ArrowDown: '↓',
  ArrowLeft: '←',
  ArrowRight: '→',
  ' ': 'Space',
  Escape: 'Esc',
}

/** A shortcut's keys as they're shown: `Shift+ArrowDown` is Shift and ↓. */
function keyNames(keys: string) {
  return keys.split(/\+(?!$)/).map((key) => NAMES[key] ?? key)
}
</script>

<template>
  <AppDialog v-model:open="open" title="Keyboard shortcuts" size="sm">
    <section v-for="[group, list] in groups" :key="group">
      <h3 class="shortcut-help__group">{{ group }}</h3>
      <dl class="shortcut-help__list">
        <template v-for="shortcut in list" :key="shortcut.keys">
          <dt>
            <kbd v-for="key in keyNames(shortcut.keys)" :key="key">{{ key }}</kbd>
          </dt>
          <dd>{{ shortcut.description }}</dd>
        </template>
      </dl>
    </section>
    <template #footer>
      <AppButton @click="open = false">Close</AppButton>
    </template>
  </AppDialog>
</template>

<style>
/* Not scoped: the dialog is rendered at the end of <body>. */
.shortcut-help__group {
  margin: var(--space-2) 0 var(--space-1);
  font-size: var(--font-size-md);
  font-weight: var(--font-weight-semibold);
}

.shortcut-help__list {
  display: grid;
  grid-template-columns: max-content 1fr;
  gap: var(--space-1) var(--space-4);
  align-items: center;
  margin: 0;
}

.shortcut-help__list dd {
  margin: 0;
}

.shortcut-help__list kbd {
  display: inline-block;
  min-width: 1.8em;
  margin-right: var(--space-1);
  padding: 0 var(--space-1);
  border: var(--border-width) solid var(--color-border-strong);
  border-radius: var(--radius-sm);
  background: var(--color-surface-sunken);
  font-family: var(--font-family-mono);
  font-size: var(--font-size-sm);
  text-align: center;
}
</style>
