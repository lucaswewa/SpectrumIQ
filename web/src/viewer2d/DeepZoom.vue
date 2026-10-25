<script setup lang="ts">
// A deep-zoom view (ADR-0009, amended) on OpenSeadragon, which is loaded when first needed: one
// tiled image per layer, all in one place, composited additively ('lighter'), as an image's
// channels are. The wheel and a pinch zoom, and dragging pans.
//
// A layer whose URL changes, for a new mapping or plane, is loaded hidden beside the one shown,
// which is removed only once the new one is fully loaded: the view never blanks, and the zoom and
// pan stay as they were. A hidden layer is drawn at opacity 0.
//
// It tells the image's pixel under the pointer, for the probe, and its zoom, in screen pixels per
// image pixel, for the scale bar. Overlay layers are drawn over the image, placed again whenever
// the view pans, zooms or changes size.
import type OpenSeadragon from 'openseadragon'
import { onBeforeUnmount, onMounted, ref, useTemplateRef, watch } from 'vue'

import OverlayLayers from '@/overlay/OverlayLayers.vue'
import type { OverlayLayer, Placement } from '@/overlay/types'

/** A layer: a DZI descriptor's URL, and whether it's shown. */
export interface DeepZoomLayer {
  url: string
  visible: boolean
}

const props = defineProps<{
  layers: DeepZoomLayer[]
  /** Drawn over the image, in its pixels. */
  overlays?: OverlayLayer[]
}>()
const emit = defineEmits<{
  /** The image's pixel under the pointer, or undefined off the image. */
  pointer: [pixel: [number, number] | undefined]
  /** How many screen pixels an image pixel takes. */
  zoom: [zoom: number]
}>()

const element = useTemplateRef<HTMLElement>('element')
const status = ref<'loading' | 'shown' | 'failed'>('loading')
/** Where the image is, in the view's pixels. */
const placement = ref<Placement>()
let osd: typeof OpenSeadragon | undefined
let viewer: OpenSeadragon.Viewer | undefined
let unmounted = false
/** Each layer's image shown, with the URL it came from. */
const shown: ({ url: string; item: OpenSeadragon.TiledImage } | undefined)[] = []
/** The URL each layer is loading, if any. */
const loading: (string | undefined)[] = []

/** Loads layer `index` from its URL, hidden, then shows it in place of the one before. */
function load(index: number, { url }: DeepZoomLayer) {
  if (!viewer || shown[index]?.url === url || loading[index] === url) return
  loading[index] = url
  viewer.addTiledImage({
    tileSource: url,
    opacity: 0,
    // Loaded while hidden.
    preload: true,
    compositeOperation: 'lighter',
    success: (event) => {
      const { item } = event as unknown as { item: OpenSeadragon.TiledImage }
      item.whenFullyLoaded(() => {
        // A newer URL was asked for meanwhile, or the view is gone.
        if (unmounted || loading[index] !== url) return void viewer?.world.removeItem(item)
        loading[index] = undefined
        const before = shown[index]
        shown[index] = { url, item }
        item.setOpacity(props.layers[index]?.visible ? 1 : 0)
        if (before) viewer!.world.removeItem(before.item)
        status.value = 'shown'
        place()
      })
    },
    error: () => {
      if (loading[index] === url) loading[index] = undefined
      status.value = 'failed'
    },
  })
}

/** Brings the view to `layers`: new URLs loaded, visibility set, layers gone removed. */
function sync(layers: DeepZoomLayer[]) {
  layers.forEach((layer, index) => {
    load(index, layer)
    shown[index]?.item.setOpacity(layer.visible ? 1 : 0)
  })
  for (let index = layers.length; index < shown.length; index++) {
    const gone = shown[index]
    if (gone) viewer?.world.removeItem(gone.item)
    shown[index] = loading[index] = undefined
  }
}

/** The first layer shown: the others lie on it. */
const reference = () => shown.find((layer) => layer)?.item

/** Measures where the image is, and its zoom. */
function place() {
  const item = reference()
  if (!viewer || !item) return
  const { x, y, width, height } = viewer.viewport.viewportToViewerElementRectangle(
    item.getBounds(true),
  )
  placement.value = { x, y, width, height }
  emit('zoom', item.viewportToImageZoom(viewer.viewport.getZoom(true)))
}

function point(event: PointerEvent) {
  const item = reference()
  if (!osd || !item || !element.value) return
  const box = element.value.getBoundingClientRect()
  const at = item.viewerElementToImageCoordinates(
    new osd.Point(event.clientX - box.left, event.clientY - box.top),
  )
  const size = item.getContentSize()
  const inside = at.x >= 0 && at.y >= 0 && at.x < size.x && at.y < size.y
  emit('pointer', inside ? [Math.floor(at.x), Math.floor(at.y)] : undefined)
}

onMounted(async () => {
  const { default: openSeadragon } = await import('openseadragon')
  if (unmounted || !element.value) return
  osd = openSeadragon
  viewer = openSeadragon({
    element: element.value,
    // The app draws its own controls, which follow the theme.
    showNavigationControl: false,
    crossOriginPolicy: 'Anonymous',
    visibilityRatio: 1,
  })
  viewer.addHandler('update-viewport', place)
  viewer.addHandler('resize', place)
  sync(props.layers)
})
watch(() => props.layers, sync, { deep: true })
onBeforeUnmount(() => {
  unmounted = true
  viewer?.destroy()
  viewer = undefined
})

defineExpose({
  /** Zooms in by `factor`, or out by less than 1. */
  zoom(factor: number) {
    viewer?.viewport.zoomBy(factor)
    viewer?.viewport.applyConstraints()
  },
  /** Shows it all again. */
  home: () => viewer?.viewport.goHome(),
})
</script>

<template>
  <div
    class="deep-zoom"
    :data-status="status"
    @pointermove="point"
    @pointerleave="emit('pointer', undefined)"
  >
    <div ref="element" class="deep-zoom__view" />
    <OverlayLayers :layers="overlays ?? []" :placement="placement" />
    <p v-if="status === 'failed'" class="deep-zoom__failed">It couldn't be shown.</p>
  </div>
</template>

<style scoped>
.deep-zoom {
  position: relative;
  min-height: 0;
  background: var(--color-image-backdrop);
}

.deep-zoom__view {
  position: absolute;
  inset: 0;
}

.deep-zoom__failed {
  position: absolute;
  inset: 0;
  display: grid;
  place-items: center;
  margin: 0;
  color: var(--color-text-muted);
}
</style>
