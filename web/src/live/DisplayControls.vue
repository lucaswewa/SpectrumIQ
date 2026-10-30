<script setup lang="ts">
// How the live previews show the frames (`display.lut`): automatic or manual contrast, the
// window's ends and gamma, the colours, and the ROI's histogram, whose markers move the window. A
// colour frame's white balance is automatic, or fixed from the ROI.
import { computed, ref } from 'vue'

import { useCameraStore } from '@/stores/camera'
import { useDisplayStore } from '@/stores/display'
import { useImagingStore } from '@/stores/imaging'
import AppButton from '@/ui/AppButton.vue'
import AppSelect from '@/ui/AppSelect.vue'
import AppSlider from '@/ui/AppSlider.vue'
import AppToggle from '@/ui/AppToggle.vue'
import FormField from '@/ui/FormField.vue'

import HistogramChart from './HistogramChart.vue'
import { formatBits, fullScale, shownWindow, withEnd, type Colormap, type Lut } from './histogram'

const display = useDisplayStore()
const camera = useCameraStore()
const imaging = useImagingStore()

const log = ref(true)
const bits = computed(() => formatBits(camera.pixelFormat))
const window = computed(() =>
  shownWindow(display.lut, display.histogram?.window as [number, number] | undefined),
)

/** Sends the newest mapping, one at a time: a slider being dragged sends only the latest. */
let pending: Lut | undefined
let sending = false
async function send(next: Lut) {
  pending = next
  if (sending) return
  sending = true
  while (pending) {
    const lut = pending
    pending = undefined
    await display.setLut(lut)
  }
  sending = false
}

const auto = computed({
  get: () => display.lut?.contrast.kind !== 'manual',
  set: (on: boolean) => {
    const lut = display.lut
    if (!lut) return
    if (on) void send({ ...lut, contrast: { kind: 'auto', low_pct: 0.1, high_pct: 99.9 } })
    else
      void send(
        withEnd(lut, window.value ?? [0, fullScale(bits.value)], 'min', window.value?.[0] ?? 0),
      )
  },
})

/** Whether the frames are in colour: their histogram has red, green and blue. */
const colour = computed(() => (display.histogram?.colours?.length ?? 0) > 0)
const autoWhite = computed({
  get: () => display.lut?.white_balance?.kind !== 'fixed',
  set: (on: boolean) => {
    const lut = display.lut
    if (on && lut) void send({ ...lut, white_balance: { kind: 'auto' } })
    else if (!on) void display.whiteFromRoi()
  },
})

/** Moves one end of the window: from the sliders and the histogram's markers. */
function setEnd(end: 'min' | 'max', value: number) {
  const lut = display.lut
  if (lut) void send(withEnd(lut, window.value ?? [0, fullScale(bits.value)], end, value))
}

const gamma = computed({
  get: () => (display.lut?.contrast.kind === 'manual' ? display.lut.contrast.gamma : 1),
  set: (value: number) => {
    const lut = display.lut
    if (lut?.contrast.kind === 'manual')
      void send({ ...lut, contrast: { ...lut.contrast, gamma: value } })
  },
})

/** The channel's colour, as the colormap takes it: unless it is white. */
const channelColour = computed((): Colormap | undefined => {
  const hex = /^#([0-9a-f]{6})$/i.exec(imaging.current?.colour ?? '')?.[1]
  if (!hex || hex.toLowerCase() === 'ffffff') return undefined
  return { colour: [0, 2, 4].map((at) => parseInt(hex.slice(at, at + 2), 16)) }
})
const colourOptions = computed(() => [
  { value: 'grey', label: 'Grey' },
  { value: 'inverted', label: 'Inverted' },
  ...(channelColour.value ? [{ value: 'channel', label: `${imaging.current?.name} colour` }] : []),
])
const colourChoice = computed({
  get: () => (typeof display.lut?.colormap === 'string' ? display.lut.colormap : 'channel'),
  set: (choice: string) => {
    const lut = display.lut
    const colormap = choice === 'channel' ? channelColour.value : (choice as 'grey' | 'inverted')
    if (lut && colormap) void send({ ...lut, colormap })
  },
})
</script>

<template>
  <div class="display-controls">
    <AppToggle v-model="auto" label="Automatic contrast" />
    <HistogramChart
      :counts="display.histogram?.bins ?? []"
      :bits="bits"
      :window="window"
      :log="log"
      :colours="display.histogram?.colours?.map((counts) => counts.bins)"
      @update="setEnd"
    />
    <div class="display-controls__row">
      <AppToggle v-model="log" label="Log scale" />
      <AppButton size="sm" @click="display.autoContrastOnce()">Auto once</AppButton>
    </div>
    <template v-if="!auto && window">
      <FormField :label="`Black: ${window[0]}`">
        <AppSlider
          :model-value="window[0]"
          label="Black level"
          :max="fullScale(bits)"
          @update:model-value="(value) => setEnd('min', value)"
        />
      </FormField>
      <FormField :label="`White: ${window[1]}`">
        <AppSlider
          :model-value="window[1]"
          label="White level"
          :max="fullScale(bits)"
          @update:model-value="(value) => setEnd('max', value)"
        />
      </FormField>
      <FormField :label="`Gamma: ${gamma.toFixed(2)}`">
        <AppSlider v-model="gamma" label="Gamma" :min="0.2" :max="5" :step="0.05" />
      </FormField>
    </template>
    <div v-if="colour" class="display-controls__row">
      <AppToggle v-model="autoWhite" label="Automatic white balance" />
      <AppButton size="sm" @click="display.whiteFromRoi()">White from ROI</AppButton>
    </div>
    <FormField v-else label="Colours">
      <AppSelect v-model="colourChoice" :options="colourOptions" size="sm" />
    </FormField>
  </div>
</template>

<style scoped>
.display-controls {
  display: flex;
  flex-direction: column;
  gap: var(--space-3);
}

.display-controls__row {
  display: flex;
  align-items: center;
  justify-content: space-between;
}
</style>
