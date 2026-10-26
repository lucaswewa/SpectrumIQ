<script setup lang="ts">
// The calibration wizard (ADR-0051): a dialog that goes through its tasks' steps with Back and
// Next. While a step hasn't done its part, Next says Skip. Closing it early asks first.
import { computed, ref, watch } from 'vue'

import AppButton from '@/ui/AppButton.vue'
import AppDialog from '@/ui/AppDialog.vue'
import { useConfirm } from '@/ui/overlays'

import { WELCOME } from './tasks'
import { useCalibrationWizard } from './wizard'

const wizard = useCalibrationWizard()
const confirm = useConfirm()
const index = ref(0)
const waiting = ref(false)

const steps = computed(() =>
  wizard.tasks.flatMap((task) => task.steps.map((step) => ({ task, step }))),
)
const current = computed(() => steps.value[index.value])
const last = computed(() => index.value === steps.value.length - 1)
const title = computed(() => {
  const task = current.value?.task
  if (task === WELCOME) return 'Welcome to SpectrumIQ'
  return task?.title ? `Calibration: ${task.title}` : 'Calibration'
})
const next = computed(() => {
  if (waiting.value) return last.value ? 'Skip and finish' : 'Skip'
  return last.value ? 'Finish' : 'Next'
})

watch(
  () => wizard.tasks,
  () => {
    index.value = 0
    waiting.value = false
  },
)
watch(index, () => (waiting.value = false))

function forward() {
  if (last.value) wizard.open = false
  else index.value += 1
}

const askToClose = () =>
  confirm({
    title: 'Close the calibration wizard?',
    message: 'You can calibrate from the Calibration page at any time.',
    confirmLabel: 'Close the wizard',
    cancelLabel: 'Keep calibrating',
  })
</script>

<template>
  <AppDialog
    v-model:open="wizard.open"
    :title="title"
    size="lg"
    :close-on-outside-click="false"
    :before-close="askToClose"
  >
    <component
      :is="current.step.component"
      v-if="current"
      :key="index"
      v-bind="current.step.props"
      @waiting="(on: boolean) => (waiting = on)"
    />
    <template #footer>
      <AppButton v-if="index > 0" @click="index -= 1">Back</AppButton>
      <AppButton variant="primary" @click="forward">{{ next }}</AppButton>
    </template>
  </AppDialog>
</template>
