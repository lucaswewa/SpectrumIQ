<script setup lang="ts">
// The live mosaic behind a map (ADR-0060): the server's rendering of the map's view, from the
// `mosaic_live` stream through the stream budget. The view is written as the viewport, at most
// every 100 ms; each image is placed where the viewport the server last took puts it, so that it
// moves with the map until the next arrives. Unpainted stage is black.
import { computed, ref, watch } from 'vue'

import { useBudgetedStream } from '@/api/streams'
import { watchStream } from '@/live/streams'
import type { View } from '@/scan/geometry'
import { useConnectionStore } from '@/stores/connection'
import { useMosaicStore } from '@/stores/mosaic'

import { placement, viewportOf, type Viewport } from './viewport'

const props = defineProps<{ view: View }>()

const connection = useConnectionStore()
const mosaic = useMosaicStore()
const root = ref<HTMLElement>()
const canvas = ref<HTMLCanvasElement>()
/** The viewport of the image drawn. */
const drawnFor = ref<Viewport>()

watch(
  () => [props.view, mosaic.pxPerUm] as const,
  ([view, pxPerUm]) => {
    if (pxPerUm && view.size[0] >= 1 && view.size[1] >= 1) {
      mosaic.setViewport(viewportOf(view, pxPerUm))
    }
  },
  { immediate: true },
)

/** The stream's URL, while connected to a server with the live mosaic. */
const url = computed(() => {
  const td = connection.descriptions?.mosaic_live
  if (connection.state !== 'connected' || !connection.client || !td) return undefined
  try {
    return connection.client.consume(td).streamUrl('view')
  } catch {
    return undefined
  }
})
const shownIn = computed(() => (url.value ? root.value : undefined))
useBudgetedStream(shownIn, () => watchStream(connection.client!, url.value!, { onFrame: draw }))

function draw(frame: ImageBitmap) {
  const target = canvas.value
  if (!target) return
  if (target.width !== frame.width || target.height !== frame.height) {
    target.width = frame.width
    target.height = frame.height
  }
  target.getContext('2d')?.drawImage(frame, 0, 0)
  drawnFor.value = mosaic.viewport
}

const placed = computed(() => {
  const [viewport, pxPerUm] = [drawnFor.value, mosaic.pxPerUm]
  if (!viewport || !pxPerUm) return { display: 'none' }
  const { left, top, width, height } = placement(props.view, viewport, pxPerUm)
  return { left: `${left}px`, top: `${top}px`, width: `${width}px`, height: `${height}px` }
})
</script>

<template>
  <div ref="root" class="mosaic-backdrop" aria-hidden="true">
    <canvas ref="canvas" class="mosaic-backdrop__image" :style="placed" />
  </div>
</template>

<style scoped>
.mosaic-backdrop {
  position: absolute;
  inset: 0;
  overflow: hidden;
  /* Unpainted stage is black in the images: the surround matches it, in either theme. */
  background: var(--palette-neutral-950);
}

.mosaic-backdrop__image {
  position: absolute;
}
</style>
