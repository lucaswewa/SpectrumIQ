<script setup lang="ts">
// Page buttons for a long list, on Reka UI's Pagination (ADR-0013):
// previous, the first and last pages, the pages around the current one with
// gaps marked "…", and next. `v-model:page` is the current page, from 1;
// `total` is the number of items, shown `perPage` at a time. It names itself
// "Pages" and marks the current page.
import chevronLeft from '@material-symbols/svg-400/outlined/chevron_left.svg?raw'
import chevronRight from '@material-symbols/svg-400/outlined/chevron_right.svg?raw'
import {
  PaginationEllipsis,
  PaginationList,
  PaginationListItem,
  PaginationNext,
  PaginationPrev,
  PaginationRoot,
} from 'reka-ui'

import AppIcon from './AppIcon.vue'

const page = defineModel<number>('page', { default: 1 })
defineProps<{ total: number; perPage: number }>()
</script>

<template>
  <PaginationRoot
    v-model:page="page"
    :total="total"
    :items-per-page="perPage"
    :sibling-count="1"
    show-edges
    as="nav"
    aria-label="Pages"
    class="app-pagination"
  >
    <PaginationList v-slot="{ items }" class="app-pagination__list">
      <PaginationPrev class="app-pagination__button" aria-label="Previous page">
        <AppIcon :svg="chevronLeft" size="var(--icon-size-sm)" />
      </PaginationPrev>
      <template v-for="(item, index) in items" :key="index">
        <PaginationListItem
          v-if="item.type === 'page'"
          :value="item.value"
          class="app-pagination__button"
        />
        <PaginationEllipsis v-else :index="index" class="app-pagination__gap">…</PaginationEllipsis>
      </template>
      <PaginationNext class="app-pagination__button" aria-label="Next page">
        <AppIcon :svg="chevronRight" size="var(--icon-size-sm)" />
      </PaginationNext>
    </PaginationList>
  </PaginationRoot>
</template>

<style scoped>
.app-pagination__list {
  display: flex;
  gap: var(--space-1);
  align-items: center;
}

.app-pagination__button {
  display: inline-grid;
  place-items: center;
  min-width: var(--control-height-sm);
  height: var(--control-height-sm);
  padding: 0 var(--space-1);
  border: var(--border-width) solid transparent;
  border-radius: var(--radius-sm);
  background: none;
  color: var(--color-text);
  font: inherit;
  font-size: var(--font-size-sm);
  cursor: pointer;
}

.app-pagination__button:hover:not(:disabled) {
  background: color-mix(in srgb, currentcolor 10%, transparent);
}

.app-pagination__button[aria-current='page'] {
  border-color: var(--color-accent);
  background: var(--color-accent);
  color: var(--color-on-accent);
}

.app-pagination__button:disabled {
  color: var(--color-text-disabled);
  cursor: not-allowed;
}

.app-pagination__gap {
  min-width: var(--control-height-sm);
  color: var(--color-text-muted);
  text-align: center;
}
</style>
