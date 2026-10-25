<script setup lang="ts">
// The camera on the Live page: live view on and off, a snap, the exposure, and three readout
// presets: the whole sensor, the ROI only (faster), and 2 × 2 binning.
import camera from '@material-symbols/svg-400/outlined/photo_camera.svg?raw'
import { computed, ref } from 'vue'

import { useCameraStore } from '@/stores/camera'
import { useDisplayStore } from '@/stores/display'
import AppButton from '@/ui/AppButton.vue'
import FormField from '@/ui/FormField.vue'
import NumberField from '@/ui/NumberField.vue'

import { flashLiveImage } from './flash'

const store = useCameraStore()
const display = useDisplayStore()

const exposure = computed({
  get: () => store.exposureMs ?? 0,
  set: () => {},
})
const lastSnap = ref<string>()

async function snap() {
  const jpeg = await store.snap()
  if (!jpeg) return
  flashLiveImage()
  lastSnap.value = jpeg.href
}

/** A region on a grid of 8 sensor pixels, which every readout accepts. */
const ALIGN = 8
const align = (n: number) => Math.floor(n / ALIGN) * ALIGN

const presets = computed(() => {
  const sensor = store.sensor
  const roi = display.roiRegion?.region
  const whole = sensor && { offset_x: 0, offset_y: 0, width: sensor.width, height: sensor.height }
  return [
    { label: 'Full frame', region: whole, binning: 1 },
    {
      label: 'ROI only',
      region: roi && {
        offset_x: align(roi.x),
        offset_y: align(roi.y),
        width: Math.max(align(roi.width), ALIGN),
        height: Math.max(align(roi.height), ALIGN),
      },
      binning: 1,
    },
    { label: 'Binning 2 × 2', region: whole, binning: 2 },
  ]
})

async function apply(preset: (typeof presets.value)[number]) {
  if (!preset.region) return
  const binning = { horizontal: preset.binning, vertical: preset.binning }
  // The binning first: a region is checked against the binning in place.
  if (await store.setBinning(binning)) await store.setRegion(preset.region)
}

const lost = computed(() => (store.stats ? store.stats.lost_no_slot + store.stats.lost_device : 0))
</script>

<template>
  <div class="camera-controls">
    <div class="camera-controls__row">
      <AppButton v-if="store.live" @click="store.stopLive()">Stop live</AppButton>
      <AppButton v-else variant="primary" @click="store.startLive()">Start live</AppButton>
      <AppButton :icon="camera" @click="snap">Snap</AppButton>
    </div>
    <FormField label="Exposure (ms)" help="Press Enter to set it.">
      <NumberField
        v-model="exposure"
        :min="0.01"
        :step="0.01"
        size="sm"
        @submit="(ms) => store.setExposure(ms)"
      />
    </FormField>
    <div class="camera-controls__presets" role="group" aria-label="Readout">
      <AppButton
        v-for="preset in presets"
        :key="preset.label"
        size="sm"
        :disabled="!preset.region"
        @click="apply(preset)"
      >
        {{ preset.label }}
      </AppButton>
    </div>
    <dl class="camera-controls__readouts">
      <dt>Rate</dt>
      <dd>{{ store.stats ? `${store.stats.fps.toFixed(1)} frames/s` : '…' }}</dd>
      <dt>Frames</dt>
      <dd>{{ store.stats?.published ?? '…' }}</dd>
      <dt>Lost</dt>
      <dd :class="{ 'camera-controls__lost': lost > 0 }" data-test="lost">{{ lost }}</dd>
    </dl>
    <img v-if="lastSnap" :src="lastSnap" alt="The last snap" class="camera-controls__snap" />
  </div>
</template>

<style scoped>
.camera-controls {
  display: flex;
  flex-direction: column;
  gap: var(--space-3);
}

.camera-controls__row,
.camera-controls__presets {
  display: flex;
  flex-wrap: wrap;
  gap: var(--space-2);
}

.camera-controls__readouts {
  display: grid;
  grid-template-columns: max-content 1fr;
  gap: var(--space-1) var(--space-3);
  margin: 0;
}

.camera-controls__readouts dt {
  color: var(--color-text-muted);
}

.camera-controls__readouts dd {
  margin: 0;
  font-variant-numeric: tabular-nums;
}

.camera-controls__lost {
  color: var(--color-danger);
  font-weight: var(--font-weight-semibold);
}

.camera-controls__snap {
  width: 100%;
  border-radius: var(--radius-sm);
}
</style>
