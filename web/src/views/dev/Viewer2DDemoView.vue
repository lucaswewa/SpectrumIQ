<script setup lang="ts">
// The 2D viewer against a synthetic dataset of the connected server: the example of P4.8. Choose
// the dataset and its image. A demo overlay layer outlines the image's 512-pixel chunks, to show
// the overlay layers that M8 fills. Make a synthetic dataset with `siq-datagen` (P4.9), or with
// `cargo run --release -p siq-jobs --example export_dzi -- --out <the server's data_root>`. For
// development only.
import { computed, ref, shallowRef, watch } from 'vue'

import type { OverlayLayer, Shape } from '@/overlay/types'
import { useConnectionStore } from '@/stores/connection'
import { useDatasetsStore } from '@/stores/datasets'
import AppSelect from '@/ui/AppSelect.vue'
import FormField from '@/ui/FormField.vue'
import { dataRoot, infoUrl, type ImageInfo } from '@/viewer2d/tiles'
import Viewer2D from '@/viewer2d/Viewer2D.vue'

const CHUNK = 512

const connection = useConnectionStore()
const datasets = useDatasetsStore()
const dataset = ref<string>()
const image = ref<string>()
const info = shallowRef<ImageInfo>()

const withImages = computed(() => (datasets.list ?? []).filter((entry) => entry.images.length))
const entry = computed(() => withImages.value.find((each) => each.id === dataset.value))
watch(
  withImages,
  (list) => {
    if (entry.value || !list[0]) return
    dataset.value = list[0].id
  },
  { immediate: true },
)
watch(entry, (next) => (image.value = next?.images[0]))
watch([dataset, image], async ([id, name]) => {
  info.value = undefined
  if (!id || !name || !connection.client) return
  const at = { data: dataRoot(connection.client), dataset: id, image: name }
  info.value = (await connection.client.request(infoUrl(at)).catch(() => undefined)) as ImageInfo
})

const overlays = computed<OverlayLayer[]>(() => {
  const size = info.value?.size
  if (!size) return []
  const shapes: Shape[] = []
  for (let y = 0; y < size.y; y += CHUNK)
    for (let x = 0; x < size.x; x += CHUNK) {
      const [width, height] = [Math.min(CHUNK, size.x - x), Math.min(CHUNK, size.y - y)]
      shapes.push({ kind: 'rect', x, y, width, height, dashed: true })
    }
  return [{ id: 'dev-chunks', label: 'Chunks of 512', size: [size.x, size.y], shapes }]
})
</script>

<template>
  <section class="viewer-demo">
    <div class="viewer-demo__bar">
      <p class="viewer-demo__note">
        The 2D viewer against the connected server's datasets, with a demo overlay layer (Layers).
      </p>
      <FormField label="Dataset" class="viewer-demo__choice">
        <AppSelect
          v-model="dataset"
          :options="withImages.map((each) => ({ value: each.id, label: each.name }))"
          size="sm"
        />
      </FormField>
      <FormField v-if="entry" label="Image" class="viewer-demo__choice">
        <AppSelect
          v-model="image"
          :options="entry.images.map((name) => ({ value: name, label: name }))"
          size="sm"
        />
      </FormField>
    </div>
    <Viewer2D
      v-if="dataset && image"
      :key="`${dataset}/${image}`"
      :dataset="dataset"
      :image="image"
      :overlays="overlays"
    />
    <p v-else class="viewer-demo__note">
      The server has no dataset with an image. Make a synthetic one with siq-datagen, or with the
      export_dzi example, in its data root.
    </p>
  </section>
</template>

<style scoped>
.viewer-demo {
  display: flex;
  flex: 1;
  flex-direction: column;
  min-height: 0;
}

.viewer-demo__bar {
  display: flex;
  gap: var(--space-3);
  align-items: end;
  padding: var(--space-2) var(--space-4);
  border-bottom: var(--border-width) solid var(--color-border);
}

.viewer-demo__note {
  flex: 1;
  margin: 0;
  padding: var(--space-2) 0;
  color: var(--color-text-muted);
}

.viewer-demo__choice {
  width: 14rem;
}
</style>
