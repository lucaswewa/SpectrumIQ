<script setup lang="ts">
// An image, contain-fit in its box, with overlay layers over it (ADR-0009, amended).
// Where the image is drawn is measured once it has loaded, and again
// whenever its box changes size.
import { onBeforeUnmount, onMounted, ref, useTemplateRef } from 'vue'

import OverlayLayers from './OverlayLayers.vue'
import type { OverlayLayer, Placement } from './types'

defineProps<{ src: string; alt: string; layers: OverlayLayer[] }>()

const image = useTemplateRef<HTMLImageElement>('image')
const placement = ref<Placement>()

function measure() {
  const img = image.value
  if (!img?.naturalWidth || !img.naturalHeight) return
  const [box, natural] = [
    [img.clientWidth, img.clientHeight],
    [img.naturalWidth, img.naturalHeight],
  ] as const
  const scale = Math.min(box[0] / natural[0], box[1] / natural[1])
  const [width, height] = [natural[0] * scale, natural[1] * scale]
  placement.value = {
    x: img.offsetLeft + (box[0] - width) / 2,
    y: img.offsetTop + (box[1] - height) / 2,
    width,
    height,
  }
}

let observer: ResizeObserver | undefined
onMounted(() => {
  observer = new ResizeObserver(measure)
  if (image.value) observer.observe(image.value)
})
onBeforeUnmount(() => observer?.disconnect())
</script>

<template>
  <div class="overlaid-image">
    <img ref="image" class="overlaid-image__image" :src="src" :alt="alt" @load="measure" />
    <OverlayLayers :layers="layers" :placement="placement" />
  </div>
</template>

<style scoped>
.overlaid-image {
  position: relative;
  width: 100%;
  height: 100%;
}

.overlaid-image__image {
  display: block;
  width: 100%;
  height: 100%;
  object-fit: contain;
}
</style>
