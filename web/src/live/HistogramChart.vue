<script setup lang="ts">
// The ROI's histogram: 256 bars over the pixel format's whole range, linear or logarithmic,
// with the display window's two ends as markers. Dragging a marker, or moving it with the arrow
// keys, sets that end (`update`); the page makes the mapping manual from there.
import { computed, onMounted, ref, watch } from 'vue'

import { barHeights, shareToValue, valueToShare } from './histogram'

const props = defineProps<{
  counts: readonly number[]
  /** The pixel format's bits: the bars span 0 to 2^bits − 1. */
  bits: number
  window?: readonly [number, number]
  log?: boolean
}>()
const emit = defineEmits<{ update: [end: 'min' | 'max', value: number] }>()

const root = ref<HTMLElement>()
const canvas = ref<HTMLCanvasElement>()
const heights = computed(() => barHeights(props.counts, props.log ?? false))

function draw() {
  const target = canvas.value
  const context = target?.getContext('2d')
  if (!target || !context) return
  const [width, height] = [target.clientWidth, target.clientHeight]
  target.width = width
  target.height = height
  context.clearRect(0, 0, width, height)
  context.fillStyle = getComputedStyle(target).color
  const bar = width / Math.max(heights.value.length, 1)
  heights.value.forEach((share, index) => {
    const tall = share * height
    context.fillRect(index * bar, height - tall, Math.ceil(bar), tall)
  })
}
watch(heights, draw)
onMounted(draw)

const ends = computed(() =>
  props.window
    ? (['min', 'max'] as const).map((end, index) => ({
        end,
        value: props.window![index]!,
        left: `${valueToShare(props.window![index]!, props.bits) * 100}%`,
      }))
    : [],
)

/** Moves an end to where the pointer is, while it is held. */
function drag(end: 'min' | 'max', event: PointerEvent) {
  const box = root.value?.getBoundingClientRect()
  if (!box) return
  const target = event.currentTarget as HTMLElement
  target.setPointerCapture?.(event.pointerId)
  const move = (moved: PointerEvent) =>
    emit('update', end, shareToValue((moved.clientX - box.left) / box.width, props.bits))
  const up = () => {
    target.removeEventListener('pointermove', move)
    target.removeEventListener('pointerup', up)
  }
  target.addEventListener('pointermove', move)
  target.addEventListener('pointerup', up)
}

/** The arrow keys move an end by 1/256 of the range; Shift by 1/16. */
function key(end: 'min' | 'max', value: number, event: KeyboardEvent) {
  const step = (2 ** props.bits / 256) * (event.shiftKey ? 16 : 1)
  const sign = { ArrowLeft: -1, ArrowDown: -1, ArrowRight: 1, ArrowUp: 1 }[event.key]
  if (sign === undefined) return
  event.preventDefault()
  emit('update', end, Math.round(value + sign * step))
}
</script>

<template>
  <div ref="root" class="histogram">
    <canvas ref="canvas" class="histogram__bars" role="img" aria-label="Histogram of the ROI" />
    <div
      v-for="marker in ends"
      :key="marker.end"
      class="histogram__marker"
      :class="`histogram__marker--${marker.end}`"
      :style="{ left: marker.left }"
      role="slider"
      tabindex="0"
      :aria-label="marker.end === 'min' ? 'Black level' : 'White level'"
      aria-valuemin="0"
      :aria-valuemax="2 ** bits - 1"
      :aria-valuenow="marker.value"
      @pointerdown="drag(marker.end, $event)"
      @keydown="key(marker.end, marker.value, $event)"
    />
  </div>
</template>

<style scoped>
.histogram {
  position: relative;
  height: 96px;
  border: var(--border-width) solid var(--color-border);
  border-radius: var(--radius-sm);
  background: var(--color-surface-sunken);
  touch-action: none;
}

.histogram__bars {
  display: block;
  width: 100%;
  height: 100%;
  color: var(--color-text-muted);
}

.histogram__marker {
  position: absolute;
  top: 0;
  bottom: 0;
  width: var(--space-2);
  margin-left: calc(-1 * var(--space-1));
  cursor: ew-resize;
}

.histogram__marker::after {
  position: absolute;
  inset: 0 calc(var(--space-1) - 1px);
  background: var(--color-accent);
  content: '';
}

.histogram__marker:focus-visible {
  outline: var(--focus-ring-width) solid var(--color-focus);
}
</style>
