<script setup lang="ts">
// The overview with the ROI on it: the area the ROI stream shows, outlined, and a rectangle
// dragged on the image to choose a new one (`select`, in sensor pixels). The image is the default
// slot. Pointer presses bubble here from it, so its double-click still reaches it.
import { computed, onBeforeUnmount, onMounted, ref } from 'vue'

import type { Point, Size } from './contain'
import {
  dragged,
  frameSize,
  frameToBox,
  frameToSensor,
  roiArea,
  sensorToFrame,
  type Readout,
  type Rect,
} from './geometry'

const props = defineProps<{
  readout?: Readout
  /** The overview's frames' size: the image shown, contain-fit. */
  image?: Size
  /** The ROI asked for, in sensor pixels, and its zoom. */
  region?: Rect | null
  zoom: 1 | 2
}>()
const emit = defineEmits<{ select: [region: Rect] }>()

/** The smallest drag that chooses a region, in screen pixels: smaller is a click. */
const MIN_DRAG = 6

const root = ref<HTMLElement>()
const box = ref<Size>({ width: 0, height: 0 })
let resize: ResizeObserver | undefined
function measure() {
  const shown = root.value?.getBoundingClientRect()
  if (shown) box.value = { width: shown.width, height: shown.height }
}
onMounted(() => {
  measure()
  if (typeof ResizeObserver !== 'undefined') {
    resize = new ResizeObserver(measure)
    resize.observe(root.value!)
  }
})
onBeforeUnmount(() => resize?.disconnect())

const frame = computed(() => props.readout && frameSize(props.readout))

/** Where the ROI stream reads, drawn on the overview. */
const outline = computed(() => {
  if (!frame.value || !props.image || !props.readout || box.value.width === 0) return undefined
  const asked = props.region ? sensorToFrame(props.region, props.readout) : undefined
  const area = roiArea(frame.value, asked, props.zoom)
  return frameToBox(area, box.value, props.image, frame.value)
})

const start = ref<Point>()
const end = ref<Point>()
const draft = computed(() => {
  if (!start.value || !end.value) return undefined
  const [a, b] = [start.value, end.value]
  return {
    x: Math.min(a.x, b.x),
    y: Math.min(a.y, b.y),
    width: Math.abs(a.x - b.x),
    height: Math.abs(a.y - b.y),
  }
})

const at = (event: PointerEvent): Point => {
  const shown = root.value!.getBoundingClientRect()
  return { x: event.clientX - shown.left, y: event.clientY - shown.top }
}

function press(event: PointerEvent) {
  if (event.button !== 0 || !frame.value || !props.image) return
  // Measured again: the box may have changed where nothing observes its size.
  measure()
  start.value = end.value = at(event)
}

function move(event: PointerEvent) {
  if (!start.value) return
  end.value = at(event)
  // Captured only once it is a drag: a captured pointer's clicks and double-clicks go to the
  // selector, and the image's double-click moves the stage (ADR-0039).
  const [dx, dy] = [end.value.x - start.value.x, end.value.y - start.value.y]
  if (Math.max(Math.abs(dx), Math.abs(dy)) >= MIN_DRAG) {
    root.value?.setPointerCapture?.(event.pointerId)
  }
}

function release() {
  const [a, b] = [start.value, end.value]
  start.value = end.value = undefined
  if (!a || !b || !frame.value || !props.image || !props.readout) return
  if (Math.abs(a.x - b.x) < MIN_DRAG || Math.abs(a.y - b.y) < MIN_DRAG) return
  const area = dragged(a, b, box.value, props.image, frame.value)
  emit('select', frameToSensor(area, props.readout))
}

const style = (rect: Rect) => ({
  left: `${rect.x}px`,
  top: `${rect.y}px`,
  width: `${rect.width}px`,
  height: `${rect.height}px`,
})
</script>

<template>
  <div
    ref="root"
    class="roi-selector"
    @pointerdown="press"
    @pointermove="move"
    @pointerup="release"
    @pointercancel="start = end = undefined"
  >
    <slot />
    <div v-if="outline && !draft" class="roi-selector__outline" :style="style(outline)" />
    <div v-if="draft" class="roi-selector__draft" :style="style(draft)" />
  </div>
</template>

<style scoped>
.roi-selector {
  position: relative;
  display: flex;
  flex: 1;
  min-width: 0;
  min-height: 0;
  cursor: crosshair;
  touch-action: none;
}

.roi-selector__outline,
.roi-selector__draft {
  position: absolute;
  pointer-events: none;
  border: 2px solid var(--color-accent);
}

.roi-selector__draft {
  border-style: dashed;
  background: color-mix(in srgb, var(--color-accent) 12%, transparent);
}
</style>
