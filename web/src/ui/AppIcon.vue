<script setup lang="ts">
// One Material Symbols icon (ADR-0012), drawn in the current text colour.
// Icons are imported as raw SVG text, for example:
//   import settings from '@material-symbols/svg-400/outlined/settings.svg?raw'
// so only the icons the app uses are bundled.
import { computed } from 'vue'

const props = withDefaults(
  defineProps<{
    /** The icon's SVG source. */
    svg: string
    /** Its width and height, as a CSS length. */
    size?: string
  }>(),
  { size: 'var(--icon-size-md)' },
)

const viewBox = computed(() => /viewBox="([^"]+)"/.exec(props.svg)?.[1] ?? '0 0 24 24')
const paths = computed(() =>
  [...props.svg.matchAll(/<path[^>]*\sd="([^"]+)"/g)].map((match) => match[1]),
)
</script>

<template>
  <svg
    class="app-icon"
    :viewBox="viewBox"
    :style="{ width: size, height: size }"
    fill="currentColor"
    aria-hidden="true"
    focusable="false"
  >
    <path v-for="(d, index) in paths" :key="index" :d="d" />
  </svg>
</template>

<style scoped>
.app-icon {
  display: block;
  flex: none;
}
</style>
