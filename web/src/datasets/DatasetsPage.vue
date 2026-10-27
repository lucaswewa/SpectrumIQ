<script setup lang="ts">
// The Datasets page (ADR-0041, amended): the newest datasets, as cards, 18 to a page, filtered by
// kind and by name. The list is read when the page opens, on
// Refresh, and whenever a dataset is made. Details shows a dataset's manifest beside the cards:
// where it came from, and what made its images, with its exports, and Stitch for a scan's frames
// (ADR-0064). View opens the viewer.
import refreshIcon from '@material-symbols/svg-400/outlined/refresh.svg?raw'
import { computed, ref, shallowRef, watch } from 'vue'
import { useRouter } from 'vue-router'

import { ApiError } from '@/api/wot/errors'
import { useConnectionStore } from '@/stores/connection'
import { useDatasetsStore, type DatasetEntry } from '@/stores/datasets'
import AppButton from '@/ui/AppButton.vue'
import AppPagination from '@/ui/AppPagination.vue'
import FormField from '@/ui/FormField.vue'
import MainView from '@/ui/MainView.vue'
import MultiSelect from '@/ui/MultiSelect.vue'
import { useToast } from '@/ui/overlays'
import SectionHeading from '@/ui/SectionHeading.vue'
import TextField from '@/ui/TextField.vue'
import { dataRoot, manifestUrl } from '@/viewer2d/tiles'

import DatasetCard from './DatasetCard.vue'
import StitchDialog from './StitchDialog.vue'

const PER_PAGE = 18

/** What the details show of a manifest. */
interface Manifest {
  id: string
  provenance: { software: string; version: string; host: string }
  instrument?: { sha256: string } | null
  history: { id: string; kind: string; state: string; finished?: string | null }[]
}

const datasets = useDatasetsStore()
const connection = useConnectionStore()
const router = useRouter()
const toast = useToast()

const kinds = ref<string[]>([])
const search = ref('')
const page = ref(1)
const selected = shallowRef<DatasetEntry>()
const manifest = shallowRef<Manifest>()
const loading = ref(false)
const stitching = ref(false)

const entries = computed(() => datasets.list ?? [])
const known = computed(() => [...new Set(entries.value.map((entry) => entry.kind))].sort())
const filtered = computed(() => {
  const words = search.value.trim().toLowerCase()
  return entries.value.filter(
    (entry) =>
      (!kinds.value.length || kinds.value.includes(entry.kind)) &&
      entry.name.toLowerCase().includes(words),
  )
})
const first = computed(() => (page.value - 1) * PER_PAGE)
const onPage = computed(() => filtered.value.slice(first.value, first.value + PER_PAGE))
const showing = computed(() => {
  const { length } = filtered.value
  const more = datasets.total > entries.value.length ? `, the newest of ${datasets.total}` : ''
  return length
    ? `Showing ${first.value + 1}–${first.value + onPage.value.length} of ${length}${more}`
    : ''
})
watch([kinds, search], () => (page.value = 1))
watch(filtered, (list) => {
  page.value = Math.min(page.value, Math.max(1, Math.ceil(list.length / PER_PAGE)))
  if (selected.value && !list.some((entry) => entry.id === selected.value?.id))
    selected.value = undefined
})

async function refresh() {
  loading.value = true
  await datasets.load()
  loading.value = false
}

/** A step of the history: its kind, its state, and when it ended. */
const step = ({ kind, state, finished }: Manifest['history'][number]) =>
  `${kind}: ${state}${finished ? `, ${new Date(finished).toLocaleTimeString()}` : ''}`

const view = (entry: DatasetEntry) =>
  void router.push({ name: 'viewer', query: { dataset: entry.id, image: entry.images[0] } })

watch(selected, async (entry) => {
  manifest.value = undefined
  if (!entry || !connection.client) return
  try {
    const url = manifestUrl(dataRoot(connection.client), entry.id)
    manifest.value = (await connection.client.request(url)) as Manifest
  } catch (error) {
    toast.error(`Couldn't read ${entry.name}'s manifest.`, {
      details: ApiError.fromFailure(error).message,
    })
  }
})

async function exportAs(kind: 'OME-TIFF' | 'DZI', entry: DatasetEntry, image: string) {
  const run = kind === 'OME-TIFF' ? datasets.exportOmeTiff : datasets.exportDzi
  const job = await run(entry.id, image)
  if (job !== undefined) toast.success(`${image} is being exported as ${kind}: job ${job}.`)
}
</script>

