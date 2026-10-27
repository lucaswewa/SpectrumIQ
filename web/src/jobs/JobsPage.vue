<script setup lang="ts">
// The Jobs page (ADR-0043, ADR-0064): the background jobs running, with their progress, and those
// waiting, in the order they will be considered, each with Cancel; and whether a tile scan is
// stitched by itself when it finishes. The server's lists are followed as they change.
import { computed } from 'vue'

import { useJobsStore, type JobInfo } from '@/stores/jobs'
import AppButton from '@/ui/AppButton.vue'
import AppToggle from '@/ui/AppToggle.vue'
import MainView from '@/ui/MainView.vue'
import ProgressBar from '@/ui/ProgressBar.vue'
import SectionHeading from '@/ui/SectionHeading.vue'

const jobs = useJobsStore()

const KINDS: Record<string, string> = {
  stitch: 'Stitch',
  composite: 'Composite',
  pyramid: 'Pyramid',
  export_ome_tiff: 'OME-TIFF export',
  export_dzi: 'DZI export',
  zarr_from_raw: 'Snap image',
  shading: 'Flat estimate',
}

/** What a job is: its kind, number and, if it has one, its dataset. */
const named = (job: JobInfo) =>
  `${KINDS[job.kind] ?? job.kind} ${job.id}${job.dataset ? `, ${job.dataset.split(/[\\/]/).at(-1)}` : ''}`
const since = (job: JobInfo) => {
  const at = job.started ?? job.queued
  return at ? new Date(at).toLocaleTimeString() : ''
}
const sections = computed(() => [
  { title: 'Running', list: jobs.running ?? [], empty: 'Nothing is running.' },
  { title: 'Waiting', list: jobs.queue ?? [], empty: 'Nothing is waiting.' },
])
</script>

<template>
  <MainView class="jobs-page">
    <h1 class="jobs-page__title">Jobs</h1>
    <AppToggle
      :model-value="jobs.autoStitch ?? false"
      label="Stitch tile scans when they finish"
      :disabled="jobs.autoStitch === undefined"
      @update:model-value="jobs.setAutoStitch"
    />
    <section v-for="section in sections" :key="section.title" :aria-label="section.title">
      <SectionHeading>{{ section.title }}</SectionHeading>
      <p v-if="!section.list.length" class="jobs-page__empty">{{ section.empty }}</p>
      <ul v-else class="jobs-page__list">
        <li v-for="job in section.list" :key="job.id" class="jobs-page__job">
          <div class="jobs-page__head">
            <span class="jobs-page__name">{{ named(job) }}</span>
            <span class="jobs-page__when">{{ job.priority }}, since {{ since(job) }}</span>
            <AppButton size="sm" @click="jobs.cancel(job.id)">Cancel</AppButton>
          </div>
          <ProgressBar
            v-if="job.state === 'running'"
            :value="Math.round(job.progress * 100)"
            :label="`${named(job)}'s progress`"
          />
          <p v-if="job.message" class="jobs-page__message">{{ job.message }}</p>
        </li>
      </ul>
    </section>
  </MainView>
</template>

<style scoped>
.jobs-page {
  display: flex;
  flex-direction: column;
  gap: var(--space-4);
}

.jobs-page__title {
  margin: 0;
}

.jobs-page__list {
  display: flex;
  flex-direction: column;
  gap: var(--space-3);
  margin: 0;
  padding: 0;
  list-style: none;
}

.jobs-page__job {
  display: flex;
  flex-direction: column;
  gap: var(--space-2);
  padding: var(--space-3);
  border: var(--border-width) solid var(--color-border);
  border-radius: var(--radius-md);
}

.jobs-page__head {
  display: flex;
  gap: var(--space-3);
  align-items: center;
}

.jobs-page__name {
  font-weight: 600;
}

.jobs-page__when {
  flex: 1;
}

.jobs-page__when,
.jobs-page__empty,
.jobs-page__message {
  margin: 0;
  color: var(--color-text-muted);
  font-size: var(--font-size-sm);
}
</style>
