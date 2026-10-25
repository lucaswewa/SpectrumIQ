<script setup lang="ts">
// A turning ring in the current text colour, for work of unknown length.
// With a `label` it is announced as a busy indicator; without one it is
// decoration, for example beside a button's own text.
withDefaults(defineProps<{ label?: string; size?: string }>(), {
  label: undefined,
  size: 'var(--icon-size-md)',
})
</script>

<template>
  <svg
    class="app-spinner"
    viewBox="0 0 24 24"
    :style="{ width: size, height: size }"
    :role="label ? 'progressbar' : undefined"
    :aria-label="label"
    :aria-hidden="label ? undefined : 'true'"
    focusable="false"
  >
    <circle class="app-spinner__track" cx="12" cy="12" r="9" />
    <circle class="app-spinner__arc" cx="12" cy="12" r="9" pathLength="100" />
  </svg>
</template>

<style scoped>
.app-spinner {
  display: block;
  flex: none;
  animation: app-spinner-turn 0.8s linear infinite;
}

.app-spinner circle {
  fill: none;
  stroke: currentcolor;
  stroke-width: 3;
}

.app-spinner__track {
  opacity: 0.2;
}

.app-spinner__arc {
  stroke-dasharray: 30 100;
  stroke-linecap: round;
}

@keyframes app-spinner-turn {
  to {
    transform: rotate(1turn);
  }
}
</style>
