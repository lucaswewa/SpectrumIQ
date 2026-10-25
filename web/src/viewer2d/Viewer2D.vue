<script setup lang="ts">
// The 2D viewer (ADR-0009, amended): an image of a dataset in deep zoom, one tiled image per
// channel, added together in their colours. Each channel's visibility, colour, window and gamma
// (the LUT store) go into its tiles' URLs; the z and t sliders choose the plane, fetched once they
// rest; the scale bar follows the zoom; the probe gives the true values under the pointer, from
// level 0. Overlay layers, chosen in the Layers menu, are drawn over the image (M8).
import fitIcon from '@material-symbols/svg-400/outlined/fit_screen.svg?raw'
import zoomInIcon from '@material-symbols/svg-400/outlined/zoom_in.svg?raw'
import zoomOutIcon from '@material-symbols/svg-400/outlined/zoom_out.svg?raw'
import { computed, ref, shallowRef, useTemplateRef, watch } from 'vue'

import { ApiError } from '@/api/wot/errors'
import LayersMenu from '@/overlay/LayersMenu.vue'
import { useOverlayLayers } from '@/overlay/layers'
import type { OverlayLayer } from '@/overlay/types'
import { useConnectionStore } from '@/stores/connection'
import AppSlider from '@/ui/AppSlider.vue'
import ControlPane from '@/ui/ControlPane.vue'
import FormField from '@/ui/FormField.vue'
import IconButton from '@/ui/IconButton.vue'
import SectionHeading from '@/ui/SectionHeading.vue'

import ChannelControls from './ChannelControls.vue'
import DeepZoom from './DeepZoom.vue'
import { SETTLE_MS, useViewerLuts } from './lut'
import { useProbe } from './probe'
import { scaleBar } from './scale'
import { dataRoot, descriptorUrl, infoUrl, valueUrl, type ImageInfo, type ImageRef } from './tiles'

const props = defineProps<{ dataset: string; image: string; overlays?: OverlayLayer[] }>()

const connection = useConnectionStore()
const luts = useViewerLuts()
const choices = useOverlayLayers()
const view = useTemplateRef<InstanceType<typeof DeepZoom>>('view')

const image = computed<ImageRef | undefined>(() =>
  connection.client
    ? { data: dataRoot(connection.client), dataset: props.dataset, image: props.image }
    : undefined,
)
const info = shallowRef<ImageInfo>()
const failed = ref<string>()
/** The sliders' plane, and the one shown: it follows once the sliders rest. */
const z = ref(0)
const t = ref(0)
const plane = ref({ z: 0, t: 0 })

watch(
  image,
  async (next) => {
    info.value = failed.value = undefined
    if (!next) return
    try {
      const found = (await connection.client!.request(infoUrl(next))) as ImageInfo
      luts.load(found.channels)
      z.value = t.value = 0
      plane.value = { z: 0, t: 0 }
      info.value = found
    } catch (error) {
      failed.value = ApiError.fromFailure(error).message
    }
  },
  { immediate: true },
)

let settling: ReturnType<typeof setTimeout> | undefined
watch([z, t], ([zNext, tNext]) => {
  clearTimeout(settling)
  settling = setTimeout(() => (plane.value = { z: zNext, t: tNext }), SETTLE_MS)
})

const layers = computed(() => {
  const at = image.value
  if (!at || !info.value) return []
  return luts.applied.map((lut, c) => ({
    url: descriptorUrl(at, { c, ...plane.value }, lut),
    visible: lut.visible,
  }))
})
const zoom = ref(0)
const bar = computed(() =>
  info.value && zoom.value > 0 ? scaleBar(info.value.pixel_size[2] / zoom.value) : undefined,
)

const probe = useProbe(async (at, signal) => {
  const where = image.value!
  const shown = luts.applied.flatMap((lut, c) => (lut.visible ? [{ lut, c }] : []))
  return Promise.all(
    shown.map(async ({ lut, c }) => {
      const url = valueUrl(where, { c, ...plane.value }, 0, at)
      const found = (await connection.client!.request(url, { signal })) as { value: number }
      return { label: lut.label, colour: lut.colour, value: found.value }
    }),
  )
})
const overlaysShown = computed(() => choices.visible(props.overlays ?? []))
</script>

