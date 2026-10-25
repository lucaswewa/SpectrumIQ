<script setup lang="ts">
import { computed } from 'vue'
import { RouterLink } from 'vue-router'

import AppIcon from '@/ui/AppIcon.vue'

import type { Destination } from './navigation'

const props = defineProps<{ destinations: readonly Destination[] }>()

const groups = computed(() => [
  { name: 'top', items: props.destinations.filter((d) => d.group === 'top') },
  { name: 'bottom', items: props.destinations.filter((d) => d.group === 'bottom') },
])
</script>

<template>
  <nav class="rail" aria-label="Main">
    <ul
      v-for="group in groups"
      :key="group.name"
      class="rail__group"
      :class="`rail__group--${group.name}`"
    >
      <li v-for="destination in group.items" :key="destination.id">
        <RouterLink
          :to="{ name: destination.id }"
          class="rail__item"
          active-class="rail__item--active"
          :title="destination.label"
        >
          <AppIcon :svg="destination.icon" />
          <span class="rail__label">{{ destination.label }}</span>
        </RouterLink>
      </li>
    </ul>
  </nav>
</template>

<style scoped>
.rail {
  display: flex;
  flex: none;
  flex-direction: column;
  width: var(--rail-width);
  height: 100%;
  overflow: hidden auto;
  border-right: var(--border-width) solid var(--color-border);
  background: var(--color-surface-rail);
}

.rail__group {
  margin: 0;
  padding: 0;
  list-style: none;
}

.rail__group--top {
  padding-bottom: var(--space-2);
  border-bottom: var(--border-width) solid var(--color-border);
}

.rail__group--bottom {
  margin-top: auto;
}

.rail__item {
  display: flex;
  flex-direction: column;
  gap: var(--space-1);
  align-items: center;
  justify-content: center;
  height: var(--rail-item-height);
  padding: 0 var(--space-1);
  color: var(--color-rail-text);
  font-size: var(--font-size-sm);
  line-height: var(--line-height-tight);
  text-align: center;
  text-decoration: none;
  transition: background-color var(--duration-fast) var(--easing-standard);
}

.rail__item:hover {
  background: var(--color-rail-hover-bg);
}

.rail__item--active,
.rail__item--active:hover {
  background: var(--color-rail-active-bg);
  color: var(--color-rail-active-text);
}

.rail__item:focus-visible {
  outline-offset: calc(-1 * var(--focus-ring-width) - 2px);
}

.rail__item--active:focus-visible {
  outline-color: var(--color-rail-active-text);
}
</style>
