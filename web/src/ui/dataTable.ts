import {
  createSortedRowModel,
  rowSortingFeature,
  sortFns,
  tableFeatures,
  type ColumnDef,
  type RowData,
} from '@tanstack/vue-table'

/** What `AppDataTable`'s tables can do (ADR-0068): sort, by a column, either way. */
export const TABLE_FEATURES = tableFeatures({
  rowSortingFeature,
  sortedRowModel: createSortedRowModel(),
  sortFns,
})

/** A column of an `AppDataTable`: its `id` or `accessorKey`, its `header`, and its cells. */
// eslint-disable-next-line @typescript-eslint/no-explicit-any -- each column's value type differs
export type TableColumn<Row extends RowData> = ColumnDef<typeof TABLE_FEATURES, Row, any>
