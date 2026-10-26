<script setup lang="ts">
// The run under way (ADR-0057, amended in P6.7), wherever it was started: its progress, time
// left and step; thumbnails of its latest frames, from the overview stream; Pause or Resume, and
// Cancel once confirmed; and, while a failed step waits, Retry or Skip. Once it ends, how it
// ended, until closed.
import { computed, ref, watch } from 'vue'

import { useBudgetedStream } from '@/api/streams'
import { watchStream } from '@/live/streams'
import { useConnectionStore } from '@/stores/connection'
import { useWorkflowStore } from '@/stores/workflow'
import AppButton from '@/ui/AppButton.vue'
import { useConfirm } from '@/ui/overlays'
import ProgressBar from '@/ui/ProgressBar.vue'

import { duration } from './plan'
import { useThumbnails } from './thumbnails'

const workflow = useWorkflowStore()
const connection = useConnectionStore()
const confirm = useConfirm()
const root = ref<HTMLElement>()

const active = computed(() => !!workflow.state && workflow.state !== 'idle')
const progress = computed(() => workflow.progress)
const eta = computed(() => {
  const left = progress.value?.eta_s
  return left === undefined || left === null ? 'time left unknown' : `about ${duration(left)} left`
})

// Each frame's thumbnail: the overview stream's next image after it is taken, made small at once,
// as the stream closes its images.
const thumbnails = useThumbnails<string>(8, (url) => URL.revokeObjectURL(url))
watch(
  () => workflow.acquired,
  (key) => key && thumbnails.acquired(key),
)
watch(active, (now) => now && thumbnails.clear())
const overview = computed(() => {
  const td = connection.descriptions?.camera
  if (!active.value || !td || !connection.client) return undefined
  try {
    return connection.client.consume(td).streamUrl('overview')
  } catch {
    return undefined
  }
})
useBudgetedStream(
  computed(() => (overview.value ? root.value : undefined)),
  () => watchStream(connection.client!, overview.value!, { onFrame: shrink }),
)
function shrink(frame: ImageBitmap) {
  const key = thumbnails.claim()
  if (!key || typeof OffscreenCanvas === 'undefined') return
  const width = 160
  const canvas = new OffscreenCanvas(width, Math.round((frame.height * width) / frame.width))
  canvas.getContext('2d')?.drawImage(frame, 0, 0, canvas.width, canvas.height)
  void canvas
    .convertToBlob({ type: 'image/jpeg', quality: 0.7 })
    .then((blob) => thumbnails.add(key, URL.createObjectURL(blob)))
}

async function cancel() {
  const sure = await confirm({
    title: 'Cancel the run?',
    message: 'It stops after the frame under way; the frames taken so far are kept.',
    confirmLabel: 'Cancel the run',
    cancelLabel: 'Keep running',
    danger: true,
  })
  if (sure) await workflow.cancel()
}

const ended = computed(() => {
  const end = workflow.finished
  if (!end || active.value) return undefined
  const frames = `${end.frames} frame${end.frames === 1 ? '' : 's'}`
  const how = { finished: 'finished', cancelled: 'was cancelled', failed: 'failed' }[end.outcome]
  return `${end.name} ${how}: ${frames}${end.error ? `. ${end.error}` : ''}.`
})
</script>

<template>
  <section v-if="active || ended" ref="root" class="run-panel" aria-label="The run">
    <template v-if="active">
      <ProgressBar
        :value="progress?.done ?? null"
        :max="progress?.total ?? 1"
        label="The run's steps"
      />
      <p class="run-panel__status">
        <template v-if="progress"
          >{{ progress.done }} of {{ progress.total }} steps, {{ eta }}</template
        >
        <template v-if="workflow.state === 'paused'"> · paused</template>
      </p>
      <p v-if="progress?.step" class="run-panel__status">{{ progress.step }}</p>
      <div
        v-if="workflow.state === 'asking' && workflow.problem"
        class="run-panel__problem"
        role="alert"
      >
        <p>
          {{ workflow.problem.step }} failed, at attempt {{ workflow.problem.attempt }}:
          {{ workflow.problem.error }}
        </p>
        <div class="run-panel__buttons">
          <AppButton size="sm" variant="primary" @click="workflow.retry()">Retry</AppButton>
          <AppButton size="sm" @click="workflow.skip()">Skip</AppButton>
        </div>
      </div>
      <div class="run-panel__buttons">
        <AppButton v-if="workflow.state === 'paused'" size="sm" @click="workflow.resume()">
          Resume
        </AppButton>
        <AppButton
          v-else
          size="sm"
          :disabled="workflow.state !== 'running'"
          @click="workflow.pause()"
        >
          Pause
        </AppButton>
        <AppButton size="sm" variant="danger" @click="cancel">Cancel</AppButton>
      </div>
      <ul v-if="thumbnails.items.value.length" class="run-panel__thumbnails">
        <li v-for="{ key, image } in thumbnails.items.value" :key="image">
          <img :src="image" :alt="`t${key.t} p${key.p} z${key.z} c${key.c}`" />
          <span>p{{ key.p }} z{{ key.z }} t{{ key.t }}</span>
        </li>
      </ul>
    </template>
    <template v-else>
      <p class="run-panel__status" role="status">{{ ended }}</p>
      <AppButton size="sm" @click="workflow.finished = undefined">Close</AppButton>
    </template>
  </section>
</template>

<style scoped>
.run-panel {
  display: flex;
  flex-direction: column;
  gap: var(--space-2);
  margin-top: var(--space-3);
  font-size: var(--font-size-sm);
}

.run-panel__status {
  margin: 0;
  color: var(--color-text-muted);
}

.run-panel__problem {
  padding: var(--space-2);
  border: var(--border-width) solid var(--color-danger);
  border-radius: var(--radius-md);
}

.run-panel__problem p {
  margin: 0 0 var(--space-2);
}

.run-panel__buttons {
  display: flex;
  gap: var(--space-1);
}

.run-panel__thumbnails {
  display: grid;
  grid-template-columns: repeat(4, 1fr);
  gap: var(--space-1);
  margin: 0;
  padding: 0;
  list-style: none;
}

.run-panel__thumbnails img {
  width: 100%;
  display: block;
}

.run-panel__thumbnails span {
  color: var(--color-text-muted);
}
</style>
