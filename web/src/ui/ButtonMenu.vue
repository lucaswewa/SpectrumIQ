<script setup lang="ts">
// A button that opens a menu of actions, such as Download with a choice of
// formats: an AppMenu with an AppButton, marked by a chevron.
import chevron from '@material-symbols/svg-400/outlined/keyboard_arrow_down.svg?raw'

import AppButton from './AppButton.vue'
import AppIcon from './AppIcon.vue'
import AppMenu, { type MenuItems } from './AppMenu.vue'

withDefaults(
  defineProps<{
    label: string
    items: MenuItems
    variant?: 'default' | 'primary' | 'danger' | 'ghost'
    size?: 'sm' | 'md'
    /** An icon before the label, as raw SVG text (see AppIcon). */
    icon?: string
  }>(),
  { variant: 'default', size: 'md', icon: undefined },
)
</script>

<template>
  <AppMenu :items="items">
    <AppButton :variant="variant" :size="size" :icon="icon">
      {{ label }}
      <AppIcon :svg="chevron" size="var(--icon-size-sm)" />
    </AppButton>
  </AppMenu>
</template>
