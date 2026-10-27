<script setup lang="ts">
// The Mosaic page (ADR-0060): the live mosaic, painted as the stage moves, panned by dragging and
// zoomed with the wheel about the pointer; the camera's field where the stage is; and a minimap of
// what was painted, with the view in it, which a click recentres. A double-click moves the stage
// there, within its travel. Painting on or off, the painted area whole or the stage centred,
// Clear once confirmed, and Save as a dataset.
import { computed, onBeforeUnmount, onMounted, ref, useTemplateRef, watch } from 'vue'

import { fit, pan, placed, svgPoints, toMap, toStage, zoom } from '@/scan/geometry'
import type { Point, View } from '@/scan/geometry'
import { useMosaicStore } from '@/stores/mosaic'
import { useStageStore } from '@/stores/stage'
import { useWorkflowStore } from '@/stores/workflow'
import AppButton from '@/ui/AppButton.vue'
import AppToggle from '@/ui/AppToggle.vue'
import ControlPane from '@/ui/ControlPane.vue'
import { useConfirm } from '@/ui/overlays'

import MosaicBackdrop from './MosaicBackdrop.vue'
import { shownBy, union } from './viewport'

/** The minimap's size, in pixels. */
const MINIMAP: Point = [180, 120]

const mosaic = useMosaicStore()
const stage = useStageStore()
const workflow = useWorkflowStore()
const confirm = useConfirm()
const host = useTemplateRef<SVGSVGElement>('host')
const view = ref<View>({ centre: [0, 0], scale: 0.5, size: [800, 600] })

const at = computed<Point | undefined>(() => stage.position && [stage.position.x, stage.position.y])
const painted = computed<[Point, Point] | undefined>(() => {
  const box = mosaic.bounds
  return box
    ? [
        [box.x0, box.y0],
        [box.x1, box.y1],
      ]
    : undefined
})
const field = computed(
  () => workflow.field && at.value && placed(workflow.field as Point[], at.value),
)

/** Shows what was painted, whole. */
function showPainted() {
  if (painted.value) view.value = fit(view.value.size, painted.value)
}

/** Centres the view on the stage. */
function showStage() {
  if (at.value) view.value = { ...view.value, centre: at.value }
}

// The view takes its element's size, and first shows what was painted, or else the stage.
let observer: ResizeObserver | undefined
onMounted(() => {
  const measure = () => {
    const box = host.value?.getBoundingClientRect()
    if (box?.width) view.value = { ...view.value, size: [box.width, box.height] }
  }
  measure()
  if (typeof ResizeObserver !== 'undefined') {
    observer = new ResizeObserver(measure)
    observer.observe(host.value!)
  }
})
onBeforeUnmount(() => observer?.disconnect())
// Once the bounds are read: `null` if nothing was painted.
let placedOnce = false
watch(
  [() => mosaic.bounds, at],
  ([bounds]) => {
    if (placedOnce || bounds === undefined || !(painted.value || at.value)) return
    placedOnce = true
    if (painted.value) showPainted()
    else showStage()
  },
  { immediate: true },
)

/** Where `event` is in the view: in its pixels, and on the stage. */
function pointer(event: MouseEvent): [Point, Point] {
  const box = host.value!.getBoundingClientRect()
  const point: Point = [event.clientX - box.left, event.clientY - box.top]
  return [point, toStage(view.value, point)]
}
let dragged: Point | undefined
function down(event: PointerEvent) {
  dragged = pointer(event)[0]
}
function move(event: PointerEvent) {
  if (!dragged) return
  const [point] = pointer(event)
  view.value = pan(view.value, [point[0] - dragged[0], point[1] - dragged[1]])
  dragged = point
}
function wheel(event: WheelEvent) {
  view.value = zoom(view.value, Math.exp(-event.deltaY / 500), pointer(event)[0])
}

/** Moves the stage to the point double-clicked, or as near as its travel allows. */
async function moveHere(event: MouseEvent) {
  const [, [x, y]] = pointer(event)
  const [position, limits] = [stage.position, stage.limits]
  if (!position) return
  const within = (value: number, axis: 'x' | 'y') =>
    limits ? Math.min(Math.max(value, limits.min[axis]), limits.max[axis]) : value
  await stage.moveTo({ x: within(x, 'x'), y: within(y, 'y'), z: position.z })
}

async function clear() {
  const sure = await confirm({
    title: 'Clear the mosaic?',
    message: 'What was painted is gone; frames paint again as they come.',
    confirmLabel: 'Clear',
    danger: true,
  })
  if (sure) await mosaic.clear()
}

