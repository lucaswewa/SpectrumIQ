<script setup lang="ts">
// Overlay layers drawn over an image (ADR-0009, amended), in one SVG the size of the
// element below it. Each shape is placed on screen from its layer's pixels
// by `placement`, so lines and labels stay the same size at any zoom. Each
// is drawn over a dark halo, so it shows on light and dark images. A shape's
// title shows when the pointer rests on its line or label; elsewhere the
// pointer goes through to the image.
import { computed } from 'vue'

import { toScreen, type OverlayLayer, type Placement } from './types'

const props = defineProps<{
  /** The layers shown, the first drawn first. */
  layers: OverlayLayer[]
  /** Where the image is; nothing is drawn until it's known. */
  placement: Placement | undefined
}>()

/** How far a rectangle's label sits inside its corner, in screen pixels. */
const INSET = [4, 14]

const drawn = computed(() => {
  const placement = props.placement
  if (!placement) return []
  return props.layers.map((layer) => {
    const at = (x: number, y: number) => toScreen([x, y], layer.size, placement)
    const shapes = layer.shapes.map((shape) => {
      switch (shape.kind) {
        case 'rect': {
          const [x, y] = at(shape.x, shape.y)
          const [right, bottom] = at(shape.x + shape.width, shape.y + shape.height)
          return { ...shape, x, y, width: right - x, height: bottom - y }
        }
        case 'line':
          return { ...shape, at: shape.points.map(([x, y]) => at(x, y).join(',')).join(' ') }
        default: {
          const [x, y] = at(shape.x, shape.y)
          return { ...shape, x, y }
        }
      }
    })
    return { id: layer.id, shapes }
  })
})
</script>

<template>
  <svg v-if="placement" class="overlay-layers" aria-hidden="true">
    <g v-for="layer in drawn" :key="layer.id" :data-layer="layer.id">
      <g v-for="(shape, index) in layer.shapes" :key="index">
        <title v-if="shape.title">{{ shape.title }}</title>
        <template v-if="shape.kind === 'rect'">
          <rect
            v-for="part in ['halo', 'stroke']"
            :key="part"
            :class="[`overlay-layers__${part}`, { 'overlay-layers__dashed': shape.dashed }]"
            :x="shape.x"
            :y="shape.y"
            :width="shape.width"
            :height="shape.height"
          />
          <text
            v-if="shape.label"
            class="overlay-layers__text"
            :x="shape.x + INSET[0]!"
            :y="shape.y + INSET[1]!"
          >
            {{ shape.label }}
          </text>
        </template>
        <template v-else-if="shape.kind === 'line'">
          <polyline
            v-for="part in ['halo', 'stroke']"
            :key="part"
            :class="[`overlay-layers__${part}`, { 'overlay-layers__dashed': shape.dashed }]"
            :points="shape.at"
          />
        </template>
        <template v-else-if="shape.kind === 'point'">
          <circle
            v-for="part in ['halo', 'stroke']"
            :key="part"
            :class="`overlay-layers__${part}`"
            :cx="shape.x"
            :cy="shape.y"
            r="4"
          />
        </template>
        <text v-else class="overlay-layers__text" :x="shape.x" :y="shape.y">{{ shape.text }}</text>
      </g>
    </g>
  </svg>
</template>

<style scoped>
.overlay-layers {
  position: absolute;
  inset: 0;
  width: 100%;
  height: 100%;
  overflow: hidden;
  pointer-events: none;
}

.overlay-layers__halo,
.overlay-layers__stroke {
  fill: none;
}

.overlay-layers__halo {
  stroke: var(--color-annotation-halo);
  stroke-width: var(--annotation-halo-width);
}

.overlay-layers__stroke {
  stroke: var(--color-annotation);
  stroke-width: var(--annotation-stroke-width);
  pointer-events: visibleStroke;
}

.overlay-layers__stroke.overlay-layers__dashed {
  stroke-dasharray: 6 4;
}

.overlay-layers__text {
  font-family: var(--font-family-sans);
  font-size: var(--font-size-sm);
  font-weight: var(--font-weight-semibold);
  fill: var(--color-annotation);
  stroke: var(--color-annotation-halo);
  stroke-width: var(--annotation-halo-width);
  paint-order: stroke;
  pointer-events: visiblePainted;
}
</style>
