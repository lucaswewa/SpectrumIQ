<script setup lang="ts">
// The stage map (ADR-0057): the stage's travel, the frame where the stage is, the saved
// positions, the region drawn, the focus map's places, and the tiles planned, in their order;
// over the live mosaic, once something was painted (ADR-0060).
// The wheel zooms about the pointer; dragging pans, or draws with the tool chosen: a rectangle
// corner to corner, a polygon vertex by vertex (closed at its first), or places (a click on one
// removes it).
import { computed, onBeforeUnmount, onMounted, ref, useTemplateRef, watch } from 'vue'

import MosaicBackdrop from '@/mosaic/MosaicBackdrop.vue'
import { useMosaicStore } from '@/stores/mosaic'
import { useStageStore } from '@/stores/stage'
import { useWorkflowStore } from '@/stores/workflow'
import AppButton from '@/ui/AppButton.vue'

import { bounds, fit, pan, placed, svgPoints, toMap, toStage, zoom } from './geometry'
import type { Point, View } from './geometry'
import type { Region } from './plan'

export type Tool = 'pan' | 'rect' | 'polygon' | 'place'

const props = defineProps<{ tool: Tool; tiles?: { x: number; y: number }[]; frame?: Point[] }>()
const region = defineModel<Region>('region')
const places = defineModel<Point[]>('places', { default: () => [] })

const stage = useStageStore()
const workflow = useWorkflowStore()
const mosaic = useMosaicStore()
const overMosaic = computed(() => mosaic.fitted && !!mosaic.bounds)
const host = useTemplateRef<SVGSVGElement>('host')
const view = ref<View>({ centre: [0, 0], scale: 0.01, size: [800, 600] })

const travel = computed<[Point, Point] | undefined>(() => {
  const limits = stage.limits
  return (
    limits && [
      [limits.min.x, limits.min.y],
      [limits.max.x, limits.max.y],
    ]
  )
})
const corners = (region: Region): Point[] =>
  region.shape === 'rect'
    ? [
        [region.x0, region.y0],
        [region.x1, region.y0],
        [region.x1, region.y1],
        [region.x0, region.y1],
      ]
    : (region.points as Point[])

/** Shows the box `[low, high]` whole. */
function show(box: [Point, Point] | undefined) {
  if (box) view.value = fit(view.value.size, box)
}

// The map takes its element's size, and first shows the whole travel.
let observer: ResizeObserver | undefined
onMounted(() => {
  const measure = () => {
    const box = host.value?.getBoundingClientRect()
    if (box?.width) view.value = { ...view.value, size: [box.width, box.height] }
  }
  measure()
  show(travel.value)
  if (typeof ResizeObserver !== 'undefined') {
    observer = new ResizeObserver(measure)
    observer.observe(host.value!)
  }
})
onBeforeUnmount(() => observer?.disconnect())
watch(travel, (box, before) => before ?? show(box), { immediate: true })

/** Where `event` is: on the map, and on the stage. */
function pointer(event: MouseEvent): [Point, Point] {
  const box = host.value!.getBoundingClientRect()
  const at: Point = [event.clientX - box.left, event.clientY - box.top]
  return [at, toStage(view.value, at)]
}
const near = (a: Point, b: Point) => Math.hypot(a[0] - b[0], a[1] - b[1]) * view.value.scale < 8
let dragged: Point | undefined
/** The rectangle's corners, or the polygon's vertices, while drawn. */
const draft = ref<Point[]>([])

function down(event: PointerEvent) {
  const [at, point] = pointer(event)
  if (props.tool === 'pan' || event.button === 1) dragged = at
  else if (props.tool === 'rect') draft.value = [point, point]
  else if (props.tool === 'polygon') {
    const first = draft.value[0]
    if (first && draft.value.length >= 3 && near(first, point)) {
      region.value = { shape: 'polygon', points: draft.value }
      draft.value = []
    } else draft.value = [...draft.value, point]
  } else {
    const found = places.value.findIndex((place) => near(place, point))
    places.value = found < 0 ? [...places.value, point] : places.value.filter((_, i) => i !== found)
  }
}

function move(event: PointerEvent) {
  const [at, point] = pointer(event)
  if (dragged) {
    view.value = pan(view.value, [at[0] - dragged[0], at[1] - dragged[1]])
    dragged = at
  } else if (props.tool === 'rect' && draft.value.length === 2) {
    draft.value = [draft.value[0]!, point]
  }
}

function up() {
  dragged = undefined
  if (props.tool !== 'rect' || draft.value.length !== 2) return
  const [[x0, y0], [x1, y1]] = draft.value as [Point, Point]
  if (x0 !== x1 && y0 !== y1) {
    const [low, high] = bounds([
      [x0, y0],
      [x1, y1],
    ])
    region.value = { shape: 'rect', x0: low[0], y0: low[1], x1: high[0], y1: high[1] }
  }
  draft.value = []
}

