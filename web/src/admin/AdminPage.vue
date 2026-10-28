<script setup lang="ts">
// The admin's page (ADR-0068): the users, added, given a role or a password, and removed; and the
// audit trail (ADR-0067), filtered by user and kind, newest first, a page at a time. A new
// password is asked for in a dialog, not in the clear, and a removal asks first. Others are told it
// is an admin's.
import { computed, h, onMounted, ref, watch } from 'vue'

import { siq } from '@/api/siq'
import { describeError } from '@/api/wot/errors'
import { useGate } from '@/auth/gate'
import type { Role } from '@/auth/session'
import AppButton from '@/ui/AppButton.vue'
import AppDataTable from '@/ui/AppDataTable.vue'
import AppPagination from '@/ui/AppPagination.vue'
import AppSelect from '@/ui/AppSelect.vue'
import type { TableColumn } from '@/ui/dataTable'
import FormField from '@/ui/FormField.vue'
import MainView from '@/ui/MainView.vue'
import TextField from '@/ui/TextField.vue'
import { useConfirm, useToast } from '@/ui/overlays'

interface User {
  name: string
  role: Role
  created: string
}
interface Entry {
  seq: number
  time: string
  kind: string
  user: string | null
  address: string | null
  detail: Record<string, unknown>
}

const PER_PAGE = 25
const ROLES = (['viewer', 'operator', 'admin'] as const).map((value) => ({ value, label: value }))
const KINDS = ['login', 'login_failed', 'login_refused', 'logout', 'token', 'request', 'refused']
  .concat(['job', 'workflow', 'dataset_deleted'])
  .map((value) => ({ value, label: value.replace('_', ' ') }))

const refused = useGate(() => 'admin')
const toast = useToast()
const confirm = useConfirm()
const users = ref<User[]>([])
const entries = ref<Entry[]>([])
const total = ref(0)
const page = ref(1)
const filter = ref<{ user: string; kind?: string }>({ user: '' })
/** Who, as typed: the filter takes it at Enter, or on leaving the field. */
const who = ref('')
const added = ref<{ name: string; password: string; role: Role }>({
  name: '',
  password: '',
  role: 'viewer',
})

const userColumns: TableColumn<User>[] = [
  { accessorKey: 'name', header: 'Name' },
  { accessorKey: 'role', header: 'Role' },
  {
    accessorKey: 'created',
    header: 'Since',
    cell: (c) => new Date(c.getValue()).toLocaleDateString(),
  },
  { id: 'actions', header: '', enableSorting: false },
]
const auditColumns: TableColumn<Entry>[] = [
  { accessorKey: 'time', header: 'Time', cell: (c) => new Date(c.getValue()).toLocaleString() },
  { accessorKey: 'kind', header: 'What' },
  { accessorKey: 'user', header: 'Who' },
  { accessorKey: 'address', header: 'From' },
  { id: 'detail', header: 'Detail', enableSorting: false, accessorFn: (e) => summary(e.detail) },
]

/** An entry's detail in a line: a request's route and answer, a token's reason, a run's state. */
function summary(detail: Record<string, unknown>): string {
  if (detail.path) return `${detail.method} ${detail.path} → ${detail.status}`
  const reason = detail.reason as { kind?: string } | undefined
  if (reason?.kind) return `${reason.kind}${detail.holder ? `: ${detail.holder}` : ''}`
  return ['name', 'run', 'state', 'kind']
    .flatMap((k) => (detail[k] ? [String(detail[k])] : []))
    .join(' ')
}

async function act(action: () => Promise<unknown>, done?: string) {
  try {
    await action()
    if (done) toast.success(done)
    users.value = (await siq('users')) as User[]
  } catch (error) {
    toast.error(describeError(error).message ?? 'The server refused.')
  }
}

const path = (name: string) => `users/${encodeURIComponent(name)}`
const add = () =>
  act(async () => {
    await siq('users', { body: added.value })
    added.value = { name: '', password: '', role: 'viewer' }
  }, 'The user is added.')
