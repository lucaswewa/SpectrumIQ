<script setup lang="ts">
// The focus measure, now and over the last minute or so: a line of its recent values, scaled
// to their range, so that turning the focus shows whether it is getting sharper.
import { computed, ref, watch } from 'vue'

const props = defineProps<{ value?: number | null }>()

/** How many values the line keeps: at 2 a second, half a minute. */
const KEEP = 60
const recent = ref<number[]>([])
watch(
  () => props.value,
  (value) => {
    if (typeof value === 'number') recent.value = [...recent.value, value].slice(-KEEP)
  },
  { immediate: true },
)

const points = computed(() => {
  const values = recent.value
  if (values.length < 2) return ''
  const [low, high] = [Math.min(...values), Math.max(...values)]
  const span = high - low || 1
  return values
    .map((value, index) => `${(index / (KEEP - 1)) * 100},${30 - ((value - low) / span) * 28 - 1}`)
    .join(' ')
})

const shown = computed(() => (typeof props.value === 'number' ? props.value.toPrecision(4) : '…'))
</script>

<template>
  <div class="focus-sparkline">
    <span class="focus-sparkline__value" aria-live="off">{{ shown }}</span>
    <svg
      viewBox="0 0 100 30"
      preserveAspectRatio="none"
      class="focus-sparkline__line"
      aria-hidden="true"
    >
      <polyline :points="points" />
    </svg>
  </div>
</template>

<style scoped>
.focus-sparkline {
  display: flex;
  gap: var(--space-2);
  align-items: center;
}

.focus-sparkline__value {
  min-width: 6ch;
  font-variant-numeric: tabular-nums;
}

.focus-sparkline__line {
  flex: 1;
  height: 30px;
}

.focus-sparkline__line polyline {
  fill: none;
  stroke: var(--color-accent);
  stroke-width: 1.5;
  vector-effect: non-scaling-stroke;
}
</style>
