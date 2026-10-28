<script setup lang="ts" generic="Row extends RowData">
// A table of rows (ADR-0008, amended in P3.1; ADR-0068): TanStack Table's headless model, drawn
// here with the tokens. Each column gives its header and its cells' text, or a slot named
// `cell-<column id>`, given the row, draws its cells; a sortable column's header sorts by it,
// and again the other way. Without rows, it says `empty`.
import { FlexRender, useTable, type RowData } from '@tanstack/vue-table'
import { computed } from 'vue'

import { TABLE_FEATURES, type TableColumn } from './dataTable'

const props = withDefaults(
  defineProps<{ columns: TableColumn<Row>[]; rows: Row[]; label: string; empty?: string }>(),
  { empty: 'Nothing to show.' },
)

const data = computed(() => props.rows)
const table = useTable({ features: TABLE_FEATURES, columns: props.columns, data })

type Sorted = false | 'asc' | 'desc'
const ariaSort = (sorted: Sorted) =>
  sorted === 'asc' ? 'ascending' : sorted === 'desc' ? 'descending' : undefined
const arrow = (sorted: Sorted) => (sorted === 'asc' ? ' ▲' : sorted === 'desc' ? ' ▼' : '')
</script>

<template>
  <table class="data-table" :aria-label="label">
    <thead>
      <tr v-for="group in table.getHeaderGroups()" :key="group.id">
        <th
          v-for="header in group.headers"
          :key="header.id"
          scope="col"
          :aria-sort="ariaSort(header.column.getIsSorted())"
        >
          <button
            v-if="header.column.getCanSort()"
            type="button"
            class="data-table__sort"
            @click="header.column.getToggleSortingHandler()?.($event)"
          >
            <FlexRender :header="header" />
            <span aria-hidden="true">{{ arrow(header.column.getIsSorted()) }}</span>
          </button>
          <FlexRender v-else :header="header" />
        </th>
      </tr>
    </thead>
    <tbody>
      <tr v-for="row in table.getRowModel().rows" :key="row.id">
        <td v-for="cell in row.getAllCells()" :key="cell.id">
          <slot :name="`cell-${cell.column.id}`" :row="row.original">
            <FlexRender :cell="cell" />
          </slot>
        </td>
      </tr>
      <tr v-if="rows.length === 0">
        <td class="data-table__empty" :colspan="columns.length">{{ empty }}</td>
      </tr>
    </tbody>
  </table>
</template>

<style scoped>
.data-table {
  width: 100%;
  border-collapse: collapse;
}

.data-table th,
.data-table td {
  padding: var(--space-1) var(--space-2);
  border-bottom: var(--border-width) solid var(--color-border);
  text-align: left;
}

.data-table th {
  color: var(--color-text-muted);
  font-weight: normal;
}

.data-table__sort {
  padding: 0;
  border: 0;
  background: none;
  color: inherit;
  font: inherit;
  cursor: pointer;
}

.data-table__empty {
  color: var(--color-text-muted);
}
</style>
