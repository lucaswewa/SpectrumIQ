// REQ-SEC-003, REQ-VIS-005: the data table's contract (ADR-0068): a header and a row each, cells
// by their columns or by a slot, sorting by a header's button either way with `aria-sort`, and
// the empty text without rows.
import { mount } from '@vue/test-utils'
import { h } from 'vue'
import { describe, expect, it } from 'vitest'

import AppDataTable from '@/ui/AppDataTable.vue'
import type { TableColumn } from '@/ui/dataTable'

interface Row {
  name: string
  role: string
  frames: number
}

const rows: Row[] = [
  { name: 'olga', role: 'operator', frames: 12 },
  { name: 'ada', role: 'admin', frames: 3 },
  { name: 'vic', role: 'viewer', frames: 40 },
]
const columns: TableColumn<Row>[] = [
  { accessorKey: 'name', header: 'Name' },
  { accessorKey: 'role', header: 'Role', enableSorting: false },
  { accessorKey: 'frames', header: 'Frames', cell: (c) => `${c.getValue()} frames` },
]

const names = (wrapper: ReturnType<typeof mount>) =>
  wrapper.findAll('tbody tr').map((row) => row.find('td').text())

describe('AppDataTable', () => {
  it('draws a header and a row each, with cells by column or by slot', () => {
    const wrapper = mount(AppDataTable<Row>, {
      props: { columns, rows, label: 'Users' },
      slots: { 'cell-role': ({ row }: { row: Row }) => h('em', row.role.toUpperCase()) },
    })
    expect(wrapper.get('table').attributes('aria-label')).toBe('Users')
    expect(wrapper.findAll('th').map((th) => th.text())).toEqual(['Name', 'Role', 'Frames'])
    expect(names(wrapper)).toEqual(['olga', 'ada', 'vic'])
    const second = wrapper
      .findAll('tbody tr')[1]!
      .findAll('td')
      .map((td) => td.text())
    expect(second).toEqual(['ada', 'ADMIN', '3 frames'])
  })

  it('sorts by a header, then the other way, and not by an unsortable one', async () => {
    const wrapper = mount(AppDataTable<Row>, { props: { columns, rows, label: 'Users' } })
    const [name, role] = wrapper.findAll('th')
    expect(role!.find('button').exists()).toBe(false)
    await name!.get('button').trigger('click')
    expect(names(wrapper)).toEqual(['ada', 'olga', 'vic'])
    expect(name!.attributes('aria-sort')).toBe('ascending')
    await name!.get('button').trigger('click')
    expect(names(wrapper)).toEqual(['vic', 'olga', 'ada'])
    expect(name!.attributes('aria-sort')).toBe('descending')
  })

  it('says so when there are no rows', () => {
    const wrapper = mount(AppDataTable<Row>, {
      props: { columns, rows: [], label: 'Users', empty: 'No one yet.' },
    })
    expect(wrapper.get('tbody').text()).toBe('No one yet.')
    expect(wrapper.get('td').attributes('colspan')).toBe('3')
  })
})
