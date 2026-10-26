<script setup lang="ts">
// The Scan page (ADR-0057): a tile scan planned on the stage map, its form, its estimate as the
// plan changes, and Start once the calibrations allow it (ADR-0051) and the disk holds it. A run
// shows its progress here; the run monitor follows in P6.7.
import { computed, ref, watch } from 'vue'

import { scanGate } from '@/calibration/gating'
import { useCalibrationStore } from '@/stores/calibration'
import { useImagingStore } from '@/stores/imaging'
import { usePeripheralsStore } from '@/stores/peripherals'
import { useWorkflowStore, type ScanEstimate } from '@/stores/workflow'
import AppButton from '@/ui/AppButton.vue'
import ControlPane from '@/ui/ControlPane.vue'
import ProgressBar from '@/ui/ProgressBar.vue'
import SectionHeading from '@/ui/SectionHeading.vue'

import EstimatePanel from './EstimatePanel.vue'
import PlanForm from './PlanForm.vue'
import type { Point } from './geometry'
import { newForm, problems, request } from './plan'
import StageMap, { type Tool } from './StageMap.vue'

const workflow = useWorkflowStore()
const calibration = useCalibrationStore()
const imaging = useImagingStore()
const peripherals = usePeripheralsStore()

const form = ref(newForm())
const tool = ref<Tool>('rect')
const TOOLS: { value: Tool; label: string }[] = [
  { value: 'pan', label: 'Pan' },
  { value: 'rect', label: 'Rectangle' },
  { value: 'polygon', label: 'Polygon' },
  { value: 'place', label: 'Places' },
]

/** The objective in use: the turret's, or the only one. */
const objective = computed(() => {
  const status = calibration.status
  const current = peripherals.objectives.current
  const only = status?.length === 1 ? status[0] : undefined
  return current
    ? { id: current.id, name: current.name }
    : only && { id: only.objective, name: only.name }
})
const gate = computed(() => {
  const channels = form.value.channels.length ? form.value.channels : [imaging.channel ?? null]
  return scanGate(calibration.status, objective.value?.id ?? -1, channels)
})
const asked = computed(() => request(form.value))
const wrong = computed(() => problems(form.value))

// The estimate, asked a moment after the plan settles; an answer to an older plan is dropped.
const estimate = ref<ScanEstimate | string>()
let timer: ReturnType<typeof setTimeout> | undefined
watch(
  [asked, wrong],
  ([next, found]) => {
    clearTimeout(timer)
    estimate.value = undefined
    if (!next || found.length) return
    const key = JSON.stringify(next)
    timer = setTimeout(async () => {
      const answer = await workflow.estimate(next)
      if (JSON.stringify(asked.value) === key) estimate.value = answer
    }, 300)
  },
  { immediate: true },
)

const blockers = computed(() => [...gate.value.blockers, ...wrong.value])
const startable = computed(
  () =>
    blockers.value.length === 0 &&
    typeof estimate.value === 'object' &&
    estimate.value.fits &&
    workflow.state === 'idle',
)
const known = computed(() => (typeof estimate.value === 'object' ? estimate.value : undefined))
const frame = computed(() => known.value?.frame as Point[] | undefined)

async function start() {
  if (asked.value && (await workflow.startTileScan(asked.value))) form.value.name = newForm().name
}
</script>

<template>
  <div class="scan-page">
    <ControlPane width="wide">
      <h1 class="scan-page__title">Scan</h1>
      <div role="group" aria-label="Map tools" class="scan-page__tools">
        <AppButton
          v-for="each in TOOLS"
          :key="each.value"
          size="sm"
          :variant="tool === each.value ? 'primary' : 'default'"
          :aria-pressed="tool === each.value"
          @click="tool = each.value"
        >
          {{ each.label }}
        </AppButton>
      </div>
      <PlanForm v-model="form" :objective="objective?.name" />
      <SectionHeading>Estimate</SectionHeading>
      <EstimatePanel :estimate :blockers :warnings="gate.warnings" />
      <AppButton variant="primary" :disabled="!startable" @click="start">Start</AppButton>
      <div v-if="workflow.state !== 'idle' && workflow.progress" class="scan-page__run">
        <ProgressBar
          :value="workflow.progress.done"
          :max="workflow.progress.total"
          label="The run"
        />
        <span>{{ workflow.progress.step }}</span>
      </div>
    </ControlPane>
    <section class="scan-page__map" aria-label="The stage">
      <StageMap
        v-model:region="form.region"
        v-model:places="form.places"
        :tool
        :tiles="known?.tiles"
        :frame
      />
    </section>
  </div>
</template>

<style scoped>
.scan-page {
  display: flex;
  flex: 1;
  min-width: 0;
  min-height: 0;
}

.scan-page__title {
  margin: 0 0 var(--space-3);
}

.scan-page__tools {
  display: flex;
  flex-wrap: wrap;
  gap: var(--space-1);
  margin-bottom: var(--space-3);
}

.scan-page__map {
  display: flex;
  flex: 1;
  min-width: 0;
}

.scan-page__run {
  display: flex;
  flex-direction: column;
  gap: var(--space-1);
  margin-top: var(--space-3);
  font-size: var(--font-size-sm);
}
</style>
