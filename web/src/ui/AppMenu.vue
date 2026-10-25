<script lang="ts">
/** One action in an AppMenu. */
export interface MenuAction {
  label: string
  /** An icon before the label, as raw SVG text (see AppIcon). */
  icon?: string
  disabled?: boolean
  /** Shows the action as destructive. */
  danger?: boolean
  run: () => void
}

/** One choice in an AppMenu, on or off, such as a layer to show. */
export interface MenuCheck {
  label: string
  checked: boolean
  disabled?: boolean
  toggle: (checked: boolean) => void
}

/** A menu's items: actions and choices, and 'separator' between groups of them. */
export type MenuItems = readonly (MenuAction | MenuCheck | 'separator')[]
</script>

<script setup lang="ts">
// A menu of actions that opens from its trigger, the default slot (a button),
// on Reka UI's DropdownMenu (ADR-0013). ↑ and ↓ move between the actions,
// skipping disabled ones; Enter, Space or a click runs one; typing jumps to an
// action by its label; Escape closes the menu and gives the focus back to the
// trigger. A choice is ticked when it's on, and turning it over leaves the
// menu open, for the next.
import checkIcon from '@material-symbols/svg-400/outlined/check.svg?raw'
import {
  DropdownMenuCheckboxItem,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuItemIndicator,
  DropdownMenuPortal,
  DropdownMenuRoot,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from 'reka-ui'
import { ref } from 'vue'

import AppIcon from './AppIcon.vue'
import { regionTheme } from './regionTheme'

defineProps<{ items: MenuItems }>()

// The menu is moved to <body>, in the theme of its trigger's region.
const root = ref<HTMLElement>()
const theme = ref<string>()
function onOpen(open: boolean) {
  if (open) theme.value = regionTheme(root.value)
}
</script>

<template>
  <span ref="root" class="app-menu">
    <DropdownMenuRoot :modal="false" @update:open="onOpen">
      <DropdownMenuTrigger as-child><slot /></DropdownMenuTrigger>
      <DropdownMenuPortal>
        <DropdownMenuContent
          :side-offset="2"
          align="start"
          :data-theme="theme"
          class="app-menu__list"
        >
          <template v-for="(item, index) in items" :key="index">
            <DropdownMenuSeparator v-if="item === 'separator'" class="app-menu__separator" />
            <DropdownMenuCheckboxItem
              v-else-if="'checked' in item"
              :model-value="item.checked"
              :disabled="item.disabled"
              class="app-menu__item"
              @update:model-value="(on) => item.toggle(on === true)"
              @select="(event: Event) => event.preventDefault()"
            >
              <span class="app-menu__check">
                <DropdownMenuItemIndicator>
                  <AppIcon :svg="checkIcon" size="var(--icon-size-sm)" />
                </DropdownMenuItemIndicator>
              </span>
              {{ item.label }}
            </DropdownMenuCheckboxItem>
            <DropdownMenuItem
              v-else
              :disabled="item.disabled"
              class="app-menu__item"
              :class="{ 'app-menu__item--danger': item.danger }"
              @select="item.run()"
            >
              <AppIcon v-if="item.icon" :svg="item.icon" size="var(--icon-size-sm)" />
              {{ item.label }}
            </DropdownMenuItem>
          </template>
        </DropdownMenuContent>
      </DropdownMenuPortal>
    </DropdownMenuRoot>
  </span>
</template>

<style>
/* Not scoped: the menu is rendered at the end of <body>. */
.app-menu {
  display: inline-flex;
}

.app-menu__list {
  z-index: var(--z-menu);
  min-width: 160px;
  padding: var(--space-1) 0;
  border: var(--border-width) solid var(--color-border-strong);
  background: var(--color-surface-raised);
  box-shadow: var(--shadow-md);
  color: var(--color-text);
  font-family: var(--font-family-sans);
  font-size: var(--font-size-md);
}

.app-menu__item {
  display: flex;
  gap: var(--space-2);
  align-items: center;
  min-height: var(--control-height-sm);
  padding: 0 var(--space-3) 0 var(--space-2);
  outline: none;
  cursor: pointer;
  user-select: none;
}

.app-menu__item[data-highlighted] {
  background: var(--color-accent-subtle);
}

.app-menu__check {
  display: inline-flex;
  width: var(--icon-size-sm);
}

.app-menu__item--danger {
  color: var(--color-danger);
}

.app-menu__item[data-disabled] {
  color: var(--color-text-disabled);
  cursor: not-allowed;
}

.app-menu__separator {
  height: var(--border-width);
  margin: var(--space-1) 0;
  background: var(--color-border);
}
</style>
