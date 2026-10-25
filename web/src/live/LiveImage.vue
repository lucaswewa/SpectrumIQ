<script setup lang="ts">
// The live image: a camera's MJPEG stream drawn to a canvas, contain-fit
// with letterboxing. It watches a shared stream while the app is connected
// and the stream isn't disabled, through the stream budget, which pauses it
// while it is off screen or the page is hidden (ADR-0037, ADR-0038). It says
// why when there is no image, reports the frames' size, and where the image
// is double-clicked.
import { computed, ref, watch } from 'vue'

import { useBudgetedStream } from '@/api/streams'
import type { ApiError } from '@/api/wot/errors'
import { useConnectionStore } from '@/stores/connection'
import AppButton from '@/ui/AppButton.vue'
import AppSpinner from '@/ui/AppSpinner.vue'

import { containPoint, type Point } from './contain'
import { useCaptureFlashes } from './flash'
import { useLiveImagePreferences } from './preferences'
import { watchStream, type StreamState } from './streams'

const props = withDefaults(
  defineProps<{
    /** The camera Thing. */
    thing?: string
    /** Its stream: `overview`, the whole frame, or `roi`. */
    stream?: string
  }>(),
  { thing: 'camera', stream: 'overview' },
)

const emit = defineEmits<{
  /** A double-click on the image: where, from its centre, as shares of its width and height. */
  doubleClick: [point: Point]
  /** The frames' size, in pixels, when it changes. */
  frameSize: [size: { width: number; height: number }]
}>()

const connection = useConnectionStore()
const preferences = useLiveImagePreferences()
const root = ref<HTMLElement>()
const canvas = ref<HTMLCanvasElement>()
const streamState = ref<StreamState>('connecting')
const streamError = ref<ApiError>()
const hasFrame = ref(false)

/** The stream's URL, while connected to a camera that has it. */
const url = computed(() => {
  const td = connection.descriptions?.[props.thing]
  if (connection.state !== 'connected' || !connection.client || !td) return undefined
  try {
    return connection.client.consume(td).streamUrl(props.stream)
  } catch {
    return undefined
  }
})

// The stream takes a place in the budget while there is one to show.
const shown = computed(() =>
  connection.client && url.value && !preferences.streamDisabled ? root.value : undefined,
)
const running = useBudgetedStream(shown, () => {
  hasFrame.value = false
  return watchStream(connection.client!, url.value!, {
    onFrame: draw,
    onState: (state, error) => {
      streamState.value = state
      streamError.value = error
    },
  })
})

let drawn = 0
function draw(frame: ImageBitmap) {
  const target = canvas.value
  if (!target) return
  if (target.width !== frame.width || target.height !== frame.height) {
    target.width = frame.width
    target.height = frame.height
    emit('frameSize', { width: frame.width, height: frame.height })
  }
  target.getContext('2d')?.drawImage(frame, 0, 0)
  drawn += 1
  target.dataset.frames = String(drawn)
  hasFrame.value = true
}

function onDoubleClick(event: MouseEvent) {
  const target = canvas.value
  if (!target || !hasFrame.value) return
  const box = target.getBoundingClientRect()
  const at = { x: event.clientX - box.left, y: event.clientY - box.top }
  const point = containPoint(at, box, target)
  if (point) emit('doubleClick', point)
}

const status = computed(() => {
  if (preferences.streamDisabled) return 'disabled'
  if (connection.state !== 'connected') return 'offline'
  if (!url.value || streamState.value === 'error') return 'error'
  if (!running.value) return 'paused'
  return hasFrame.value ? 'live' : 'connecting'
})

const errorMessage = computed(() =>
  url.value ? (streamError.value?.message ?? '') : 'The camera has no live stream.',
)

// Each flash gets a new element, which restarts the animation.
const flash = ref(0)
watch(useCaptureFlashes(), (count) => (flash.value = count))
</script>

<template>
  <div ref="root" class="live-image" :data-status="status">
    <canvas
      v-show="status !== 'disabled'"
      ref="canvas"
      class="live-image__canvas"
      role="img"
      aria-label="Live image"
      @dblclick="onDoubleClick"
    />
    <div v-if="status !== 'live'" class="live-image__status" role="status">
      <div class="live-image__message">
        <template v-if="status === 'disabled'">
          <p>The live image is turned off.</p>
          <AppButton @click="preferences.streamDisabled = false">Turn it on</AppButton>
        </template>
        <p v-else-if="status === 'offline'">No connection to the instrument.</p>
        <p v-else-if="status === 'paused'">
          Paused: other live images are open. It starts again when one closes.
        </p>
        <p v-else-if="status === 'error'">The live image isn't available. {{ errorMessage }}</p>
        <template v-else>
          <AppSpinner />
          <p>Connecting to the camera…</p>
        </template>
      </div>
    </div>
    <div v-if="flash" :key="flash" class="live-image__flash" @animationend="flash = 0" />
  </div>
</template>

<style scoped>
.live-image {
  position: relative;
  flex: 1;
  min-width: 0;
  min-height: 0;
  overflow: hidden;
  background: var(--color-image-backdrop);
}

.live-image__canvas {
  display: block;
  width: 100%;
  height: 100%;
  object-fit: contain;
}

.live-image__status {
  position: absolute;
  inset: 0;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: var(--space-6);
}

.live-image__message {
  display: flex;
  flex-direction: column;
  gap: var(--space-3);
  align-items: center;
  max-width: 28rem;
  padding: var(--space-4) var(--space-6);
  color: var(--color-text);
  text-align: center;
  background: var(--color-surface-raised);
  border-radius: var(--radius-md);
  box-shadow: var(--shadow-md);
}

.live-image__message p {
  margin: 0;
}

.live-image__flash {
  position: absolute;
  inset: 0;
  pointer-events: none;
  background: var(--color-capture-flash);
  opacity: 0;
  animation: live-image-flash 400ms var(--easing-standard);
}

@keyframes live-image-flash {
  from {
    opacity: 0.7;
  }

  to {
    opacity: 0;
  }
}
</style>
