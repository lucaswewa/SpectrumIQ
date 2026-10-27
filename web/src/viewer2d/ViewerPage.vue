<script setup lang="ts">
// The Viewer page: the 2D viewer (ADR-0009, amended) of `?dataset=<id>&image=<name>`, or, without
// them, of the newest dataset with an image. A dataset with several images has a choice of them.
// A stitched image has its QA's overlays: its tiles' outlines and its pairs' residuals (ADR-0064).
import { computed, shallowRef, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'

import type { OverlayLayer } from '@/overlay/types'
import { useConnectionStore } from '@/stores/connection'
import { useDatasetsStore } from '@/stores/datasets'
import AppSelect from '@/ui/AppSelect.vue'
import FormField from '@/ui/FormField.vue'
import MainView from '@/ui/MainView.vue'

import { dataRoot } from './tiles'
import Viewer2D from './Viewer2D.vue'

const route = useRoute()
const router = useRouter()
const datasets = useDatasetsStore()

const asked = (name: string) => {
  const value = route.query[name]
  return typeof value === 'string' ? value : undefined
}
const entry = computed(() => {
  const list = datasets.list ?? []
  const id = asked('dataset')
  return id ? list.find((each) => each.id === id) : list.find((each) => each.images.length)
})
const image = computed(() => asked('image') ?? entry.value?.images[0])
const choose = (next: string | undefined) =>
  entry.value && void router.replace({ query: { dataset: entry.value.id, image: next } })

// The image's overlays, if it was stitched: none otherwise.
const connection = useConnectionStore()
const overlays = shallowRef<OverlayLayer[]>([])
watch(
  [() => entry.value?.id, image],
  async ([dataset, name]) => {
    overlays.value = []
    const client = connection.client
    if (!dataset || !name || !client) return
    const at = (part: string) => encodeURIComponent(part)
    const url = new URL(`${at(dataset)}/${at(name)}/overlays`, dataRoot(client))
    try {
      overlays.value = (await client.request(url)) as OverlayLayer[]
    } catch {
      // Not stitched: no report, no overlays.
    }
  },
  { immediate: true },
)
</script>

<template>
  <Viewer2D
    v-if="entry && image"
    :key="`${entry.id}/${image}`"
    :dataset="entry.id"
    :image="image"
    :overlays="overlays"
  >
    <template v-if="entry.images.length > 1" #toolbar>
      <FormField label="Image" class="viewer-page__image">
        <AppSelect
          :model-value="image"
          :options="entry.images.map((name) => ({ value: name, label: name }))"
          size="sm"
          @update:model-value="choose"
        />
      </FormField>
    </template>
  </Viewer2D>
  <MainView v-else>
    <h1 class="visually-hidden">Viewer</h1>
    <p class="viewer-page__empty">
      {{
        datasets.list && asked('dataset')
          ? 'There is no such dataset, or it has no image yet.'
          : 'No dataset has an image yet. Snap one into a dataset, then view it here.'
      }}
    </p>
  </MainView>
</template>

<style scoped>
.viewer-page__image {
  width: 10rem;
}

.viewer-page__empty {
  margin: 0;
  color: var(--color-text-muted);
}
</style>