function wheel(event: WheelEvent) {
  view.value = zoom(view.value, Math.exp(-event.deltaY / 500), pointer(event)[0])
}

// What's drawn, on the map.
const svg = (points: Point[]) => svgPoints(view.value, points)
const drafted = computed(() => {
  const [a, b] = draft.value
  return props.tool === 'rect' && a && b
    ? corners({ shape: 'rect', x0: a[0], y0: a[1], x1: b[0], y1: b[1] })
    : draft.value
})
const at = computed<Point | undefined>(() => stage.position && [stage.position.x, stage.position.y])
const field = computed(
  () => workflow.field && at.value && placed(workflow.field as Point[], at.value),
)
const frames = computed(() =>
  props.frame ? (props.tiles ?? []).map((tile) => placed(props.frame!, [tile.x, tile.y])) : [],
)
const dot = (point: Point) => {
  const [cx, cy] = toMap(view.value, point)
  return { cx, cy }
}
</script>

<template>
  <div class="stage-map" :class="{ 'stage-map--mosaic': overMosaic }">
    <MosaicBackdrop v-if="overMosaic" :view />
    <svg
      ref="host"
      class="stage-map__svg"
      role="img"
      aria-label="The stage map"
      :data-tool="tool"
      @pointerdown="down"
      @pointermove="move"
      @pointerup="up"
      @dblclick.prevent
      @wheel.prevent="wheel"
    >
      <polygon
        v-if="travel"
        :points="
          svg(
            corners({
              shape: 'rect',
              x0: travel[0][0],
              y0: travel[0][1],
              x1: travel[1][0],
              y1: travel[1][1],
            }),
          )
        "
        class="stage-map__travel"
      />
      <polygon
        v-for="(outline, i) in frames"
        :key="i"
        :points="svg(outline)"
        class="stage-map__tile"
      />
      <polyline
        v-if="tiles?.length"
        :points="svg(tiles.map((t) => [t.x, t.y]))"
        class="stage-map__path"
      />
      <polygon v-if="region" :points="svg(corners(region))" class="stage-map__region" />
      <polyline v-if="draft.length" :points="svg(drafted)" class="stage-map__draft" />
      <circle
        v-for="(place, i) in places"
        :key="`p${i}`"
        v-bind="dot(place)"
        r="5"
        class="stage-map__place"
      />
      <g v-for="position in workflow.positions ?? []" :key="position.name">
        <circle v-bind="dot([position.x, position.y])" r="3" class="stage-map__position" />
        <text
          :x="dot([position.x, position.y]).cx + 6"
          :y="dot([position.x, position.y]).cy"
          class="stage-map__label"
        >
          {{ position.name }}
        </text>
      </g>
      <polygon v-if="field" :points="svg(field)" class="stage-map__field" />
      <circle v-else-if="at" v-bind="dot(at)" r="4" class="stage-map__field" />
    </svg>
    <div class="stage-map__tools">
      <AppButton size="sm" :disabled="!travel" @click="show(travel)">Whole travel</AppButton>
      <AppButton size="sm" :disabled="!region" @click="show(region && bounds(corners(region)))">
        The region
      </AppButton>
    </div>
  </div>
</template>

<style scoped>
.stage-map {
  position: relative;
  flex: 1;
  min-height: 0;
}

.stage-map__svg {
  width: 100%;
  height: 100%;
  background: var(--color-surface-sunken);
  cursor: crosshair;
  touch-action: none;
}

/* Over the live mosaic, the map is see-through. */
.stage-map--mosaic .stage-map__svg {
  position: relative;
  background: none;
}

.stage-map--mosaic .stage-map__travel {
  fill: none;
}

.stage-map__svg[data-tool='pan'] {
  cursor: grab;
}

.stage-map__tools {
  position: absolute;
  top: var(--space-2);
  right: var(--space-2);
  display: flex;
  gap: var(--space-1);
}

.stage-map__travel {
  fill: var(--color-bg);
  stroke: var(--color-border-strong);
}

.stage-map__tile {
  fill: var(--color-accent-subtle);
  fill-opacity: 0.5;
  stroke: var(--color-accent);
  stroke-width: 0.5;
}

.stage-map__path {
  fill: none;
  stroke: var(--color-accent);
  stroke-dasharray: 2 3;
}

.stage-map__region,
.stage-map__draft {
  fill: none;
  stroke: var(--color-warning);
  stroke-width: 2;
}

.stage-map__draft {
  stroke-dasharray: 4 3;
}

.stage-map__place {
  fill: var(--color-info);
}

.stage-map__position {
  fill: var(--color-text-muted);
}

.stage-map__label {
  fill: var(--color-text-muted);
  font-size: var(--font-size-sm);
}

.stage-map__field {
  fill: none;
  stroke: var(--color-success);
  stroke-width: 2;
}
</style>