<template>
  <div class="datasets-page">
    <MainView class="datasets-page__main">
      <h1 class="visually-hidden">Datasets</h1>
      <div class="datasets-page__bar">
        <FormField label="Kinds" class="datasets-page__filter">
          <MultiSelect
            v-model="kinds"
            :options="known.map((kind) => ({ value: kind, label: kind }))"
            placeholder="All"
            size="sm"
          />
        </FormField>
        <FormField label="Name" class="datasets-page__filter">
          <TextField v-model="search" size="sm" />
        </FormField>
        <p class="datasets-page__showing">{{ showing }}</p>
        <AppButton :icon="refreshIcon" :disabled="loading" @click="refresh">Refresh</AppButton>
      </div>
      <p v-if="datasets.list && !entries.length" class="datasets-page__empty">
        There are no datasets yet. A snap made into a dataset, or a scan, appears here.
      </p>
      <p v-else-if="datasets.list && !filtered.length" class="datasets-page__empty">
        No dataset matches the filters.
      </p>
      <ul class="datasets-page__grid">
        <li v-for="entry in onPage" :key="entry.id">
          <DatasetCard
            :entry="entry"
            :selected="selected?.id === entry.id"
            @view="view(entry)"
            @details="selected = selected?.id === entry.id ? undefined : entry"
          />
        </li>
      </ul>
      <AppPagination
        v-if="filtered.length > PER_PAGE"
        v-model:page="page"
        :total="filtered.length"
        :per-page="PER_PAGE"
      />
    </MainView>
    <aside
      v-if="selected"
      class="datasets-page__details"
      :aria-label="`${selected.name}'s details`"
    >
      <SectionHeading>{{ selected.name }}</SectionHeading>
      <dl class="datasets-page__facts">
        <dt>Id</dt>
        <dd>{{ selected.id }}</dd>
        <dt>Made</dt>
        <dd>{{ new Date(selected.created).toLocaleString() }}, by {{ selected.user }}</dd>
        <template v-if="manifest">
          <dt>With</dt>
          <dd>
            {{ manifest.provenance.software }} {{ manifest.provenance.version }}, on
            {{ manifest.provenance.host }}
          </dd>
          <template v-if="manifest.instrument">
            <dt>Instrument</dt>
            <dd>configuration {{ manifest.instrument.sha256.slice(0, 12) }}</dd>
          </template>
        </template>
      </dl>
      <AppButton v-if="selected.frames > 1" size="sm" @click="stitching = true">
        Stitch…
      </AppButton>
      <StitchDialog v-model:open="stitching" :entry="selected" />
      <SectionHeading :level="3">History</SectionHeading>
      <ol v-if="manifest?.history.length" class="datasets-page__history">
        <li v-for="record in manifest.history" :key="record.id">{{ step(record) }}</li>
      </ol>
      <p v-else class="datasets-page__empty">Nothing has been done to it yet.</p>
      <template v-for="image in selected.images" :key="image">
        <SectionHeading :level="3">{{ image }}</SectionHeading>
        <div class="datasets-page__exports">
          <AppButton size="sm" @click="exportAs('OME-TIFF', selected, image)">
            Export OME-TIFF
          </AppButton>
          <AppButton size="sm" @click="exportAs('DZI', selected, image)">Export DZI</AppButton>
        </div>
      </template>
    </aside>
  </div>
</template>

<style scoped>
.datasets-page {
  display: flex;
  flex: 1;
  min-width: 0;
  height: 100%;
}

.datasets-page__main {
  display: flex;
  flex-direction: column;
  gap: var(--space-4);
}

.datasets-page__bar {
  display: flex;
  gap: var(--space-3);
  align-items: end;
}

.datasets-page__filter {
  width: 12rem;
}

.datasets-page__showing {
  flex: 1;
  margin: 0;
  color: var(--color-text-muted);
}

.datasets-page__empty {
  margin: 0;
  color: var(--color-text-muted);
}

.datasets-page__grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, 17rem);
  gap: var(--space-6);
  margin: 0;
  padding: 0;
  list-style: none;
}

.datasets-page__details {
  display: flex;
  flex: none;
  flex-direction: column;
  gap: var(--space-3);
  width: var(--pane-width-wide);
  height: 100%;
  padding: var(--space-4);
  overflow: hidden auto;
  border-left: var(--border-width) solid var(--color-border);
  background: var(--color-surface-panel);
}

.datasets-page__facts {
  display: grid;
  grid-template-columns: auto 1fr;
  gap: var(--space-1) var(--space-3);
  margin: 0;
  font-size: var(--font-size-sm);
}

.datasets-page__facts dt {
  color: var(--color-text-muted);
}

.datasets-page__facts dd {
  margin: 0;
  overflow-wrap: anywhere;
}

.datasets-page__history {
  margin: 0;
  padding-left: var(--space-5);
  font-size: var(--font-size-sm);
}

.datasets-page__exports {
  display: flex;
  gap: var(--space-2);
}
</style>
