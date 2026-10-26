<script setup lang="ts">
// A step of the calibration wizard that runs a calibration of the objective and channel in use,
// shows what it found, and runs it again on request: accepting it is going on. Next says Skip
// until one has run in this step. A failure shows as a toast, with the server's reason.
import { computed, ref, shallowRef } from 'vue'

import MiniStream from '@/live/MiniStream.vue'
import {
  useCalibrationStore,
  type ShadingRecord,
  type StageCameraRecord,
} from '@/stores/calibration'
import AppButton from '@/ui/AppButton.vue'
import AppSpinner from '@/ui/AppSpinner.vue'

import { summary, type CalibrationKind } from './tasks'

const props = defineProps<{ kind: CalibrationKind; lines: string[]; stream?: boolean }>()
const emit = defineEmits<{ waiting: [waiting: boolean] }>()
const calibration = useCalibrationStore()
const running = ref(false)
const found = shallowRef<StageCameraRecord | ShadingRecord>()

const LABELS: Record<CalibrationKind, string> = {
  stage_camera: 'Calibrate',
  dark: 'Take the dark',
  flat: 'Take the flat',
}
const RUNS = {
  stage_camera: () => calibration.stageCamera(),
  dark: () => calibration.captureDark(),
  flat: () => calibration.captureFlat(),
}
emit('waiting', true)

async function run() {
  running.value = true
  const output = await RUNS[props.kind]()
  running.value = false
  if (!output) return
  found.value = output
  emit('waiting', false)
}

const facts = computed(() => (found.value ? summary(props.kind, found.value) : []))
</script>

<template>
  <div class="wizard-step">
    <p v-for="line in lines" :key="line">{{ line }}</p>
    <MiniStream v-if="stream" />
    <div class="wizard-step__actions">
      <AppButton :variant="found ? 'default' : 'primary'" :disabled="running" @click="run">
        {{ found ? 'Again' : LABELS[kind] }}
      </AppButton>
      <AppSpinner v-if="running" label="Calibrating" />
    </div>
    <dl v-if="found" class="wizard-step__facts" aria-label="What it found">
      <template v-for="[label, value] in facts" :key="label">
        <dt>{{ label }}</dt>
        <dd>{{ value }}</dd>
      </template>
    </dl>
  </div>
</template>