// The minimap: what was painted and the view, whole; a click centres the view there.
const minimap = computed<View | undefined>(
  () => painted.value && fit(MINIMAP, union(painted.value, shownBy(view.value)), 0.1),
)
function recentre(event: MouseEvent) {
  const box = (event.currentTarget as Element).getBoundingClientRect()
  const point: Point = [event.clientX - box.left, event.clientY - box.top]
  if (minimap.value) view.value = { ...view.value, centre: toStage(minimap.value, point) }
}
const box = (map: View, [low, high]: [Point, Point]) => {
  const corners: Point[] = [low, [high[0], low[1]], high, [low[0], high[1]]]
  return svgPoints(map, corners)
}
const dot = (map: View, point: Point) => {
  const [cx, cy] = toMap(map, point)
  return { cx, cy }
}

const status = computed(() => {
  if (!mosaic.enabled) return 'Not painting: turn on Paint, then move the stage.'
  if (!painted.value)
    return 'Painting: a frame paints where the stage stands still, once calibrated.'
  const done = mosaic.stats
  if (!done) return 'Painting.'
  const mb = (bytes: number) => Math.round(bytes / 1e6)
  return (
    `Painting: ${done.painted} frames painted; ${done.pages_on_gpu} pages on the GPU, ` +
    `${mb(done.vram_bytes)} of ${mb(done.vram_budget_bytes)} MB.`
  )
})
</script>

<template>
  <div class="mosaic-page">
    <ControlPane>
      <h1 class="mosaic-page__title">Mosaic</h1>
      <AppToggle
        :model-value="mosaic.enabled ?? false"
        label="Paint"
        :disabled="mosaic.enabled === undefined"
        @update:model-value="mosaic.setEnabled"
      />
      <p class="mosaic-page__note" role="status">{{ status }}</p>
      <div class="mosaic-page__tools">
        <AppButton size="sm" :disabled="!painted" @click="showPainted">Show it all</AppButton>
        <AppButton size="sm" :disabled="!at" @click="showStage">Go to the stage</AppButton>
      </div>
      <div class="mosaic-page__tools">
        <AppButton :disabled="!painted" @click="clear">Clear</AppButton>
        <AppButton :disabled="!painted" @click="mosaic.save">Save</AppButton>
      </div>
      <p class="mosaic-page__note">
        Drag to pan, and zoom with the wheel; double-click to move the stage there.
      </p>
    </ControlPane>
    <section class="mosaic-page__view" aria-label="The live mosaic">
      <MosaicBackdrop :view />
      <svg
        ref="host"
        class="mosaic-page__svg"
        role="img"
        aria-label="The live mosaic"
        @pointerdown="down"
        @pointermove="move"
        @pointerup="dragged = undefined"
        @pointerleave="dragged = undefined"
        @dblclick.prevent="moveHere"
        @wheel.prevent="wheel"
      >
        <polygon v-if="field" :points="svgPoints(view, field)" class="mosaic-page__field" />
        <circle v-else-if="at" v-bind="dot(view, at)" r="4" class="mosaic-page__field" />
      </svg>
      <svg
        v-if="minimap && painted"
        class="mosaic-page__minimap"
        role="img"
        aria-label="What was painted, and the view"
        :width="MINIMAP[0]"
        :height="MINIMAP[1]"
        @click="recentre"
      >
        <polygon :points="box(minimap, painted)" class="mosaic-page__painted" />
        <polygon :points="box(minimap, shownBy(view))" class="mosaic-page__shown" />
        <circle v-if="at" v-bind="dot(minimap, at)" r="3" class="mosaic-page__stage" />
      </svg>
    </section>
  </div>
</template>

<style scoped>
.mosaic-page {
  display: flex;
  flex: 1;
  min-width: 0;
  min-height: 0;
}

.mosaic-page__title {
  margin: 0 0 var(--space-3);
}

.mosaic-page__tools {
  display: flex;
  flex-wrap: wrap;
  gap: var(--space-1);
  margin: var(--space-3) 0;
}

.mosaic-page__note {
  margin: var(--space-2) 0;
  color: var(--color-text-muted);
  font-size: var(--font-size-sm);
}

.mosaic-page__view {
  position: relative;
  flex: 1;
  min-width: 0;
}

.mosaic-page__svg {
  position: absolute;
  inset: 0;
  width: 100%;
  height: 100%;
  cursor: grab;
  touch-action: none;
}

.mosaic-page__field {
  fill: none;
  stroke: var(--color-success);
  stroke-width: 2;
}

.mosaic-page__minimap {
  position: absolute;
  right: var(--space-2);
  bottom: var(--space-2);
  border: var(--border-width) solid var(--color-border-strong);
  background: var(--color-surface-panel);
  cursor: pointer;
}

.mosaic-page__painted {
  fill: var(--color-accent-subtle);
  stroke: var(--color-accent);
}

.mosaic-page__shown {
  fill: none;
  stroke: var(--color-warning);
  stroke-width: 1.5;
}

.mosaic-page__stage {
  fill: var(--color-success);
}
</style>
