<script setup lang="ts">
// One dataset in the browser (ADR-0041, amended): its thumbnail, or a placeholder if it has no
// image yet or the thumbnail won't load, which opens the viewer; its name, when it was made, and
// what it holds; View, Details, and Delete, which asks first and is an admin's (ADR-0068).
import imageIcon from '@material-symbols/svg-400/outlined/image.svg?raw'
import { computed, ref } from 'vue'

import { useGate } from '@/auth/gate'
import { useConnectionStore } from '@/stores/connection'
import { useDatasetsStore, type DatasetEntry } from '@/stores/datasets'
import AppButton from '@/ui/AppButton.vue'
import AppCard from '@/ui/AppCard.vue'
import AppIcon from '@/ui/AppIcon.vue'
import { useConfirm } from '@/ui/overlays'
import { dataRoot, thumbnailUrl } from '@/viewer2d/tiles'

const props = defineProps<{ entry: DatasetEntry; selected?: boolean }>()
const emit = defineEmits<{ view: []; details: [] }>()
const connection = useConnectionStore()
const datasets = useDatasetsStore()
const confirm = useConfirm()
const refused = useGate(() => 'admin')

const unloadable = ref(false)
const thumbnail = computed(() =>
  connection.client && props.entry.images.length && !unloadable.value
    ? thumbnailUrl(dataRoot(connection.client), props.entry.id).href
    : undefined,
)
const created = computed(() => new Date(props.entry.created).toLocaleString())
const holds = computed(() => {
  const { kind, frames, images } = props.entry
  const parts = [kind.replace('_', ' '), `${frames} frame${frames === 1 ? '' : 's'}`]
  if (images.length) parts.push(images.join(', '))
  return parts.join(' · ')
})

async function remove() {
  const { name, id } = props.entry
  const message = `Delete ${name}, and everything in it? This can't be undone.`
  if (await confirm({ title: 'Delete', message, confirmLabel: 'Delete', danger: true }))
    await datasets.remove(id)
}
</script>

<template>
  <AppCard class="dataset-card" :class="{ 'dataset-card--selected': selected }">
    <button
      type="button"
      class="dataset-card__thumbnail"
      :aria-label="`View ${entry.name}`"
      :disabled="!entry.images.length"
      @click="emit('view')"
    >
      <img v-if="thumbnail" :src="thumbnail" alt="" @error="unloadable = true" />
      <AppIcon v-else :svg="imageIcon" size="var(--space-12)" />
    </button>
    <h2 class="dataset-card__title">{{ entry.name }}</h2>
    <p class="dataset-card__created">{{ created }}</p>
    <p class="dataset-card__holds">{{ holds }}</p>
    <div class="dataset-card__buttons">
      <AppButton variant="primary" :disabled="!entry.images.length" @click="emit('view')">
        View
      </AppButton>
      <AppButton :aria-pressed="selected" @click="emit('details')">Details</AppButton>
      <AppButton :disabled="refused !== undefined" :title="refused" @click="remove">
        Delete
      </AppButton>
    </div>
  </AppCard>
</template>

<style scoped>
.dataset-card {
  display: flex;
  flex-direction: column;
  gap: var(--space-2);
}

.dataset-card--selected {
  outline: 2px solid var(--color-accent-text);
}

.dataset-card__thumbnail {
  display: grid;
  aspect-ratio: 4 / 3;
  place-items: center;
  padding: 0;
  overflow: hidden;
  border: none;
  border-radius: var(--radius-md);
  background: var(--color-image-backdrop);
  color: var(--color-text-muted);
  cursor: pointer;
}

.dataset-card__thumbnail:disabled {
  cursor: default;
}

.dataset-card__thumbnail img {
  width: 100%;
  height: 100%;
  object-fit: contain;
}

.dataset-card__title {
  margin: 0;
  overflow: hidden;
  font-size: var(--font-size-md);
  text-overflow: ellipsis;
  white-space: nowrap;
}

.dataset-card__created,
.dataset-card__holds {
  margin: 0;
  color: var(--color-text-muted);
  font-size: var(--font-size-sm);
}

.dataset-card__buttons {
  display: flex;
  flex-wrap: wrap;
  gap: var(--space-2);
}
</style>