<template>
  <div class="viewer-2d">
    <ControlPane width="wide">
      <SectionHeading>Channels</SectionHeading>
      <div class="viewer-2d__channels">
        <ChannelControls v-for="(_, c) in luts.channels" :key="c" :c="c" />
      </div>
      <template v-if="info && (info.size.z > 1 || info.size.t > 1)">
        <SectionHeading>Plane</SectionHeading>
        <FormField v-if="info.size.z > 1" :label="`Focus plane ${z} of ${info.size.z - 1}`">
          <AppSlider v-model="z" label="Focus plane" :min="0" :max="info.size.z - 1" />
        </FormField>
        <FormField v-if="info.size.t > 1" :label="`Time point ${t} of ${info.size.t - 1}`">
          <AppSlider v-model="t" label="Time point" :min="0" :max="info.size.t - 1" />
        </FormField>
      </template>
    </ControlPane>
    <section class="viewer-2d__view" aria-labelledby="viewer-2d-heading">
      <div class="viewer-2d__toolbar">
        <h1 id="viewer-2d-heading" class="viewer-2d__title">
          {{ info ? `${image?.image}: ${info.size.x} × ${info.size.y}` : image?.image }}
        </h1>
        <slot name="toolbar" />
        <LayersMenu v-if="overlays?.length" :layers="overlays" />
        <IconButton :icon="zoomOutIcon" label="Zoom out" size="sm" @click="view?.zoom(1 / 1.5)" />
        <IconButton :icon="zoomInIcon" label="Zoom in" size="sm" @click="view?.zoom(1.5)" />
        <IconButton :icon="fitIcon" label="Show it all" size="sm" @click="view?.home()" />
      </div>
      <p v-if="failed" class="viewer-2d__failed">The image couldn't be opened: {{ failed }}</p>
      <div v-else class="viewer-2d__image">
        <DeepZoom
          ref="view"
          class="viewer-2d__deep-zoom"
          :layers="layers"
          :overlays="overlaysShown"
          @pointer="probe.move"
          @zoom="(next) => (zoom = next)"
        />
        <div v-if="bar" class="viewer-2d__scale" data-testid="scale-bar">
          <span class="viewer-2d__bar" :style="{ width: `${bar.pixels}px` }" />
          {{ bar.label }}
        </div>
        <p v-if="probe.at.value" class="viewer-2d__probe" data-testid="probe">
          ({{ probe.at.value[0] }}, {{ probe.at.value[1] }})
          <span v-for="found in probe.value.value ?? []" :key="found.label">
            · {{ found.label }} {{ found.value }}
          </span>
        </p>
      </div>
    </section>
  </div>
</template>

<style scoped>
.viewer-2d {
  display: flex;
  flex: 1;
  min-width: 0;
  min-height: 0;
}

.viewer-2d__channels {
  display: grid;
  gap: var(--space-3);
}

.viewer-2d__view {
  display: flex;
  flex: 1;
  flex-direction: column;
  min-width: 0;
}

.viewer-2d__toolbar {
  display: flex;
  gap: var(--space-2);
  align-items: center;
  padding: var(--space-2) var(--space-3);
  border-bottom: var(--border-width) solid var(--color-border);
}

.viewer-2d__title {
  margin: 0 auto 0 0;
  font-size: var(--font-size-md);
}

.viewer-2d__image {
  position: relative;
  display: flex;
  flex: 1;
  min-height: 0;
}

.viewer-2d__deep-zoom {
  flex: 1;
}

.viewer-2d__failed {
  margin: var(--space-6);
  color: var(--color-text-muted);
}

.viewer-2d__scale,
.viewer-2d__probe {
  position: absolute;
  bottom: var(--space-3);
  margin: 0;
  padding: var(--space-1) var(--space-2);
  border-radius: var(--radius-sm);
  background: var(--color-overlay);
  color: var(--color-on-overlay);
  font-size: var(--font-size-sm);
  font-variant-numeric: tabular-nums;
  pointer-events: none;
}

.viewer-2d__scale {
  left: var(--space-3);
  display: grid;
  gap: var(--space-1);
}

.viewer-2d__bar {
  display: block;
  height: 4px;
  background: currentcolor;
}

.viewer-2d__probe {
  right: var(--space-3);
}
</style>