const setRole = (user: User, role: Role) =>
  act(() => siq(path(user.name), { method: 'PUT', body: { role } }))
async function setPassword(user: User) {
  const password = ref('')
  const content = () =>
    h(FormField, { label: 'New password' }, () =>
      h(TextField, {
        modelValue: password.value,
        type: 'password',
        autocomplete: 'new-password',
        'onUpdate:modelValue': (value: string) => (password.value = value),
      }),
    )
  const [title, message] = [`A new password for ${user.name}`, 'At least 8 characters.']
  if ((await confirm({ title, message, content, confirmLabel: 'Change' })) && password.value)
    await act(
      () => siq(path(user.name), { method: 'PUT', body: { password: password.value } }),
      'Changed.',
    )
}
async function remove(user: User) {
  const message = `Remove ${user.name}? Their sessions end, and they can't log in again.`
  if (await confirm({ title: 'Remove', message, confirmLabel: 'Remove', danger: true }))
    await act(() => siq(path(user.name), { method: 'DELETE' }), 'Removed.')
}

const query = computed(() => {
  const asked = new URLSearchParams({
    offset: String((page.value - 1) * PER_PAGE),
    limit: String(PER_PAGE),
  })
  if (filter.value.user) asked.set('user', filter.value.user)
  if (filter.value.kind) asked.set('kind', filter.value.kind)
  return asked.toString()
})
async function read() {
  try {
    const found = (await siq(`audit?${query.value}`)) as { entries: Entry[]; total: number }
    ;[entries.value, total.value] = [found.entries, found.total]
  } catch (error) {
    toast.error(describeError(error).message ?? 'The audit trail couldn’t be read.')
  }
}
watch(query, read)
onMounted(() => {
  if (refused.value) return
  void act(async () => undefined)
  void read()
})
</script>

<template>
  <MainView aria-label="Admin" class="admin">
    <h1 class="admin__title">Admin</h1>
    <p v-if="refused" role="note">{{ refused }}</p>
    <template v-else>
      <section aria-label="Users" class="admin__section">
        <h2>Users</h2>
        <AppDataTable :columns="userColumns" :rows="users" label="Users">
          <template #cell-role="{ row }">
            <AppSelect
              :model-value="row.role"
              :options="ROLES"
              size="sm"
              @update:model-value="(role) => role && setRole(row, role)"
            />
          </template>
          <template #cell-actions="{ row }">
            <AppButton size="sm" @click="setPassword(row)">New password</AppButton>
            <AppButton size="sm" variant="danger" @click="remove(row)">Remove</AppButton>
          </template>
        </AppDataTable>
        <form class="admin__add" aria-label="Add a user" @submit.prevent="add">
          <FormField label="Name"><TextField v-model="added.name" /></FormField>
          <FormField label="Password">
            <TextField v-model="added.password" type="password" autocomplete="new-password" />
          </FormField>
          <FormField label="Role"><AppSelect v-model="added.role" :options="ROLES" /></FormField>
          <AppButton type="submit" :disabled="!added.name || !added.password">Add</AppButton>
        </form>
      </section>
      <section aria-label="Audit trail" class="admin__section">
        <h2>Audit trail</h2>
        <div class="admin__filters">
          <FormField label="Who">
            <TextField v-model="who" @change="filter.user = who.trim()" />
          </FormField>
          <FormField label="What">
            <AppSelect v-model="filter.kind" :options="KINDS" placeholder="Anything" />
          </FormField>
          <AppButton @click="read">Refresh</AppButton>
        </div>
        <AppDataTable :columns="auditColumns" :rows="entries" label="Audit trail" />
        <AppPagination v-model:page="page" :total="total" :per-page="PER_PAGE" />
      </section>
    </template>
  </MainView>
</template>

<style scoped>
.admin {
  overflow: auto;
  padding: var(--space-4);
}

.admin__title {
  margin: 0 0 var(--space-3);
}

.admin__section {
  margin-bottom: var(--space-5);
}

.admin__add,
.admin__filters {
  display: flex;
  gap: var(--space-3);
  align-items: end;
  margin-block: var(--space-3);
}
</style>
